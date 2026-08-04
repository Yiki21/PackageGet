use std::{collections::HashSet, path::Path, process::Output, time::Duration};

use async_trait::async_trait;
use semver::Version;
use tokio::time::timeout;
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId,
    ManagerResult, PackageAction, PackageInfo, PackageManager, PackageOrigin, PackageScope,
    PackageTarget, PackageUpdate, Platform, ProgressEvent, ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, decode_stdout, manager_availability, require_success, resolve_executable,
        run_output,
    },
    progress::run_cancellable_command_with_progress,
};

const BUN_ID: &str = "builtin:bun";
const BUN_COMMAND: &str = "bun";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

/// Direct `updater-manager-api` implementation for Bun global packages.
#[derive(Debug, Clone)]
pub struct BunManager {
    descriptor: ManagerDescriptor,
}

impl BunManager {
    /// Creates the built-in Bun global package manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(BUN_ID).expect("Bun manager ID must remain valid"),
            "Bun",
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
        .expect("Bun descriptor must remain valid")
        .with_description("Global JavaScript tools installed with Bun")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
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
                "Bun package target belongs to another manager",
                &target.name,
            ));
        }
        validate_package_name(&target.name)?;
        if target.scope != PackageScope::Unknown || target.origin.is_some() {
            if target.scope != PackageScope::User {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "Bun global target scope is not supported",
                )
                .with_detail(&target.name));
            }
            let origin = target.origin.as_ref().ok_or_else(|| {
                protocol(
                    "Bun global target is missing its typed origin",
                    &target.name,
                )
            })?;
            validate_origin(origin, &target.name)?;
        }

        let bun = resolve_executable(config, BUN_COMMAND);
        match action {
            PackageAction::Install => {
                let package = package_spec(&target.name, target.version.as_deref())?;
                Ok(bun_command(&bun)
                    .args(["add", "--global", "--no-progress", "--no-summary"])
                    .arg(package))
            }
            PackageAction::Update => {
                let package = package_spec(&target.name, target.version.as_deref())?;
                let mut command =
                    bun_command(&bun).args(["update", "--global", "--no-progress", "--no-summary"]);
                if target.version.is_none() {
                    command = command.arg("--latest");
                }
                Ok(command.arg(package))
            }
            PackageAction::Uninstall if target.version.is_none() => Ok(bun_command(&bun)
                .args(["remove", "--global", "--no-progress", "--no-summary"])
                .arg(&target.name)),
            PackageAction::Uninstall => Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "version-pinned Bun uninstall targets are not supported",
            )
            .with_detail(&target.name)),
            _ => Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "Bun global package action is unsupported",
            )),
        }
    }
}

impl Default for BunManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for BunManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(self.descriptor(), config, BUN_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let bun = resolve_executable(config, BUN_COMMAND);
        let spec = bun_command(&bun).args(["list", "--global", "--depth", "0"]);
        let output = run_with_timeout(&spec, "Bun global listing timed out").await?;
        if is_empty_global_state(&output) {
            return Ok(Vec::new());
        }
        let output = require_success(&spec, output, "Bun global listing failed")?;
        let value = decode_stdout(output, "Bun global listing is not valid UTF-8")?;
        parse_installed(&value, self.descriptor.id())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.validate_config(config)?;
        let bun = resolve_executable(config, BUN_COMMAND);
        let spec = bun_command(&bun).args(["outdated", "--global"]);
        let output = run_with_timeout(&spec, "Bun global outdated query timed out").await?;
        if is_empty_global_state(&output) {
            return Ok(Vec::new());
        }
        let output = require_success(&spec, output, "Bun global outdated query failed")?;
        let value = decode_stdout(output, "Bun global outdated output is not valid UTF-8")?;
        parse_updates(&value, self.descriptor.id())
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
                ManagerError::new(
                    ManagerErrorKind::Timeout,
                    "Bun global write command timed out",
                )
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

fn parse_installed(value: &str, manager_id: &ManagerId) -> ManagerResult<Vec<PackageInfo>> {
    let mut root_seen = false;
    let mut identities = HashSet::new();
    let mut packages = Vec::new();
    for line in value.lines().filter(|line| !line.trim().is_empty()) {
        if is_inventory_root(line) {
            if root_seen || !packages.is_empty() {
                return Err(protocol("Bun global listing contains multiple roots", line));
            }
            root_seen = true;
            continue;
        }
        if !root_seen {
            return Err(protocol("Bun global listing root is missing", line));
        }
        let mut fields = line.split_whitespace();
        let branch = fields
            .next()
            .ok_or_else(|| protocol("Bun global package row is malformed", line))?;
        let package = fields
            .next()
            .ok_or_else(|| protocol("Bun global package row is malformed", line))?;
        if !matches!(branch, "├──" | "└──") || fields.next().is_some() {
            return Err(protocol("Bun global package row is malformed", line));
        }
        let (name, version) = split_installed_spec(package)?;
        if !identities.insert(name.to_owned()) {
            return Err(protocol(
                "Bun global listing contains a duplicate package",
                name,
            ));
        }
        let mut info = PackageInfo::new(manager_id.clone(), name, version);
        info.scope = PackageScope::User;
        info.origin = Some(package_origin(name));
        packages.push(info);
    }
    if !root_seen {
        return Err(protocol("Bun global listing root is missing", value));
    }
    packages.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(packages)
}

fn parse_updates(value: &str, manager_id: &ManagerId) -> ManagerResult<Vec<PackageUpdate>> {
    let mut table_seen = false;
    let mut header_seen = false;
    let mut identities = HashSet::new();
    let mut updates = Vec::new();
    for line in value.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with("bun outdated v") {
            continue;
        }
        if is_table_border(line) {
            table_seen = true;
            continue;
        }
        let cells = table_cells(line)?;
        table_seen = true;
        if cells == ["Package", "Current", "Update", "Latest"] {
            if header_seen {
                return Err(protocol(
                    "Bun outdated output contains multiple headers",
                    line,
                ));
            }
            header_seen = true;
            continue;
        }
        if !header_seen || cells.len() != 4 {
            return Err(protocol("Bun outdated package row is malformed", line));
        }
        let name = cells[0];
        let current = cells[1];
        let compatible = cells[2];
        let latest = cells[3];
        validate_package_name(name)?;
        validate_version(current)?;
        validate_version(compatible)?;
        validate_version(latest)?;
        if !identities.insert(name.to_owned()) {
            return Err(protocol(
                "Bun outdated output contains a duplicate package",
                name,
            ));
        }
        if current == latest {
            continue;
        }
        let mut target = PackageTarget::new(manager_id.clone(), name);
        target.version = Some(latest.to_owned());
        target.scope = PackageScope::User;
        target.origin = Some(package_origin(name));
        updates.push(PackageUpdate::new(target, current, latest));
    }
    if table_seen && !header_seen {
        return Err(protocol("Bun outdated table header is missing", value));
    }
    updates.sort_by(|left, right| left.target.name.cmp(&right.target.name));
    Ok(updates)
}

fn is_inventory_root(line: &str) -> bool {
    line.rsplit_once(" node_modules (")
        .is_some_and(|(path, count)| {
            !path.trim().is_empty()
                && count
                    .strip_suffix(')')
                    .is_some_and(|count| count.parse::<usize>().is_ok())
        })
}

fn split_installed_spec(value: &str) -> ManagerResult<(&str, &str)> {
    let index = value
        .rfind('@')
        .filter(|index| *index > 0)
        .ok_or_else(|| protocol("Bun installed package identity is malformed", value))?;
    let name = &value[..index];
    let version = &value[index + 1..];
    validate_package_name(name)?;
    validate_version(version)?;
    Ok((name, version))
}

fn is_table_border(line: &str) -> bool {
    line.strip_prefix('|')
        .and_then(|line| line.strip_suffix('|'))
        .is_some_and(|line| !line.is_empty() && line.chars().all(|ch| matches!(ch, '-' | '|')))
}

fn table_cells(line: &str) -> ManagerResult<Vec<&str>> {
    let row = line
        .strip_prefix('|')
        .and_then(|line| line.strip_suffix('|'))
        .ok_or_else(|| protocol("Bun outdated output contains an unexpected line", line))?;
    Ok(row.split('|').map(str::trim).collect())
}

fn package_spec(name: &str, version: Option<&str>) -> ManagerResult<String> {
    match version {
        Some(version) => {
            validate_version(version)?;
            Ok(format!("{name}@{version}"))
        }
        None => Ok(name.to_owned()),
    }
}

fn package_origin(name: &str) -> PackageOrigin {
    PackageOrigin::new("Bun global").with_reference(format!("package:{name}"))
}

fn validate_origin(origin: &PackageOrigin, name: &str) -> ManagerResult<()> {
    let expected = format!("package:{name}");
    if origin.name == "Bun global" && origin.reference.as_deref() == Some(expected.as_str()) {
        Ok(())
    } else {
        Err(protocol("Bun global package origin is malformed", name))
    }
}

fn validate_package_name(name: &str) -> ManagerResult<()> {
    if name.is_empty() || name.len() > 214 || !name.is_ascii() || name.starts_with('-') {
        return Err(protocol("Bun package name is malformed", name));
    }
    let valid = if let Some(scoped) = name.strip_prefix('@') {
        scoped.split_once('/').is_some_and(|(scope, package)| {
            valid_name_component(scope) && valid_name_component(package) && !package.contains('/')
        })
    } else {
        valid_name_component(name)
    };
    if valid {
        Ok(())
    } else {
        Err(protocol("Bun package name is malformed", name))
    }
}

fn valid_name_component(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(['.', '_'])
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '.' | '_' | '~')
        })
}

fn validate_version(version: &str) -> ManagerResult<()> {
    Version::parse(version).map(|_| ()).map_err(|error| {
        protocol(
            "Bun package version is not valid semver",
            &error.to_string(),
        )
    })
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "Bun global package action is unsupported",
        )),
    }
}

fn bun_command(path: &Path) -> CommandSpec {
    CommandSpec::new(path)
        .env("NO_COLOR", "1")
        .env("CI", "true")
}

async fn run_with_timeout(spec: &CommandSpec, message: &str) -> ManagerResult<Output> {
    timeout(COMMAND_TIMEOUT, run_output(spec))
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, message)
                .with_detail(spec.program().to_string_lossy())
        })?
}

fn is_empty_global_state(output: &Output) -> bool {
    if output.status.code() != Some(1) {
        return false;
    }
    let detail = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    [
        "missingpackagejson",
        "no package.json was found for directory",
        "missing lockfile, nothing outdated",
        "lockfile not found",
    ]
    .iter()
    .any(|marker| detail.contains(marker))
}

fn protocol(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(detail)
}
