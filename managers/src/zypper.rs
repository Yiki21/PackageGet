use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::Path,
    process::ExitStatus,
};

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
        CommandSpec, command_status_error, decode_stdout, manager_availability, resolve_executable,
        run_output, system_helper_command,
    },
    progress::{CommandProgress, run_command_with_progress_and_status},
};

const ZYPPER_ID: &str = "builtin:zypper";
const ZYPPER_COMMAND: &str = "zypper";
const RPM_COMMAND: &str = "rpm";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const RPM_QUERY_FORMAT: &str =
    "%{NAME}\t%{VERSION}-%{RELEASE}\t%{SUMMARY}\t%{SIZE}\t%{INSTALLTIME}\t%{URL}\n";
const RPM_VERSION_MAP_FORMAT: &str = "%{NAME}\t%{VERSION}-%{RELEASE}\n";

/// Direct `updater-manager-api` implementation for Zypper.
#[derive(Debug, Clone)]
pub struct ZypperManager {
    descriptor: ManagerDescriptor,
}

impl ZypperManager {
    /// Creates the built-in Zypper manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(ZYPPER_ID).expect("Zypper manager ID must remain valid"),
            "Zypper",
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
        .expect("Zypper descriptor must remain valid")
        .with_description("openSUSE/SUSE 系统包管理器")
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
        let spec = rpm_current_version_command(package_name);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "zypper package version is unavailable",
            )
            .with_detail(package_name));
        }

        let version = decode_stdout(output, "zypper package version is not valid UTF-8")?
            .trim()
            .to_owned();
        if version.is_empty() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "zypper package version is empty",
            )
            .with_detail(package_name));
        }

        Ok(version)
    }

    /// Executes a Zypper package group while exposing normalized command
    /// progress.
    ///
    /// This compatibility surface lets the existing core dispatcher reuse the
    /// direct implementation until the UI moves to [`PackageManager::execute`].
    ///
    /// # Errors
    ///
    /// Returns a protocol error for a mismatched manager configuration, an
    /// unsupported error for unknown future actions, or a typed Zypper status
    /// error when Zypper or `pkexec` fails.
    pub async fn execute_packages_with_progress(
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
        run_command_with_progress_and_status(&command, zypper_status_error, on_progress).await
    }

    async fn list_updates(
        &self,
        config: &ManagerConfig,
        refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.validate_config(config)?;
        let zypper_path = resolve_executable(config, ZYPPER_COMMAND);

        if refresh {
            let refresh = refresh_command();
            run_command_with_progress_and_status(&refresh, zypper_status_error, |_| {}).await?;
        }

        let spec = list_updates_command(&zypper_path);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            let tail = command_output_tail(&output.stdout, &output.stderr);
            return Err(zypper_status_error(&spec, output.status, &tail));
        }

        let stdout = decode_stdout(output, "zypper update listing is not valid UTF-8")?;
        let mut updates = Vec::new();
        let mut seen = HashSet::new();
        for row in parse_table_rows(&stdout, &update_headers()) {
            let Some(name) = row.get("name") else {
                continue;
            };
            let Some(current_version) = row.get("current_version") else {
                continue;
            };
            let Some(available_version) = row.get("available_version") else {
                continue;
            };
            if name.is_empty()
                || current_version.is_empty()
                || available_version.is_empty()
                || !seen.insert(name.clone())
            {
                continue;
            }

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

    async fn installed_version_map(&self) -> ManagerResult<HashMap<String, String>> {
        let spec =
            CommandSpec::new(RPM_COMMAND).args(["-qa", "--queryformat", RPM_VERSION_MAP_FORMAT]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(HashMap::new());
        }

        let stdout = decode_stdout(output, "zypper installed versions are not valid UTF-8")?;
        Ok(parse_installed_versions(&stdout))
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            return Ok(());
        }

        Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "zypper configuration ID does not match the manager",
        )
        .with_detail(format!(
            "expected {}, received {}",
            self.descriptor.id(),
            config.id
        )))
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
            PackageAction::Install => system_helper_command("install", "zypper"),
            PackageAction::Update => system_helper_command("update", "zypper"),
            PackageAction::Uninstall => system_helper_command("remove", "zypper"),
            _ => return Err(unsupported_action_error()),
        };

        Ok(command.args(package_names.iter().map(OsString::from)))
    }
}

impl Default for ZypperManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for ZypperManager {
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

        Ok(manager_availability(config, ZYPPER_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let spec = CommandSpec::new(RPM_COMMAND).args(["-qa", "--queryformat", RPM_QUERY_FORMAT]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            let tail = command_output_tail(&output.stdout, &output.stderr);
            return Err(command_status_error(&spec, output.status, &tail));
        }

        let stdout = decode_stdout(
            output,
            "zypper installed package listing is not valid UTF-8",
        )?;
        Ok(parse_installed_packages(&stdout, self.descriptor.id()))
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        self.validate_config(config)?;
        let spec = CommandSpec::new(RPM_COMMAND).arg("-qa");
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(self.installed(config).await?.len());
        }

        let stdout = decode_stdout(output, "zypper installed package count is not valid UTF-8")?;
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
        let zypper_path = resolve_executable(config, ZYPPER_COMMAND);
        let spec = search_command(&zypper_path, query);
        let output = run_output(&spec).await?;
        if output.status.code() == Some(104) {
            return Ok(Vec::new());
        }
        if !output.status.success() {
            let tail = command_output_tail(&output.stdout, &output.stderr);
            return Err(zypper_status_error(&spec, output.status, &tail));
        }

        let stdout = decode_stdout(output, "zypper search output is not valid UTF-8")?;
        let installed_versions = self.installed_version_map().await?;
        let mut packages = Vec::new();
        let mut seen = HashSet::new();
        for row in parse_table_rows(&stdout, &search_headers()) {
            let Some(name) = row.get("name") else {
                continue;
            };
            if name.is_empty() || !seen.insert(name.clone()) {
                continue;
            }

            let version = installed_versions
                .get(name)
                .map_or(NOT_INSTALLED_VERSION, String::as_str);
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
                        "zypper package target belongs to another manager",
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
        self.execute_packages_with_progress(config, action, &package_names, |event| {
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

fn rpm_current_version_command(package_name: &str) -> CommandSpec {
    CommandSpec::new(RPM_COMMAND).args([
        OsString::from("-q"),
        OsString::from("--queryformat"),
        OsString::from("%{VERSION}-%{RELEASE}"),
        OsString::from(package_name),
    ])
}

fn search_command(zypper_path: &Path, query: &str) -> CommandSpec {
    CommandSpec::new(zypper_path).env("LC_ALL", "C").args([
        "--non-interactive",
        "search",
        "--details",
        query,
    ])
}

fn list_updates_command(zypper_path: &Path) -> CommandSpec {
    CommandSpec::new(zypper_path)
        .env("LC_ALL", "C")
        .args(["--non-interactive", "list-updates"])
}

fn refresh_command() -> CommandSpec {
    system_helper_command("refresh", "zypper")
}

fn zypper_status_error(spec: &CommandSpec, status: ExitStatus, tail: &str) -> ManagerError {
    let fallback = command_status_error(spec, status, tail);
    let kind = match status.code() {
        Some(5) => ManagerErrorKind::Permission,
        Some(7) => ManagerErrorKind::Busy,
        Some(102) => ManagerErrorKind::RebootRequired,
        Some(105) => ManagerErrorKind::Cancelled,
        Some(106) => ManagerErrorKind::Network,
        Some(103 | 104 | 107) => ManagerErrorKind::Other,
        _ => fallback.kind(),
    };
    let detail = fallback
        .detail()
        .unwrap_or_else(|| fallback.message())
        .to_owned();

    ManagerError::new(kind, "zypper command failed").with_detail(detail)
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
            let size = fields
                .next()
                .map(str::trim)
                .and_then(|value| value.parse::<u64>().ok());
            let install_date = fields
                .next()
                .map(str::trim)
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

fn parse_installed_versions(stdout: &str) -> HashMap<String, String> {
    stdout
        .lines()
        .filter_map(|line| {
            let (name, version) = line.split_once('\t')?;
            let name = name.trim();
            let version = version.trim();
            (!name.is_empty() && !version.is_empty()).then(|| (name.to_owned(), version.to_owned()))
        })
        .collect()
}

fn optional_rpm_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "(none)")
        .map(ToOwned::to_owned)
}

fn update_headers() -> [(&'static str, &'static str); 3] {
    [
        ("name", "name"),
        ("currentversion", "current_version"),
        ("availableversion", "available_version"),
    ]
}

fn search_headers() -> [(&'static str, &'static str); 2] {
    [("name", "name"), ("version", "version")]
}

fn parse_table_rows(
    output: &str,
    required_headers: &[(&'static str, &'static str)],
) -> Vec<HashMap<&'static str, String>> {
    let mut rows = Vec::new();
    let mut header_mapping: Option<HashMap<usize, &'static str>> = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_separator_line(trimmed) || !trimmed.contains('|') {
            continue;
        }

        let columns = split_table_row(trimmed);
        if columns.is_empty() {
            continue;
        }

        if header_mapping.is_none() {
            let normalized_headers = columns
                .iter()
                .enumerate()
                .map(|(index, value)| (index, normalize_header(value)))
                .collect::<Vec<_>>();
            let mut mapping = HashMap::new();
            for (header_name, key_name) in required_headers {
                if let Some((index, _)) = normalized_headers
                    .iter()
                    .find(|(_, normalized)| normalized == header_name)
                {
                    mapping.insert(*index, *key_name);
                }
            }
            if mapping.len() == required_headers.len() {
                header_mapping = Some(mapping);
                continue;
            }
        }

        let Some(mapping) = header_mapping.as_ref() else {
            continue;
        };
        let mut row = HashMap::new();
        for (index, key_name) in mapping {
            let value = columns
                .get(*index)
                .map(|value| value.trim().to_owned())
                .unwrap_or_default();
            row.insert(*key_name, value);
        }
        if !row.values().all(String::is_empty) {
            rows.push(row);
        }
    }

    rows
}

fn split_table_row(line: &str) -> Vec<String> {
    line.split('|').map(|part| part.trim().to_owned()).collect()
}

fn normalize_header(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_separator_line(line: &str) -> bool {
    line.chars()
        .all(|character| matches!(character, '-' | '+' | '=' | '|'))
}

fn command_output_tail(stdout: &[u8], stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    if !stderr.trim().is_empty() {
        return stderr.trim().to_owned();
    }

    String::from_utf8_lossy(stdout).trim().to_owned()
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(unsupported_action_error()),
    }
}

fn unsupported_action_error() -> ManagerError {
    ManagerError::new(
        ManagerErrorKind::Unsupported,
        "zypper action is not supported",
    )
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    use super::*;

    #[test]
    fn write_commands_preserve_zypper_batch_semantics() {
        let manager = ZypperManager::new();
        let config =
            ManagerConfig::new(manager.descriptor().id().clone()).with_executable("/custom/zypper");
        let names = vec!["bash".to_owned(), "curl".to_owned()];

        for (action, command_name) in [
            (PackageAction::Install, "install"),
            (PackageAction::Update, "update"),
            (PackageAction::Uninstall, "remove"),
        ] {
            let command = manager
                .write_command(&config, action, &names)
                .expect("build Zypper write command");
            assert_eq!(command.program(), Path::new("/usr/bin/pkexec"));
            assert_eq!(
                command.arguments(),
                [
                    "/usr/lib/updater/updater-system-helper",
                    command_name,
                    "zypper",
                    "bash",
                    "curl",
                ]
                .map(OsString::from)
                .as_slice()
            );
        }
    }

    #[test]
    fn read_commands_preserve_refresh_and_locale_semantics() {
        let zypper = Path::new("/custom/zypper");
        let search = search_command(zypper, "shell");
        assert_eq!(search.program(), zypper);
        assert_eq!(
            search.arguments(),
            ["--non-interactive", "search", "--details", "shell"]
                .map(OsString::from)
                .as_slice()
        );
        assert_eq!(
            search.environment(),
            [(OsString::from("LC_ALL"), OsString::from("C"))]
        );

        let updates = list_updates_command(zypper);
        assert_eq!(
            updates.arguments(),
            ["--non-interactive", "list-updates"]
                .map(OsString::from)
                .as_slice()
        );
        assert_eq!(updates.environment(), search.environment());

        let refresh = refresh_command();
        assert_eq!(refresh.program(), Path::new("/usr/bin/pkexec"));
        assert_eq!(
            refresh.arguments(),
            [
                "/usr/lib/updater/updater-system-helper",
                "refresh",
                "zypper",
            ]
            .map(OsString::from)
            .as_slice()
        );
    }

    #[test]
    fn parses_rpm_metadata_and_version_map() {
        let id = ManagerId::parse(ZYPPER_ID).expect("valid Zypper ID");
        let installed = parse_installed_packages(
            "bash\t5.2-3.1\tGNU shell\t4096\t0\thttps://gnu.org/bash\n\
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

        let versions = parse_installed_versions("bash\t5.2-3.1\nvim\t9.1-2.1\ninvalid\n");
        assert_eq!(versions.get("vim").map(String::as_str), Some("9.1-2.1"));
    }

    #[test]
    fn parses_reordered_update_and_search_tables() {
        let updates = parse_table_rows(
            "Available Version | Arch | Name | Current Version\n\
             ------------------+------+------|----------------\n\
             5.2-4.1 | x86_64 | bash | 5.2-3.1\n\
             5.2-5.1 | x86_64 | bash | 5.2-4.1\n",
            &update_headers(),
        );
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].get("name").map(String::as_str), Some("bash"));
        assert_eq!(
            updates[0].get("available_version").map(String::as_str),
            Some("5.2-4.1")
        );

        let search = parse_table_rows(
            "S | Name | Type | Version | Repository\n\
             --+------+------|---------|-----------\n\
               | ripgrep | package | 14.1.0-1 | repo-oss\n",
            &search_headers(),
        );
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].get("name").map(String::as_str), Some("ripgrep"));
    }

    #[cfg(unix)]
    #[test]
    fn maps_zypper_exit_codes_without_polluting_shared_classification() {
        let spec = CommandSpec::new("zypper").arg("list-updates");
        for (code, expected) in [
            (5, ManagerErrorKind::Permission),
            (7, ManagerErrorKind::Busy),
            (102, ManagerErrorKind::RebootRequired),
            (103, ManagerErrorKind::Other),
            (104, ManagerErrorKind::Other),
            (105, ManagerErrorKind::Cancelled),
            (106, ManagerErrorKind::Network),
            (107, ManagerErrorKind::Other),
        ] {
            let status = ExitStatus::from_raw(code << 8);
            assert_eq!(
                zypper_status_error(&spec, status, "diagnostic").kind(),
                expected,
                "exit code {code}"
            );
        }
    }

    #[tokio::test]
    async fn empty_execution_does_not_run_zypper() {
        ZypperManager::new()
            .execute_packages_with_progress(
                &ManagerConfig::new(ManagerId::parse(ZYPPER_ID).expect("valid Zypper ID")),
                PackageAction::Install,
                &[],
                |_| {},
            )
            .await
            .expect("execute empty Zypper group");
    }
}
