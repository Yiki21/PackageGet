use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::Path,
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
/// Temporary sync database the privileged helper mirrors and this manager
/// reads. The helper binary in `ui/src/bin/updater-system-helper.rs` defines
/// the same fixed path; the two crates share no constants module.
const PACMAN_SYNC_DATABASE: &str = "/var/lib/updater/pacman-sync";

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

        // Arch has no partial upgrade, so the live sync database may only move
        // together with `-u`. Discovery therefore mirrors the sync databases
        // into a fixed temporary database through the privileged helper and
        // reads that, the way the pacman-contrib `checkupdates` script does.
        // A cancelled or failed authorisation is reported as the refresh
        // failure and never retried against the live database.
        let (refresh_command, spec) =
            update_commands(&pacman_path, refresh, Path::new(PACMAN_SYNC_DATABASE));
        if let Some(refresh_command) = refresh_command {
            run_command_with_progress(&refresh_command, |_| {}).await?;
        }
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
            PackageAction::Install => system_helper_command("install", "pacman")
                .args(package_names.iter().map(OsString::from)),
            // Pacman updates are one `pacman -Syu` system transaction; naming
            // a subset would install packages newer than the rest of the
            // system, which Arch does not support.
            PackageAction::Update => system_helper_command("update", "pacman"),
            PackageAction::Uninstall => system_helper_command("remove", "pacman")
                .args(package_names.iter().map(OsString::from)),
            _ => return Err(unsupported_action_error()),
        };

        Ok(command)
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
        Ok(manager_availability_with_version(
            self.descriptor(),
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

/// Builds the privileged refresh command, when one was requested, and the
/// `-Qu` query that reads the result.
///
/// `sync_database` is a parameter so that both branches can be tested against
/// a temporary path; every caller passes [`PACMAN_SYNC_DATABASE`].
fn update_commands(
    pacman: &Path,
    refresh: bool,
    sync_database: &Path,
) -> (Option<CommandSpec>, CommandSpec) {
    let refresh_command = refresh.then(|| system_helper_command("refresh", "pacman"));
    // Without a refresh the mirror is only read once the helper has created
    // it; until then the live database still answers the query.
    let mirror_exists = refresh || sync_database.join("sync").is_dir();
    let mut query = CommandSpec::new(pacman).arg("-Qu");
    if mirror_exists {
        query = query.arg("--dbpath").arg(sync_database.as_os_str());
    }

    (refresh_command, query)
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

        let install = manager
            .write_command(&config, PackageAction::Install, &names)
            .expect("build Pacman install command");
        assert_eq!(install.program(), Path::new("/usr/bin/pkexec"));
        assert_eq!(
            install.arguments(),
            [
                "/usr/lib/updater/updater-system-helper",
                "install",
                "pacman",
                "bash",
                "curl",
            ]
            .map(OsString::from)
            .as_slice()
        );

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
    fn update_command_is_one_full_system_transaction_without_a_partial_target_list() {
        let manager = PacmanManager::new();
        let config =
            ManagerConfig::new(manager.descriptor().id().clone()).with_executable("/custom/pacman");

        let update = manager
            .write_command(
                &config,
                PackageAction::Update,
                &["bash".to_owned(), "curl".to_owned()],
            )
            .expect("build Pacman update command");
        assert_eq!(update.program(), Path::new("/usr/bin/pkexec"));
        assert!(
            update.is_privileged(),
            "pacman updates run through the privileged helper"
        );
        assert_eq!(
            update.arguments(),
            ["/usr/lib/updater/updater-system-helper", "update", "pacman",]
                .map(OsString::from)
                .as_slice()
        );
    }

    #[test]
    fn refreshed_update_listing_queries_the_mirrored_database() {
        let sync_database = Path::new("/var/lib/updater/pacman-sync");
        let (refresh, query) = update_commands(Path::new("/custom/pacman"), true, sync_database);

        let refresh = refresh.expect("a refreshed listing asks the helper to sync");
        assert_eq!(refresh.program(), Path::new("/usr/bin/pkexec"));
        assert!(refresh.is_privileged());
        assert_eq!(
            refresh.arguments(),
            [
                "/usr/lib/updater/updater-system-helper",
                "refresh",
                "pacman",
            ]
            .map(OsString::from)
            .as_slice()
        );

        assert_eq!(query.program(), Path::new("/custom/pacman"));
        assert_eq!(
            query.arguments(),
            ["-Qu", "--dbpath", "/var/lib/updater/pacman-sync"]
                .map(OsString::from)
                .as_slice()
        );
    }

    #[test]
    fn unrefreshed_update_listing_uses_the_mirror_only_once_it_exists() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let missing = directory.path().join("pacman-sync");
        let (refresh, query) = update_commands(Path::new("/custom/pacman"), false, &missing);

        assert!(refresh.is_none(), "an unrefreshed listing must not sync");
        assert_eq!(query.arguments(), [OsString::from("-Qu")].as_slice());

        std::fs::create_dir_all(missing.join("sync")).expect("create mirrored sync database");
        let (refresh, query) = update_commands(Path::new("/custom/pacman"), false, &missing);

        assert!(refresh.is_none());
        assert_eq!(
            query.arguments(),
            [
                OsString::from("-Qu"),
                OsString::from("--dbpath"),
                missing.as_os_str().to_owned(),
            ]
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
