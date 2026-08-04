use std::{collections::HashMap, ffi::OsString, path::PathBuf, process::Output};

use async_trait::async_trait;
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapabilities,
    ManagerCapability, ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError,
    ManagerErrorKind, ManagerId, ManagerResult, PackageAction, PackageInfo, PackageManager,
    PackageOrigin, PackageScope, PackageTarget, PackageUpdate, Platform, ProgressEvent,
    ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, command_status_error, decode_stdout, manager_availability_with_version,
        require_success, resolve_executable, run_output, system_helper_command,
    },
    progress::run_cancellable_command_with_progress,
};

const XBPS_ID: &str = "builtin:xbps";
const XBPS_QUERY_COMMAND: &str = "xbps-query";
const XBPS_INSTALL_COMMAND: &str = "xbps-install";
const XBPS_REMOVE_COMMAND: &str = "xbps-remove";
const ORIGIN_NAME: &str = "XBPS";
const NOT_INSTALLED_VERSION: &str = "Not Installed";

/// Direct implementation for Void Linux packages managed by XBPS.
#[derive(Debug, Clone)]
pub struct XbpsManager {
    descriptor: ManagerDescriptor,
}

impl XbpsManager {
    /// Creates the built-in XBPS manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(XBPS_ID).expect("XBPS manager ID must remain valid"),
            "XBPS",
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
        .expect("XBPS descriptor must remain valid")
        .with_description("Void Linux system packages managed by XBPS")
        .with_authorization(AuthorizationHint::RequiresElevation {
            message: Some("System package changes require administrator approval.".to_owned()),
        });

        Self { descriptor }
    }

    /// Returns the installed version of one XBPS package.
    ///
    /// # Errors
    ///
    /// Returns a typed command or protocol error when `xbps-query` cannot
    /// identify the requested package.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        package_name: &str,
    ) -> ManagerResult<String> {
        self.validate_config(config)?;
        validate_package_name(package_name)?;
        let query = resolve_executable(config, XBPS_QUERY_COMMAND);
        let spec = CommandSpec::new(query).args(["--property", "pkgver", package_name]);
        let output = require_success(
            &spec,
            run_output(&spec).await?,
            "XBPS package version query failed",
        )?;
        let stdout = decode_stdout(output, "XBPS package version is not valid UTF-8")?;
        let (name, version) = split_pkgver(stdout.trim())
            .ok_or_else(|| protocol("XBPS package version output is malformed", stdout.trim()))?;
        if name != package_name {
            return Err(protocol(
                "XBPS package version output changed package identity",
                stdout.trim(),
            ));
        }
        Ok(version.to_owned())
    }

    async fn installed_packages(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let query = resolve_executable(config, XBPS_QUERY_COMMAND);
        let spec = CommandSpec::new(query).arg("--list-pkgs");
        let output = require_success(
            &spec,
            run_output(&spec).await?,
            "XBPS installed package listing failed",
        )?;
        let stdout = decode_stdout(output, "XBPS installed package listing is not valid UTF-8")?;
        parse_installed(&stdout, self.descriptor.id())
    }

    fn write_command(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        packages: &[PackageTarget],
    ) -> ManagerResult<CommandSpec> {
        self.validate_config(config)?;
        ensure_supported_action(action)?;
        let names = packages
            .iter()
            .map(|target| {
                if &target.manager_id != self.descriptor.id() {
                    return Err(protocol(
                        "XBPS package target belongs to another manager",
                        &target.name,
                    ));
                }
                validate_package_name(&target.name)?;
                if target.version.is_some() {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "version-pinned XBPS operations are not supported",
                    )
                    .with_detail(&target.name));
                }
                if !matches!(target.scope, PackageScope::System | PackageScope::Unknown) {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "XBPS only supports system package scope",
                    )
                    .with_detail(&target.name));
                }
                if let Some(origin) = &target.origin
                    && origin.name != ORIGIN_NAME
                {
                    return Err(protocol("XBPS target origin is not XBPS", &target.name));
                }
                Ok(OsString::from(&target.name))
            })
            .collect::<ManagerResult<Vec<_>>>()?;

        let command = match action {
            PackageAction::Install => system_helper_command("install", "xbps"),
            PackageAction::Update => system_helper_command("update", "xbps"),
            PackageAction::Uninstall => system_helper_command("remove", "xbps"),
            _ => return Err(ManagerError::unsupported(action.capability())),
        };
        Ok(command.args(names))
    }
}

impl Default for XbpsManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for XbpsManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        let query_availability = manager_availability_with_version(
            self.descriptor(),
            config,
            XBPS_QUERY_COMMAND,
            &["--version"],
            detect_xbps_version,
        )
        .await;
        if !query_availability.is_available() {
            return Ok(query_availability);
        }

        for command in [XBPS_INSTALL_COMMAND, XBPS_REMOVE_COMMAND] {
            let path = companion_executable(config, command);
            let mut companion_config = config.clone();
            companion_config.executable = Some(path.clone());
            let availability = manager_availability_with_version(
                self.descriptor(),
                &companion_config,
                command,
                &["--version"],
                detect_xbps_version,
            )
            .await;
            if !availability.is_available() {
                return Ok(remap_companion_absence(availability, path));
            }
        }

        Ok(query_availability)
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
        refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.validate_config(config)?;
        if refresh {
            let command = system_helper_command("refresh", "xbps");
            run_cancellable_command_with_progress(
                &command,
                &updater_manager_api::NoopProgressSink,
                |_| {},
            )
            .await?;
        }

        let installed = self.installed_packages(config).await?;
        let installed = installed
            .into_iter()
            .map(|package| (package.name.clone(), package))
            .collect::<HashMap<_, _>>();
        let install = companion_executable(config, XBPS_INSTALL_COMMAND);
        let spec = CommandSpec::new(install).args(["--update", "--dry-run"]);
        let output = require_success(
            &spec,
            run_output(&spec).await?,
            "XBPS update dry-run failed",
        )?;
        let stdout = decode_stdout(output, "XBPS update dry-run output is not valid UTF-8")?;
        parse_updates(&stdout, &installed)
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        validate_search_query(query)?;
        let installed = self.installed_packages(config).await?;
        let installed = installed
            .into_iter()
            .map(|package| (package.name.clone(), package.version))
            .collect::<HashMap<_, _>>();
        let executable = resolve_executable(config, XBPS_QUERY_COMMAND);
        let spec = CommandSpec::new(executable).args(["--repository", "--search", query]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            let detail = command_output_detail(&output);
            return Err(command_status_error(&spec, output.status, &detail));
        }
        let stdout = decode_stdout(output, "XBPS search output is not valid UTF-8")?;
        parse_search(&stdout, self.descriptor.id(), &installed)
    }

    async fn execute(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        packages: &[PackageTarget],
        progress: &dyn ProgressSink,
    ) -> ManagerResult<()> {
        let command = self.write_command(config, action, packages)?;
        let total = packages.len();
        progress.emit(ProgressEvent::Started { action, total });
        if packages.is_empty() {
            progress.emit(ProgressEvent::Finished {
                completed: 0,
                total: 0,
            });
            return Ok(());
        }
        run_cancellable_command_with_progress(&command, progress, |event| {
            let (fraction, message) = event.into_parts();
            if let Some(message) = message {
                progress.emit(ProgressEvent::Message { message });
            }
            progress.emit(ProgressEvent::Advanced {
                completed: ((fraction * total as f32).floor() as usize).min(total),
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

fn companion_executable(config: &ManagerConfig, command: &str) -> PathBuf {
    config.executable().map_or_else(
        || resolve_executable(&ManagerConfig::new(config.id.clone()), command),
        |query| {
            query
                .parent()
                .map_or_else(|| PathBuf::from(command), |parent| parent.join(command))
        },
    )
}

fn remap_companion_absence(
    availability: ManagerAvailability,
    path: PathBuf,
) -> ManagerAvailability {
    match availability {
        ManagerAvailability::Unavailable {
            reason: AvailabilityReason::CommandMissing { .. },
        } => ManagerAvailability::Unavailable {
            reason: AvailabilityReason::CommandMissing {
                command: path.to_string_lossy().into_owned(),
            },
        },
        other => other,
    }
}

fn detect_xbps_version(output: &Output) -> Option<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| line.contains("XBPS"))
        .map(ToOwned::to_owned)
}

fn parse_installed(stdout: &str, manager_id: &ManagerId) -> ManagerResult<Vec<PackageInfo>> {
    let mut packages = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let mut fields = line.splitn(3, char::is_whitespace);
        let state = fields.next().unwrap_or_default();
        let pkgver = fields.next().unwrap_or_default().trim();
        let description = fields.next().map(str::trim).unwrap_or_default();
        if state != "ii" {
            continue;
        }
        let (name, version) = split_pkgver(pkgver)
            .ok_or_else(|| protocol("XBPS installed package identity is malformed", pkgver))?;
        validate_package_name(name)?;
        if !seen.insert(name.to_owned()) {
            return Err(protocol(
                "XBPS installed listing contains a duplicate package",
                name,
            ));
        }
        let mut package = PackageInfo::new(manager_id.clone(), name, version);
        package.description = (!description.is_empty()).then(|| description.to_owned());
        package.scope = PackageScope::System;
        package.origin = Some(PackageOrigin::new(ORIGIN_NAME));
        packages.push(package);
    }
    packages.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(packages)
}

fn parse_updates(
    stdout: &str,
    installed: &HashMap<String, PackageInfo>,
) -> ManagerResult<Vec<PackageUpdate>> {
    let mut updates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 6 {
            return Err(protocol("XBPS dry-run transaction row is malformed", line));
        }
        if fields[1] != "update" {
            continue;
        }
        let (name, available_version) = split_pkgver(fields[0])
            .ok_or_else(|| protocol("XBPS update package identity is malformed", fields[0]))?;
        let package = installed.get(name).ok_or_else(|| {
            protocol(
                "XBPS update package is absent from installed inventory",
                name,
            )
        })?;
        if !seen.insert(name.to_owned()) {
            return Err(protocol(
                "XBPS update dry-run contains a duplicate package",
                name,
            ));
        }
        updates.push(PackageUpdate::new(
            package.target(),
            &package.version,
            available_version,
        ));
    }
    Ok(updates)
}

fn parse_search(
    stdout: &str,
    manager_id: &ManagerId,
    installed: &HashMap<String, String>,
) -> ManagerResult<Vec<PackageInfo>> {
    let mut packages = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some(rest) = line
            .strip_prefix("[*]")
            .or_else(|| line.strip_prefix("[-]"))
        else {
            continue;
        };
        let mut fields = rest.trim().splitn(2, char::is_whitespace);
        let pkgver = fields.next().unwrap_or_default();
        let description = fields.next().map(str::trim).unwrap_or_default();
        let (name, _available_version) = split_pkgver(pkgver)
            .ok_or_else(|| protocol("XBPS search package identity is malformed", pkgver))?;
        validate_package_name(name)?;
        if !seen.insert(name.to_owned()) {
            continue;
        }
        let version = installed
            .get(name)
            .map_or(NOT_INSTALLED_VERSION, String::as_str);
        let mut package = PackageInfo::new(manager_id.clone(), name, version);
        package.description = (!description.is_empty()).then(|| description.to_owned());
        package.scope = PackageScope::System;
        package.origin = Some(PackageOrigin::new(ORIGIN_NAME));
        packages.push(package);
    }
    Ok(packages)
}

fn split_pkgver(value: &str) -> Option<(&str, &str)> {
    value.match_indices('-').rev().find_map(|(index, _)| {
        let name = &value[..index];
        let version = &value[index + 1..];
        (!name.is_empty()
            && version
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit()))
        .then_some((name, version))
    })
}

fn validate_package_name(name: &str) -> ManagerResult<()> {
    let valid = (1..=255).contains(&name.len())
        && name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.' | '_')
        });
    if valid {
        Ok(())
    } else {
        Err(protocol("XBPS package name is malformed", name))
    }
}

fn validate_search_query(query: &str) -> ManagerResult<()> {
    if query.starts_with('-') || query.chars().any(char::is_control) {
        Err(protocol("XBPS search query is malformed", query))
    } else {
        Ok(())
    }
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

fn command_output_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        stderr.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::Path};

    use super::*;

    fn manager_id() -> ManagerId {
        ManagerId::parse(XBPS_ID).expect("valid XBPS manager ID")
    }

    #[test]
    fn parses_installed_search_and_update_contracts() {
        let installed = parse_installed(
            "ii base-files-0.142_11 Void Linux base system files\n\
             uu pending-1.0_1 unpacked\n\
             ii xbps-0.60.7_1 XBPS package system utilities\n",
            &manager_id(),
        )
        .expect("parse installed packages");
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].name, "base-files");
        assert_eq!(installed[0].version, "0.142_11");

        let installed_map = installed
            .iter()
            .cloned()
            .map(|package| (package.name.clone(), package))
            .collect::<HashMap<_, _>>();
        let updates = parse_updates(
            "dependency-1.0_1 install x86_64 repo 1 1\n\
             xbps-0.60.8_1 update x86_64 repo 2 2\n",
            &installed_map,
        )
        .expect("parse update transaction");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].current_version, "0.60.7_1");
        assert_eq!(updates[0].available_version, "0.60.8_1");

        let versions = installed_map
            .iter()
            .map(|(name, package)| (name.clone(), package.version.clone()))
            .collect::<HashMap<_, _>>();
        let search = parse_search(
            "[*] xbps-0.60.8_1 XBPS utilities\n[-] xtools-0.70_1 helper tools\n",
            &manager_id(),
            &versions,
        )
        .expect("parse search results");
        assert_eq!(search[0].version, "0.60.7_1");
        assert_eq!(search[1].version, NOT_INSTALLED_VERSION);
    }

    #[test]
    fn package_version_split_keeps_hyphenated_names() {
        assert_eq!(
            split_pkgver("base-files-0.142_11"),
            Some(("base-files", "0.142_11"))
        );
        assert_eq!(split_pkgver("missing-version"), None);
    }

    #[test]
    fn write_commands_use_the_fixed_helper() {
        let manager = XbpsManager::new();
        let config = ManagerConfig::new(manager.descriptor().id().clone());
        let mut target = PackageTarget::new(manager.descriptor().id().clone(), "xbps");
        target.scope = PackageScope::System;
        target.origin = Some(PackageOrigin::new(ORIGIN_NAME));

        for (action, helper_action) in [
            (PackageAction::Install, "install"),
            (PackageAction::Update, "update"),
            (PackageAction::Uninstall, "remove"),
        ] {
            let command = manager
                .write_command(&config, action, std::slice::from_ref(&target))
                .expect("build XBPS command");
            assert_eq!(command.program(), Path::new("/usr/bin/pkexec"));
            assert_eq!(
                command.arguments(),
                [
                    "/usr/lib/updater/updater-system-helper",
                    helper_action,
                    "xbps",
                    "xbps",
                ]
                .map(OsString::from)
                .as_slice()
            );
        }
    }
}
