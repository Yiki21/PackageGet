use std::{collections::HashMap, path::Path, process::Output, time::Duration};

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

const SNAP_ID: &str = "builtin:snap";
const SNAP_COMMAND: &str = "snap";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
const ORIGIN_NAME: &str = "Snap";
const STABLE_CHANNEL: &str = "latest/stable";

/// Direct `updater-manager-api` implementation for Linux Snap packages.
#[derive(Debug, Clone)]
pub struct SnapManager {
    descriptor: ManagerDescriptor,
}

impl SnapManager {
    /// Creates the built-in Snap manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(SNAP_ID).expect("Snap manager ID must remain valid"),
            "Snap",
            ManagerCategory::Application,
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
        .expect("Snap descriptor must remain valid")
        .with_description("Linux applications managed by snapd")
        .with_authorization(AuthorizationHint::RequiresElevation {
            message: Some("Snap writes are authorized by snapd through Polkit.".to_owned()),
        });
        Self { descriptor }
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            Ok(())
        } else {
            Err(protocol(
                "Snap configuration ID does not match the manager",
                &format!("expected {}, received {}", self.descriptor.id(), config.id),
            ))
        }
    }

    async fn installed_entries(&self, config: &ManagerConfig) -> ManagerResult<Vec<SnapEntry>> {
        self.validate_config(config)?;
        let snap = resolve_executable(config, SNAP_COMMAND);
        let output = run_success(
            &snap_command(&snap).arg("list"),
            "Snap installed listing timed out",
        )
        .await?;
        parse_installed(&decode_utf8(
            &output.stdout,
            "Snap installed listing is not valid UTF-8",
        )?)
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
                "Snap target belongs to another manager",
                &target.name,
            ));
        }
        validate_snap_name(&target.name)?;
        if target.scope != PackageScope::System {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "Snap target scope must be system",
            )
            .with_detail(&target.name));
        }
        if target.version.is_some() {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "version-pinned Snap operations are not supported",
            )
            .with_detail(&target.name));
        }
        let identity =
            parse_origin(target.origin.as_ref().ok_or_else(|| {
                protocol("Snap target is missing its typed origin", &target.name)
            })?)?;
        if identity.name != target.name {
            return Err(protocol(
                "Snap target origin does not match its package name",
                &target.name,
            ));
        }

        let snap = resolve_executable(config, SNAP_COMMAND);
        let command = match action {
            PackageAction::Install => {
                if identity.channel == "-" {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "local Snap origins cannot be installed from the store",
                    )
                    .with_detail(&target.name));
                }
                let mut command = snap_command(&snap).args(["install", target.name.as_str()]);
                if identity.channel != STABLE_CHANNEL {
                    command = command.args(["--channel", identity.channel.as_str()]);
                }
                match identity.confinement {
                    Confinement::Strict => command,
                    Confinement::Classic => command.arg("--classic"),
                    Confinement::DevMode => command.arg("--devmode"),
                    Confinement::JailMode => {
                        return Err(ManagerError::new(
                            ManagerErrorKind::Unsupported,
                            "jailmode Snap installation is not supported",
                        )
                        .with_detail(&target.name));
                    }
                }
            }
            PackageAction::Update => snap_command(&snap).args(["refresh", target.name.as_str()]),
            PackageAction::Uninstall => snap_command(&snap).args(["remove", target.name.as_str()]),
            _ => unreachable!("supported actions were checked above"),
        };
        Ok(command)
    }
}

impl Default for SnapManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for SnapManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(config, SNAP_COMMAND, &["version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        Ok(self
            .installed_entries(config)
            .await?
            .into_iter()
            .map(|entry| entry.info(self.descriptor.id()))
            .collect())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.installed_entries(config).await?.len())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        let installed = self.installed_entries(config).await?;
        let installed_by_name = installed
            .iter()
            .map(|entry| (entry.identity.name.as_str(), entry))
            .collect::<HashMap<_, _>>();
        let snap = resolve_executable(config, SNAP_COMMAND);
        let output = run_success(
            &snap_command(&snap).args(["refresh", "--list"]),
            "Snap update listing timed out",
        )
        .await?;
        let candidates = parse_updates(&decode_utf8(
            &output.stdout,
            "Snap update listing is not valid UTF-8",
        )?)?;
        let mut updates = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let installed = installed_by_name
                .get(candidate.name.as_str())
                .ok_or_else(|| {
                    protocol(
                        "Snap update candidate is absent from installed inventory",
                        &candidate.name,
                    )
                })?;
            updates.push(PackageUpdate::new(
                installed.target(self.descriptor.id()),
                &installed.version,
                candidate.version,
            ));
        }
        Ok(updates)
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        validate_search_term(query)?;
        let snap = resolve_executable(config, SNAP_COMMAND);
        let output = run_success(
            &snap_command(&snap).args(["find", "--narrow", query]),
            "Snap store search timed out",
        )
        .await?;
        parse_search(
            &decode_utf8(&output.stdout, "Snap store search is not valid UTF-8")?,
            self.descriptor.id(),
        )
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
                    let (fraction, message) = event.into_parts();
                    if let Some(message) = message {
                        progress.emit(ProgressEvent::Message { message });
                    }
                    progress.emit(ProgressEvent::Advanced {
                        completed: index + usize::from(fraction >= 1.0),
                        total,
                        current_package: Some(target.name.clone()),
                    });
                }),
            )
            .await
            .map_err(|_| {
                ManagerError::new(ManagerErrorKind::Timeout, "Snap write command timed out")
                    .with_detail(command.program().to_string_lossy())
            })??;
        }
        progress.emit(ProgressEvent::Finished {
            completed: total,
            total,
        });
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Confinement {
    Strict,
    Classic,
    DevMode,
    JailMode,
}

impl Confinement {
    fn from_notes(notes: &str) -> ManagerResult<Self> {
        let tokens = note_tokens(notes)?;
        let modes = [
            ("classic", Self::Classic),
            ("devmode", Self::DevMode),
            ("jailmode", Self::JailMode),
        ]
        .into_iter()
        .filter(|(token, _)| tokens.contains(token))
        .map(|(_, mode)| mode)
        .collect::<Vec<_>>();
        match modes.as_slice() {
            [] => Ok(Self::Strict),
            [mode] => Ok(*mode),
            _ => Err(protocol(
                "Snap notes contain conflicting confinement",
                notes,
            )),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Classic => "classic",
            Self::DevMode => "devmode",
            Self::JailMode => "jailmode",
        }
    }

    fn parse(value: &str) -> ManagerResult<Self> {
        match value {
            "strict" => Ok(Self::Strict),
            "classic" => Ok(Self::Classic),
            "devmode" => Ok(Self::DevMode),
            "jailmode" => Ok(Self::JailMode),
            _ => Err(protocol("Snap confinement is malformed", value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapIdentity {
    name: String,
    channel: String,
    confinement: Confinement,
    refresh: String,
    notes: String,
}

impl SnapIdentity {
    fn from_fields(name: &str, channel: &str, notes: &str, store: bool) -> ManagerResult<Self> {
        validate_snap_name(name)?;
        validate_channel(channel)?;
        validate_notes(notes)?;
        let confinement = Confinement::from_notes(notes)?;
        let tokens = note_tokens(notes)?;
        let refresh = if store {
            "store"
        } else if tokens.contains(&"held") {
            "held"
        } else if tokens.contains(&"disabled") {
            "disabled"
        } else {
            "automatic"
        };
        Ok(Self {
            name: name.to_owned(),
            channel: channel.to_owned(),
            confinement,
            refresh: refresh.to_owned(),
            notes: notes.to_owned(),
        })
    }

    fn origin(&self) -> PackageOrigin {
        PackageOrigin::new(ORIGIN_NAME).with_reference(format!(
            "snap:{};channel:{};confinement:{};refresh:{};notes:{}",
            self.name,
            self.channel,
            self.confinement.as_str(),
            self.refresh,
            self.notes
        ))
    }
}

#[derive(Debug)]
struct SnapEntry {
    identity: SnapIdentity,
    version: String,
    revision: String,
    publisher: String,
}

impl SnapEntry {
    fn target(&self, manager_id: &ManagerId) -> PackageTarget {
        let mut target = PackageTarget::new(manager_id.clone(), &self.identity.name);
        target.scope = PackageScope::System;
        target.origin = Some(self.identity.origin());
        target
    }

    fn info(self, manager_id: &ManagerId) -> PackageInfo {
        let description = format!(
            "Publisher: {}; revision: {}; channel: {}; confinement: {}; refresh: {}; notes: {}",
            self.publisher,
            self.revision,
            self.identity.channel,
            self.identity.confinement.as_str(),
            self.identity.refresh,
            self.identity.notes
        );
        let mut info = PackageInfo::new(manager_id.clone(), &self.identity.name, self.version);
        info.description = Some(description);
        info.scope = PackageScope::System;
        info.origin = Some(self.identity.origin());
        info
    }
}

#[derive(Debug)]
struct UpdateCandidate {
    name: String,
    version: String,
}

fn parse_installed(value: &str) -> ManagerResult<Vec<SnapEntry>> {
    let rows = table_rows(
        value,
        &["Name", "Version", "Rev", "Tracking", "Publisher", "Notes"],
    )?;
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let columns = split_columns(row, 6, "Snap installed row is malformed")?;
        let identity = SnapIdentity::from_fields(columns[0], columns[3], columns[5], false)?;
        validate_field(columns[1], "Snap installed version is malformed")?;
        validate_field(columns[2], "Snap installed revision is malformed")?;
        validate_field(columns[4], "Snap publisher is malformed")?;
        entries.push(SnapEntry {
            identity,
            version: columns[1].to_owned(),
            revision: columns[2].to_owned(),
            publisher: columns[4].to_owned(),
        });
    }
    reject_duplicate_names(entries.iter().map(|entry| entry.identity.name.as_str()))?;
    Ok(entries)
}

fn parse_updates(value: &str) -> ManagerResult<Vec<UpdateCandidate>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let rows = table_rows(
        value,
        &["Name", "Version", "Rev", "Size", "Publisher", "Notes"],
    )?;
    let mut updates = Vec::with_capacity(rows.len());
    for row in rows {
        let columns = split_columns(row, 6, "Snap update row is malformed")?;
        validate_snap_name(columns[0])?;
        validate_field(columns[1], "Snap update version is malformed")?;
        validate_field(columns[2], "Snap update revision is malformed")?;
        validate_field(columns[3], "Snap update size is malformed")?;
        validate_field(columns[4], "Snap update publisher is malformed")?;
        validate_notes(columns[5])?;
        updates.push(UpdateCandidate {
            name: columns[0].to_owned(),
            version: columns[1].to_owned(),
        });
    }
    reject_duplicate_names(updates.iter().map(|entry| entry.name.as_str()))?;
    Ok(updates)
}

fn parse_search(value: &str, manager_id: &ManagerId) -> ManagerResult<Vec<PackageInfo>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let rows = table_rows(value, &["Name", "Version", "Publisher", "Notes", "Summary"])?;
    let mut packages = Vec::with_capacity(rows.len());
    for row in rows {
        let columns = split_columns(row, 5, "Snap search row is malformed")?;
        let identity = SnapIdentity::from_fields(columns[0], STABLE_CHANNEL, columns[3], true)?;
        validate_field(columns[1], "Snap search version is malformed")?;
        validate_field(columns[2], "Snap search publisher is malformed")?;
        if columns[4].trim().is_empty() {
            return Err(protocol("Snap search summary is empty", columns[0]));
        }
        let mut info = PackageInfo::new(manager_id.clone(), columns[0], columns[1]);
        info.description = Some(format!("{} (Publisher: {})", columns[4], columns[2]));
        info.scope = PackageScope::System;
        info.origin = Some(identity.origin());
        packages.push(info);
    }
    reject_duplicate_names(packages.iter().map(|entry| entry.name.as_str()))?;
    Ok(packages)
}

fn table_rows<'a>(value: &'a str, header: &[&str]) -> ManagerResult<Vec<&'a str>> {
    let mut lines = value.lines().filter(|line| !line.trim().is_empty());
    let Some(actual_header) = lines.next() else {
        return Ok(Vec::new());
    };
    if actual_header.split_whitespace().collect::<Vec<_>>() != header {
        return Err(protocol("Snap table header is unsupported", actual_header));
    }
    Ok(lines.collect())
}

fn split_columns<'a>(row: &'a str, count: usize, message: &str) -> ManagerResult<Vec<&'a str>> {
    let mut remainder = row.trim();
    let mut columns = Vec::with_capacity(count);
    for _ in 1..count {
        let Some(boundary) = remainder.find(char::is_whitespace) else {
            return Err(protocol(message, row));
        };
        columns.push(&remainder[..boundary]);
        remainder = remainder[boundary..].trim_start();
    }
    columns.push(remainder);
    if columns.iter().any(|column| column.is_empty()) {
        return Err(protocol(message, row));
    }
    Ok(columns)
}

fn parse_origin(origin: &PackageOrigin) -> ManagerResult<SnapIdentity> {
    if origin.name != ORIGIN_NAME {
        return Err(protocol(
            "Snap target origin name is malformed",
            &origin.name,
        ));
    }
    let reference = origin
        .reference
        .as_deref()
        .ok_or_else(|| protocol("Snap target origin reference is missing", &origin.name))?;
    let parts = reference.split(';').collect::<Vec<_>>();
    if parts.len() != 5 {
        return Err(protocol(
            "Snap target origin reference is malformed",
            reference,
        ));
    }
    let name = strip_prefix(parts[0], "snap:", reference)?;
    let channel = strip_prefix(parts[1], "channel:", reference)?;
    let confinement = Confinement::parse(strip_prefix(parts[2], "confinement:", reference)?)?;
    let refresh = strip_prefix(parts[3], "refresh:", reference)?;
    let notes = strip_prefix(parts[4], "notes:", reference)?;
    let parsed = SnapIdentity::from_fields(name, channel, notes, refresh == "store")?;
    if parsed.confinement != confinement || parsed.refresh != refresh {
        return Err(protocol(
            "Snap target origin state is inconsistent",
            reference,
        ));
    }
    Ok(parsed)
}

fn strip_prefix<'a>(value: &'a str, prefix: &str, detail: &str) -> ManagerResult<&'a str> {
    value
        .strip_prefix(prefix)
        .ok_or_else(|| protocol("Snap target origin reference is malformed", detail))
}

fn note_tokens(notes: &str) -> ManagerResult<Vec<&str>> {
    validate_notes(notes)?;
    if notes == "-" {
        Ok(Vec::new())
    } else {
        Ok(notes.split(',').collect())
    }
}

fn validate_notes(notes: &str) -> ManagerResult<()> {
    if notes == "-" {
        return Ok(());
    }
    if notes.is_empty()
        || notes.contains([';', ':', '|'])
        || notes
            .split(',')
            .any(|token| token.is_empty() || token.chars().any(char::is_whitespace))
    {
        return Err(protocol("Snap notes are malformed", notes));
    }
    Ok(())
}

fn validate_snap_name(value: &str) -> ManagerResult<()> {
    let mut parts = value.split('_');
    let base = parts.next().unwrap_or_default();
    let instance = parts.next();
    let valid_base = (1..=40).contains(&base.len())
        && base.chars().any(|character| character.is_ascii_lowercase())
        && base.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && base
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && base
            .chars()
            .last()
            .is_some_and(|character| character.is_ascii_alphanumeric());
    let valid_instance = instance.is_none_or(|instance| {
        (1..=10).contains(&instance.len())
            && instance
                .chars()
                .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    });
    let valid = valid_base && valid_instance && parts.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(protocol("Snap package name is malformed", value))
    }
}

fn validate_channel(value: &str) -> ManagerResult<()> {
    if value == "-" {
        return Ok(());
    }
    if value.is_empty()
        || value.starts_with('-')
        || value.contains([';', ':', '|', '\\'])
        || value.chars().any(char::is_whitespace)
    {
        Err(protocol("Snap channel is malformed", value))
    } else {
        Ok(())
    }
}

fn validate_field(value: &str, message: &str) -> ManagerResult<()> {
    if value.is_empty() || value.contains(['\n', '\r', '\0', ';']) {
        Err(protocol(message, value))
    } else {
        Ok(())
    }
}

fn validate_search_term(value: &str) -> ManagerResult<()> {
    if value.starts_with('-') || value.contains(['\n', '\r', '\0']) {
        Err(protocol("Snap search term is malformed", value))
    } else {
        Ok(())
    }
}

fn reject_duplicate_names<'a>(names: impl Iterator<Item = &'a str>) -> ManagerResult<()> {
    let mut seen = std::collections::HashSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(protocol(
                "Snap table contains duplicate package identity",
                name,
            ));
        }
    }
    Ok(())
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "Snap package action is unsupported",
        )),
    }
}

fn snap_command(path: &Path) -> CommandSpec {
    CommandSpec::new(path).env("LC_ALL", "C").env("LANG", "C")
}

async fn run_success(spec: &CommandSpec, timeout_message: &str) -> ManagerResult<Output> {
    let output = timeout(COMMAND_TIMEOUT, run_output(spec))
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, timeout_message)
                .with_detail(spec.program().to_string_lossy())
        })??;
    if output.status.success() {
        return Ok(output);
    }
    Err(command_status_error(
        spec,
        output.status,
        &format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    ))
}

fn decode_utf8(bytes: &[u8], message: &str) -> ManagerResult<String> {
    String::from_utf8(bytes.to_vec()).map_err(|error| protocol(message, &error.to_string()))
}

fn protocol(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_parser_rejects_duplicate_identity_and_conflicting_confinement() {
        let duplicate = "Name Version Rev Tracking Publisher Notes\ncode 1 1 latest/stable pub -\ncode 2 2 latest/edge pub -\n";
        assert_eq!(
            parse_installed(duplicate)
                .expect_err("duplicate Snap names must fail")
                .kind(),
            ManagerErrorKind::Protocol
        );

        let conflict = "Name Version Rev Tracking Publisher Notes\ncode 1 1 latest/stable pub classic,devmode\n";
        assert_eq!(
            parse_installed(conflict)
                .expect_err("conflicting confinement must fail")
                .kind(),
            ManagerErrorKind::Protocol
        );
    }

    #[test]
    fn parsers_reject_schema_drift_and_preserve_extensible_notes() {
        let drifted =
            "Name Version Revision Tracking Publisher Notes\ncode 1 1 latest/stable pub -\n";
        assert_eq!(
            parse_installed(drifted)
                .expect_err("header drift must fail")
                .kind(),
            ManagerErrorKind::Protocol
        );

        let value = "Name Version Rev Tracking Publisher Notes\nbase-app 1 1 latest/stable pub base,components[1/2],held\n";
        let parsed = parse_installed(value).expect("extensible notes remain representable");
        assert_eq!(parsed[0].identity.notes, "base,components[1/2],held");
        assert_eq!(parsed[0].identity.refresh, "held");
    }

    #[test]
    fn package_names_follow_snap_and_parallel_instance_grammar() {
        for valid in ["code", "hello-world", "core22", "code_work"] {
            validate_snap_name(valid).expect("valid Snap name");
        }
        for invalid in ["123", "-code", "code-", "Code", "code_", "code_a_b"] {
            assert!(validate_snap_name(invalid).is_err(), "{invalid} must fail");
        }
    }

    #[test]
    fn local_tracking_is_readable_but_not_a_store_install_origin() {
        let value = "Name Version Rev Tracking Publisher Notes\nlocal-app 1 x1 - local dangerous\n";
        let parsed = parse_installed(value).expect("local Snap remains visible");
        assert_eq!(parsed[0].identity.channel, "-");
        assert_eq!(parsed[0].identity.notes, "dangerous");
    }
}
