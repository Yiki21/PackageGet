use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    process::Output,
};

use async_trait::async_trait;
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId,
    ManagerResult, PackageAction, PackageInfo, PackageManager, PackageScope, PackageTarget,
    PackageUpdate, Platform, ProgressEvent, ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, command_status_error, decode_stdout, manager_availability_with_version,
        require_success, resolve_executable, run_output, system_helper_command,
    },
    progress::{CommandProgress, run_cancellable_command_with_progress, run_command_with_progress},
};

const PACMAN_ID: &str = "builtin:pacman";
const PACMAN_COMMAND: &str = "pacman";
const NOT_INSTALLED_VERSION: &str = "Not Installed";

/// Direct `updater-manager-api` implementation for Pacman.
#[derive(Debug, Clone)]
pub struct PacmanManager {
    descriptor: ManagerDescriptor,
}

impl PacmanManager {
    /// Creates the built-in Pacman manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(PACMAN_ID).expect("Pacman manager ID must remain valid"),
            "Pacman",
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
        .expect("Pacman descriptor must remain valid")
        .with_description("Arch Linux 系统包管理器")
        .with_authorization(AuthorizationHint::RequiresElevation {
            message: Some("System package changes require administrator approval.".to_owned()),
        });

        Self { descriptor }
    }

    /// Returns the installed version of one Pacman package.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when Pacman cannot identify the package or its
    /// output is malformed. Command startup failures retain their typed command
    /// classification.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        package_name: &str,
    ) -> ManagerResult<String> {
        self.validate_config(config)?;
        let pacman_path = resolve_executable(config, PACMAN_COMMAND);
        let spec = CommandSpec::new(pacman_path).args(["-Q", package_name]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "pacman package version is unavailable",
            )
            .with_detail(package_name));
        }

        let stdout = decode_stdout(output, "pacman package version is not valid UTF-8")?;
        parse_installed_line(stdout.trim())
            .map(|(_name, version)| version.to_owned())
            .ok_or_else(|| {
                ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "pacman package version output is malformed",
                )
                .with_detail(package_name)
            })
    }

    /// Executes a Pacman package group while exposing normalized command
    /// progress.
    ///
    /// This compatibility surface lets the existing core dispatcher reuse the
    /// direct implementation until the UI moves to [`PackageManager::execute`].
    ///
    /// # Errors
    ///
    /// Returns a protocol error for a mismatched manager configuration, an
    /// unsupported error for unknown future actions, or a typed command error
    /// when Pacman or `pkexec` fails.
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
        run_command_with_progress(&command, on_progress).await
    }

    async fn list_updates(
        &self,
        config: &ManagerConfig,
        refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.validate_config(config)?;
        let pacman_path = resolve_executable(config, PACMAN_COMMAND);

        if refresh {
            let refresh = refresh_command();
            run_command_with_progress(&refresh, |_| {}).await?;
        }

        let spec = CommandSpec::new(pacman_path).arg("-Qu");
        let output = run_output(&spec).await?;
        if !output.status.success() {
            if output.stdout.iter().all(u8::is_ascii_whitespace)
                && output.stderr.iter().all(u8::is_ascii_whitespace)
            {
                return Ok(Vec::new());
            }

            let tail = command_output_tail(&output.stdout, &output.stderr);
            return Err(command_status_error(&spec, output.status, &tail));
        }

        let stdout = decode_stdout(output, "pacman update listing is not valid UTF-8")?;
        Ok(stdout
            .lines()
            .filter_map(parse_update_entry)
            .map(|entry| {
                let mut target = PackageTarget::new(self.descriptor.id().clone(), entry.name);
                target.scope = PackageScope::System;
                PackageUpdate::new(target, entry.current_version, entry.available_version)
            })
            .collect())
    }

    async fn installed_version_map(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<HashMap<String, String>> {
        let pacman_path = resolve_executable(config, PACMAN_COMMAND);
        let spec = CommandSpec::new(pacman_path).arg("-Q");
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(HashMap::new());
        }

        let stdout = decode_stdout(output, "pacman installed versions are not valid UTF-8")?;
        Ok(parse_installed_versions(&stdout))
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
            PackageAction::Install => system_helper_command("install", "pacman"),
            PackageAction::Update => system_helper_command("update", "pacman"),
            PackageAction::Uninstall => system_helper_command("remove", "pacman"),
            _ => return Err(unsupported_action_error()),
        };

        Ok(command.args(package_names.iter().map(OsString::from)))
    }
}

impl Default for PacmanManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for PacmanManager {
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

        Ok(manager_availability_with_version(
            config,
            PACMAN_COMMAND,
            &["--version"],
            detect_pacman_version,
        )
        .await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let pacman_path = resolve_executable(config, PACMAN_COMMAND);
        let spec = CommandSpec::new(pacman_path).arg("-Q");
        let output = require_success(
            &spec,
            run_output(&spec).await?,
            "pacman installed package listing failed",
        )?;
        let stdout = decode_stdout(
            output,
            "pacman installed package listing is not valid UTF-8",
        )?;
        Ok(parse_installed_packages(&stdout, self.descriptor.id()))
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        self.validate_config(config)?;
        let pacman_path = resolve_executable(config, PACMAN_COMMAND);
        let spec = CommandSpec::new(pacman_path).arg("-Qq");
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(self.installed(config).await?.len());
        }

        let stdout = decode_stdout(output, "pacman installed package count is not valid UTF-8")?;
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
        let pacman_path = resolve_executable(config, PACMAN_COMMAND);
        let spec = CommandSpec::new(pacman_path).args(["-Ss", query]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = decode_stdout(output, "pacman search output is not valid UTF-8")?;
        let installed_versions = self.installed_version_map(config).await?;
        Ok(parse_search_entries(&stdout)
            .into_iter()
            .map(|entry| {
                let version = installed_versions
                    .get(&entry.name)
                    .map_or(NOT_INSTALLED_VERSION, String::as_str);
                let mut package =
                    PackageInfo::new(self.descriptor.id().clone(), entry.name, version);
                package.description = entry.description;
                package.scope = PackageScope::System;
                package
            })
            .collect())
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
                        "pacman package target belongs to another manager",
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
        run_cancellable_command_with_progress(&command, progress, |event| {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateEntry {
    name: String,
    current_version: String,
    available_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchEntry {
    name: String,
    available_version: String,
    description: Option<String>,
}

fn parse_installed_line(line: &str) -> Option<(&str, &str)> {
    let mut fields = line.split_whitespace();
    let name = fields.next()?;
    let version = fields.next()?;
    (!name.is_empty() && !version.is_empty()).then_some((name, version))
}

fn detect_pacman_version(output: &Output) -> Option<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| line.contains("Pacman v"))
        .map(ToOwned::to_owned)
}

fn parse_installed_packages(stdout: &str, manager_id: &ManagerId) -> Vec<PackageInfo> {
    stdout
        .lines()
        .filter_map(parse_installed_line)
        .map(|(name, version)| {
            let mut package = PackageInfo::new(manager_id.clone(), name, version);
            package.scope = PackageScope::System;
            package
        })
        .collect()
}

fn parse_installed_versions(stdout: &str) -> HashMap<String, String> {
    stdout
        .lines()
        .filter_map(parse_installed_line)
        .map(|(name, version)| (name.to_owned(), version.to_owned()))
        .collect()
}

fn parse_update_entry(line: &str) -> Option<UpdateEntry> {
    let mut fields = line.split_whitespace();
    let name = fields.next()?;
    let current_version = fields.next()?;
    if fields.next()? != "->" {
        return None;
    }
    let available_version = fields.next()?;

    Some(UpdateEntry {
        name: name.to_owned(),
        current_version: current_version.to_owned(),
        available_version: available_version.to_owned(),
    })
}

fn parse_search_entries(stdout: &str) -> Vec<SearchEntry> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut lines = stdout.lines().peekable();

    while let Some(line) = lines.next() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.starts_with([' ', '\t']) {
            continue;
        }

        let mut fields = line.split_whitespace();
        let Some(repository_and_name) = fields.next() else {
            continue;
        };
        let Some(available_version) = fields.next() else {
            continue;
        };
        let Some((_repository, name)) = repository_and_name.split_once('/') else {
            continue;
        };
        if !seen.insert(name.to_owned()) {
            continue;
        }

        let description = lines
            .next_if(|next| next.starts_with([' ', '\t']))
            .map(str::trim)
            .filter(|description| !description.is_empty())
            .map(ToOwned::to_owned);
        entries.push(SearchEntry {
            name: name.to_owned(),
            available_version: available_version.to_owned(),
            description,
        });
    }

    entries
}

fn refresh_command() -> CommandSpec {
    system_helper_command("refresh", "pacman")
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
        "pacman action is not supported",
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn write_commands_preserve_pacman_batch_semantics() {
        let manager = PacmanManager::new();
        let config =
            ManagerConfig::new(manager.descriptor().id().clone()).with_executable("/custom/pacman");
        let names = vec!["bash".to_owned(), "curl".to_owned()];

        for action in [PackageAction::Install, PackageAction::Update] {
            let command = manager
                .write_command(&config, action, &names)
                .expect("build Pacman sync command");
            assert_eq!(command.program(), Path::new("/usr/bin/pkexec"));
            let action_name = match action {
                PackageAction::Install => "install",
                PackageAction::Update => "update",
                _ => unreachable!(),
            };
            assert_eq!(
                command.arguments(),
                [
                    "/usr/lib/updater/updater-system-helper",
                    action_name,
                    "pacman",
                    "bash",
                    "curl",
                ]
                .map(OsString::from)
                .as_slice()
            );
        }

        let uninstall = manager
            .write_command(&config, PackageAction::Uninstall, &names)
            .expect("build Pacman uninstall command");
        assert_eq!(
            uninstall.arguments(),
            [
                "/usr/lib/updater/updater-system-helper",
                "remove",
                "pacman",
                "bash",
                "curl",
            ]
            .map(OsString::from)
            .as_slice()
        );
    }

    #[test]
    fn refresh_command_preserves_database_sync_semantics() {
        let command = refresh_command();
        assert_eq!(command.program(), Path::new("/usr/bin/pkexec"));
        assert_eq!(
            command.arguments(),
            [
                "/usr/lib/updater/updater-system-helper",
                "refresh",
                "pacman",
            ]
            .map(OsString::from)
            .as_slice()
        );
    }

    #[test]
    fn parses_installed_update_and_search_outputs() {
        let id = ManagerId::parse(PACMAN_ID).expect("valid Pacman ID");
        let installed = parse_installed_packages("bash 5.2.037-1\ncurl 8.15.0-1\ninvalid\n", &id);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].name, "bash");
        assert_eq!(installed[0].version, "5.2.037-1");
        assert_eq!(installed[0].scope, PackageScope::System);

        let versions = parse_installed_versions("bash 5.2.037-1\ncurl 8.15.0-1\n");
        assert_eq!(versions.get("curl").map(String::as_str), Some("8.15.0-1"));

        assert_eq!(
            parse_update_entry("linux 6.8.9.arch1-1 -> 6.8.10.arch1-1"),
            Some(UpdateEntry {
                name: "linux".to_owned(),
                current_version: "6.8.9.arch1-1".to_owned(),
                available_version: "6.8.10.arch1-1".to_owned(),
            })
        );
        assert!(parse_update_entry("linux 6.8.9.arch1-1 6.8.10.arch1-1").is_none());

        let search = parse_search_entries(
            "core/bash 5.2.037-1\n    The GNU Bourne Again shell\n\
             extra/fzf 0.65.0-1\n    Command-line fuzzy finder\n\
             testing/bash 5.3-1\n    Duplicate package\n",
        );
        assert_eq!(search.len(), 2);
        assert_eq!(search[0].name, "bash");
        assert_eq!(search[0].available_version, "5.2.037-1");
        assert_eq!(
            search[0].description.as_deref(),
            Some("The GNU Bourne Again shell")
        );
        assert_eq!(search[1].name, "fzf");
    }

    #[tokio::test]
    async fn empty_execution_does_not_run_pacman() {
        PacmanManager::new()
            .execute_packages_with_progress(
                &ManagerConfig::new(ManagerId::parse(PACMAN_ID).expect("valid Pacman ID")),
                PackageAction::Install,
                &[],
                |_| {},
            )
            .await
            .expect("execute empty Pacman group");
    }
}
