use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use async_trait::async_trait;
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId,
    ManagerResult, PackageAction, PackageInfo, PackageManager, PackageOrigin, PackageScope,
    PackageTarget, PackageUpdate, Platform, ProgressEvent, ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, command_status_error, decode_stdout, manager_availability, resolve_executable,
        run_output,
    },
    progress::{CommandProgress, run_cancellable_command_with_progress, run_command_with_progress},
};

const FLATPAK_ID: &str = "builtin:flatpak";
const FLATPAK_COMMAND: &str = "flatpak";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const INSTALLED_COLUMNS: &str =
    "--columns=application:f,name:f,version:f,branch:f,size,origin:f,installation:f,ref:f";
const UPDATE_COLUMNS: &str = "--columns=application:f,ref:f,branch:f,version:f,commit:f,origin:f";
const SEARCH_COLUMNS: &str =
    "--columns=application:f,name:f,description:f,version:f,branch:f,remotes:f";

/// Direct `updater-manager-api` implementation for Flatpak.
#[derive(Debug, Clone)]
pub struct FlatpakManager {
    descriptor: ManagerDescriptor,
}

impl FlatpakManager {
    /// Creates the built-in Flatpak manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(FLATPAK_ID).expect("Flatpak manager ID must remain valid"),
            "Flatpak",
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
        .expect("Flatpak descriptor must remain valid")
        .with_description("跨平台应用沙箱管理器")
        .with_authorization(AuthorizationHint::MayRequireElevation {
            message: Some(
                "System Flatpak operations may request desktop authorization.".to_owned(),
            ),
        });

        Self { descriptor }
    }

    /// Returns the installed display version of one Flatpak application.
    ///
    /// A bare application ID or full application ref must identify exactly one
    /// installed entry. Scoped direct callers should retain the target returned
    /// by [`PackageManager::installed`] instead of using this legacy helper.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when no installed application matches, or a
    /// typed command error when the installed listing fails.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        package_name: &str,
    ) -> ManagerResult<String> {
        self.validate_config(config)?;
        let installed = self.installed_entries(config).await?;
        let normalized_ref = normalize_app_ref(package_name).ok();
        let matches = installed
            .iter()
            .filter(|entry| {
                entry.application_id == package_name
                    || normalized_ref.as_deref() == Some(entry.reference.as_str())
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [entry] => Ok(entry.display_version()),
            [] => Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "flatpak application version is unavailable",
            )
            .with_detail(package_name)),
            _ => Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "flatpak application version is ambiguous across installations",
            )
            .with_detail(package_name)),
        }
    }

    /// Executes one Flatpak target while exposing normalized command progress.
    ///
    /// This compatibility surface lets the existing core dispatcher preserve
    /// its single-application flow until the UI consumes scoped targets.
    ///
    /// # Errors
    ///
    /// Returns a protocol or unsupported error for incomplete target identity,
    /// or a typed command error when Flatpak fails.
    #[allow(dead_code)]
    async fn execute_target_with_progress(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
        on_progress: impl FnMut(CommandProgress),
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        let command = self.write_command(config, action, target)?;
        run_command_with_progress(&command, on_progress).await
    }

    async fn installed_entries(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<Vec<InstalledEntry>> {
        self.validate_config(config)?;
        let flatpak_path = resolve_executable(config, FLATPAK_COMMAND);
        let spec = installed_command(&flatpak_path);
        let output = run_output(&spec).await?;
        if !output.status.success() {
            let tail = command_output_tail(&output.stdout, &output.stderr);
            return Err(command_status_error(&spec, output.status, &tail));
        }

        let stdout = decode_stdout(output, "flatpak installed listing is not valid UTF-8")?;
        parse_installed_entries(&stdout)
    }

    async fn list_updates(
        &self,
        config: &ManagerConfig,
        refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.validate_config(config)?;
        let installed = self.installed_entries(config).await?;
        let installed_by_identity = installed
            .iter()
            .map(|entry| (entry.identity(), entry))
            .collect::<HashMap<_, _>>();
        let flatpak_path = resolve_executable(config, FLATPAK_COMMAND);
        let mut updates = Vec::new();

        for installation in [Installation::System, Installation::User] {
            let spec = updates_command(&flatpak_path, installation, refresh);
            let output = run_output(&spec).await?;
            if !output.status.success() {
                let tail = command_output_tail(&output.stdout, &output.stderr);
                return Err(command_status_error(&spec, output.status, &tail));
            }

            let stdout = decode_stdout(output, "flatpak update listing is not valid UTF-8")?;
            for entry in parse_update_entries(&stdout)? {
                let identity = FlatpakIdentity {
                    installation,
                    reference: entry.reference.clone(),
                };
                let Some(installed) = installed_by_identity.get(&identity) else {
                    continue;
                };

                let current_version = installed.display_version();
                let advertised_version = display_version(&entry.version, &entry.branch);
                let available_version = if advertised_version == current_version {
                    format!("new build ({advertised_version})")
                } else {
                    advertised_version
                };
                let mut target =
                    PackageTarget::new(self.descriptor.id().clone(), entry.application_id.clone());
                target.scope = installation.package_scope();
                target.origin = Some(
                    PackageOrigin::new(if entry.origin.is_empty() {
                        &installed.origin
                    } else {
                        &entry.origin
                    })
                    .with_reference(&entry.reference),
                );
                updates.push(PackageUpdate::new(
                    target,
                    current_version,
                    available_version,
                ));
            }
        }

        Ok(updates)
    }

    async fn default_arch(&self, config: &ManagerConfig) -> ManagerResult<String> {
        let flatpak_path = resolve_executable(config, FLATPAK_COMMAND);
        let spec = CommandSpec::new(flatpak_path)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .arg("--default-arch");
        let output = run_output(&spec).await?;
        if !output.status.success() {
            let tail = command_output_tail(&output.stdout, &output.stderr);
            return Err(command_status_error(&spec, output.status, &tail));
        }

        let architecture =
            decode_stdout(output, "flatpak default architecture is not valid UTF-8")?
                .trim()
                .to_owned();
        if architecture.is_empty() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "flatpak default architecture is empty",
            ));
        }
        Ok(architecture)
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
                "flatpak package target belongs to another manager",
            )
            .with_detail(format!(
                "expected {}, received {} for package {}",
                self.descriptor.id(),
                target.manager_id,
                target.name
            )));
        }

        let flatpak_path = resolve_executable(config, FLATPAK_COMMAND);
        let scope_argument = match target.scope {
            PackageScope::System => Some("--system"),
            PackageScope::User => Some("--user"),
            PackageScope::Unknown => {
                return Ok(CommandSpec::new(flatpak_path)
                    .arg(command_name(action)?)
                    .arg("-y")
                    .arg(&target.name));
            }
            PackageScope::Project => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "flatpak project scope is not supported",
                ));
            }
            _ => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "flatpak target scope is not supported",
                ));
            }
        };

        let reference = target
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref())
            .filter(|reference| !reference.trim().is_empty())
            .ok_or_else(|| {
                ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "scoped flatpak target is missing its full ref",
                )
                .with_detail(&target.name)
            })
            .and_then(normalize_app_ref)?;
        if application_id_from_ref(&reference) != target.name {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "flatpak target name does not match its application ref",
            )
            .with_detail(format!("{} != {reference}", target.name)));
        }

        let mut command = CommandSpec::new(flatpak_path).arg(command_name(action)?);
        if let Some(scope) = scope_argument {
            command = command.arg(scope);
        }
        command = command.args(["-y", "--noninteractive"]);

        if action == PackageAction::Install {
            let remote = target
                .origin
                .as_ref()
                .map(|origin| origin.name.trim())
                .filter(|remote| !remote.is_empty());
            if remote.is_none() {
                return Err(ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "scoped flatpak install target is missing its remote",
                )
                .with_detail(&target.name));
            }
            if let Some(remote) = remote {
                command = command.arg(remote);
            }
        }

        Ok(command.arg(reference))
    }
}

impl Default for FlatpakManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for FlatpakManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(self.descriptor(), config, FLATPAK_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        let entries = self.installed_entries(config).await?;
        Ok(entries
            .into_iter()
            .map(|entry| entry.package_info(self.descriptor.id()))
            .collect())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.installed_entries(config).await?.len())
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
        let installed = self.installed_entries(config).await?;
        let installed_by_identity = installed
            .iter()
            .map(|entry| (entry.identity(), entry))
            .collect::<HashMap<_, _>>();
        let architecture = self.default_arch(config).await?;
        let flatpak_path = resolve_executable(config, FLATPAK_COMMAND);
        let mut packages = Vec::new();
        let mut seen = HashSet::new();

        for installation in [Installation::System, Installation::User] {
            let spec = search_command(&flatpak_path, installation, query);
            let output = run_output(&spec).await?;
            if !output.status.success() {
                let tail = command_output_tail(&output.stdout, &output.stderr);
                return Err(command_status_error(&spec, output.status, &tail));
            }

            let stdout = decode_stdout(output, "flatpak search output is not valid UTF-8")?;
            for entry in parse_search_entries(&stdout)? {
                let reference = normalize_app_ref(&format!(
                    "app/{}/{}/{}",
                    entry.application_id, architecture, entry.branch
                ))?;
                let identity = FlatpakIdentity {
                    installation,
                    reference: reference.clone(),
                };
                let version = installed_by_identity.get(&identity).map_or_else(
                    || NOT_INSTALLED_VERSION.to_owned(),
                    |entry| entry.display_version(),
                );

                for remote in entry.remotes {
                    if !seen.insert((identity.clone(), remote.clone())) {
                        continue;
                    }
                    let mut package = PackageInfo::new(
                        self.descriptor.id().clone(),
                        &entry.application_id,
                        &version,
                    );
                    package.description = entry.description.clone();
                    package.scope = installation.package_scope();
                    package.origin =
                        Some(PackageOrigin::new(remote).with_reference(reference.clone()));
                    packages.push(package);
                }
            }
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
        let commands = packages
            .iter()
            .map(|target| self.write_command(config, action, target))
            .collect::<ManagerResult<Vec<_>>>()?;

        let total = packages.len();
        progress.emit(ProgressEvent::Started { action, total });
        for (index, (target, command)) in packages.iter().zip(commands.iter()).enumerate() {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Installation {
    System,
    User,
}

impl Installation {
    fn parse(value: &str) -> ManagerResult<Self> {
        match value.trim() {
            "system" => Ok(Self::System),
            "user" => Ok(Self::User),
            name if !name.is_empty() => Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "named flatpak installations are not supported",
            )
            .with_detail(name)),
            _ => Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "flatpak installation scope is missing",
            )),
        }
    }

    fn package_scope(self) -> PackageScope {
        match self {
            Self::System => PackageScope::System,
            Self::User => PackageScope::User,
        }
    }

    fn argument(self) -> &'static str {
        match self {
            Self::System => "--system",
            Self::User => "--user",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FlatpakIdentity {
    installation: Installation,
    reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledEntry {
    application_id: String,
    name: String,
    version: String,
    branch: String,
    size: Option<u64>,
    origin: String,
    installation: Installation,
    reference: String,
}

impl InstalledEntry {
    fn identity(&self) -> FlatpakIdentity {
        FlatpakIdentity {
            installation: self.installation,
            reference: self.reference.clone(),
        }
    }

    fn display_version(&self) -> String {
        display_version(&self.version, &self.branch)
    }

    fn package_info(self, manager_id: &ManagerId) -> PackageInfo {
        let version = self.display_version();
        let mut package = PackageInfo::new(manager_id.clone(), self.application_id, version);
        package.description = (!self.name.is_empty()).then_some(self.name);
        package.size = self.size;
        package.scope = self.installation.package_scope();
        package.origin = Some(PackageOrigin::new(self.origin).with_reference(self.reference));
        package
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateEntry {
    application_id: String,
    reference: String,
    branch: String,
    version: String,
    origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchEntry {
    application_id: String,
    description: Option<String>,
    branch: String,
    remotes: Vec<String>,
}

fn installed_command(flatpak_path: &Path) -> CommandSpec {
    CommandSpec::new(flatpak_path)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .args(["list", "--app", INSTALLED_COLUMNS])
}

fn updates_command(flatpak_path: &Path, installation: Installation, refresh: bool) -> CommandSpec {
    let command = CommandSpec::new(flatpak_path)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .args(["remote-ls", installation.argument(), "--updates", "--app"]);
    if refresh {
        command.arg(UPDATE_COLUMNS)
    } else {
        command.args(["--cached", UPDATE_COLUMNS])
    }
}

fn search_command(flatpak_path: &Path, installation: Installation, query: &str) -> CommandSpec {
    CommandSpec::new(flatpak_path)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .args(["search", installation.argument(), SEARCH_COLUMNS, query])
}

fn parse_installed_entries(stdout: &str) -> ManagerResult<Vec<InstalledEntry>> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !first_field(line).eq_ignore_ascii_case("application"))
        .map(|line| {
            let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if fields.len() != 8 {
                return Err(ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "flatpak installed row is malformed",
                )
                .with_detail(line));
            }
            let application_id = required_field(fields[0], "flatpak application ID is missing")?;
            let branch = required_field(fields[3], "flatpak branch is missing")?;
            let installation = Installation::parse(fields[6])?;
            let reference = normalize_app_ref(fields[7])?;
            validate_ref_identity(&application_id, &branch, &reference)?;

            Ok(InstalledEntry {
                application_id,
                name: fields[1].to_owned(),
                version: fields[2].to_owned(),
                branch,
                size: parse_size(fields[4]),
                origin: fields[5].to_owned(),
                installation,
                reference,
            })
        })
        .collect()
}

fn parse_update_entries(stdout: &str) -> ManagerResult<Vec<UpdateEntry>> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !first_field(line).eq_ignore_ascii_case("application"))
        .map(|line| {
            let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if fields.len() != 6 {
                return Err(ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "flatpak update row is malformed",
                )
                .with_detail(line));
            }
            let application_id =
                required_field(fields[0], "flatpak update application ID is missing")?;
            let reference = normalize_app_ref(fields[1])?;
            let branch = required_field(fields[2], "flatpak update branch is missing")?;
            validate_ref_identity(&application_id, &branch, &reference)?;
            Ok(UpdateEntry {
                application_id,
                reference,
                branch,
                version: fields[3].to_owned(),
                origin: fields[5].to_owned(),
            })
        })
        .collect()
}

fn parse_search_entries(stdout: &str) -> ManagerResult<Vec<SearchEntry>> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !first_field(line).eq_ignore_ascii_case("application"))
        .map(|line| {
            let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if fields.len() != 6 {
                return Err(ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "flatpak search row is malformed",
                )
                .with_detail(line));
            }
            let application_id =
                required_field(fields[0], "flatpak search application ID is missing")?;
            let branch = required_field(fields[4], "flatpak search branch is missing")?;
            let remotes = fields[5]
                .split(',')
                .map(str::trim)
                .filter(|remote| !remote.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if remotes.is_empty() {
                return Err(ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "flatpak search remotes are missing",
                )
                .with_detail(line));
            }

            Ok(SearchEntry {
                application_id,
                description: (!fields[2].is_empty()).then(|| fields[2].to_owned()),
                branch,
                remotes,
            })
        })
        .collect()
}

fn normalize_app_ref(value: &str) -> ManagerResult<String> {
    let value = value.trim();
    let fields = value.split('/').collect::<Vec<_>>();
    if fields.len() == 3 && fields.iter().all(|field| !field.is_empty()) {
        return Ok(format!("app/{value}"));
    }
    if fields.len() == 4 && fields[0] == "app" && fields[1..].iter().all(|field| !field.is_empty())
    {
        return Ok(value.to_owned());
    }

    Err(ManagerError::new(
        ManagerErrorKind::Protocol,
        "flatpak application ref is malformed",
    )
    .with_detail(value))
}

fn application_id_from_ref(reference: &str) -> &str {
    reference.split('/').nth(1).unwrap_or_default()
}

fn validate_ref_identity(application_id: &str, branch: &str, reference: &str) -> ManagerResult<()> {
    let fields = reference.split('/').collect::<Vec<_>>();
    if fields.get(1) == Some(&application_id) && fields.get(3) == Some(&branch) {
        return Ok(());
    }

    Err(ManagerError::new(
        ManagerErrorKind::Protocol,
        "flatpak application columns do not match the full ref",
    )
    .with_detail(format!("{application_id} {branch} {reference}")))
}

fn parse_size(value: &str) -> Option<u64> {
    let mut fields = value.split_whitespace();
    let amount = fields.next()?.parse::<f64>().ok()?;
    let multiplier = match fields.next()?.to_ascii_uppercase().as_str() {
        "B" | "BYTE" | "BYTES" => 1.0,
        "KB" => 1_000.0,
        "MB" => 1_000_000.0,
        "GB" => 1_000_000_000.0,
        "KIB" => 1_024.0,
        "MIB" => 1_048_576.0,
        "GIB" => 1_073_741_824.0,
        _ => return None,
    };
    let bytes = amount * multiplier;
    (amount.is_finite() && amount >= 0.0 && bytes <= u64::MAX as f64).then_some(bytes as u64)
}

fn display_version(version: &str, branch: &str) -> String {
    if version.is_empty() {
        format!("branch: {branch}")
    } else if branch.is_empty() {
        version.to_owned()
    } else {
        format!("{version} ({branch})")
    }
}

fn first_field(line: &str) -> &str {
    line.split('\t').next().unwrap_or_default().trim()
}

fn required_field(value: &str, message: &str) -> ManagerResult<String> {
    if value.is_empty() {
        Err(ManagerError::new(ManagerErrorKind::Protocol, message))
    } else {
        Ok(value.to_owned())
    }
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

fn command_name(action: PackageAction) -> ManagerResult<&'static str> {
    match action {
        PackageAction::Install => Ok("install"),
        PackageAction::Update => Ok("update"),
        PackageAction::Uninstall => Ok("uninstall"),
        _ => Err(unsupported_action_error()),
    }
}

fn unsupported_action_error() -> ManagerError {
    ManagerError::new(
        ManagerErrorKind::Unsupported,
        "flatpak action is not supported",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn parses_scoped_installed_entries_and_size_units() {
        let entries = parse_installed_entries(
            "org.example.App\tExample\t1.2\tstable\t14.8\u{a0}MB\tflathub\tsystem\torg.example.App/x86_64/stable\n\
             org.example.App\tExample User\t\tbeta\t2 MiB\tflathub-beta\tuser\tapp/org.example.App/x86_64/beta\n",
        )
        .expect("parse Flatpak installed fixture");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].installation, Installation::System);
        assert_eq!(entries[0].size, Some(14_800_000));
        assert_eq!(entries[1].installation, Installation::User);
        assert_eq!(entries[1].size, Some(2_097_152));
        assert_eq!(entries[0].reference, "app/org.example.App/x86_64/stable");
        assert_eq!(entries[1].reference, "app/org.example.App/x86_64/beta");
        assert_eq!(entries[1].display_version(), "branch: beta");
    }

    #[test]
    fn rejects_named_installations_and_malformed_refs() {
        let named = parse_installed_entries(
            "org.example.App\tExample\t1\tstable\t1 MB\tremote\twork\torg.example.App/x86_64/stable\n",
        )
        .expect_err("reject named installation");
        assert_eq!(named.kind(), ManagerErrorKind::Unsupported);

        let malformed = normalize_app_ref("org.example.App/stable")
            .expect_err("reject incomplete application ref");
        assert_eq!(malformed.kind(), ManagerErrorKind::Protocol);

        let runtime = normalize_app_ref("runtime/org.example.Platform/x86_64/stable")
            .expect_err("reject runtime ref");
        assert_eq!(runtime.kind(), ManagerErrorKind::Protocol);

        let mismatch = parse_installed_entries(
            "org.example.App\tExample\t1\tstable\t1 MB\tremote\tsystem\torg.other.App/x86_64/stable\n",
        )
        .expect_err("reject mismatched application identity");
        assert_eq!(mismatch.kind(), ManagerErrorKind::Protocol);
    }

    #[test]
    fn parses_updates_search_remotes_and_new_build_versions() {
        let updates = parse_update_entries(
            "org.example.App\tapp/org.example.App/x86_64/stable\tstable\t1.2\tabc123\tflathub\n",
        )
        .expect("parse Flatpak updates");
        assert_eq!(updates[0].reference, "app/org.example.App/x86_64/stable");

        let search = parse_search_entries(
            "org.example.App\tExample\tDescription\t1.2\tstable\tflathub, flathub-beta\n",
        )
        .expect("parse Flatpak search fixture");
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].remotes, ["flathub", "flathub-beta"]);
        assert_eq!(display_version("1.2", "stable"), "1.2 (stable)");
        assert_eq!(display_version("", "stable"), "branch: stable");
    }

    #[test]
    fn read_commands_preserve_scope_cache_and_full_columns() {
        let flatpak = Path::new("/custom/flatpak");
        let installed = installed_command(flatpak);
        assert_eq!(installed.program(), flatpak);
        assert_eq!(
            installed.arguments(),
            ["list", "--app", INSTALLED_COLUMNS]
                .map(OsString::from)
                .as_slice()
        );

        let cached = updates_command(flatpak, Installation::System, false);
        assert_eq!(
            cached.arguments(),
            [
                "remote-ls",
                "--system",
                "--updates",
                "--app",
                "--cached",
                UPDATE_COLUMNS,
            ]
            .map(OsString::from)
            .as_slice()
        );
        let refreshed = updates_command(flatpak, Installation::User, true);
        assert_eq!(
            refreshed.arguments(),
            ["remote-ls", "--user", "--updates", "--app", UPDATE_COLUMNS,]
                .map(OsString::from)
                .as_slice()
        );
        assert_eq!(cached.environment(), installed.environment());
    }

    #[test]
    fn write_commands_freeze_scope_ref_remote_and_legacy_unknown() {
        let manager = FlatpakManager::new();
        let config = ManagerConfig::new(manager.descriptor().id().clone())
            .with_executable("/custom/flatpak");
        let mut target = PackageTarget::new(manager.descriptor().id().clone(), "org.example.App");
        target.scope = PackageScope::User;
        target.origin =
            Some(PackageOrigin::new("flathub").with_reference("app/org.example.App/x86_64/stable"));

        let install = manager
            .write_command(&config, PackageAction::Install, &target)
            .expect("build scoped Flatpak install");
        assert_eq!(
            install.arguments(),
            [
                "install",
                "--user",
                "-y",
                "--noninteractive",
                "flathub",
                "app/org.example.App/x86_64/stable",
            ]
            .map(OsString::from)
            .as_slice()
        );

        let update = manager
            .write_command(&config, PackageAction::Update, &target)
            .expect("build scoped Flatpak update");
        assert_eq!(
            update.arguments(),
            [
                "update",
                "--user",
                "-y",
                "--noninteractive",
                "app/org.example.App/x86_64/stable",
            ]
            .map(OsString::from)
            .as_slice()
        );

        let legacy = PackageTarget::new(manager.descriptor().id().clone(), "org.example.Legacy");
        let legacy_command = manager
            .write_command(&config, PackageAction::Uninstall, &legacy)
            .expect("build legacy Flatpak uninstall");
        assert_eq!(
            legacy_command.arguments(),
            ["uninstall", "-y", "org.example.Legacy"]
                .map(OsString::from)
                .as_slice()
        );
    }

    #[test]
    fn rejects_incomplete_scoped_and_project_targets() {
        let manager = FlatpakManager::new();
        let config = ManagerConfig::new(manager.descriptor().id().clone());
        let mut scoped = PackageTarget::new(manager.descriptor().id().clone(), "org.example.App");
        scoped.scope = PackageScope::System;
        assert_eq!(
            manager
                .write_command(&config, PackageAction::Update, &scoped)
                .expect_err("reject scoped target without ref")
                .kind(),
            ManagerErrorKind::Protocol
        );

        scoped.scope = PackageScope::Project;
        assert_eq!(
            manager
                .write_command(&config, PackageAction::Update, &scoped)
                .expect_err("reject project scope")
                .kind(),
            ManagerErrorKind::Unsupported
        );
    }
}
