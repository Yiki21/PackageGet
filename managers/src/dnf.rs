use std::{collections::HashSet, ffi::OsString, process::ExitStatus};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId,
    ManagerResult, PackageAction, PackageInfo, PackageManager, PackageScope, PackageTarget,
    PackageUpdate, Platform, ProgressEvent, ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, command_status_error, decode_stdout, manager_availability, require_success,
        resolve_executable, run_output, system_helper_command,
    },
    progress::{
        CommandProgress, run_cancellable_dnf_command_with_progress, run_dnf_command_with_progress,
    },
};

const DNF_ID: &str = "builtin:dnf";
const DNF_COMMAND: &str = "dnf";
const RPM_COMMAND: &str = "rpm";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const RPM_QUERY_FORMAT: &str =
    "%{NAME}\t%{VERSION}-%{RELEASE}\t%{SUMMARY}\t%{SIZE}\t%{INSTALLTIME}\t%{URL}\n";

/// Direct `updater-manager-api` implementation for DNF.
#[derive(Debug, Clone)]
pub struct DnfManager {
    descriptor: ManagerDescriptor,
}

impl DnfManager {
    /// Creates the built-in DNF manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(DNF_ID).expect("DNF manager ID must remain valid"),
            "DNF",
            ManagerCategory::System,
            SupportedPlatforms::from([Platform::Linux]),
            ManagerCapabilities::from([
                ManagerCapability::Installed,
                ManagerCapability::Updates,
                ManagerCapability::Search,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ]),
        )
        .expect("DNF descriptor must remain valid")
        .with_description("Fedora/RHEL 系统包管理器")
        .with_authorization(AuthorizationHint::RequiresElevation {
            message: Some("System package changes require administrator approval.".to_owned()),
        });

        Self { descriptor }
    }

    /// Returns the installed version of one RPM package.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when RPM cannot identify the package or emits
    /// invalid UTF-8. Command startup failures retain their typed command
    /// classification.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        package_name: &str,
    ) -> ManagerResult<String> {
        self.validate_config(config)?;
        let spec = CommandSpec::new(RPM_COMMAND).args([
            OsString::from("-q"),
            OsString::from("--queryformat"),
            OsString::from("%{VERSION}-%{RELEASE}"),
            OsString::from(package_name),
        ]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "dnf package version is unavailable",
            )
            .with_detail(package_name));
        }

        let version = decode_stdout(output, "dnf package version is not valid UTF-8")?
            .trim()
            .to_owned();
        if version.is_empty() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "dnf package version is empty",
            )
            .with_detail(package_name));
        }

        Ok(version)
    }

    /// Executes a DNF package group while exposing normalized two-phase
    /// command progress.
    ///
    /// This compatibility surface lets the existing core dispatcher reuse the
    /// direct implementation until the UI moves to [`PackageManager::execute`].
    ///
    /// # Errors
    ///
    /// Returns a protocol error for a mismatched manager configuration, an
    /// unsupported error for unknown future actions, or a typed command error
    /// when DNF or `pkexec` fails.
    #[allow(dead_code)]
    async fn execute_packages_with_progress(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        package_names: &[String],
        on_progress: impl FnMut(CommandProgress),
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        ensure_supported_action(action)?;
        if package_names.is_empty() {
            return Ok(());
        }

        let command = self.write_command(config, action, package_names)?;
        run_dnf_command_with_progress(&command, on_progress).await
    }

    async fn list_updates(
        &self,
        config: &ManagerConfig,
        refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.validate_config(config)?;
        let dnf_path = resolve_executable(config, DNF_COMMAND);
        let spec = check_upgrade_command(&dnf_path, refresh);
        let output = run_output(&spec).await?;

        if !is_check_upgrade_status_ok(&output.status) {
            let tail = command_output_tail(&output.stdout, &output.stderr);
            return Err(command_status_error(&spec, output.status, &tail));
        }

        let stdout = decode_stdout(output, "dnf update listing is not valid UTF-8")?;
        let mut updates = Vec::new();
        let mut seen_packages = HashSet::new();

        for raw_line in stdout.lines() {
            let Some((name, available_version)) = parse_check_upgrade_entry(raw_line) else {
                continue;
            };
            if !seen_packages.insert(name.to_owned()) {
                continue;
            }

            let current_version = self
                .current_version(config, name)
                .await
                .unwrap_or_else(|_| "unknown".to_owned());
            let mut target = PackageTarget::new(self.descriptor.id().clone(), name);
            target.scope = PackageScope::System;
            updates.push(PackageUpdate::new(
                target,
                current_version,
                available_version,
            ));
        }

        Ok(updates)
    }

    fn write_command(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        package_names: &[String],
    ) -> ManagerResult<CommandSpec> {
        self.validate_config(config)?;
        ensure_supported_action(action)?;
        let command = match action {
            PackageAction::Install => system_helper_command("install", "dnf"),
            PackageAction::Update => system_helper_command("update", "dnf"),
            PackageAction::Uninstall => system_helper_command("remove", "dnf"),
            _ => return Err(unsupported_action_error()),
        };

        Ok(command.args(package_names.iter().map(OsString::from)))
    }
}

impl Default for DnfManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for DnfManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        if !cfg!(target_os = "linux") {
            return Ok(ManagerAvailability::Unavailable {
                reason: updater_manager_api::AvailabilityReason::UnsupportedPlatform {
                    platform: Platform::current(),
                },
            });
        }

        Ok(manager_availability(config, DNF_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let spec = CommandSpec::new(RPM_COMMAND).args(["-qa", "--queryformat", RPM_QUERY_FORMAT]);
        let output = require_success(
            &spec,
            run_output(&spec).await?,
            "dnf installed package listing failed",
        )?;
        let stdout = decode_stdout(output, "dnf installed package listing is not valid UTF-8")?;
        Ok(parse_installed_packages(&stdout, self.descriptor.id()))
    }

    async fn package_info(
        &self,
        config: &ManagerConfig,
        target: &PackageTarget,
    ) -> ManagerResult<Option<PackageInfo>> {
        self.validate_config(config)?;
        if &target.manager_id != self.descriptor.id() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "dnf package target belongs to another manager",
            )
            .with_detail(&target.name));
        }

        let spec = CommandSpec::new(RPM_COMMAND).args([
            OsString::from("-q"),
            OsString::from("--queryformat"),
            OsString::from(RPM_QUERY_FORMAT),
            OsString::from(&target.name),
        ]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(None);
        }
        let stdout = decode_stdout(output, "dnf package information is not valid UTF-8")?;
        Ok(parse_installed_packages(&stdout, self.descriptor.id())
            .into_iter()
            .next())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        self.validate_config(config)?;
        let spec = CommandSpec::new(RPM_COMMAND).args(["-qa", "--queryformat", "%{NAME}\n"]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(self.installed(config).await?.len());
        }

        let stdout = decode_stdout(output, "dnf installed package count is not valid UTF-8")?;
        Ok(stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.list_updates(config, refresh).await
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let dnf_path = resolve_executable(config, DNF_COMMAND);
        let spec = CommandSpec::new(dnf_path).args(["search", "--quiet", query]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = decode_stdout(output, "dnf search output is not valid UTF-8")?;
        let mut packages = Vec::new();
        for name in parse_search_names(&stdout) {
            let version = self
                .current_version(config, &name)
                .await
                .unwrap_or_else(|_| NOT_INSTALLED_VERSION.to_owned());
            let mut package = PackageInfo::new(self.descriptor.id().clone(), name, version);
            package.scope = PackageScope::System;
            packages.push(package);
        }
        Ok(packages)
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
        let package_names = packages
            .iter()
            .map(|package| {
                if &package.manager_id != self.descriptor.id() {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Protocol,
                        "dnf package target belongs to another manager",
                    )
                    .with_detail(format!(
                        "expected {}, received {} for package {}",
                        self.descriptor.id(),
                        package.manager_id,
                        package.name
                    )));
                }
                Ok(package.name.clone())
            })
            .collect::<ManagerResult<Vec<_>>>()?;

        let total = package_names.len();
        progress.emit(ProgressEvent::Started { action, total });
        if package_names.is_empty() {
            progress.emit(ProgressEvent::Finished {
                completed: 0,
                total: 0,
            });
            return Ok(());
        }
        let command = self.write_command(config, action, &package_names)?;
        run_cancellable_dnf_command_with_progress(&command, progress, |event| {
            let (fraction, message) = event.into_parts();
            if let Some(message) = message {
                progress.emit(ProgressEvent::Message { message });
            }
            let completed = if fraction >= 1.0 {
                total
            } else {
                ((fraction * total as f32).floor() as usize).min(total)
            };
            progress.emit(ProgressEvent::Advanced {
                completed,
                total,
                current_package: None,
            });
        })
        .await?;
        progress.emit(ProgressEvent::Finished {
            completed: total,
            total,
        });
        Ok(())
    }
}

fn parse_installed_packages(stdout: &str, manager_id: &ManagerId) -> Vec<PackageInfo> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?.trim();
            let version = fields.next()?.trim();
            if name.is_empty() || version.is_empty() {
                return None;
            }

            let description = optional_rpm_field(fields.next());
            let size = fields.next().and_then(|value| value.parse::<u64>().ok());
            let install_date = fields
                .next()
                .and_then(|value| value.parse::<i64>().ok())
                .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
                .map(|datetime| datetime.format("%Y-%m-%d %H:%M:%S").to_string());
            let homepage = optional_rpm_field(fields.next());

            let mut package = PackageInfo::new(manager_id.clone(), name, version);
            package.description = description;
            package.size = size;
            package.install_date = install_date;
            package.homepage = homepage;
            package.scope = PackageScope::System;
            Some(package)
        })
        .collect()
}

fn optional_rpm_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "(none)")
        .map(ToOwned::to_owned)
}

fn parse_search_names(stdout: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();

    for line in stdout.lines().map(str::trim) {
        if line.is_empty() || line.starts_with("Matched fields:") {
            continue;
        }
        let Some((name_with_arch, _summary)) = line.split_once('\t') else {
            continue;
        };
        let name_with_arch = name_with_arch.trim();
        let name = name_with_arch
            .rsplit_once('.')
            .map_or(name_with_arch, |(name, _architecture)| name);
        if !name.is_empty() && seen.insert(name.to_owned()) {
            names.push(name.to_owned());
        }
    }

    names.sort();
    names
}

fn check_upgrade_command(dnf_path: &std::path::Path, refresh: bool) -> CommandSpec {
    if refresh {
        return system_helper_command("refresh", "dnf");
    }

    CommandSpec::new(dnf_path).arg("check-upgrade")
}

fn is_check_upgrade_status_ok(status: &ExitStatus) -> bool {
    status.success() || status.code() == Some(100)
}

fn parse_check_upgrade_entry(raw_line: &str) -> Option<(&str, &str)> {
    if raw_line.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }

    let line = raw_line.trim();
    if line.is_empty()
        || line.starts_with("Updating and loading repositories:")
        || line.starts_with("Repositories loaded.")
        || line.starts_with("Available upgrades")
        || line.starts_with("Obsoleting packages")
    {
        return None;
    }

    let mut fields = line.split_whitespace();
    let package_with_arch = fields.next()?;
    let available_version = fields.next()?;
    let _repository = fields.next()?;
    let (name, architecture) = package_with_arch.rsplit_once('.')?;
    if name.is_empty() || architecture.is_empty() {
        return None;
    }

    Some((name, available_version))
}

fn command_output_tail(stdout: &[u8], stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    if !stderr.trim().is_empty() {
        return stderr.trim().to_owned();
    }

    let stdout = String::from_utf8_lossy(stdout);
    stdout.trim().to_owned()
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(unsupported_action_error()),
    }
}

fn unsupported_action_error() -> ManagerError {
    ManagerError::new(ManagerErrorKind::Unsupported, "dnf action is not supported")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn write_commands_preserve_dnf_batch_semantics() {
        let manager = DnfManager::new();
        let config =
            ManagerConfig::new(manager.descriptor().id().clone()).with_executable("/custom/dnf5");
        let names = vec!["bash".to_owned(), "curl".to_owned()];

        let install = manager
            .write_command(&config, PackageAction::Install, &names)
            .expect("build install command");
        assert_eq!(install.program(), Path::new("/usr/bin/pkexec"));
        assert_eq!(
            install.arguments(),
            [
                "/usr/lib/updater/updater-system-helper",
                "install",
                "dnf",
                "bash",
                "curl",
            ]
            .map(OsString::from)
            .as_slice()
        );

        let update = manager
            .write_command(&config, PackageAction::Update, &names)
            .expect("build update command");
        assert_eq!(
            update.arguments(),
            [
                "/usr/lib/updater/updater-system-helper",
                "update",
                "dnf",
                "bash",
                "curl",
            ]
            .map(OsString::from)
            .as_slice()
        );

        let uninstall = manager
            .write_command(&config, PackageAction::Uninstall, &names)
            .expect("build uninstall command");
        assert_eq!(
            uninstall.arguments(),
            [
                "/usr/lib/updater/updater-system-helper",
                "remove",
                "dnf",
                "bash",
                "curl",
            ]
            .map(OsString::from)
            .as_slice()
        );
    }

    #[test]
    fn check_upgrade_commands_preserve_refresh_semantics() {
        let direct = check_upgrade_command(Path::new("/usr/bin/dnf5"), false);
        assert_eq!(direct.program(), Path::new("/usr/bin/dnf5"));
        assert_eq!(
            direct.arguments(),
            ["check-upgrade"].map(OsString::from).as_slice()
        );

        let refresh = check_upgrade_command(Path::new("/usr/bin/dnf5"), true);
        assert_eq!(refresh.program(), Path::new("/usr/bin/pkexec"));
        assert_eq!(
            refresh.arguments(),
            ["/usr/lib/updater/updater-system-helper", "refresh", "dnf",]
                .map(OsString::from)
                .as_slice()
        );
    }

    #[test]
    fn parses_installed_search_and_update_outputs() {
        let id = ManagerId::parse(DNF_ID).expect("valid DNF ID");
        let installed = parse_installed_packages(
            "bash\t5.2-1.fc43\tGNU shell\t4096\t0\thttps://gnu.org/bash\n\
             empty\t1-1\t(none)\tinvalid\t(none)\t(none)\n\
             invalid\n",
            &id,
        );
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].name, "bash");
        assert_eq!(installed[0].description.as_deref(), Some("GNU shell"));
        assert_eq!(installed[0].size, Some(4096));
        assert_eq!(installed[0].scope, PackageScope::System);
        assert_eq!(installed[1].description, None);
        assert_eq!(installed[1].homepage, None);

        let search = parse_search_names(
            "Matched fields: name, summary\nrg.x86_64\tsearch tool\nrg.noarch\tduplicate\nvim.x86_64\teditor\n",
        );
        assert_eq!(search, vec!["rg", "vim"]);

        assert_eq!(
            parse_check_upgrade_entry("akonadi-calendar.x86_64 25.12.3-1.fc43 updates"),
            Some(("akonadi-calendar", "25.12.3-1.fc43"))
        );
        assert!(parse_check_upgrade_entry("Repositories loaded.").is_none());
        assert!(
            parse_check_upgrade_entry("    kernel-headers.x86_64 6.18.3-200.fc43 updates")
                .is_none()
        );
    }

    #[tokio::test]
    async fn empty_execution_does_not_run_dnf() {
        DnfManager::new()
            .execute_packages_with_progress(
                &ManagerConfig::new(ManagerId::parse(DNF_ID).expect("valid DNF ID")),
                PackageAction::Install,
                &[],
                |_| {},
            )
            .await
            .expect("execute empty DNF group");
    }
}
