use std::{collections::HashMap, path::Path, time::Duration};

use async_trait::async_trait;
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
    progress::run_cancellable_command_with_progress,
};

const SCOOP_ID: &str = "builtin:scoop";
const SCOOP_COMMAND: &str = "scoop";
const ORIGIN_NAME: &str = "Scoop";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

/// Direct implementation for Windows Scoop user and global applications.
#[derive(Debug, Clone)]
pub struct ScoopManager {
    descriptor: ManagerDescriptor,
}

impl ScoopManager {
    /// Creates the built-in Scoop manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(SCOOP_ID).expect("Scoop manager ID must remain valid"),
            "Scoop",
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
        .expect("Scoop descriptor must remain valid")
        .with_description("Windows command-line applications managed by Scoop")
        .with_authorization(AuthorizationHint::None);

        Self { descriptor }
    }

    async fn installed_packages(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let scoop = resolve_executable(config, SCOOP_COMMAND);
        let output = run_success(
            &scoop_command(&scoop).arg("export"),
            "Scoop export timed out",
        )
        .await?;
        let stdout = decode_stdout(output, "Scoop export is not valid UTF-8")?;
        parse_export(&stdout, self.descriptor.id())
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
                "Scoop package target belongs to another manager",
                &target.name,
            ));
        }
        validate_identifier(&target.name)?;
        if target.version.is_some() {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "version-pinned Scoop operations are not supported",
            )
            .with_detail(&target.name));
        }
        let global = match target.scope {
            PackageScope::User => false,
            PackageScope::System => true,
            PackageScope::Unknown => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "Scoop package target is missing its installation scope",
                )
                .with_detail(&target.name));
            }
            PackageScope::Project | _ => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "Scoop project scope is not supported",
                )
                .with_detail(&target.name));
            }
        };
        let reference = target
            .origin
            .as_ref()
            .ok_or_else(|| protocol("Scoop target is missing its bucket origin", &target.name))?;
        validate_origin(reference, &target.name)?;

        let scoop = resolve_executable(config, SCOOP_COMMAND);
        let verb = match action {
            PackageAction::Install => "install",
            PackageAction::Update => "update",
            PackageAction::Uninstall => "uninstall",
            _ => unreachable!("supported actions were checked above"),
        };
        let package_spec = if matches!(action, PackageAction::Install) {
            reference
                .reference
                .as_deref()
                .and_then(bucket_reference)
                .map_or_else(
                    || target.name.clone(),
                    |bucket| format!("{bucket}/{}", target.name),
                )
        } else {
            target.name.clone()
        };
        let mut command = scoop_command(&scoop).arg(verb).arg(package_spec);
        if global {
            command = command.arg("--global");
        }
        Ok(command)
    }
}

impl Default for ScoopManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for ScoopManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(self.descriptor(), config, SCOOP_COMMAND, &["--version"]).await)
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
                .insert(package.name.clone(), package)
                .is_some()
            {
                return Err(protocol(
                    "Scoop status cannot disambiguate duplicate package names",
                    "duplicate local and global app",
                ));
            }
        }

        let scoop = resolve_executable(config, SCOOP_COMMAND);
        let output = run_success(
            &scoop_command(&scoop).arg("status"),
            "Scoop status timed out",
        )
        .await?;
        let stdout = decode_stdout(output, "Scoop status is not valid UTF-8")?;
        parse_status(&stdout, self.descriptor.id(), &installed_by_name)
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        validate_search_query(query)?;
        let scoop = resolve_executable(config, SCOOP_COMMAND);
        let spec = scoop_command(&scoop).args(["search", query]);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            let detail = command_output_detail(&output);
            if detail.to_ascii_lowercase().contains("no matches") {
                return Ok(Vec::new());
            }
            return Err(command_status_error(&spec, output.status, &detail));
        }
        let stdout = decode_stdout(output, "Scoop search is not valid UTF-8")?;
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
                ManagerError::new(ManagerErrorKind::Timeout, "Scoop package command timed out")
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

fn scoop_command(executable: &Path) -> CommandSpec {
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

#[derive(Debug, Deserialize)]
struct ExportDocument {
    #[serde(default)]
    apps: Vec<ExportApp>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ExportApp {
    name: String,
    version: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    updated: Option<String>,
    #[serde(default)]
    info: Option<String>,
}

fn parse_export(stdout: &str, manager_id: &ManagerId) -> ManagerResult<Vec<PackageInfo>> {
    let document: ExportDocument = serde_json::from_str(stdout)
        .map_err(|error| protocol("Scoop export JSON is malformed", &error.to_string()))?;
    let mut seen = HashMap::new();
    let mut packages = Vec::with_capacity(document.apps.len());
    for app in document.apps {
        validate_identifier(&app.name)?;
        if app.version.trim().is_empty() {
            return Err(protocol("Scoop export package version is empty", &app.name));
        }
        let global = app
            .info
            .as_deref()
            .is_some_and(|info| info.split(',').any(|item| item.trim() == "Global install"));
        let scope = if global {
            PackageScope::System
        } else {
            PackageScope::User
        };
        if seen.insert((app.name.clone(), global), ()).is_some() {
            return Err(protocol(
                "Scoop export contains a duplicate package",
                &app.name,
            ));
        }
        let mut package = PackageInfo::new(manager_id.clone(), &app.name, app.version);
        package.scope = scope;
        package.install_date = app.updated.filter(|value| !value.trim().is_empty());
        if let Some(source) = app.source.filter(|value| !value.trim().is_empty()) {
            package.origin =
                Some(PackageOrigin::new(ORIGIN_NAME).with_reference(format!("bucket:{source}")));
        }
        packages.push(package);
    }
    packages.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    Ok(packages)
}

fn parse_status(
    stdout: &str,
    manager_id: &ManagerId,
    installed: &HashMap<String, PackageInfo>,
) -> ManagerResult<Vec<PackageUpdate>> {
    let rows = table_rows(stdout, 3, |line| {
        line.starts_with("Name") && line.contains("Installed Version")
    });
    let mut updates = Vec::with_capacity(rows.len());
    for fields in rows {
        let [name, current, available] = fields.as_slice() else {
            continue;
        };
        if available.is_empty() || available == current {
            continue;
        }
        validate_identifier(name)?;
        let Some(package) = installed.get(name) else {
            return Err(protocol(
                "Scoop status package is absent from installed inventory",
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
    let rows = table_rows(stdout, 3, |line| {
        line.starts_with("Name") && line.contains("Version") && line.contains("Source")
    });
    let mut packages = Vec::with_capacity(rows.len());
    for fields in rows {
        let [name, version, source] = fields.as_slice() else {
            continue;
        };
        validate_identifier(name)?;
        if version.is_empty() || source.is_empty() {
            continue;
        }
        let mut package = PackageInfo::new(manager_id.clone(), name, version);
        package.scope = PackageScope::User;
        package.origin =
            Some(PackageOrigin::new(ORIGIN_NAME).with_reference(format!("bucket:{source}")));
        packages.push(package);
    }
    Ok(packages)
}

fn table_rows(
    stdout: &str,
    minimum_columns: usize,
    is_header: impl Fn(&str) -> bool,
) -> Vec<Vec<String>> {
    let mut started = false;
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !started {
                if is_header(line) {
                    started = true;
                }
                return None;
            }
            if line.is_empty() || line.chars().all(|character| character == '-') {
                return None;
            }
            let fields = line
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            (fields.len() >= minimum_columns).then_some(fields)
        })
        .collect()
}

fn validate_origin(origin: &PackageOrigin, package_name: &str) -> ManagerResult<()> {
    if origin.name != ORIGIN_NAME {
        return Err(protocol("Scoop target origin is not Scoop", package_name));
    }
    let Some(reference) = origin.reference.as_deref() else {
        return Err(protocol(
            "Scoop target origin is missing its source",
            package_name,
        ));
    };
    if reference.trim().is_empty() {
        return Err(protocol(
            "Scoop target origin source is empty",
            package_name,
        ));
    }
    Ok(())
}

fn bucket_reference(reference: &str) -> Option<&str> {
    reference.strip_prefix("bucket:").filter(|bucket| {
        !bucket.trim().is_empty()
            && bucket.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
    })
}

fn validate_identifier(identifier: &str) -> ManagerResult<()> {
    let identifier = identifier.trim();
    if identifier.is_empty()
        || identifier.starts_with('-')
        || identifier
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(protocol("Scoop package identifier is invalid", identifier));
    }
    Ok(())
}

fn validate_search_query(query: &str) -> ManagerResult<()> {
    if query.chars().any(char::is_control) {
        return Err(protocol(
            "Scoop search query contains control characters",
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
    use super::{SCOOP_ID, parse_export, parse_search, parse_status};
    use std::collections::HashMap;
    use std::{ffi::OsString, path::PathBuf};
    use updater_manager_api::{
        ManagerConfig, ManagerId, PackageAction, PackageInfo, PackageManager, PackageOrigin,
        PackageScope, PackageTarget,
    };

    fn manager_id() -> ManagerId {
        ManagerId::parse(SCOOP_ID).expect("valid Scoop ID")
    }

    #[test]
    fn export_parser_preserves_scope_source_and_updated_time() {
        let packages = parse_export(
            r#"{"apps":[{"Name":"7zip","Version":"24.09","Source":"main","Updated":"2026-08-01","Info":""},{"Name":"nodejs","Version":"22.1.0","Source":"main","Updated":"2026-08-01","Info":"Global install"}]}"#,
            &manager_id(),
        )
        .expect("parse Scoop export");

        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].scope, updater_manager_api::PackageScope::User);
        assert_eq!(packages[0].install_date.as_deref(), Some("2026-08-01"));
        assert_eq!(packages[1].scope, updater_manager_api::PackageScope::System);
    }

    #[test]
    fn status_parser_joins_updates_to_installed_identity() {
        let manager = manager_id();
        let mut installed = HashMap::new();
        installed.insert(
            "7zip".to_owned(),
            PackageInfo::new(manager.clone(), "7zip", "24.08"),
        );
        let updates = parse_status(
            "Name Installed Version Latest Version Info\n---- ----------------- -------------- ----\n7zip 24.08 24.09\n",
            &manager,
            &installed,
        )
        .expect("parse Scoop status");

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].current_version, "24.08");
        assert_eq!(updates[0].available_version, "24.09");
    }

    #[test]
    fn search_parser_preserves_bucket_origin() {
        let packages = parse_search(
            "Results from local buckets...\nName Version Source Binaries\n---- ------- ------ --------\n7zip 24.09 main\n",
            &manager_id(),
        )
        .expect("parse Scoop search");

        assert_eq!(packages.len(), 1);
        assert_eq!(
            packages[0].origin.as_ref().unwrap().reference.as_deref(),
            Some("bucket:main")
        );
    }

    #[test]
    fn write_commands_preserve_bucket_and_global_scope() {
        let manager = super::ScoopManager::new();
        let config = ManagerConfig::new(manager.descriptor().id().clone())
            .with_executable(PathBuf::from("scoop"));
        let mut target = PackageTarget::new(manager.descriptor().id().clone(), "7zip");
        target.scope = PackageScope::User;
        target.origin = Some(PackageOrigin::new("Scoop").with_reference("bucket:main"));

        let install = manager
            .write_command(&config, PackageAction::Install, &target)
            .expect("build Scoop install command");
        assert_eq!(
            install.arguments(),
            vec![OsString::from("install"), OsString::from("main/7zip")]
        );

        target.scope = PackageScope::System;
        let update = manager
            .write_command(&config, PackageAction::Update, &target)
            .expect("build Scoop global update command");
        assert_eq!(
            update.arguments(),
            vec![
                OsString::from("update"),
                OsString::from("7zip"),
                OsString::from("--global"),
            ]
        );
    }
}
