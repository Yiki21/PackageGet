use std::{
    path::{Path, PathBuf},
    process::{ExitStatus, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use serde::Deserialize;
use unicode_width::UnicodeWidthChar;
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapabilities,
    ManagerCapability, ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError,
    ManagerErrorKind, ManagerId, ManagerResult, PackageAction, PackageInfo, PackageManager,
    PackageOrigin, PackageScope, PackageTarget, PackageUpdate, Platform, ProgressEvent,
    ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, command_status_error, manager_availability, resolve_executable, run_output,
    },
    progress::{CommandProgress, run_command_with_progress_and_status},
};

const WINGET_ID: &str = "builtin:winget";
const WINGET_COMMAND: &str = "winget";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const UNKNOWN_VERSION: &str = "Unknown";

const NO_APPLICATIONS_FOUND: u32 = 0x8A15_0014;
const COMMAND_REQUIRES_ADMIN: u32 = 0x8A15_0019;
const UPDATE_NOT_APPLICABLE: u32 = 0x8A15_002B;
const NOT_ALL_PACKAGES_FOUND: u32 = 0x8A15_0035;
const AUTHENTICATION_CANCELLED_BY_USER: u32 = 0x8A15_0077;
const INSTALL_PACKAGE_IN_USE: u32 = 0x8A15_0101;
const INSTALL_IN_PROGRESS: u32 = 0x8A15_0102;
const INSTALL_NO_NETWORK: u32 = 0x8A15_0107;
const REBOOT_REQUIRED_TO_FINISH: u32 = 0x8A15_0109;
const REBOOT_REQUIRED_FOR_INSTALL: u32 = 0x8A15_010A;
const REBOOT_INITIATED: u32 = 0x8A15_010B;
const INSTALL_CANCELLED_BY_USER: u32 = 0x8A15_010C;

static EXPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Direct `updater-manager-api` implementation for Windows Package Manager.
#[derive(Debug, Clone)]
pub struct WingetManager {
    descriptor: ManagerDescriptor,
}

impl WingetManager {
    /// Creates the built-in Winget manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(WINGET_ID).expect("Winget manager ID must remain valid"),
            "Winget",
            ManagerCategory::System,
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
        .expect("Winget descriptor must remain valid")
        .with_description("Windows Package Manager")
        .with_authorization(AuthorizationHint::MayRequireElevation {
            message: Some("Some installers may request Windows elevation through UAC.".to_owned()),
        });

        Self { descriptor }
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            return Ok(());
        }
        Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "winget configuration ID does not match the manager",
        )
        .with_detail(format!(
            "expected {}, received {}",
            self.descriptor.id(),
            config.id
        )))
    }

    async fn export_inventory(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let export_path = export_path();
        let winget_path = resolve_executable(config, WINGET_COMMAND);
        let spec = export_command(&winget_path, &export_path);
        let output = run_output(&spec).await?;
        let accepted_partial = status_hresult(output.status) == Some(NOT_ALL_PACKAGES_FOUND);
        if !output.status.success() && !accepted_partial {
            let error = winget_status_error(&spec, output.status, &output_tail(&output));
            let _ = tokio::fs::remove_file(&export_path).await;
            return Err(error);
        }

        let contents = tokio::fs::read_to_string(&export_path)
            .await
            .map_err(|error| {
                ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "winget export did not create a readable inventory",
                )
                .with_detail(error.to_string())
            });
        let _ = tokio::fs::remove_file(&export_path).await;
        parse_export(&contents?)
    }

    async fn table_output(
        &self,
        config: &ManagerConfig,
        spec: CommandSpec,
        empty_codes: &[u32],
    ) -> ManagerResult<Option<String>> {
        self.validate_config(config)?;
        let output = run_output(&spec).await?;
        if !output.status.success() {
            if status_hresult(output.status).is_some_and(|code| empty_codes.contains(&code)) {
                return Ok(None);
            }
            return Err(winget_status_error(
                &spec,
                output.status,
                &output_tail(&output),
            ));
        }
        String::from_utf8(output.stdout).map(Some).map_err(|error| {
            ManagerError::new(
                ManagerErrorKind::Protocol,
                "winget table output is not valid UTF-8",
            )
            .with_detail(error.to_string())
        })
    }

    fn write_command(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
    ) -> ManagerResult<CommandSpec> {
        ensure_supported_action(action)?;
        if &target.manager_id != self.descriptor.id() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "winget package target belongs to another manager",
            )
            .with_detail(format!(
                "expected {}, received {} for package {}",
                self.descriptor.id(),
                target.manager_id,
                target.name
            )));
        }
        validate_identifier(&target.name)?;
        if target.version.is_some() {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "version-pinned winget targets are not supported",
            )
            .with_detail(&target.name));
        }

        let source = target
            .origin
            .as_ref()
            .map(|origin| origin.name.trim())
            .filter(|source| !source.is_empty());
        if !matches!(action, PackageAction::Install) && source.is_none() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "winget update and uninstall targets require a frozen source",
            )
            .with_detail(&target.name));
        }

        let winget_path = resolve_executable(config, WINGET_COMMAND);
        let mut command = CommandSpec::new(winget_path)
            .arg(command_name(action)?)
            .args(["--id", target.name.as_str(), "--exact"]);
        if let Some(source) = source {
            command = command.args(["--source", source]);
        }
        command = match target.scope {
            PackageScope::System => command.args(["--scope", "machine"]),
            PackageScope::User => command.args(["--scope", "user"]),
            PackageScope::Unknown => command,
            PackageScope::Project => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "winget project scope is not supported",
                )
                .with_detail(&target.name));
            }
            _ => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "winget target scope is not supported",
                )
                .with_detail(&target.name));
            }
        };
        if matches!(action, PackageAction::Install | PackageAction::Update) {
            command = command.arg("--accept-package-agreements");
        }
        Ok(command.args(["--accept-source-agreements", "--disable-interactivity"]))
    }
}

impl Default for WingetManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for WingetManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        if !cfg!(target_os = "windows") {
            return Ok(ManagerAvailability::Unavailable {
                reason: AvailabilityReason::UnsupportedPlatform {
                    platform: Platform::current(),
                },
            });
        }
        Ok(manager_availability(config, WINGET_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.export_inventory(config).await
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.export_inventory(config).await?.len())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        let winget_path = resolve_executable(config, WINGET_COMMAND);
        let spec = CommandSpec::new(winget_path)
            .arg("upgrade")
            .args(["--accept-source-agreements", "--disable-interactivity"]);
        let Some(stdout) = self
            .table_output(
                config,
                spec,
                &[UPDATE_NOT_APPLICABLE, NO_APPLICATIONS_FOUND],
            )
            .await?
        else {
            return Ok(Vec::new());
        };
        parse_updates(&stdout, self.descriptor.id())
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let winget_path = resolve_executable(config, WINGET_COMMAND);
        let spec = CommandSpec::new(winget_path)
            .arg("search")
            .args(["--query", query])
            .args(["--accept-source-agreements", "--disable-interactivity"]);
        let Some(stdout) = self
            .table_output(config, spec, &[NO_APPLICATIONS_FOUND])
            .await?
        else {
            return Ok(Vec::new());
        };
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
        for (index, (target, command)) in packages.iter().zip(commands.iter()).enumerate() {
            run_command_with_progress_and_status(command, winget_status_error, |event| {
                emit_command_progress(progress, index, total, &target.name, event);
            })
            .await?;
        }
        progress.emit(ProgressEvent::Finished {
            completed: total,
            total,
        });
        Ok(())
    }
}

fn emit_command_progress(
    progress: &dyn ProgressSink,
    index: usize,
    total: usize,
    package: &str,
    event: CommandProgress,
) {
    let (fraction, message) = event.into_parts();
    if let Some(message) = message {
        progress.emit(ProgressEvent::Message { message });
    }
    progress.emit(ProgressEvent::Advanced {
        completed: index + usize::from(fraction >= 1.0),
        total,
        current_package: Some(package.to_owned()),
    });
}

fn export_path() -> PathBuf {
    let sequence = EXPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "updater-winget-export-{}-{sequence}.json",
        std::process::id()
    ))
}

fn export_command(executable: &Path, output: &Path) -> CommandSpec {
    CommandSpec::new(executable)
        .arg("export")
        .args(["--output".into(), output.as_os_str().to_owned()])
        .args([
            "--include-versions",
            "--accept-source-agreements",
            "--disable-interactivity",
        ])
}

fn command_name(action: PackageAction) -> ManagerResult<&'static str> {
    match action {
        PackageAction::Install => Ok("install"),
        PackageAction::Update => Ok("upgrade"),
        PackageAction::Uninstall => Ok("uninstall"),
        _ => Err(ManagerError::unsupported(action.capability())),
    }
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    command_name(action).map(|_| ())
}

fn validate_identifier(identifier: &str) -> ManagerResult<()> {
    let identifier = identifier.trim();
    if identifier.is_empty()
        || identifier.starts_with('-')
        || identifier.chars().any(|character| character.is_control())
    {
        return Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "winget package identifier is invalid",
        )
        .with_detail(identifier));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ExportDocument {
    #[serde(default)]
    sources: Vec<ExportSource>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ExportSource {
    source_details: ExportSourceDetails,
    #[serde(default)]
    packages: Vec<ExportPackage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ExportSourceDetails {
    name: String,
    identifier: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ExportPackage {
    package_identifier: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

fn parse_export(contents: &str) -> ManagerResult<Vec<PackageInfo>> {
    let document: ExportDocument = serde_json::from_str(contents).map_err(|error| {
        ManagerError::new(
            ManagerErrorKind::Protocol,
            "winget export inventory is invalid",
        )
        .with_detail(error.to_string())
    })?;
    let manager = ManagerId::parse(WINGET_ID).expect("Winget manager ID must remain valid");
    let mut packages = Vec::new();
    for source in document.sources {
        let source_name =
            required_field(source.source_details.name, "winget source name is missing")?;
        let source_id = required_field(
            source.source_details.identifier,
            "winget source identifier is missing",
        )?;
        for package in source.packages {
            let identifier = required_field(
                package.package_identifier,
                "winget package identifier is missing",
            )?;
            validate_identifier(&identifier)?;
            let version = package
                .version
                .filter(|version| !version.trim().is_empty())
                .unwrap_or_else(|| UNKNOWN_VERSION.to_owned());
            let mut info = PackageInfo::new(manager.clone(), identifier, version);
            info.scope = parse_scope(package.scope.as_deref());
            info.origin =
                Some(PackageOrigin::new(source_name.clone()).with_reference(source_id.clone()));
            packages.push(info);
        }
    }
    Ok(packages)
}

fn parse_scope(scope: Option<&str>) -> PackageScope {
    match scope.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("machine") => PackageScope::System,
        Some("user") => PackageScope::User,
        _ => PackageScope::Unknown,
    }
}

fn required_field(value: String, message: &str) -> ManagerResult<String> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(ManagerError::new(ManagerErrorKind::Protocol, message))
    } else {
        Ok(value)
    }
}

fn parse_updates(contents: &str, manager: &ManagerId) -> ManagerResult<Vec<PackageUpdate>> {
    parse_table(contents, &[5])?
        .into_iter()
        .map(|columns| {
            let identifier = table_field(&columns, 1, "winget update identifier is missing")?;
            let current = table_field(&columns, 2, "winget current version is missing")?;
            let available = table_field(&columns, 3, "winget available version is missing")?;
            let source = table_field(&columns, 4, "winget update source is missing")?;
            let mut target = PackageTarget::new(manager.clone(), identifier);
            target.origin = Some(PackageOrigin::new(source));
            Ok(PackageUpdate::new(target, current, available))
        })
        .collect()
}

fn parse_search(contents: &str, manager: &ManagerId) -> ManagerResult<Vec<PackageInfo>> {
    parse_table(contents, &[4, 5])?
        .into_iter()
        .map(|columns| {
            let identifier = table_field(&columns, 1, "winget search identifier is missing")?;
            let advertised_version = table_field(&columns, 2, "winget search version is missing")?;
            let source_index = columns.len() - 1;
            let source = table_field(&columns, source_index, "winget search source is missing")?;
            let display_name = table_field(&columns, 0, "winget search name is missing")?;
            let mut package = PackageInfo::new(manager.clone(), identifier, NOT_INSTALLED_VERSION);
            package.description = Some(format!("{display_name} ({advertised_version})"));
            package.origin = Some(PackageOrigin::new(source));
            Ok(package)
        })
        .collect()
}

fn table_field(columns: &[String], index: usize, message: &str) -> ManagerResult<String> {
    columns
        .get(index)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ManagerError::new(ManagerErrorKind::Protocol, message))
}

fn parse_table(contents: &str, accepted_columns: &[usize]) -> ManagerResult<Vec<Vec<String>>> {
    let lines = contents.lines().collect::<Vec<_>>();
    let separator = lines
        .iter()
        .position(|line| {
            let line = line.trim();
            line.len() >= 3 && line.chars().all(|character| character == '-')
        })
        .ok_or_else(|| {
            ManagerError::new(
                ManagerErrorKind::Protocol,
                "winget table separator is missing",
            )
        })?;
    let header = lines[..separator]
        .iter()
        .rev()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| {
            ManagerError::new(ManagerErrorKind::Protocol, "winget table header is missing")
        })?;
    let starts = column_starts(header);
    if !accepted_columns.contains(&starts.len()) {
        return Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "winget table has an unexpected column count",
        )
        .with_detail(starts.len().to_string()));
    }

    let mut rows = Vec::new();
    for line in &lines[separator + 1..] {
        if line.trim().is_empty() {
            if !rows.is_empty() {
                break;
            }
            continue;
        }
        let columns = starts
            .iter()
            .enumerate()
            .map(|(index, start)| {
                display_slice(line, *start, starts.get(index + 1).copied())
                    .trim()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        if columns.iter().all(String::is_empty) {
            continue;
        }
        rows.push(columns);
    }
    Ok(rows)
}

fn column_starts(header: &str) -> Vec<usize> {
    let mut starts = vec![0];
    let mut display = 0;
    let mut spaces = 0;
    for character in header.chars() {
        let width = character.width().unwrap_or(0);
        if character == ' ' {
            spaces += width;
        } else {
            if spaces >= 2 {
                starts.push(display);
            }
            spaces = 0;
        }
        display += width;
    }
    starts
}

fn display_slice(line: &str, start: usize, end: Option<usize>) -> String {
    let mut display = 0;
    let mut result = String::new();
    for character in line.chars() {
        let width = character.width().unwrap_or(0);
        let next = display + width;
        if next > start && end.is_none_or(|end| display < end) {
            result.push(character);
        }
        display = next;
        if end.is_some_and(|end| display >= end) {
            break;
        }
    }
    result
}

fn output_tail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.trim().is_empty() {
        stdout.trim().to_owned()
    } else if stdout.trim().is_empty() {
        stderr.trim().to_owned()
    } else {
        format!("{}\n{}", stdout.trim(), stderr.trim())
    }
}

fn status_hresult(status: ExitStatus) -> Option<u32> {
    status.code().map(|code| code as u32)
}

fn winget_status_error(spec: &CommandSpec, status: ExitStatus, tail: &str) -> ManagerError {
    if let Some(code) = status_hresult(status)
        && let Some(kind) = classify_hresult(code)
    {
        return ManagerError::new(kind, hresult_message(code)).with_detail(format!(
            "winget exited with HRESULT 0x{code:08X}: {}",
            tail.trim()
        ));
    }
    command_status_error(spec, status, tail)
}

fn classify_hresult(code: u32) -> Option<ManagerErrorKind> {
    match code {
        COMMAND_REQUIRES_ADMIN => Some(ManagerErrorKind::Permission),
        AUTHENTICATION_CANCELLED_BY_USER | INSTALL_CANCELLED_BY_USER => {
            Some(ManagerErrorKind::Cancelled)
        }
        INSTALL_PACKAGE_IN_USE | INSTALL_IN_PROGRESS => Some(ManagerErrorKind::Busy),
        INSTALL_NO_NETWORK => Some(ManagerErrorKind::Network),
        REBOOT_REQUIRED_TO_FINISH | REBOOT_REQUIRED_FOR_INSTALL | REBOOT_INITIATED => {
            Some(ManagerErrorKind::RebootRequired)
        }
        NO_APPLICATIONS_FOUND | UPDATE_NOT_APPLICABLE | NOT_ALL_PACKAGES_FOUND => {
            Some(ManagerErrorKind::Protocol)
        }
        _ => None,
    }
}

fn hresult_message(code: u32) -> &'static str {
    match classify_hresult(code) {
        Some(ManagerErrorKind::Permission) => "winget requires administrator authorization",
        Some(ManagerErrorKind::Cancelled) => "winget operation was cancelled",
        Some(ManagerErrorKind::Busy) => "winget or the installer is busy",
        Some(ManagerErrorKind::Network) => "winget installer could not access the network",
        Some(ManagerErrorKind::RebootRequired) => "winget installer requires a reboot",
        Some(ManagerErrorKind::Protocol) => "winget could not resolve the requested package set",
        _ => "winget command failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_id() -> ManagerId {
        ManagerId::parse(WINGET_ID).expect("valid Winget manager ID")
    }

    #[test]
    fn export_inventory_preserves_source_scope_and_version() {
        let packages = parse_export(include_str!("../tests/fixtures/winget/export.json"))
            .expect("parse Winget export");

        assert_eq!(packages.len(), 3);
        assert_eq!(packages[0].name, "Microsoft.PowerToys");
        assert_eq!(packages[0].version, "0.92.1");
        assert_eq!(packages[0].scope, PackageScope::User);
        assert_eq!(packages[0].origin.as_ref().unwrap().name, "winget");
        assert_eq!(
            packages[0].origin.as_ref().unwrap().reference.as_deref(),
            Some("Microsoft.Winget.Source_8wekyb3d8bbwe")
        );
        assert_eq!(packages[2].version, UNKNOWN_VERSION);
        assert_eq!(packages[2].scope, PackageScope::Unknown);
    }

    #[test]
    fn update_table_freezes_identifier_versions_and_source() {
        let updates = parse_updates(
            include_str!("../tests/fixtures/winget/upgrade.txt"),
            &manager_id(),
        )
        .expect("parse Winget upgrades");

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].target.name, "Microsoft.PowerToys");
        assert_eq!(updates[0].current_version, "0.92.0");
        assert_eq!(updates[0].available_version, "0.92.1");
        assert_eq!(updates[0].target.origin.as_ref().unwrap().name, "winget");
        assert_eq!(updates[1].target.name, "VideoLAN.VLC");
    }

    #[test]
    fn search_table_supports_unicode_and_optional_match_column() {
        let packages = parse_search(
            include_str!("../tests/fixtures/winget/search.txt"),
            &manager_id(),
        )
        .expect("parse Winget search");
        let matched = parse_search(
            include_str!("../tests/fixtures/winget/search-match.txt"),
            &manager_id(),
        )
        .expect("parse Winget search with Match");

        assert_eq!(packages[0].name, "Tencent.WeChat.Universal");
        assert_eq!(packages[0].version, NOT_INSTALLED_VERSION);
        assert_eq!(packages[0].origin.as_ref().unwrap().name, "winget");
        assert_eq!(matched[0].name, "Microsoft.VisualStudioCode");
        assert_eq!(matched[0].origin.as_ref().unwrap().name, "winget");
    }

    #[test]
    fn malformed_table_is_a_protocol_error() {
        let error = parse_updates(
            "Name Id Version\nPowerToys Microsoft.PowerToys 1.0",
            &manager_id(),
        )
        .expect_err("reject missing separator");
        assert_eq!(error.kind(), ManagerErrorKind::Protocol);
    }

    #[test]
    fn write_commands_replay_identifier_source_scope_and_noninteractive_flags() {
        let manager = WingetManager::new();
        let config = ManagerConfig::new(manager_id()).with_executable("winget.exe");
        let mut target = PackageTarget::new(manager_id(), "Microsoft.PowerToys");
        target.scope = PackageScope::User;
        target.origin = Some(PackageOrigin::new("winget"));

        let install = manager
            .write_command(&config, PackageAction::Install, &target)
            .expect("build Winget install command");
        let update = manager
            .write_command(&config, PackageAction::Update, &target)
            .expect("build Winget update command");
        let uninstall = manager
            .write_command(&config, PackageAction::Uninstall, &target)
            .expect("build Winget uninstall command");

        assert_eq!(
            render_args(&install),
            "install --id Microsoft.PowerToys --exact --source winget --scope user --accept-package-agreements --accept-source-agreements --disable-interactivity"
        );
        assert!(render_args(&update).starts_with("upgrade --id Microsoft.PowerToys --exact"));
        assert!(!render_args(&uninstall).contains("--accept-package-agreements"));
        assert!(render_args(&uninstall).ends_with("--disable-interactivity"));
    }

    #[test]
    fn update_requires_frozen_source_but_legacy_install_accepts_an_explicit_id() {
        let manager = WingetManager::new();
        let config = ManagerConfig::new(manager_id());
        let target = PackageTarget::new(manager_id(), "Contoso.App");

        assert!(
            manager
                .write_command(&config, PackageAction::Install, &target)
                .is_ok()
        );
        assert_eq!(
            manager
                .write_command(&config, PackageAction::Update, &target)
                .expect_err("reject source-less update")
                .kind(),
            ManagerErrorKind::Protocol
        );
    }

    #[test]
    fn official_hresults_map_to_stable_error_kinds() {
        assert_eq!(
            classify_hresult(COMMAND_REQUIRES_ADMIN),
            Some(ManagerErrorKind::Permission)
        );
        assert_eq!(
            classify_hresult(INSTALL_NO_NETWORK),
            Some(ManagerErrorKind::Network)
        );
        assert_eq!(
            classify_hresult(INSTALL_IN_PROGRESS),
            Some(ManagerErrorKind::Busy)
        );
        assert_eq!(
            classify_hresult(REBOOT_REQUIRED_FOR_INSTALL),
            Some(ManagerErrorKind::RebootRequired)
        );
        assert_eq!(
            classify_hresult(AUTHENTICATION_CANCELLED_BY_USER),
            Some(ManagerErrorKind::Cancelled)
        );
        assert_eq!(classify_hresult(7), None);
    }

    fn render_args(spec: &CommandSpec) -> String {
        spec.arguments()
            .iter()
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }
}
