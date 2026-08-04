use std::{collections::HashMap, path::Path, time::Duration};

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
    progress::run_cancellable_command_with_progress,
};

const CHOCOLATEY_ID: &str = "builtin:chocolatey";
const CHOCOLATEY_COMMAND: &str = "choco";
const ORIGIN_NAME: &str = "Chocolatey";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

/// Direct implementation for machine-scoped Chocolatey packages on Windows.
#[derive(Debug, Clone)]
pub struct ChocolateyManager {
    descriptor: ManagerDescriptor,
}

impl ChocolateyManager {
    /// Creates the built-in Chocolatey manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(CHOCOLATEY_ID).expect("Chocolatey manager ID must remain valid"),
            "Chocolatey",
            ManagerCategory::Application,
            SupportedPlatforms::from([Platform::Windows]),
            ManagerCapabilities::from([
                ManagerCapability::Installed,
                ManagerCapability::Updates,
                ManagerCapability::Search,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ]),
        )
        .expect("Chocolatey descriptor must remain valid")
        .with_description("Machine-scoped Windows applications managed by Chocolatey")
        .with_authorization(AuthorizationHint::RequiresElevation {
            message: Some("Chocolatey writes machine-wide packages and requires elevation.".into()),
        });

        Self { descriptor }
    }

    async fn installed_packages(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let choco = resolve_executable(config, CHOCOLATEY_COMMAND);
        let output = run_success(
            &chocolatey_command(&choco).args(["list", "--limit-output"]),
            "Chocolatey list timed out",
        )
        .await?;
        let stdout = decode_stdout(output, "Chocolatey list output is not valid UTF-8")?;
        parse_installed(&stdout, self.descriptor.id())
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
                "Chocolatey package target belongs to another manager",
                &target.name,
            ));
        }
        validate_identifier(&target.name)?;
        if target.version.is_some() {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "version-pinned Chocolatey operations are not supported",
            )
            .with_detail(&target.name));
        }
        if target.scope != PackageScope::System {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "Chocolatey only supports system package scope",
            )
            .with_detail(&target.name));
        }
        if let Some(origin) = &target.origin
            && origin.name != ORIGIN_NAME
        {
            return Err(protocol(
                "Chocolatey target origin is not Chocolatey",
                &target.name,
            ));
        }

        let choco = resolve_executable(config, CHOCOLATEY_COMMAND);
        let verb = match action {
            PackageAction::Install => "install",
            PackageAction::Update => "upgrade",
            PackageAction::Uninstall => "uninstall",
            _ => unreachable!("supported actions were checked above"),
        };
        Ok(chocolatey_command(&choco)
            .arg(verb)
            .arg(target.name.clone())
            .args(["--yes", "--no-progress"]))
    }
}

impl Default for ChocolateyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for ChocolateyManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(
            self.descriptor(),
            config,
            CHOCOLATEY_COMMAND,
            &["--version"],
        )
        .await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.installed_packages(config).await
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.installed_packages(config).await?.len())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.validate_config(config)?;
        let installed = self.installed_packages(config).await?;
        let mut installed_by_name = HashMap::with_capacity(installed.len());
        for package in installed {
            if installed_by_name
                .insert(package.name.to_ascii_lowercase(), package)
                .is_some()
            {
                return Err(protocol(
                    "Chocolatey outdated cannot disambiguate duplicate packages",
                    "duplicate installed identity",
                ));
            }
        }

        let choco = resolve_executable(config, CHOCOLATEY_COMMAND);
        let output = run_success(
            &chocolatey_command(&choco).args(["outdated", "--limit-output"]),
            "Chocolatey outdated timed out",
        )
        .await?;
        let stdout = decode_stdout(output, "Chocolatey outdated output is not valid UTF-8")?;
        parse_updates(&stdout, self.descriptor.id(), &installed_by_name)
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        validate_search_query(query)?;
        let choco = resolve_executable(config, CHOCOLATEY_COMMAND);
        let spec = chocolatey_command(&choco).args(["search", query, "--limit-output"]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            let detail = command_output_detail(&output);
            if detail.to_ascii_lowercase().contains("no results") {
                return Ok(Vec::new());
            }
            return Err(command_status_error(&spec, output.status, &detail));
        }
        let stdout = decode_stdout(output, "Chocolatey search output is not valid UTF-8")?;
        parse_search(&stdout, self.descriptor.id())
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
                    let (_, message) = event.into_parts();
                    if let Some(message) = message {
                        progress.emit(ProgressEvent::Message { message });
                    }
                }),
            )
            .await
            .map_err(|_| {
                ManagerError::new(
                    ManagerErrorKind::Timeout,
                    "Chocolatey package command timed out",
                )
                .with_detail(command.program().to_string_lossy())
            })??;
            progress.emit(ProgressEvent::Advanced {
                completed: index + 1,
                total,
                current_package: Some(target.name.clone()),
            });
        }
        progress.emit(ProgressEvent::Finished {
            completed: total,
            total,
        });
        Ok(())
    }
}

fn chocolatey_command(executable: &Path) -> CommandSpec {
    CommandSpec::new(executable)
}

async fn run_success(
    spec: &CommandSpec,
    timeout_message: &str,
) -> ManagerResult<std::process::Output> {
    timeout(COMMAND_TIMEOUT, run_output(spec))
        .await
        .map_err(|_| ManagerError::new(ManagerErrorKind::Timeout, timeout_message))?
        .and_then(|output| {
            if output.status.success() {
                Ok(output)
            } else {
                let detail = command_output_detail(&output);
                Err(command_status_error(spec, output.status, &detail))
            }
        })
}

fn decode_stdout(output: std::process::Output, message: &str) -> ManagerResult<String> {
    String::from_utf8(output.stdout).map_err(|error| {
        ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(error.to_string())
    })
}

fn parse_installed(stdout: &str, manager_id: &ManagerId) -> ManagerResult<Vec<PackageInfo>> {
    let rows = pipe_rows(stdout);
    let mut seen = HashMap::with_capacity(rows.len());
    let mut packages = Vec::with_capacity(rows.len());
    for fields in rows {
        let [name, version, ..] = fields.as_slice() else {
            continue;
        };
        validate_identifier(name)?;
        if version.is_empty() {
            continue;
        }
        if seen.insert(name.to_ascii_lowercase(), ()).is_some() {
            return Err(protocol(
                "Chocolatey list contains a duplicate package",
                name,
            ));
        }
        let mut package = PackageInfo::new(manager_id.clone(), name, version);
        package.scope = PackageScope::System;
        package.origin = Some(PackageOrigin::new(ORIGIN_NAME));
        packages.push(package);
    }
    packages.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    Ok(packages)
}

fn parse_updates(
    stdout: &str,
    manager_id: &ManagerId,
    installed: &HashMap<String, PackageInfo>,
) -> ManagerResult<Vec<PackageUpdate>> {
    let mut updates = Vec::new();
    for fields in pipe_rows(stdout) {
        let [name, current, available, ..] = fields.as_slice() else {
            continue;
        };
        if current.is_empty()
            || available.is_empty()
            || available.eq_ignore_ascii_case("n/a")
            || current == available
        {
            continue;
        }
        validate_identifier(name)?;
        let Some(package) = installed.get(&name.to_ascii_lowercase()) else {
            return Err(protocol(
                "Chocolatey outdated package is absent from installed inventory",
                name,
            ));
        };
        let mut target = package.target();
        target.manager_id = manager_id.clone();
        updates.push(PackageUpdate::new(target, current, available));
    }
    Ok(updates)
}

fn parse_search(stdout: &str, manager_id: &ManagerId) -> ManagerResult<Vec<PackageInfo>> {
    let mut packages = Vec::new();
    for fields in pipe_rows(stdout) {
        let [name, version, ..] = fields.as_slice() else {
            continue;
        };
        validate_identifier(name)?;
        if version.is_empty() {
            continue;
        }
        let mut package = PackageInfo::new(manager_id.clone(), name, version);
        package.scope = PackageScope::System;
        package.origin = Some(PackageOrigin::new(ORIGIN_NAME));
        packages.push(package);
    }
    Ok(packages)
}

fn pipe_rows(stdout: &str) -> Vec<Vec<String>> {
    stdout
        .lines()
        .filter_map(|line| {
            let fields = line
                .split('|')
                .map(str::trim)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if fields.len() < 2 || fields[0].is_empty() || fields[0].eq_ignore_ascii_case("name") {
                None
            } else {
                Some(fields)
            }
        })
        .collect()
}

fn validate_identifier(identifier: &str) -> ManagerResult<()> {
    let identifier = identifier.trim();
    if identifier.is_empty()
        || identifier.starts_with('-')
        || identifier
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(protocol(
            "Chocolatey package identifier is invalid",
            identifier,
        ));
    }
    Ok(())
}

fn validate_search_query(query: &str) -> ManagerResult<()> {
    if query.chars().any(char::is_control) {
        return Err(protocol(
            "Chocolatey search query contains control characters",
            query,
        ));
    }
    Ok(())
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(ManagerError::unsupported(action.capability())),
    }
}

fn protocol(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(detail)
}

fn command_output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        stderr.into_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CHOCOLATEY_ID, ChocolateyManager, ORIGIN_NAME, parse_installed, parse_search, parse_updates,
    };
    use std::{collections::HashMap, ffi::OsString, path::PathBuf};
    use updater_manager_api::{
        ManagerConfig, ManagerId, PackageAction, PackageInfo, PackageManager, PackageOrigin,
        PackageScope, PackageTarget,
    };

    fn manager_id() -> ManagerId {
        ManagerId::parse(CHOCOLATEY_ID).expect("valid Chocolatey ID")
    }

    #[test]
    fn list_parser_preserves_system_scope_and_origin() {
        let packages = parse_installed("git|2.45.1\nnodejs|22.1.0\n", &manager_id())
            .expect("parse Chocolatey list");

        assert_eq!(packages.len(), 2);
        assert!(
            packages
                .iter()
                .all(|package| package.scope == PackageScope::System)
        );
        assert_eq!(
            packages[0]
                .origin
                .as_ref()
                .map(|origin| origin.name.as_str()),
            Some(ORIGIN_NAME)
        );
    }

    #[test]
    fn outdated_parser_joins_installed_identity() {
        let manager = manager_id();
        let mut installed = HashMap::new();
        installed.insert(
            "git".to_owned(),
            PackageInfo::new(manager.clone(), "git", "2.45.0"),
        );
        let updates = parse_updates("Git|2.45.0|2.45.1|false\n", &manager, &installed)
            .expect("parse Chocolatey outdated");

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].current_version, "2.45.0");
        assert_eq!(updates[0].available_version, "2.45.1");
    }

    #[test]
    fn search_parser_ignores_non_pipe_headers() {
        let packages = parse_search(
            "Chocolatey v2.4.0\nResults\ngit|2.45.1|Approved\n",
            &manager_id(),
        )
        .expect("parse Chocolatey search");

        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "git");
    }

    #[test]
    fn write_commands_require_system_scope() {
        let manager = ChocolateyManager::new();
        let config = ManagerConfig::new(manager.descriptor().id().clone())
            .with_executable(PathBuf::from("choco"));
        let mut target = PackageTarget::new(manager.descriptor().id().clone(), "git");
        target.scope = PackageScope::System;
        target.origin = Some(PackageOrigin::new("Chocolatey"));

        let install = manager
            .write_command(&config, PackageAction::Install, &target)
            .expect("build Chocolatey install command");
        assert_eq!(
            install.arguments(),
            vec![
                OsString::from("install"),
                OsString::from("git"),
                OsString::from("--yes"),
                OsString::from("--no-progress"),
            ]
        );

        target.scope = PackageScope::User;
        assert!(
            manager
                .write_command(&config, PackageAction::Install, &target)
                .is_err()
        );
    }
}
