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

const PORTAGE_ID: &str = "builtin:portage";
const EMERGE_COMMAND: &str = "emerge";
const QLIST_COMMAND: &str = "qlist";
const ORIGIN_NAME: &str = "Portage";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const QLIST_FORMAT: &str = "%[CATEGORY]/%[PN]\t%[PVR]\t%[SLOT]\t%[REPO]";

/// Direct implementation for Gentoo packages managed by Portage.
#[derive(Debug, Clone)]
pub struct PortageManager {
    descriptor: ManagerDescriptor,
}

impl PortageManager {
    /// Creates the built-in Portage manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(PORTAGE_ID).expect("Portage manager ID must remain valid"),
            "Portage",
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
        .expect("Portage descriptor must remain valid")
        .with_description("Gentoo system packages managed by Portage")
        .with_authorization(AuthorizationHint::RequiresElevation {
            message: Some("System package changes require administrator approval.".to_owned()),
        });

        Self { descriptor }
    }

    /// Returns the installed version for one SLOT-qualified Portage identity.
    ///
    /// # Errors
    ///
    /// Returns a typed command or protocol error when `qlist` cannot identify
    /// exactly one installed package.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        package_name: &str,
    ) -> ManagerResult<String> {
        self.validate_config(config)?;
        let identity = parse_identity(package_name)?;
        let qlist = companion_executable(config, QLIST_COMMAND);
        let spec = qlist_command(&qlist).arg(package_name);
        let output = require_success(
            &spec,
            run_output(&spec).await?,
            "Portage package version query failed",
        )?;
        let stdout = decode_stdout(output, "Portage package version is not valid UTF-8")?;
        let packages = parse_installed(&stdout, self.descriptor.id())?;
        match packages.as_slice() {
            [package] if package.name == identity.render() => Ok(package.version.clone()),
            [] => Err(protocol(
                "Portage installed package is unavailable",
                package_name,
            )),
            _ => Err(protocol(
                "Portage installed identity matched multiple packages",
                package_name,
            )),
        }
    }

    async fn installed_packages(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let qlist = companion_executable(config, QLIST_COMMAND);
        let spec = qlist_command(&qlist);
        let output = require_success(
            &spec,
            run_output(&spec).await?,
            "Portage installed package listing failed",
        )?;
        let stdout = decode_stdout(
            output,
            "Portage installed package listing is not valid UTF-8",
        )?;
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
        let atoms = packages
            .iter()
            .map(|target| {
                if &target.manager_id != self.descriptor.id() {
                    return Err(protocol(
                        "Portage package target belongs to another manager",
                        &target.name,
                    ));
                }
                let identity = match action {
                    PackageAction::Install if !target.name.contains(':') => {
                        validate_package(&target.name)?;
                        None
                    }
                    PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => {
                        Some(parse_identity(&target.name)?)
                    }
                    _ => return Err(ManagerError::unsupported(action.capability())),
                };
                if target.version.is_some() {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "version-pinned Portage operations are not supported",
                    )
                    .with_detail(&target.name));
                }
                if !matches!(target.scope, PackageScope::System | PackageScope::Unknown) {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "Portage only supports system package scope",
                    )
                    .with_detail(&target.name));
                }
                let repository = target
                    .origin
                    .as_ref()
                    .map(|origin| validate_origin(origin, identity.as_ref(), &target.name))
                    .transpose()?
                    .flatten();
                let atom = repository.map_or_else(
                    || target.name.clone(),
                    |repository| format!("{}::{repository}", target.name),
                );
                Ok(OsString::from(atom))
            })
            .collect::<ManagerResult<Vec<_>>>()?;

        let command = match action {
            PackageAction::Install => system_helper_command("install", "portage"),
            PackageAction::Update => system_helper_command("update", "portage"),
            PackageAction::Uninstall => system_helper_command("remove", "portage"),
            _ => return Err(ManagerError::unsupported(action.capability())),
        };
        Ok(command.args(atoms))
    }
}

impl Default for PortageManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for PortageManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        let emerge_availability = manager_availability_with_version(
            self.descriptor(),
            config,
            EMERGE_COMMAND,
            &["--version"],
            detect_portage_version,
        )
        .await;
        if !emerge_availability.is_available() {
            return Ok(emerge_availability);
        }

        let qlist = companion_executable(config, QLIST_COMMAND);
        let mut qlist_config = config.clone();
        qlist_config.executable = Some(qlist.clone());
        let availability = manager_availability_with_version(
            self.descriptor(),
            &qlist_config,
            QLIST_COMMAND,
            &["--version"],
            detect_qlist_version,
        )
        .await;
        if availability.is_available() {
            Ok(emerge_availability)
        } else {
            Ok(remap_companion_absence(availability, qlist))
        }
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
            let command = system_helper_command("refresh", "portage");
            run_cancellable_command_with_progress(
                &command,
                &updater_manager_api::NoopProgressSink,
                |_| {},
            )
            .await?;
        }

        let installed = self.installed_packages(config).await?;
        let emerge = resolve_executable(config, EMERGE_COMMAND);
        let spec = CommandSpec::new(emerge).args([
            "--pretend",
            "--update",
            "--deep",
            "--newuse",
            "--with-bdeps=y",
            "--package-moves=n",
            "--color=n",
            "--quiet",
            "--verbose",
            "@world",
        ]);
        let output = require_success(
            &spec,
            run_output(&spec).await?,
            "Portage update pretend failed",
        )?;
        let stdout = decode_stdout(output, "Portage update output is not valid UTF-8")?;
        parse_updates(&stdout, &installed)
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        validate_search_query(query)?;
        let emerge = resolve_executable(config, EMERGE_COMMAND);
        let spec =
            CommandSpec::new(emerge).args(["--search", "--package-moves=n", "--color=n", query]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            let detail = command_output_detail(&output);
            return Err(command_status_error(&spec, output.status, &detail));
        }
        let stdout = decode_stdout(output, "Portage search output is not valid UTF-8")?;
        parse_search(&stdout, self.descriptor.id())
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct PortageIdentity {
    package: String,
    slot: String,
}

impl PortageIdentity {
    fn render(&self) -> String {
        format!("{}:{}", self.package, self.slot)
    }
}

fn qlist_command(executable: &std::path::Path) -> CommandSpec {
    CommandSpec::new(executable).args(["--installed", "--format", QLIST_FORMAT])
}

fn companion_executable(config: &ManagerConfig, command: &str) -> PathBuf {
    config.executable().map_or_else(
        || resolve_executable(&ManagerConfig::new(config.id.clone()), command),
        |emerge| {
            emerge
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

fn detect_portage_version(output: &Output) -> Option<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| line.starts_with("Portage "))
        .map(ToOwned::to_owned)
}

fn detect_qlist_version(output: &Output) -> Option<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| line.starts_with("portage-utils-"))
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
        let fields = line.split('\t').collect::<Vec<_>>();
        let [package, version, slot, repository] = fields.as_slice() else {
            return Err(protocol("Portage qlist row is malformed", line));
        };
        validate_package(package)?;
        validate_version(version)?;
        validate_slot(slot)?;
        validate_repository(repository)?;
        let identity = PortageIdentity {
            package: (*package).to_owned(),
            slot: (*slot).to_owned(),
        };
        let name = identity.render();
        if !seen.insert(name.clone()) {
            return Err(protocol(
                "Portage installed listing contains a duplicate package SLOT",
                &name,
            ));
        }
        let mut info = PackageInfo::new(manager_id.clone(), name, *version);
        info.scope = PackageScope::System;
        info.origin = Some(
            PackageOrigin::new(ORIGIN_NAME)
                .with_reference(format!("repo:{repository};slot:{slot}")),
        );
        packages.push(info);
    }
    packages.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(packages)
}

fn parse_updates(stdout: &str, installed: &[PackageInfo]) -> ManagerResult<Vec<PackageUpdate>> {
    let mut installed_by_package = HashMap::<String, Vec<&PackageInfo>>::new();
    for package in installed {
        let identity = parse_identity(&package.name)?;
        installed_by_package
            .entry(identity.package)
            .or_default()
            .push(package);
    }

    let mut updates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in stdout.lines().map(str::trim_start) {
        let Some(attributes_end) = line.find(']') else {
            continue;
        };
        let attributes = line.get(1..attributes_end).unwrap_or_default();
        if !(attributes.starts_with("ebuild") || attributes.starts_with("binary"))
            || (!attributes.contains('U') && !attributes.contains('D'))
        {
            continue;
        }
        let remainder = line[attributes_end + 1..].trim_start();
        let Some(available_cpv) = remainder.split_whitespace().next() else {
            continue;
        };
        let (package, available_version) = split_cpv(available_cpv)?;
        let Some(old_versions) = bracketed_old_versions(remainder) else {
            return Err(protocol(
                "Portage update row is missing its installed version",
                line,
            ));
        };
        let old_versions = old_versions
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if old_versions.is_empty() {
            return Err(protocol(
                "Portage update row has an empty installed version",
                line,
            ));
        }
        for version in &old_versions {
            validate_version(version)?;
        }

        let candidates = installed_by_package.get(package).ok_or_else(|| {
            protocol(
                "Portage update package is absent from installed inventory",
                package,
            )
        })?;
        let matching = candidates
            .iter()
            .filter(|candidate| old_versions.contains(&candidate.version.as_str()))
            .copied()
            .collect::<Vec<_>>();
        let [candidate] = matching.as_slice() else {
            return Err(protocol(
                "Portage update version does not identify exactly one installed SLOT",
                line,
            ));
        };
        let current_version = candidate.version.as_str();
        if !seen.insert(candidate.name.clone()) {
            return Err(protocol(
                "Portage update output contains a duplicate package SLOT",
                &candidate.name,
            ));
        }
        updates.push(PackageUpdate::new(
            candidate.target(),
            current_version,
            available_version,
        ));
    }
    Ok(updates)
}

fn bracketed_old_versions(value: &str) -> Option<&str> {
    let start = value.find('[')?;
    let end = value[start + 1..].find(']')? + start + 1;
    Some(&value[start + 1..end])
}

fn parse_search(stdout: &str, manager_id: &ManagerId) -> ManagerResult<Vec<PackageInfo>> {
    let lines = stdout.lines().collect::<Vec<_>>();
    let mut packages = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        let Some(header) = line.strip_prefix("*") else {
            index += 1;
            continue;
        };
        let header = header.trim();
        let masked = header.ends_with("[ Masked ]");
        let package = header.trim_end_matches("[ Masked ]").trim();
        if masked || !package.contains('/') {
            index += 1;
            continue;
        }
        validate_package(package)?;
        let mut available = None;
        let mut installed = None;
        let mut homepage = None;
        let mut description = None;
        index += 1;
        while index < lines.len() {
            let field = lines[index].trim();
            if field.starts_with('*') || field.starts_with("[ Applications found") {
                break;
            }
            if let Some(value) = field.strip_prefix("Latest version available:") {
                available = Some(value.trim().to_owned());
            } else if let Some(value) = field.strip_prefix("Latest version installed:") {
                installed = Some(value.trim().to_owned());
            } else if let Some(value) = field.strip_prefix("Homepage:") {
                homepage = value.split_whitespace().next().map(ToOwned::to_owned);
            } else if let Some(value) = field.strip_prefix("Description:") {
                let value = value.trim();
                description = (!value.is_empty()).then(|| value.to_owned());
            }
            index += 1;
        }
        let Some(available) = available else {
            continue;
        };
        validate_version(&available)?;
        if !seen.insert(package.to_owned()) {
            continue;
        }
        let installed = installed
            .filter(|value| value != "[ Not Installed ]")
            .unwrap_or_else(|| NOT_INSTALLED_VERSION.to_owned());
        let mut info = PackageInfo::new(manager_id.clone(), package, installed);
        info.description = description;
        info.homepage = homepage;
        info.scope = PackageScope::System;
        info.origin = Some(PackageOrigin::new(ORIGIN_NAME));
        packages.push(info);
    }
    Ok(packages)
}

fn split_cpv(value: &str) -> ManagerResult<(&str, &str)> {
    validate_package_version(value)?;
    value
        .match_indices('-')
        .rev()
        .find_map(|(index, _)| {
            let package = &value[..index];
            let version = &value[index + 1..];
            (package.contains('/')
                && version
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit()))
            .then_some((package, version))
        })
        .ok_or_else(|| protocol("Portage category/package-version is malformed", value))
}

fn parse_identity(value: &str) -> ManagerResult<PortageIdentity> {
    let (package, slot) = value
        .split_once(':')
        .ok_or_else(|| protocol("Portage package identity is missing SLOT", value))?;
    if slot.contains(':') {
        return Err(protocol(
            "Portage package identity has multiple SLOTs",
            value,
        ));
    }
    validate_package(package)?;
    validate_slot(slot)?;
    Ok(PortageIdentity {
        package: package.to_owned(),
        slot: slot.to_owned(),
    })
}

fn validate_package(value: &str) -> ManagerResult<()> {
    let mut parts = value.split('/');
    let category = parts.next().unwrap_or_default();
    let package = parts.next().unwrap_or_default();
    let valid = parts.next().is_none()
        && valid_portage_component(category)
        && valid_portage_component(package);
    if valid {
        Ok(())
    } else {
        Err(protocol("Portage category/package is malformed", value))
    }
}

fn validate_package_version(value: &str) -> ManagerResult<()> {
    if value.len() > 512
        || value.is_empty()
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        Err(protocol("Portage package version is malformed", value))
    } else {
        Ok(())
    }
}

fn valid_portage_component(value: &str) -> bool {
    (1..=255).contains(&value.len())
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.' | '_')
        })
}

fn validate_version(value: &str) -> ManagerResult<()> {
    let valid = (1..=255).contains(&value.len())
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.' | '_')
        });
    if valid {
        Ok(())
    } else {
        Err(protocol("Portage version is malformed", value))
    }
}

fn validate_slot(value: &str) -> ManagerResult<()> {
    let valid = (1..=128).contains(&value.len())
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.' | '_')
        });
    if valid {
        Ok(())
    } else {
        Err(protocol("Portage SLOT is malformed", value))
    }
}

fn validate_repository(value: &str) -> ManagerResult<()> {
    if valid_portage_component(value) {
        Ok(())
    } else {
        Err(protocol("Portage repository is malformed", value))
    }
}

fn validate_origin<'a>(
    origin: &'a PackageOrigin,
    identity: Option<&PortageIdentity>,
    package_name: &str,
) -> ManagerResult<Option<&'a str>> {
    if origin.name != ORIGIN_NAME {
        return Err(protocol(
            "Portage target origin is not Portage",
            package_name,
        ));
    }
    if let Some(reference) = &origin.reference {
        let (repository, slot) = reference
            .strip_prefix("repo:")
            .and_then(|value| value.split_once(";slot:"))
            .ok_or_else(|| protocol("Portage target origin is malformed", reference))?;
        validate_repository(repository)?;
        let Some(identity) = identity else {
            return Err(protocol(
                "unqualified Portage install target cannot carry SLOT origin",
                reference,
            ));
        };
        if slot != identity.slot {
            return Err(protocol(
                "Portage target origin SLOT does not match package identity",
                reference,
            ));
        }
        return Ok(Some(repository));
    }
    Ok(None)
}

fn validate_search_query(query: &str) -> ManagerResult<()> {
    if query.starts_with('-') || query.chars().any(char::is_control) {
        Err(protocol("Portage search query is malformed", query))
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
        ManagerId::parse(PORTAGE_ID).expect("valid Portage manager ID")
    }

    #[test]
    fn qlist_parser_preserves_slot_and_repository_identity() {
        let packages = parse_installed(
            "dev-lang/python\t3.13.14\t3.13\tgentoo\n\
             dev-lang/python\t3.14.6_p1\t3.14\tgentoo\n",
            &manager_id(),
        )
        .expect("parse qlist output");
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "dev-lang/python:3.13");
        assert_eq!(packages[1].name, "dev-lang/python:3.14");
        assert_ne!(packages[0].target(), packages[1].target());
    }

    #[test]
    fn update_parser_only_exposes_version_transitions() {
        let installed = parse_installed(
            "dev-python/librt\t0.12.0\t0\tgentoo\n\
             sys-apps/file\t5.48\t0\tgentoo\n",
            &manager_id(),
        )
        .expect("parse installed inventory");
        let updates = parse_updates(
            "[ebuild   R   ] sys-apps/file-5.48 USE=python\n\
             [ebuild  N    ] net-libs/nghttp2-1.69.0\n\
             [ebuild     UD] dev-python/librt-0.11.0 [0.12.0] USE=test\n\
             [binary      U] sys-apps/file-5.49 [5.48] USE=python\n",
            &installed,
        )
        .expect("parse Portage updates");
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].target.name, "dev-python/librt:0");
        assert_eq!(updates[0].current_version, "0.12.0");
        assert_eq!(updates[0].available_version, "0.11.0");
        assert_eq!(updates[1].target.name, "sys-apps/file:0");
        assert_eq!(updates[1].available_version, "5.49");
    }

    #[test]
    fn search_parser_rejects_masked_results_and_preserves_metadata() {
        let packages = parse_search(
            "*  app-shells/bash\n\
                   Latest version available: 5.3_p9-r2\n\
                   Latest version installed: 5.3_p9-r2\n\
                   Homepage:      https://example.test/bash other\n\
                   Description:   The standard shell\n\n\
             *  app-shells/masked [ Masked ]\n\
                   Latest version available: 1.0\n\
                   Latest version installed: [ Not Installed ]\n",
            &manager_id(),
        )
        .expect("parse Portage search");
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "app-shells/bash");
        assert_eq!(packages[0].version, "5.3_p9-r2");
        assert_eq!(
            packages[0].homepage.as_deref(),
            Some("https://example.test/bash")
        );
    }

    #[test]
    fn write_commands_preserve_slot_and_repository_qualified_atoms() {
        let manager = PortageManager::new();
        let config = ManagerConfig::new(manager.descriptor().id().clone());
        let mut target =
            PackageTarget::new(manager.descriptor().id().clone(), "dev-lang/python:3.13");
        target.scope = PackageScope::System;
        target.origin =
            Some(PackageOrigin::new(ORIGIN_NAME).with_reference("repo:gentoo;slot:3.13"));

        for (action, helper_action) in [
            (PackageAction::Install, "install"),
            (PackageAction::Update, "update"),
            (PackageAction::Uninstall, "remove"),
        ] {
            let command = manager
                .write_command(&config, action, std::slice::from_ref(&target))
                .expect("build Portage command");
            assert_eq!(command.program(), Path::new("/usr/bin/pkexec"));
            assert_eq!(
                command.arguments(),
                [
                    "/usr/lib/updater/updater-system-helper",
                    helper_action,
                    "portage",
                    "dev-lang/python:3.13::gentoo",
                ]
                .map(OsString::from)
                .as_slice()
            );
        }

        let mut search_target =
            PackageTarget::new(manager.descriptor().id().clone(), "app-shells/bash");
        search_target.scope = PackageScope::System;
        search_target.origin = Some(PackageOrigin::new(ORIGIN_NAME));
        let install = manager
            .write_command(&config, PackageAction::Install, &[search_target])
            .expect("build unqualified Portage install command");
        assert_eq!(
            install.arguments().last(),
            Some(&OsString::from("app-shells/bash"))
        );
    }
}
