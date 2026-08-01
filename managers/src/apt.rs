use std::{collections::HashMap, ffi::OsString};

use async_trait::async_trait;
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId,
    ManagerResult, PackageAction, PackageInfo, PackageManager, PackageScope, PackageTarget,
    PackageUpdate, Platform, ProgressEvent, ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, decode_stdout, manager_availability, require_success, resolve_executable,
        run_output, system_helper_command,
    },
    progress::{CommandProgress, run_command_with_progress},
};

const APT_ID: &str = "builtin:apt";
const APT_COMMAND: &str = "apt";
const APT_CACHE_COMMAND: &str = "apt-cache";
const DPKG_QUERY_COMMAND: &str = "dpkg-query";
const NOT_INSTALLED_VERSION: &str = "Not Installed";

/// Direct `updater-manager-api` implementation for APT.
#[derive(Debug, Clone)]
pub struct AptManager {
    descriptor: ManagerDescriptor,
}

impl AptManager {
    /// Creates the built-in APT manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(APT_ID).expect("APT manager ID must remain valid"),
            "APT",
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
        .expect("APT descriptor must remain valid")
        .with_description("Debian/Ubuntu 系统包管理器")
        .with_authorization(AuthorizationHint::RequiresElevation {
            message: Some("System package changes require administrator approval.".to_owned()),
        });

        Self { descriptor }
    }

    /// Returns the installed version of one package.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when `dpkg-query` cannot identify the package
    /// or returns invalid UTF-8. Command startup failures retain their typed
    /// command classification.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        package_name: &str,
    ) -> ManagerResult<String> {
        self.validate_config(config)?;
        let spec = CommandSpec::new(DPKG_QUERY_COMMAND).args([
            OsString::from("-W"),
            OsString::from("-f=${Version}"),
            OsString::from(package_name),
        ]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "APT package version is unavailable",
            )
            .with_detail(package_name));
        }

        let version = decode_stdout(output, "APT package version is not valid UTF-8")?
            .trim()
            .to_owned();
        if version.is_empty() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "APT package version is empty",
            )
            .with_detail(package_name));
        }

        Ok(version)
    }

    /// Executes an APT package group while exposing normalized command progress.
    ///
    /// This compatibility surface lets the existing core dispatcher reuse the
    /// direct implementation until the UI moves to [`PackageManager::execute`].
    ///
    /// # Errors
    ///
    /// Returns a protocol error for a mismatched manager configuration, an
    /// unsupported error for unknown future actions, or a typed command error
    /// when APT or `pkexec` fails.
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
        run_command_with_progress(&command, on_progress).await
    }

    async fn list_updates(
        &self,
        config: &ManagerConfig,
        refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.validate_config(config)?;
        let apt_path = resolve_executable(config, APT_COMMAND);

        if refresh {
            let refresh = system_helper_command("refresh", "apt");
            run_command_with_progress(&refresh, |_| {}).await?;
        }

        let spec = CommandSpec::new(&apt_path).args(["list", "--upgradable"]);
        let output = require_success(&spec, run_output(&spec).await?, "APT update listing failed")?;
        let stdout = decode_stdout(output, "APT update listing is not valid UTF-8")?;
        let mut updates = Vec::new();

        for line in stdout.lines() {
            let Some((name, mut current_version, available_version)) = parse_upgradable_line(line)
            else {
                continue;
            };

            if current_version == "unknown" {
                current_version = self
                    .current_version(config, &name)
                    .await
                    .unwrap_or_else(|_| "unknown".to_owned());
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
            CommandSpec::new(DPKG_QUERY_COMMAND).args(["-W", "-f=${binary:Package}\t${Version}\n"]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(HashMap::new());
        }

        let stdout = decode_stdout(output, "APT installed versions are not valid UTF-8")?;
        Ok(parse_installed_versions(&stdout))
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            return Ok(());
        }

        Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "APT configuration ID does not match the manager",
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
            PackageAction::Install => system_helper_command("install", "apt"),
            PackageAction::Update => system_helper_command("update", "apt"),
            PackageAction::Uninstall => system_helper_command("remove", "apt"),
            _ => return Err(unsupported_action_error()),
        };

        Ok(command.args(package_names.iter().map(OsString::from)))
    }
}

impl Default for AptManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for AptManager {
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

        Ok(manager_availability(config, APT_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let spec = CommandSpec::new(DPKG_QUERY_COMMAND).args([
            "-W",
            "-f=${binary:Package}\t${Version}\t${binary:Summary}\n",
        ]);
        let output = require_success(
            &spec,
            run_output(&spec).await?,
            "APT installed package listing failed",
        )?;
        let stdout = decode_stdout(output, "APT installed package listing is not valid UTF-8")?;
        Ok(parse_installed_packages(&stdout, self.descriptor.id()))
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        self.validate_config(config)?;
        let spec = CommandSpec::new(DPKG_QUERY_COMMAND).args(["-W", "-f=${binary:Package}\n"]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(self.installed(config).await?.len());
        }

        let stdout = decode_stdout(output, "APT installed package count is not valid UTF-8")?;
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
        let spec = CommandSpec::new(APT_CACHE_COMMAND).args(["search", query]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = decode_stdout(output, "APT search output is not valid UTF-8")?;
        let installed_versions = self.installed_version_map().await?;
        Ok(parse_search_results(
            &stdout,
            &installed_versions,
            self.descriptor.id(),
        ))
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
                        "APT package target belongs to another manager",
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

            let description = fields
                .next()
                .map(str::trim)
                .filter(|description| !description.is_empty())
                .map(ToOwned::to_owned);
            let mut package = PackageInfo::new(manager_id.clone(), name, version);
            package.description = description;
            package.scope = PackageScope::System;
            Some(package)
        })
        .collect()
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(unsupported_action_error()),
    }
}

fn unsupported_action_error() -> ManagerError {
    ManagerError::new(ManagerErrorKind::Unsupported, "APT action is not supported")
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

fn parse_search_results(
    stdout: &str,
    installed_versions: &HashMap<String, String>,
    manager_id: &ManagerId,
) -> Vec<PackageInfo> {
    stdout
        .lines()
        .filter_map(|line| {
            let (name, description) = line.split_once(" - ")?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }

            let version = installed_versions
                .get(name)
                .map_or(NOT_INSTALLED_VERSION, String::as_str);
            let mut package = PackageInfo::new(manager_id.clone(), name, version);
            package.description = Some(description.trim().to_owned());
            package.scope = PackageScope::System;
            Some(package)
        })
        .collect()
}

fn parse_upgradable_line(line: &str) -> Option<(String, String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("Listing...") {
        return None;
    }

    let (name, rest) = line.split_once('/')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let mut fields = rest.split_whitespace();
    let _distribution = fields.next()?;
    let available_version = fields.next()?.to_owned();
    let current_version = parse_upgradable_from(line).unwrap_or_else(|| "unknown".to_owned());

    Some((name.to_owned(), current_version, available_version))
}

fn parse_upgradable_from(line: &str) -> Option<String> {
    let marker = "[upgradable from: ";
    let start = line.find(marker)? + marker.len();
    let end = line[start..].find(']')? + start;
    let value = line[start..end].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn write_commands_preserve_apt_batch_semantics() {
        let manager = AptManager::new();
        let config =
            ManagerConfig::new(manager.descriptor().id().clone()).with_executable("/custom/apt");
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
                "apt",
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
                "apt",
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
                "apt",
                "bash",
                "curl",
            ]
            .map(OsString::from)
            .as_slice()
        );
    }

    #[test]
    fn parses_installed_search_and_update_outputs() {
        let id = ManagerId::parse(APT_ID).expect("valid APT ID");
        let installed =
            parse_installed_packages("bash\t5.2\tGNU shell\ncurl\t8.0\t\ninvalid\n", &id);
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].name, "bash");
        assert_eq!(installed[0].description.as_deref(), Some("GNU shell"));
        assert_eq!(installed[1].description, None);
        assert_eq!(installed[0].scope, PackageScope::System);

        let versions = parse_installed_versions("bash\t5.2\ninvalid\ncurl\t8.0\n");
        let search = parse_search_results("bash - GNU shell\nvim - editor\n", &versions, &id);
        assert_eq!(search[0].version, "5.2");
        assert_eq!(search[1].version, NOT_INSTALLED_VERSION);

        let update = parse_upgradable_line("bash/jammy-updates 5.2.1 amd64 [upgradable from: 5.2]")
            .expect("parse update");
        assert_eq!(
            update,
            ("bash".to_owned(), "5.2".to_owned(), "5.2.1".to_owned())
        );
    }

    #[test]
    fn update_parser_handles_missing_current_version_and_headers() {
        assert_eq!(
            parse_upgradable_line("vim/stable 2:9.1.1234 amd64"),
            Some((
                "vim".to_owned(),
                "unknown".to_owned(),
                "2:9.1.1234".to_owned(),
            ))
        );
        assert_eq!(parse_upgradable_line("Listing... Done"), None);
        assert_eq!(parse_upgradable_line("malformed"), None);
    }
}
