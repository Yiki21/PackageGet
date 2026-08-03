use std::{collections::HashSet, path::Path, process::Output, time::Duration};

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
    progress::{CommandProgress, run_cancellable_command_with_progress, run_command_with_progress},
};

const HOMEBREW_ID: &str = "builtin:homebrew";
const HOMEBREW_COMMAND: &str = "brew";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const READ_TIMEOUT: Duration = Duration::from_secs(90);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(180);
const SEARCH_INFO_BATCH_SIZE: usize = 32;

/// Direct `updater-manager-api` implementation for Homebrew.
#[derive(Debug, Clone)]
pub struct HomebrewManager {
    descriptor: ManagerDescriptor,
}

impl HomebrewManager {
    /// Creates the built-in Homebrew manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(HOMEBREW_ID).expect("Homebrew manager ID must remain valid"),
            "Homebrew",
            ManagerCategory::Application,
            SupportedPlatforms::from([Platform::Linux, Platform::MacOs]),
            ManagerCapabilities::from([
                ManagerCapability::Installed,
                ManagerCapability::Updates,
                ManagerCapability::Search,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ]),
        )
        .expect("Homebrew descriptor must remain valid")
        .with_description("macOS/Linux 包管理器")
        .with_authorization(AuthorizationHint::MayRequireElevation {
            message: Some("Some Homebrew casks may request system authorization.".to_owned()),
        });

        Self { descriptor }
    }

    /// Returns the installed version of one unambiguous formula or cask.
    ///
    /// A direct reference has the form `formula:FULL_NAME` or
    /// `cask:FULL_TOKEN`. A bare name is accepted only when it identifies one
    /// installed package across both kinds.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the package is missing or ambiguous, or
    /// a typed command error when the installed inventory fails.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        package_name: &str,
    ) -> ManagerResult<String> {
        let packages = self.installed_packages(config).await?;
        let reference = BrewReference::parse(package_name).ok();
        let matches = packages
            .iter()
            .filter(|package| {
                reference.as_ref().is_some_and(|reference| {
                    package.kind == reference.kind && package.canonical_name == reference.name
                }) || package.canonical_name == package_name
                    || package.short_name == package_name
            })
            .collect::<Vec<_>>();

        match matches.as_slice() {
            [package] => Ok(package.current_version.clone()),
            [] => Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "homebrew package version is unavailable",
            )
            .with_detail(package_name)),
            _ => Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "homebrew package version is ambiguous across formulae and casks",
            )
            .with_detail(package_name)),
        }
    }

    /// Executes one legacy or scoped Homebrew target with command progress.
    ///
    /// # Errors
    ///
    /// Returns a protocol or unsupported error for an invalid target, or a
    /// typed command error when Homebrew fails.
    pub async fn execute_target_with_progress(
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

    async fn installed_inventory(&self, config: &ManagerConfig) -> ManagerResult<BrewInfo> {
        self.validate_config(config)?;
        let brew_path = resolve_executable(config, HOMEBREW_COMMAND);
        run_json(
            &installed_command(&brew_path),
            READ_TIMEOUT,
            "homebrew installed inventory is invalid",
        )
        .await
    }

    async fn installed_packages(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<Vec<InstalledPackage>> {
        self.installed_inventory(config).await?.installed_packages()
    }

    async fn list_updates(
        &self,
        config: &ManagerConfig,
        refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        self.validate_config(config)?;
        let brew_path = resolve_executable(config, HOMEBREW_COMMAND);
        if refresh {
            run_success(
                &refresh_command(&brew_path),
                UPDATE_TIMEOUT,
                "homebrew update timed out",
            )
            .await?;
        }

        let installed = self.installed_packages(config).await?;
        let outdated: BrewOutdated = run_json(
            &outdated_command(&brew_path),
            READ_TIMEOUT,
            "homebrew outdated inventory is invalid",
        )
        .await?;
        let mut updates = Vec::with_capacity(outdated.formulae.len() + outdated.casks.len());
        for (kind, entry) in outdated
            .formulae
            .into_iter()
            .map(|entry| (BrewKind::Formula, entry))
            .chain(
                outdated
                    .casks
                    .into_iter()
                    .map(|entry| (BrewKind::Cask, entry)),
            )
        {
            let package = find_unique_installed(&installed, kind, &entry.name)?;
            let current_version = required_versions(&entry.installed_versions, &entry.name)?;
            let available_version = required_text(
                entry.current_version,
                "homebrew outdated current version is missing",
            )?;
            updates.push(PackageUpdate::new(
                package.target(self.descriptor.id()),
                current_version,
                available_version,
            ));
        }
        Ok(updates)
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            return Ok(());
        }

        Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "homebrew configuration ID does not match the manager",
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
        target: &PackageTarget,
    ) -> ManagerResult<CommandSpec> {
        ensure_supported_action(action)?;
        if &target.manager_id != self.descriptor.id() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "homebrew package target belongs to another manager",
            )
            .with_detail(format!(
                "expected {}, received {} for package {}",
                self.descriptor.id(),
                target.manager_id,
                target.name
            )));
        }

        let brew_path = resolve_executable(config, HOMEBREW_COMMAND);
        if target.version.is_some() {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "version-pinned homebrew targets are not supported",
            )
            .with_detail(&target.name));
        }
        match target.scope {
            PackageScope::Unknown => {
                return Ok(direct_command(&brew_path)
                    .arg(command_name(action)?)
                    .arg(&target.name));
            }
            PackageScope::User => {}
            PackageScope::System | PackageScope::Project => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "homebrew target scope is not supported",
                )
                .with_detail(&target.name));
            }
            _ => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "homebrew target scope is not supported",
                )
                .with_detail(&target.name));
            }
        }

        let reference = target
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref())
            .filter(|reference| !reference.trim().is_empty())
            .ok_or_else(|| {
                ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "scoped homebrew target is missing its typed reference",
                )
                .with_detail(&target.name)
            })
            .and_then(BrewReference::parse)?;
        if reference.short_name() != target.name {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "homebrew target name does not match its typed reference",
            )
            .with_detail(format!("{} != {}", target.name, reference.render())));
        }
        let origin_tap = target
            .origin
            .as_ref()
            .map(|origin| origin.name.trim())
            .unwrap_or_default();
        if origin_tap.is_empty() || origin_tap != reference.tap() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "homebrew target tap does not match its typed reference",
            )
            .with_detail(format!("{origin_tap} != {}", reference.tap())));
        }

        let mut command = direct_command(&brew_path)
            .arg(command_name(action)?)
            .arg(reference.kind.argument());
        if matches!(action, PackageAction::Install | PackageAction::Update) {
            command = command.arg("--yes");
        }
        Ok(command.arg(reference.name))
    }
}

impl Default for HomebrewManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for HomebrewManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        if !cfg!(any(target_os = "linux", target_os = "macos")) {
            return Ok(ManagerAvailability::Unavailable {
                reason: updater_manager_api::AvailabilityReason::UnsupportedPlatform {
                    platform: Platform::current(),
                },
            });
        }

        Ok(manager_availability(config, HOMEBREW_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        Ok(self
            .installed_packages(config)
            .await?
            .into_iter()
            .map(|package| package.info(self.descriptor.id()))
            .collect())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.installed_packages(config).await?.len())
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
        let brew_path = resolve_executable(config, HOMEBREW_COMMAND);
        let mut packages = Vec::new();
        for kind in [BrewKind::Formula, BrewKind::Cask] {
            let names = search_names(&search_command(&brew_path, kind, query)).await?;
            if names.is_empty() {
                continue;
            }
            for batch in names.chunks(SEARCH_INFO_BATCH_SIZE) {
                let info: BrewInfo = run_json(
                    &info_command(&brew_path, kind, batch),
                    READ_TIMEOUT,
                    "homebrew search metadata is invalid",
                )
                .await?;
                packages.extend(info.packages_for_kind(kind, self.descriptor.id())?);
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
enum BrewKind {
    Formula,
    Cask,
}

impl BrewKind {
    fn argument(self) -> &'static str {
        match self {
            Self::Formula => "--formula",
            Self::Cask => "--cask",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Formula => "formula",
            Self::Cask => "cask",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BrewReference {
    kind: BrewKind,
    name: String,
}

impl BrewReference {
    fn parse(value: &str) -> ManagerResult<Self> {
        let (kind, name) = value.trim().split_once(':').ok_or_else(|| {
            ManagerError::new(
                ManagerErrorKind::Protocol,
                "homebrew package reference is malformed",
            )
            .with_detail(value)
        })?;
        let kind = match kind {
            "formula" => BrewKind::Formula,
            "cask" => BrewKind::Cask,
            _ => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "homebrew package reference kind is unsupported",
                )
                .with_detail(value));
            }
        };
        let name = required_text(
            name.to_owned(),
            "homebrew package reference name is missing",
        )?;
        if name.starts_with('-')
            || name.chars().any(char::is_whitespace)
            || name.split('/').count() < 3
            || name
                .split('/')
                .any(|component| component.is_empty() || component.starts_with('-'))
        {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "homebrew package reference must include tap and package name",
            )
            .with_detail(value));
        }
        Ok(Self { kind, name })
    }

    fn short_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or_default()
    }

    fn tap(&self) -> &str {
        self.name.rsplit_once('/').map_or("", |(tap, _package)| tap)
    }

    fn render(&self) -> String {
        format!("{}:{}", self.kind.prefix(), self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledPackage {
    kind: BrewKind,
    short_name: String,
    canonical_name: String,
    tap: String,
    version: String,
    current_version: String,
    description: Option<String>,
    homepage: Option<String>,
}

impl InstalledPackage {
    fn reference(&self) -> BrewReference {
        BrewReference {
            kind: self.kind,
            name: self.canonical_name.clone(),
        }
    }

    fn origin(&self) -> PackageOrigin {
        PackageOrigin::new(&self.tap).with_reference(self.reference().render())
    }

    fn target(&self, manager_id: &ManagerId) -> PackageTarget {
        let mut target = PackageTarget::new(manager_id.clone(), &self.short_name);
        target.scope = PackageScope::User;
        target.origin = Some(self.origin());
        target
    }

    fn info(self, manager_id: &ManagerId) -> PackageInfo {
        let origin = self.origin();
        let mut package = PackageInfo::new(manager_id.clone(), self.short_name, self.version);
        package.description = self.description;
        package.homepage = self.homepage;
        package.scope = PackageScope::User;
        package.origin = Some(origin);
        package
    }
}

#[derive(Debug, Deserialize)]
struct BrewInfo {
    #[serde(default)]
    formulae: Vec<FormulaInfo>,
    #[serde(default)]
    casks: Vec<CaskInfo>,
}

impl BrewInfo {
    fn installed_packages(self) -> ManagerResult<Vec<InstalledPackage>> {
        let mut packages = Vec::with_capacity(self.formulae.len() + self.casks.len());
        for formula in self.formulae {
            if !formula.installed.is_empty() {
                packages.push(formula.package(true)?);
            }
        }
        for cask in self.casks {
            if !cask.installed.versions().is_empty() {
                packages.push(cask.package(true)?);
            }
        }
        ensure_unique_identities(&packages)?;
        Ok(packages)
    }

    fn packages_for_kind(
        self,
        kind: BrewKind,
        manager_id: &ManagerId,
    ) -> ManagerResult<Vec<PackageInfo>> {
        if (kind == BrewKind::Formula && !self.casks.is_empty())
            || (kind == BrewKind::Cask && !self.formulae.is_empty())
        {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "homebrew search metadata returned the wrong package kind",
            ));
        }
        let packages = match kind {
            BrewKind::Formula => self
                .formulae
                .into_iter()
                .map(|formula| formula.package(false))
                .collect::<ManagerResult<Vec<_>>>()?,
            BrewKind::Cask => self
                .casks
                .into_iter()
                .map(|cask| cask.package(false))
                .collect::<ManagerResult<Vec<_>>>()?,
        };
        ensure_unique_identities(&packages)?;
        Ok(packages
            .into_iter()
            .map(|package| package.info(manager_id))
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct FormulaInfo {
    #[serde(default)]
    name: String,
    #[serde(default)]
    full_name: String,
    #[serde(default)]
    tap: String,
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    installed: Vec<FormulaInstallation>,
    #[serde(default)]
    linked_keg: Option<String>,
}

impl FormulaInfo {
    fn package(self, require_installed: bool) -> ManagerResult<InstalledPackage> {
        let short_name = required_text(self.name, "homebrew formula name is missing")?;
        let full_name = required_text(self.full_name, "homebrew formula full name is missing")?;
        let tap = required_text(self.tap, "homebrew formula tap is missing")?;
        let canonical_name = canonical_name(&tap, &short_name, &full_name)?;
        let versions = self
            .installed
            .into_iter()
            .map(|installed| installed.version)
            .filter(|version| !version.trim().is_empty())
            .collect::<Vec<_>>();
        let (version, current_version) = if versions.is_empty() && !require_installed {
            (
                NOT_INSTALLED_VERSION.to_owned(),
                NOT_INSTALLED_VERSION.to_owned(),
            )
        } else {
            let display = required_versions(&versions, &canonical_name)?;
            let current = if let Some(linked_keg) = self.linked_keg {
                if !versions.contains(&linked_keg) {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Protocol,
                        "homebrew linked keg is absent from installed versions",
                    )
                    .with_detail(format!("{canonical_name}: {linked_keg}")));
                }
                linked_keg
            } else {
                versions.last().cloned().ok_or_else(|| {
                    ManagerError::new(
                        ManagerErrorKind::Protocol,
                        "homebrew formula current version is missing",
                    )
                    .with_detail(&canonical_name)
                })?
            };
            (display, current)
        };
        Ok(InstalledPackage {
            kind: BrewKind::Formula,
            short_name,
            canonical_name,
            tap,
            version,
            current_version,
            description: self.desc,
            homepage: self.homepage,
        })
    }
}

#[derive(Debug, Deserialize)]
struct FormulaInstallation {
    #[serde(default)]
    version: String,
}

#[derive(Debug, Deserialize)]
struct CaskInfo {
    #[serde(default)]
    token: String,
    #[serde(default)]
    full_token: String,
    #[serde(default)]
    tap: String,
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    installed: CaskInstalled,
}

impl CaskInfo {
    fn package(self, require_installed: bool) -> ManagerResult<InstalledPackage> {
        let short_name = required_text(self.token, "homebrew cask token is missing")?;
        let full_name = required_text(self.full_token, "homebrew cask full token is missing")?;
        let tap = required_text(self.tap, "homebrew cask tap is missing")?;
        let canonical_name = canonical_name(&tap, &short_name, &full_name)?;
        let versions = self.installed.versions();
        let version = if versions.is_empty() && !require_installed {
            NOT_INSTALLED_VERSION.to_owned()
        } else {
            required_versions(&versions, &canonical_name)?
        };
        let current_version = versions.last().cloned().unwrap_or_else(|| version.clone());
        Ok(InstalledPackage {
            kind: BrewKind::Cask,
            short_name,
            canonical_name,
            tap,
            version,
            current_version,
            description: self.desc,
            homepage: self.homepage,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum CaskInstalled {
    One(String),
    Many(Vec<String>),
    Null(()),
    #[default]
    Missing,
}

impl CaskInstalled {
    fn versions(&self) -> Vec<String> {
        match self {
            Self::One(version) if !version.trim().is_empty() => vec![version.clone()],
            Self::Many(versions) => versions
                .iter()
                .filter(|version| !version.trim().is_empty())
                .cloned()
                .collect(),
            Self::One(_) | Self::Null(()) | Self::Missing => Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct BrewOutdated {
    #[serde(default)]
    formulae: Vec<OutdatedEntry>,
    #[serde(default)]
    casks: Vec<OutdatedEntry>,
}

#[derive(Debug, Deserialize)]
struct OutdatedEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    installed_versions: Vec<String>,
    #[serde(default)]
    current_version: String,
}

fn installed_command(brew_path: &Path) -> CommandSpec {
    read_command(brew_path).args(["info", "--json=v2", "--installed"])
}

fn outdated_command(brew_path: &Path) -> CommandSpec {
    read_command(brew_path).args(["outdated", "--json=v2"])
}

fn refresh_command(brew_path: &Path) -> CommandSpec {
    CommandSpec::new(brew_path)
        .env_remove("HOMEBREW_NO_AUTO_UPDATE")
        .env("HOMEBREW_NO_ANALYTICS", "1")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .arg("update")
}

fn search_command(brew_path: &Path, kind: BrewKind, query: &str) -> CommandSpec {
    read_command(brew_path).args(["search", kind.argument(), query])
}

fn info_command(brew_path: &Path, kind: BrewKind, names: &[String]) -> CommandSpec {
    read_command(brew_path)
        .args(["info", "--json=v2", kind.argument()])
        .args(names)
}

fn read_command(brew_path: &Path) -> CommandSpec {
    direct_command(brew_path).env("HOMEBREW_NO_COLOR", "1")
}

fn direct_command(brew_path: &Path) -> CommandSpec {
    CommandSpec::new(brew_path)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ANALYTICS", "1")
        .env("HOMEBREW_NO_ASK", "1")
        .env("HOMEBREW_NO_INSTALL_CLEANUP", "1")
        .env("LC_ALL", "C")
        .env("LANG", "C")
}

async fn run_json<T: for<'de> Deserialize<'de>>(
    spec: &CommandSpec,
    duration: Duration,
    message: &str,
) -> ManagerResult<T> {
    let output = run_success(spec, duration, "homebrew command timed out").await?;
    serde_json::from_slice(&output.stdout).map_err(|error| {
        ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(error.to_string())
    })
}

async fn run_success(
    spec: &CommandSpec,
    duration: Duration,
    timeout_message: &str,
) -> ManagerResult<Output> {
    let output = timeout(duration, run_output(spec)).await.map_err(|_| {
        ManagerError::new(ManagerErrorKind::Timeout, timeout_message)
            .with_detail(command_label(spec))
    })??;
    if output.status.success() {
        return Ok(output);
    }
    Err(command_status_error(
        spec,
        output.status,
        &command_output_tail(&output),
    ))
}

async fn search_names(spec: &CommandSpec) -> ManagerResult<Vec<String>> {
    let output = timeout(READ_TIMEOUT, run_output(spec))
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, "homebrew search timed out")
                .with_detail(command_label(spec))
        })??;
    if !output.status.success() {
        let tail = command_output_tail(&output);
        if tail.contains("No formulae or casks found") {
            return Ok(Vec::new());
        }
        return Err(command_status_error(spec, output.status, &tail));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        ManagerError::new(
            ManagerErrorKind::Protocol,
            "homebrew search output is not valid UTF-8",
        )
        .with_detail(error.to_string())
    })?;
    let mut seen = HashSet::new();
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("==>"))
        .flat_map(str::split_whitespace)
        .filter(|name| seen.insert((*name).to_owned()))
        .map(ToOwned::to_owned)
        .collect())
}

fn ensure_unique_identities(packages: &[InstalledPackage]) -> ManagerResult<()> {
    let mut identities = HashSet::new();
    for package in packages {
        if !identities.insert((package.kind, package.canonical_name.as_str())) {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "homebrew inventory contains a duplicate typed identity",
            )
            .with_detail(package.reference().render()));
        }
    }
    Ok(())
}

fn find_unique_installed<'a>(
    installed: &'a [InstalledPackage],
    kind: BrewKind,
    name: &str,
) -> ManagerResult<&'a InstalledPackage> {
    let matches = installed
        .iter()
        .filter(|package| {
            package.kind == kind && (package.canonical_name == name || package.short_name == name)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [package] => Ok(package),
        [] => Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "homebrew outdated package is missing from the installed inventory",
        )
        .with_detail(name)),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "homebrew outdated package identity is ambiguous",
        )
        .with_detail(name)),
    }
}

fn required_versions(versions: &[String], name: &str) -> ManagerResult<String> {
    let versions = versions
        .iter()
        .map(|version| version.trim())
        .filter(|version| !version.is_empty())
        .collect::<Vec<_>>();
    if versions.is_empty() {
        return Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "homebrew installed version is missing",
        )
        .with_detail(name));
    }
    Ok(versions.join(", "))
}

fn canonical_name(tap: &str, short_name: &str, reported_name: &str) -> ManagerResult<String> {
    if tap.starts_with('-')
        || tap.chars().any(char::is_whitespace)
        || tap.split('/').count() < 2
        || tap.split('/').any(str::is_empty)
        || short_name.starts_with('-')
        || short_name.chars().any(char::is_whitespace)
        || short_name.contains('/')
    {
        return Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "homebrew package tap or name is malformed",
        )
        .with_detail(format!("{tap}/{short_name}")));
    }
    let canonical = format!("{tap}/{short_name}");
    if reported_name == short_name || reported_name == canonical {
        return Ok(canonical);
    }

    Err(ManagerError::new(
        ManagerErrorKind::Protocol,
        "homebrew reported package identity does not match its tap and name",
    )
    .with_detail(format!("{reported_name} != {canonical}")))
}

fn required_text(value: String, message: &str) -> ManagerResult<String> {
    if value.trim().is_empty() {
        Err(ManagerError::new(ManagerErrorKind::Protocol, message))
    } else {
        Ok(value)
    }
}

fn command_output_tail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        stderr.trim().to_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

fn command_label(spec: &CommandSpec) -> String {
    let mut command = spec.program().to_string_lossy().into_owned();
    for argument in spec.arguments() {
        command.push(' ');
        command.push_str(&argument.to_string_lossy());
    }
    command
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    command_name(action).map(|_| ())
}

fn command_name(action: PackageAction) -> ManagerResult<&'static str> {
    match action {
        PackageAction::Install => Ok("install"),
        PackageAction::Update => Ok("upgrade"),
        PackageAction::Uninstall => Ok("uninstall"),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "homebrew action is not supported",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command as StdCommand};

    use tempfile::tempdir;

    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn timed_out_command_is_terminated_on_drop() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().expect("create timeout test directory");
        let executable = directory.path().join("slow-brew");
        let pid_file = directory.path().join("pid");
        fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nsleep 10\n",
                pid_file.display()
            ),
        )
        .expect("write slow fake Homebrew executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("mark slow fake Homebrew executable");

        let error = run_success(
            &CommandSpec::new(&executable),
            Duration::from_millis(100),
            "fake Homebrew command timed out",
        )
        .await
        .expect_err("time out fake Homebrew command");
        assert_eq!(error.kind(), ManagerErrorKind::Timeout);

        tokio::time::sleep(Duration::from_millis(100)).await;
        let pid = fs::read_to_string(pid_file).expect("read fake Homebrew PID");
        assert!(
            !StdCommand::new("kill")
                .args(["-0", pid.trim()])
                .status()
                .expect("check fake Homebrew PID")
                .success(),
            "timed-out Homebrew child {pid} is still running"
        );
    }

    #[test]
    fn cask_installed_accepts_string_array_and_null_shapes() {
        for (json, expected) in [
            (r#""1.0""#, vec!["1.0"]),
            (r#"["1.0","2.0"]"#, vec!["1.0", "2.0"]),
            ("null", Vec::new()),
        ] {
            let installed: CaskInstalled =
                serde_json::from_str(json).expect("parse cask installed shape");
            assert_eq!(installed.versions(), expected);
        }
    }

    #[test]
    fn typed_references_reject_unqualified_and_option_like_names() {
        for reference in [
            "formula:jq",
            "formula:homebrew/core/-jq",
            "cask:homebrew/cask/bad name",
            "runtime:homebrew/core/jq",
        ] {
            assert_eq!(
                BrewReference::parse(reference)
                    .expect_err("reject malformed Homebrew reference")
                    .kind(),
                ManagerErrorKind::Protocol
            );
        }
    }
}
