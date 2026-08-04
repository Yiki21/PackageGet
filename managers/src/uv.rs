use std::{
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

use async_trait::async_trait;
use tokio::time::timeout;
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId,
    ManagerResult, PackageAction, PackageInfo, PackageManager, PackageOrigin, PackageScope,
    PackageTarget, PackageUpdate, Platform, ProgressEvent, ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, command_status_error, manager_availability, resolve_executable, run_output,
    },
    progress::{CommandProgress, run_cancellable_command_with_progress, run_command_with_progress},
};

const UV_ID: &str = "builtin:uv";
const UV_COMMAND: &str = "uv";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

/// Direct `updater-manager-api` implementation for `uv tool` applications.
#[derive(Debug, Clone)]
pub struct UvManager {
    descriptor: ManagerDescriptor,
}

impl UvManager {
    /// Creates the built-in `uv tool` manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(UV_ID).expect("uv manager ID must remain valid"),
            "uv tool",
            ManagerCategory::Development,
            SupportedPlatforms::from([Platform::Linux, Platform::Windows, Platform::MacOs]),
            ManagerCapabilities::from([
                ManagerCapability::Installed,
                ManagerCapability::Updates,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ]),
        )
        .expect("uv descriptor must remain valid")
        .with_description("Python applications installed with uv")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }

    /// Executes one validated target with bounded command progress.
    ///
    /// # Errors
    ///
    /// Returns a typed target-validation, timeout, or command error.
    #[allow(dead_code)]
    async fn execute_target_with_progress(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
        on_progress: impl FnMut(CommandProgress),
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        let command = self.write_command(config, action, target)?;
        timeout(
            COMMAND_TIMEOUT,
            run_command_with_progress(&command, on_progress),
        )
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, "uv tool write command timed out")
                .with_detail(command.program().to_string_lossy())
        })?
    }

    async fn tool_root(&self, config: &ManagerConfig) -> ManagerResult<PathBuf> {
        self.validate_config(config)?;
        let uv = resolve_executable(config, UV_COMMAND);
        let output = run_success(
            &uv_command(&uv).args(["tool", "dir", "--color", "never"]),
            "uv tool directory query timed out",
        )
        .await?;
        let value = decode_utf8(&output.stdout, "uv tool directory is not valid UTF-8")?;
        let path = value.trim();
        if path.is_empty() || value.lines().count() != 1 {
            return Err(protocol("uv tool directory response is malformed", &value));
        }
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(protocol(
                "uv tool directory must be an absolute path",
                &path.display().to_string(),
            ));
        }
        canonical_directory(&path, "uv tool root").await
    }

    async fn list_tools(
        &self,
        config: &ManagerConfig,
        outdated: bool,
    ) -> ManagerResult<Vec<InstalledUvTool>> {
        let root = self.tool_root(config).await?;
        let uv = resolve_executable(config, UV_COMMAND);
        let mut spec = uv_command(&uv).args(["tool", "list"]);
        if outdated {
            spec = spec.arg("--outdated");
        }
        let spec = spec.args(["--show-paths", "--color", "never"]);
        let output = run_success(&spec, "uv tool listing timed out").await?;
        let value = decode_utf8(&output.stdout, "uv tool listing is not valid UTF-8")?;
        parse_tool_list(&value, &root, outdated).await
    }

    fn write_command(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
    ) -> ManagerResult<CommandSpec> {
        ensure_supported_action(action)?;
        if &target.manager_id != self.descriptor.id() {
            return Err(protocol(
                "uv tool target belongs to another manager",
                &target.name,
            ));
        }
        validate_name(&target.name)?;
        if target.scope != PackageScope::Unknown || target.origin.is_some() {
            if target.scope != PackageScope::User {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "uv tool target scope is not supported",
                )
                .with_detail(&target.name));
            }
            let origin = target.origin.as_ref().ok_or_else(|| {
                protocol("uv tool target is missing its typed origin", &target.name)
            })?;
            validate_origin(origin, &target.name)?;
        }
        let uv = resolve_executable(config, UV_COMMAND);
        match action {
            PackageAction::Install => {
                let requirement = target.version.as_deref().map_or_else(
                    || target.name.clone(),
                    |version| format!("{}=={version}", target.name),
                );
                if let Some(version) = target.version.as_deref() {
                    validate_version(version)?;
                }
                Ok(uv_command(&uv).args(["tool", "install", requirement.as_str()]))
            }
            PackageAction::Update | PackageAction::Uninstall => {
                if target.version.is_some() {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "version-pinned uv tool operations are not supported",
                    )
                    .with_detail(&target.name));
                }
                let verb = if action == PackageAction::Update {
                    "upgrade"
                } else {
                    "uninstall"
                };
                Ok(uv_command(&uv).args(["tool", verb, target.name.as_str()]))
            }
            _ => unreachable!("supported actions were checked above"),
        }
    }
}

impl Default for UvManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for UvManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(config, UV_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        Ok(self
            .list_tools(config, false)
            .await?
            .into_iter()
            .map(|tool| tool.info(self.descriptor.id()))
            .collect())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.list_tools(config, false).await?.len())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.list_tools(config, true)
            .await?
            .into_iter()
            .map(|tool| tool.update(self.descriptor.id()))
            .collect()
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "uv tool search is not advertised because index-aware lookup is unavailable",
        )
        .with_detail(query))
    }

    async fn execute(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        packages: &[PackageTarget],
        progress: &dyn ProgressSink,
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        ensure_supported_action(action)?;
        let commands = packages
            .iter()
            .map(|target| self.write_command(config, action, target))
            .collect::<ManagerResult<Vec<_>>>()?;
        let total = packages.len();
        progress.emit(ProgressEvent::Started { action, total });
        for (index, (target, command)) in packages.iter().zip(&commands).enumerate() {
            timeout(
                COMMAND_TIMEOUT,
                run_cancellable_command_with_progress(command, progress, |event| {
                    let (fraction, message) = event.into_parts();
                    if let Some(message) = message {
                        progress.emit(ProgressEvent::Message { message });
                    }
                    progress.emit(ProgressEvent::Advanced {
                        completed: index + usize::from(fraction >= 1.0),
                        total,
                        current_package: Some(target.name.clone()),
                    });
                }),
            )
            .await
            .map_err(|_| {
                ManagerError::new(ManagerErrorKind::Timeout, "uv tool write command timed out")
                    .with_detail(command.program().to_string_lossy())
            })??;
        }
        progress.emit(ProgressEvent::Finished {
            completed: total,
            total,
        });
        Ok(())
    }
}

#[derive(Debug)]
struct InstalledUvTool {
    name: String,
    version: String,
    latest: Option<String>,
    size: u64,
}

impl InstalledUvTool {
    fn target(&self, manager_id: &ManagerId) -> PackageTarget {
        let mut target = PackageTarget::new(manager_id.clone(), &self.name);
        target.scope = PackageScope::User;
        target.origin = Some(tool_origin(&self.name));
        target
    }

    fn info(self, manager_id: &ManagerId) -> PackageInfo {
        let mut info = PackageInfo::new(manager_id.clone(), &self.name, self.version);
        info.scope = PackageScope::User;
        info.origin = Some(tool_origin(&self.name));
        info.size = Some(self.size);
        info
    }

    fn update(self, manager_id: &ManagerId) -> ManagerResult<PackageUpdate> {
        let target = self.target(manager_id);
        let latest = self.latest.ok_or_else(|| {
            protocol(
                "uv outdated entry is missing its latest version",
                &self.name,
            )
        })?;
        Ok(PackageUpdate::new(target, self.version, latest))
    }
}

async fn parse_tool_list(
    value: &str,
    root: &Path,
    require_latest: bool,
) -> ManagerResult<Vec<InstalledUvTool>> {
    let mut tools = Vec::new();
    for line in value.lines().filter(|line| !line.trim().is_empty()) {
        if line.starts_with("- ") || line.starts_with("  - ") {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            return Err(protocol(
                "uv tool listing contains an unexpected line",
                line,
            ));
        }
        let (metadata, path) = line
            .strip_suffix(')')
            .and_then(|line| line.rsplit_once(" ("))
            .ok_or_else(|| protocol("uv tool listing header is malformed", line))?;
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(protocol("uv tool environment path is not absolute", line));
        }
        let canonical = canonical_directory(&path, "uv tool environment").await?;
        if canonical.parent() != Some(root) {
            return Err(ManagerError::new(
                ManagerErrorKind::Permission,
                "uv tool environment escapes the configured tool root",
            )
            .with_detail(canonical.display().to_string()));
        }
        let (installed, latest) = match metadata.strip_suffix(']') {
            Some(metadata) => {
                let (installed, latest) = metadata
                    .rsplit_once(" [latest: ")
                    .ok_or_else(|| protocol("uv tool latest version is malformed", line))?;
                (installed, Some(latest.to_owned()))
            }
            None => (metadata, None),
        };
        if require_latest != latest.is_some() {
            return Err(protocol(
                "uv tool latest version contract is malformed",
                line,
            ));
        }
        let (name, version) = installed
            .split_once(" v")
            .ok_or_else(|| protocol("uv tool name and version are malformed", line))?;
        validate_name(name)?;
        validate_version(version)?;
        if let Some(latest) = latest.as_deref() {
            validate_version(latest)?;
        }
        let expected = root.join(name);
        let expected = canonical_directory(&expected, "uv tool environment").await?;
        if canonical != expected {
            return Err(protocol(
                "uv tool environment path does not match its identity",
                line,
            ));
        }
        tools.push(InstalledUvTool {
            name: name.to_owned(),
            version: version.to_owned(),
            latest,
            size: strict_directory_size(&canonical).await?,
        });
    }
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(tools)
}

fn tool_origin(name: &str) -> PackageOrigin {
    PackageOrigin::new("uv tool").with_reference(format!("tool:{name}"))
}

fn validate_origin(origin: &PackageOrigin, name: &str) -> ManagerResult<()> {
    if origin.name == "uv tool" && origin.reference.as_deref() == Some(&format!("tool:{name}")) {
        Ok(())
    } else {
        Err(protocol("uv tool origin is malformed", name))
    }
}

fn validate_name(value: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value.starts_with(['-', '.'])
        || value.ends_with('.')
        || value.chars().any(char::is_whitespace)
        || value.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
        })
        || value.contains([';', '=', '/', '\\'])
    {
        return Err(protocol("uv tool name is malformed", value));
    }
    Ok(())
}

fn validate_version(value: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value.chars().any(char::is_whitespace)
        || value.contains(['/', '\\', ';', '='])
    {
        return Err(protocol("uv tool version is malformed", value));
    }
    Ok(())
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "uv tool package action is unsupported",
        )),
    }
}

fn uv_command(path: &Path) -> CommandSpec {
    CommandSpec::new(path).env("NO_COLOR", "1")
}

async fn run_success(spec: &CommandSpec, timeout_message: &str) -> ManagerResult<Output> {
    let output = timeout(COMMAND_TIMEOUT, run_output(spec))
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, timeout_message)
                .with_detail(spec.program().to_string_lossy())
        })??;
    if output.status.success() {
        return Ok(output);
    }
    Err(command_status_error(
        spec,
        output.status,
        &format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    ))
}

async fn canonical_directory(path: &Path, kind: &str) -> ManagerResult<PathBuf> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| fs_error(&format!("failed to inspect {kind}"), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            format!("{kind} is not a regular directory"),
        )
        .with_detail(path.display().to_string()));
    }
    tokio::fs::canonicalize(path)
        .await
        .map_err(|error| fs_error(&format!("failed to resolve {kind}"), error))
}

async fn strict_directory_size(root: &Path) -> ManagerResult<u64> {
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut entries = tokio::fs::read_dir(directory)
            .await
            .map_err(|error| fs_error("failed to read uv tool directory", error))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| fs_error("failed to read uv tool directory entry", error))?
        {
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| fs_error("failed to inspect uv tool directory entry", error))?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                total = total
                    .checked_add(
                        entry
                            .metadata()
                            .await
                            .map_err(|error| fs_error("failed to inspect uv tool file", error))?
                            .len(),
                    )
                    .ok_or_else(|| {
                        ManagerError::new(
                            ManagerErrorKind::Other,
                            "uv tool size exceeds the supported range",
                        )
                        .with_detail(root.display().to_string())
                    })?;
            }
        }
    }
    Ok(total)
}

fn decode_utf8(bytes: &[u8], message: &str) -> ManagerResult<String> {
    String::from_utf8(bytes.to_vec()).map_err(|error| protocol(message, &error.to_string()))
}

fn protocol(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(detail)
}

fn fs_error(message: &str, error: std::io::Error) -> ManagerError {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied => ManagerErrorKind::Permission,
        std::io::ErrorKind::TimedOut => ManagerErrorKind::Timeout,
        _ => ManagerErrorKind::Other,
    };
    ManagerError::new(kind, message).with_detail(error.to_string())
}
