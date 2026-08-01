use std::{
    env,
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

use async_trait::async_trait;
use semver::Version;
use serde::Deserialize;
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
    progress::{CommandProgress, run_command_with_progress},
};

const GO_ID: &str = "builtin:go";
const GO_COMMAND: &str = "go";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

/// Direct `updater-manager-api` implementation for Go-installed binaries.
#[derive(Debug, Clone)]
pub struct GoManager {
    descriptor: ManagerDescriptor,
}

impl GoManager {
    /// Creates the built-in Go manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(GO_ID).expect("Go manager ID must remain valid"),
            "Go",
            ManagerCategory::Development,
            SupportedPlatforms::from([Platform::Linux, Platform::Windows, Platform::MacOs]),
            ManagerCapabilities::from([
                ManagerCapability::Installed,
                ManagerCapability::Updates,
                ManagerCapability::Search,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ]),
        )
        .expect("Go descriptor must remain valid")
        .with_description("Go module binary package manager")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }

    /// Returns the installed version for one unambiguous binary or module.
    ///
    /// # Errors
    ///
    /// Propagates GOBIN, filesystem, command, and build-info errors. A missing
    /// or ambiguous identity is reported as a protocol error.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        identity: &str,
    ) -> ManagerResult<String> {
        let matches = self
            .installed_binaries(config)
            .await?
            .into_iter()
            .filter(|binary| binary.name == identity || binary.module == identity)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [binary] => Ok(binary.version.clone()),
            [] => Err(protocol("Go package version is unavailable", identity)),
            _ => Err(protocol("Go package version is ambiguous", identity)),
        }
    }

    /// Executes one target and reports normalized command or removal progress.
    ///
    /// # Errors
    ///
    /// Returns a typed configuration, target, command, filesystem, or path
    /// containment error.
    pub async fn execute_target_with_progress(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
        mut on_progress: impl FnMut(CommandProgress),
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        self.validate_target(target)?;
        match action {
            PackageAction::Install | PackageAction::Update => {
                let command = self.install_command(config, target).await?;
                run_command_with_progress(&command, on_progress).await
            }
            PackageAction::Uninstall => {
                on_progress(CommandProgress::new(
                    0.0,
                    Some(format!("Removing Go binary {}", target.name)),
                ));
                self.remove_binary(config, target).await?;
                on_progress(CommandProgress::new(
                    1.0,
                    Some(format!("Removed {}", target.name)),
                ));
                Ok(())
            }
            _ => Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "Go package action is unsupported",
            )),
        }
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            Ok(())
        } else {
            Err(protocol(
                "Go configuration ID does not match the manager",
                &format!("expected {}, received {}", self.descriptor.id(), config.id),
            ))
        }
    }

    fn settings(config: &ManagerConfig) -> ManagerResult<GoSettings> {
        serde_json::from_value(config.settings.clone())
            .map_err(|error| protocol("Go manager settings are invalid", &error.to_string()))
    }

    async fn bin_dir(&self, config: &ManagerConfig) -> ManagerResult<PathBuf> {
        let settings = Self::settings(config)?;
        if let Some(path) = settings.go_bin_dir {
            return validate_bin_dir_value(path, "go_bin_dir");
        }
        let go = resolve_executable(config, GO_COMMAND);
        let environment = go_environment(&go).await?;
        if !environment.gobin.is_empty() {
            return validate_bin_dir_value(PathBuf::from(environment.gobin), "GOBIN");
        }
        let first = env::split_paths(&environment.gopath)
            .next()
            .ok_or_else(|| protocol("Go environment did not provide GOBIN or GOPATH", "GOPATH"))?;
        validate_bin_dir_value(first.join("bin"), "GOPATH/bin")
    }

    async fn installed_binaries(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<Vec<InstalledGoBinary>> {
        self.validate_config(config)?;
        let bin_dir = self.bin_dir(config).await?;
        let mut entries = match tokio::fs::read_dir(&bin_dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(fs_error("failed to read Go binary directory", error)),
        };
        let mut paths = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            fs_error(
                "failed to read an entry from the Go binary directory",
                error,
            )
        })? {
            let file_type = entry.file_type().await.map_err(|error| {
                fs_error("failed to inspect a Go binary directory entry", error)
            })?;
            if file_type.is_file() {
                paths.push(entry.path());
            }
        }
        paths.sort();
        let go = resolve_executable(config, GO_COMMAND);
        let platform = Platform::current().ok_or_else(|| {
            ManagerError::new(
                ManagerErrorKind::Unsupported,
                "Go inventory is unsupported on this platform",
            )
        })?;
        let mut binaries = Vec::with_capacity(paths.len());
        for path in paths {
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    protocol(
                        "Go binary name is not valid UTF-8",
                        &path.display().to_string(),
                    )
                })?
                .to_owned();
            let name = logical_binary_name(&file_name, platform)?;
            let spec = go_command(&go).args([
                "version".as_ref(),
                "-m".as_ref(),
                "-json".as_ref(),
                path.as_os_str(),
            ]);
            let Some(output) = run_build_probe(&spec).await? else {
                continue;
            };
            let build =
                decode_json::<GoBuildInfo>(&output.stdout, "Go build-info response is invalid")?
                    .validated()?;
            let size = tokio::fs::metadata(&path)
                .await
                .map_err(|error| fs_error("failed to inspect a Go binary", error))?
                .len();
            binaries.push(InstalledGoBinary {
                name,
                file_name,
                module: build.module,
                package: build.package,
                version: build.version,
                size,
                updateable: build.updateable,
            });
        }
        Ok(binaries)
    }

    async fn versions(
        &self,
        config: &ManagerConfig,
        module: &str,
    ) -> ManagerResult<GoModuleVersions> {
        validate_go_path(module, "Go module path is malformed")?;
        let go = resolve_executable(config, GO_COMMAND);
        let spec = go_command(&go).args(["list", "-m", "-versions", "-json", module]);
        let output = run_success(&spec, "Go module version query timed out").await?;
        decode_json::<GoModuleVersions>(&output.stdout, "Go module version response is invalid")?
            .validated(module)
    }

    async fn latest_version(&self, config: &ManagerConfig, module: &str) -> ManagerResult<String> {
        validate_go_path(module, "Go module path is malformed")?;
        let go = resolve_executable(config, GO_COMMAND);
        let query = format!("{module}@latest");
        let spec = go_command(&go).args(["list", "-m", "-json", query.as_str()]);
        let output = run_success(&spec, "Go latest module query timed out").await?;
        decode_json::<GoModuleMetadata>(&output.stdout, "Go latest module response is invalid")?
            .validated_version(module)
    }

    fn validate_target(&self, target: &PackageTarget) -> ManagerResult<()> {
        if &target.manager_id != self.descriptor.id() {
            return Err(protocol(
                "Go package target belongs to another manager",
                &target.name,
            ));
        }
        validate_binary_or_module_name(&target.name)
    }

    async fn install_command(
        &self,
        config: &ManagerConfig,
        target: &PackageTarget,
    ) -> ManagerResult<CommandSpec> {
        let package = if target.scope == PackageScope::Unknown
            && target.origin.is_none()
            && !target.name.contains('/')
        {
            let matches = self
                .installed_binaries(config)
                .await?
                .into_iter()
                .filter(|binary| binary.name == target.name)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [installed] => installed.package.clone(),
                [] => {
                    return Err(protocol(
                        "legacy Go binary target cannot be resolved to an install package",
                        &target.name,
                    ));
                }
                _ => {
                    return Err(protocol(
                        "legacy Go binary target is ambiguous",
                        &target.name,
                    ));
                }
            }
        } else {
            target_install_package(target)?
        };
        let version = target.version.as_deref().unwrap_or("latest");
        validate_version(version)?;
        let argument = format!("{package}@{version}");
        let go = resolve_executable(config, GO_COMMAND);
        let bin_dir = self.bin_dir(config).await?;
        Ok(go_command(&go)
            .args(["install", argument.as_str()])
            .env("GOBIN", bin_dir.as_os_str()))
    }

    async fn remove_binary(
        &self,
        config: &ManagerConfig,
        target: &PackageTarget,
    ) -> ManagerResult<()> {
        if target.version.is_some() {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "version-pinned Go uninstall targets are not supported",
            )
            .with_detail(&target.name));
        }
        validate_binary_name(&target.name)?;
        let bin_dir = self.bin_dir(config).await?;
        let inventory = self.installed_binaries(config).await?;
        let matches = inventory
            .iter()
            .filter(|binary| binary.name == target.name)
            .collect::<Vec<_>>();
        let installed = match matches.as_slice() {
            [installed] => *installed,
            [] => {
                return Err(protocol(
                    "Go uninstall target is not installed",
                    &target.name,
                ));
            }
            _ => return Err(protocol("Go uninstall target is ambiguous", &target.name)),
        };
        if target.scope == PackageScope::User {
            let requested = GoOrigin::from_origin(target.origin.as_ref().ok_or_else(|| {
                protocol("scoped Go target is missing its typed origin", &target.name)
            })?)?;
            if requested.module != installed.module || requested.package != installed.package {
                return Err(protocol(
                    "Go uninstall target origin does not match installed build-info",
                    &target.name,
                ));
            }
        } else if target.scope != PackageScope::Unknown || target.origin.is_some() {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "Go uninstall target scope is not supported",
            )
            .with_detail(&target.name));
        }
        let canonical_bin = tokio::fs::canonicalize(&bin_dir)
            .await
            .map_err(|error| fs_error("failed to resolve the Go binary directory", error))?;
        let candidate = bin_dir.join(&installed.file_name);
        let metadata = tokio::fs::symlink_metadata(&candidate)
            .await
            .map_err(|error| fs_error("failed to inspect the Go binary removal target", error))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "Go binary removal target is not a regular file",
            )
            .with_detail(candidate.display().to_string()));
        }
        let canonical_target = tokio::fs::canonicalize(&candidate)
            .await
            .map_err(|error| fs_error("failed to resolve the Go binary removal target", error))?;
        if canonical_target.parent() != Some(canonical_bin.as_path()) {
            return Err(ManagerError::new(
                ManagerErrorKind::Permission,
                "Go binary removal target escapes the configured GOBIN",
            )
            .with_detail(canonical_target.display().to_string()));
        }
        tokio::fs::remove_file(canonical_target)
            .await
            .map_err(|error| fs_error("failed to remove the Go binary", error))
    }
}

impl Default for GoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for GoManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(config, GO_COMMAND, &["version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        Ok(self
            .installed_binaries(config)
            .await?
            .into_iter()
            .map(|binary| binary.info(self.descriptor.id()))
            .collect())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.installed_binaries(config).await?.len())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        let binaries = self.installed_binaries(config).await?;
        let mut updates = Vec::new();
        for binary in binaries {
            if !binary.updateable {
                continue;
            }
            let available = self.latest_version(config, &binary.module).await?;
            if parse_semver(&available)? > parse_semver(&binary.version)? {
                updates.push(PackageUpdate::new(
                    binary.target(self.descriptor.id()),
                    binary.version,
                    available,
                ));
            }
        }
        Ok(updates)
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let versions = self.versions(config, query).await?;
        let installed = self
            .installed_binaries(config)
            .await?
            .into_iter()
            .find(|binary| binary.module == query);
        let (name, version) = installed.map_or_else(
            || (query.to_owned(), NOT_INSTALLED_VERSION.to_owned()),
            |binary| (binary.name, binary.version),
        );
        let mut info = PackageInfo::new(self.descriptor.id().clone(), name, version);
        info.scope = PackageScope::User;
        info.origin = Some(GoOrigin::new(query, query)?.origin());
        info.description = Some(format!("published versions: {}", versions.versions.len()));
        Ok(vec![info])
    }

    async fn execute(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        packages: &[PackageTarget],
        progress: &dyn ProgressSink,
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        for target in packages {
            self.validate_target(target)?;
        }
        let total = packages.len();
        progress.emit(ProgressEvent::Started { action, total });
        for (index, target) in packages.iter().enumerate() {
            self.execute_target_with_progress(config, action, target, |event| {
                let (fraction, message) = event.into_parts();
                if let Some(message) = message {
                    progress.emit(ProgressEvent::Message { message });
                }
                progress.emit(ProgressEvent::Advanced {
                    completed: index + usize::from(fraction >= 1.0),
                    total,
                    current_package: Some(target.name.clone()),
                });
            })
            .await?;
        }
        progress.emit(ProgressEvent::Finished {
            completed: total,
            total,
        });
        Ok(())
    }
}

#[derive(Debug, Default, Deserialize)]
struct GoSettings {
    #[serde(default)]
    go_bin_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GoOrigin {
    module: String,
    package: String,
}

impl GoOrigin {
    fn new(module: &str, package: &str) -> ManagerResult<Self> {
        validate_go_path(module, "Go module path is malformed")?;
        validate_go_path(package, "Go package path is malformed")?;
        Ok(Self {
            module: module.to_owned(),
            package: package.to_owned(),
        })
    }

    fn origin(&self) -> PackageOrigin {
        PackageOrigin::new(&self.module).with_reference(format!("package:{}", self.package))
    }

    fn from_origin(origin: &PackageOrigin) -> ManagerResult<Self> {
        let reference = origin.reference.as_deref().ok_or_else(|| {
            protocol(
                "Go package origin is missing its typed reference",
                &origin.name,
            )
        })?;
        let package = reference
            .strip_prefix("package:")
            .ok_or_else(|| protocol("Go package origin reference is malformed", reference))?;
        Self::new(&origin.name, package)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledGoBinary {
    name: String,
    file_name: String,
    module: String,
    package: String,
    version: String,
    size: u64,
    updateable: bool,
}

impl InstalledGoBinary {
    fn origin(&self) -> PackageOrigin {
        GoOrigin {
            module: self.module.clone(),
            package: self.package.clone(),
        }
        .origin()
    }

    fn target(&self, manager_id: &ManagerId) -> PackageTarget {
        let mut target = PackageTarget::new(manager_id.clone(), &self.name);
        target.scope = PackageScope::User;
        target.origin = Some(self.origin());
        target
    }

    fn info(self, manager_id: &ManagerId) -> PackageInfo {
        let origin = self.origin();
        let mut info = PackageInfo::new(manager_id.clone(), self.name, self.version);
        info.size = Some(self.size);
        info.scope = PackageScope::User;
        info.origin = Some(origin);
        info
    }
}

#[derive(Debug, Deserialize)]
struct GoEnvironment {
    #[serde(rename = "GOBIN")]
    gobin: String,
    #[serde(rename = "GOPATH")]
    gopath: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GoBuildInfo {
    path: String,
    main: GoModuleMetadata,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GoModuleMetadata {
    path: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    replace: Option<Box<GoModuleMetadata>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GoModuleVersions {
    path: String,
    #[serde(default)]
    versions: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ValidatedBuildInfo {
    package: String,
    module: String,
    version: String,
    updateable: bool,
}

impl GoBuildInfo {
    fn validated(self) -> ManagerResult<ValidatedBuildInfo> {
        let origin = GoOrigin::new(&self.main.path, &self.path)?;
        let updateable = self.main.replace.is_none()
            && !self.main.version.is_empty()
            && self.main.version != "(devel)";
        if updateable {
            parse_semver(&self.main.version)?;
        }
        Ok(ValidatedBuildInfo {
            package: origin.package,
            module: origin.module,
            version: if self.main.version.is_empty() {
                "(devel)".to_owned()
            } else {
                self.main.version
            },
            updateable,
        })
    }
}

impl GoModuleVersions {
    fn validated(self, expected: &str) -> ManagerResult<Self> {
        if self.path != expected {
            return Err(protocol(
                "Go module version response returned another module",
                &self.path,
            ));
        }
        if self.versions.is_empty() {
            return Err(protocol("Go module has no published versions", expected));
        }
        for version in &self.versions {
            parse_semver(version)?;
        }
        Ok(self)
    }
}

impl GoModuleMetadata {
    fn validated_version(self, expected: &str) -> ManagerResult<String> {
        if self.path != expected {
            return Err(protocol(
                "Go latest response returned another module",
                &self.path,
            ));
        }
        if self.replace.is_some() {
            return Err(protocol(
                "Go latest response unexpectedly contains a replacement",
                expected,
            ));
        }
        parse_semver(&self.version)?;
        Ok(self.version)
    }
}

fn target_install_package(target: &PackageTarget) -> ManagerResult<String> {
    if target.scope == PackageScope::Unknown && target.origin.is_none() {
        validate_go_path(&target.name, "legacy Go install target is malformed")?;
        return Ok(target.name.clone());
    }
    if target.scope != PackageScope::User {
        return Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "Go target scope is not supported",
        )
        .with_detail(&target.name));
    }
    let origin = target
        .origin
        .as_ref()
        .ok_or_else(|| protocol("scoped Go target is missing its typed origin", &target.name))?;
    Ok(GoOrigin::from_origin(origin)?.package)
}

fn validate_binary_or_module_name(value: &str) -> ManagerResult<()> {
    if value.contains('/') || value.contains('.') {
        validate_go_path(value, "Go target identity is malformed")
    } else {
        validate_binary_name(value)
    }
}

fn validate_binary_name(value: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.starts_with('-')
        || Path::new(value).file_name().and_then(|name| name.to_str()) != Some(value)
        || value.chars().any(char::is_whitespace)
    {
        return Err(protocol("Go binary name is malformed", value));
    }
    Ok(())
}

fn logical_binary_name(file_name: &str, platform: Platform) -> ManagerResult<String> {
    let path = Path::new(file_name);
    let name = if platform == Platform::Windows
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(file_name)
    } else {
        file_name
    };
    validate_binary_name(name)?;
    Ok(name.to_owned())
}

fn validate_go_path(value: &str, message: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value.starts_with(['-', '/'])
        || value.ends_with('/')
        || value.contains("//")
        || value.contains(['@', '#', '\\'])
        || value.chars().any(char::is_whitespace)
        || value
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(protocol(message, value));
    }
    Ok(())
}

fn validate_version(value: &str) -> ManagerResult<()> {
    if value == "latest" {
        Ok(())
    } else {
        parse_semver(value).map(|_| ())
    }
}

fn validate_bin_dir_value(path: PathBuf, source: &str) -> ManagerResult<PathBuf> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(protocol(
            "Go binary directory must be an absolute path",
            source,
        ));
    }
    Ok(path)
}

fn go_command(path: &Path) -> CommandSpec {
    CommandSpec::new(path)
}

async fn go_environment(path: &Path) -> ManagerResult<GoEnvironment> {
    let spec = go_command(path).args(["env", "-json", "GOBIN", "GOPATH"]);
    let output = run_success(&spec, "Go environment query timed out").await?;
    decode_json(&output.stdout, "Go environment response is invalid")
}

fn decode_json<T: for<'de> Deserialize<'de>>(bytes: &[u8], message: &str) -> ManagerResult<T> {
    serde_json::from_slice(bytes).map_err(|error| protocol(message, &error.to_string()))
}

fn parse_semver(value: &str) -> ManagerResult<Version> {
    Version::parse(value.strip_prefix('v').unwrap_or(value))
        .map_err(|error| protocol("Go module version is not valid semver", &error.to_string()))
}

async fn run_build_probe(spec: &CommandSpec) -> ManagerResult<Option<Output>> {
    let output = timeout(COMMAND_TIMEOUT, run_output(spec))
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, "Go build-info probe timed out")
                .with_detail(spec.program().to_string_lossy())
        })??;
    if output.status.success() {
        return Ok(Some(output));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("not a Go executable") {
        return Ok(None);
    }
    Err(command_status_error(spec, output.status, stderr.trim()))
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
    let detail = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Err(command_status_error(spec, output.status, detail.trim()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_info_preserves_package_module_and_version() {
        let parsed = decode_json::<GoBuildInfo>(
            br#"{"Path":"golang.org/x/tools/gopls","Main":{"Path":"golang.org/x/tools/gopls","Version":"v0.20.0"}}"#,
            "invalid fixture",
        )
        .expect("valid JSON")
        .validated()
        .expect("valid build info");
        assert_eq!(parsed.package, "golang.org/x/tools/gopls");
        assert_eq!(parsed.module, "golang.org/x/tools/gopls");
        assert_eq!(parsed.version, "v0.20.0");
    }

    #[test]
    fn malformed_build_info_and_versions_are_errors() {
        let missing_main = decode_json::<GoBuildInfo>(br#"{"Path":"example.com/tool"}"#, "invalid");
        assert!(missing_main.is_err());
        let wrong = decode_json::<GoModuleVersions>(
            br#"{"Path":"example.com/other","Versions":["v1.0.0"]}"#,
            "invalid",
        )
        .expect("valid JSON");
        assert!(wrong.validated("example.com/tool").is_err());
    }

    #[test]
    fn environment_uses_go_acronym_field_names() {
        let environment = decode_json::<GoEnvironment>(
            br#"{"GOBIN":"/opt/go/bin","GOPATH":"/home/user/go"}"#,
            "invalid environment fixture",
        )
        .expect("valid Go environment");
        assert_eq!(environment.gobin, "/opt/go/bin");
        assert_eq!(environment.gopath, "/home/user/go");
    }

    #[test]
    fn origin_round_trip_preserves_module_and_package() {
        let origin =
            GoOrigin::new("example.com/mod", "example.com/mod/cmd/tool").expect("valid origin");
        assert_eq!(
            GoOrigin::from_origin(&origin.origin()).expect("round trip"),
            origin
        );
    }

    #[test]
    fn windows_executable_suffix_is_not_part_of_package_identity() {
        assert_eq!(
            logical_binary_name("gopls.EXE", Platform::Windows).expect("Windows binary identity"),
            "gopls"
        );
        assert_eq!(
            logical_binary_name("gopls.exe", Platform::Linux).expect("Linux binary identity"),
            "gopls.exe"
        );
    }
}
