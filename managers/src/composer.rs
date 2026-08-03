use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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

const COMPOSER_ID: &str = "builtin:composer-global";
const COMPOSER_COMMAND: &str = "composer";
const ORIGIN_NAME: &str = "Composer Global";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_CONSTRAINT_LENGTH: usize = 512;

/// Direct `updater-manager-api` implementation for Composer global dependencies.
#[derive(Debug, Clone)]
pub struct ComposerGlobalManager {
    descriptor: ManagerDescriptor,
}

impl ComposerGlobalManager {
    /// Creates the built-in Composer Global manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(COMPOSER_ID).expect("Composer Global manager ID must remain valid"),
            "Composer Global",
            ManagerCategory::Development,
            SupportedPlatforms::from([Platform::Linux, Platform::Windows, Platform::MacOs]),
            ManagerCapabilities::from([
                ManagerCapability::Installed,
                ManagerCapability::Updates,
                ManagerCapability::Search,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ]),
        )
        .expect("Composer Global descriptor must remain valid")
        .with_description("Direct current-user dependencies installed in Composer's global home")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            Ok(())
        } else {
            Err(protocol(
                "Composer Global configuration ID does not match the manager",
                &format!("expected {}, received {}", self.descriptor.id(), config.id),
            ))
        }
    }

    async fn environment(&self, config: &ManagerConfig) -> ManagerResult<ComposerEnvironment> {
        self.validate_config(config)?;
        let composer = resolve_executable(config, COMPOSER_COMMAND);
        let output = run_success(
            &composer_command(&composer).args([
                "global",
                "config",
                "home",
                "--absolute",
                "--no-interaction",
                "--no-ansi",
            ]),
            "Composer global home query timed out",
        )
        .await?;
        let value = decode_utf8(
            &output.stdout,
            "Composer global home response is not valid UTF-8",
        )?;
        let home = validate_home(single_line(
            &value,
            "Composer global home response is malformed",
        )?)?;
        Ok(ComposerEnvironment { home })
    }

    async fn requirements(
        &self,
        environment: &ComposerEnvironment,
    ) -> ManagerResult<ComposerRequirements> {
        let path = environment.home.join("composer.json");
        let value = match tokio::fs::read(&path).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ComposerRequirements::default());
            }
            Err(error) => {
                return Err(ManagerError::new(
                    ManagerErrorKind::Permission,
                    "failed to read Composer global manifest",
                )
                .with_detail(format!("{}: {error}", path.display())));
            }
        };
        parse_requirements(&value)
    }

    async fn installed_entries(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<(
        ComposerEnvironment,
        ComposerRequirements,
        Vec<InstalledPackage>,
    )> {
        let environment = self.environment(config).await?;
        let requirements = self.requirements(&environment).await?;
        if requirements.runtime.is_empty() {
            return Ok((environment, requirements, Vec::new()));
        }
        let composer = resolve_executable(config, COMPOSER_COMMAND);
        let output = run_success(
            &environment.command(&composer).args([
                "global",
                "show",
                "--direct",
                "--format=json",
                "--no-interaction",
                "--no-ansi",
            ]),
            "Composer global installed listing timed out",
        )
        .await?;
        let value = decode_utf8(
            &output.stdout,
            "Composer global installed listing is not valid UTF-8",
        )?;
        let installed = parse_installed(&value, &requirements)?;
        Ok((environment, requirements, installed))
    }

    async fn write_command(
        &self,
        config: &ManagerConfig,
        environment: &ComposerEnvironment,
        action: PackageAction,
        target: &PackageTarget,
    ) -> ManagerResult<CommandSpec> {
        ensure_supported_action(action)?;
        if &target.manager_id != self.descriptor.id() {
            return Err(protocol(
                "Composer Global target belongs to another manager",
                &target.name,
            ));
        }
        validate_package_name(&target.name)?;
        if target.scope != PackageScope::User {
            return Err(protocol(
                "Composer Global target scope must be user",
                &target.name,
            ));
        }
        let reference = ComposerReference::parse(target.origin.as_ref().ok_or_else(|| {
            protocol(
                "Composer Global target is missing its typed origin",
                &target.name,
            )
        })?)?;
        if reference.home != environment.home || reference.package != target.name {
            return Err(protocol(
                "Composer Global target origin does not match the current home and package",
                &target.name,
            ));
        }

        let requirements = self.requirements(environment).await?;
        let composer = resolve_executable(config, COMPOSER_COMMAND);
        let command = environment.command(&composer);
        match action {
            PackageAction::Install => {
                if reference.constraint.is_some() {
                    return Err(protocol(
                        "Composer Global install target must use a search origin",
                        &target.name,
                    ));
                }
                if requirements.runtime.contains_key(&target.name) {
                    return Err(protocol(
                        "Composer Global install target is already a direct dependency",
                        &target.name,
                    ));
                }
                let package = match target.version.as_deref() {
                    Some(constraint) => {
                        validate_constraint(constraint)?;
                        format!("{}:{constraint}", target.name)
                    }
                    None => target.name.clone(),
                };
                Ok(command.args([
                    "global",
                    "require",
                    "--no-interaction",
                    "--no-progress",
                    "--no-ansi",
                    "--",
                    package.as_str(),
                ]))
            }
            PackageAction::Update | PackageAction::Uninstall => {
                if target.version.is_some() {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "version-pinned Composer Global update and uninstall are not supported",
                    )
                    .with_detail(&target.name));
                }
                let expected_constraint =
                    requirements.runtime.get(&target.name).ok_or_else(|| {
                        protocol(
                            "Composer Global target is not a current direct dependency",
                            &target.name,
                        )
                    })?;
                if reference.constraint.as_deref() != Some(expected_constraint) {
                    return Err(protocol(
                        "Composer Global target constraint is stale or malformed",
                        &target.name,
                    ));
                }
                let verb = if action == PackageAction::Update {
                    "update"
                } else {
                    "remove"
                };
                let mut command = command.args(["global", verb]);
                if action == PackageAction::Update {
                    command = command.arg("--with-dependencies");
                }
                Ok(command.args([
                    "--no-interaction",
                    "--no-progress",
                    "--no-ansi",
                    "--",
                    target.name.as_str(),
                ]))
            }
            _ => unreachable!("supported actions were checked above"),
        }
    }
}

impl Default for ComposerGlobalManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for ComposerGlobalManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(config, COMPOSER_COMMAND, &["--version", "--no-ansi"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        let (environment, _, installed) = self.installed_entries(config).await?;
        Ok(installed
            .into_iter()
            .map(|entry| entry.info(self.descriptor.id(), &environment.home))
            .collect())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.installed_entries(config).await?.2.len())
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        let (environment, requirements, installed) = self.installed_entries(config).await?;
        if installed.is_empty() {
            return Ok(Vec::new());
        }
        let installed_by_name = installed
            .into_iter()
            .map(|package| (package.name.clone(), package))
            .collect::<HashMap<_, _>>();
        let composer = resolve_executable(config, COMPOSER_COMMAND);
        let output = run_success(
            &environment.command(&composer).args([
                "global",
                "outdated",
                "--direct",
                "--format=json",
                "--no-interaction",
                "--no-ansi",
            ]),
            "Composer global outdated query timed out",
        )
        .await?;
        let value = decode_utf8(
            &output.stdout,
            "Composer global outdated response is not valid UTF-8",
        )?;
        parse_updates(
            &value,
            &requirements,
            &installed_by_name,
            self.descriptor.id(),
            &environment.home,
        )
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        validate_search_term(query)?;
        let environment = self.environment(config).await?;
        let composer = resolve_executable(config, COMPOSER_COMMAND);
        let output = run_success(
            &environment.command(&composer).args([
                "global",
                "search",
                "--format=json",
                "--no-interaction",
                "--no-ansi",
                "--",
                query,
            ]),
            "Composer global search timed out",
        )
        .await?;
        let value = decode_utf8(
            &output.stdout,
            "Composer global search response is not valid UTF-8",
        )?;
        parse_search(&value, self.descriptor.id(), &environment.home)
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
        let environment = self.environment(config).await?;
        let mut commands = Vec::with_capacity(packages.len());
        for target in packages {
            commands.push(
                self.write_command(config, &environment, action, target)
                    .await?,
            );
        }
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
                ManagerError::new(
                    ManagerErrorKind::Timeout,
                    "Composer Global write command timed out",
                )
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

#[derive(Debug)]
struct ComposerEnvironment {
    home: PathBuf,
}

impl ComposerEnvironment {
    fn command(&self, composer: &Path) -> CommandSpec {
        CommandSpec::new(composer)
            .env("COMPOSER_HOME", self.home.as_os_str())
            .env_remove("COMPOSER")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComposerReference {
    home: PathBuf,
    package: String,
    constraint: Option<String>,
}

impl ComposerReference {
    fn installed(home: &Path, package: &str, constraint: &str) -> Self {
        Self {
            home: home.to_path_buf(),
            package: package.to_owned(),
            constraint: Some(constraint.to_owned()),
        }
    }

    fn search(home: &Path, package: &str) -> Self {
        Self {
            home: home.to_path_buf(),
            package: package.to_owned(),
            constraint: None,
        }
    }

    fn origin(&self) -> PackageOrigin {
        PackageOrigin::new(ORIGIN_NAME).with_reference(
            serde_json::to_string(self).expect("validated Composer Global origin must serialize"),
        )
    }

    fn parse(origin: &PackageOrigin) -> ManagerResult<Self> {
        if origin.name != ORIGIN_NAME {
            return Err(protocol(
                "Composer Global target origin name is malformed",
                &origin.name,
            ));
        }
        let value = origin.reference.as_deref().ok_or_else(|| {
            protocol(
                "Composer Global target origin reference is missing",
                &origin.name,
            )
        })?;
        let reference: Self = serde_json::from_str(value).map_err(|error| {
            protocol(
                "Composer Global target origin reference is malformed",
                &error.to_string(),
            )
        })?;
        validate_home_path(&reference.home)?;
        validate_package_name(&reference.package)?;
        if let Some(constraint) = reference.constraint.as_deref() {
            validate_constraint(constraint)?;
        }
        Ok(reference)
    }
}

#[derive(Debug, Deserialize)]
struct RootManifest {
    #[serde(default)]
    require: HashMap<String, serde_json::Value>,
    #[serde(rename = "require-dev", default)]
    require_dev: HashMap<String, serde_json::Value>,
    #[serde(flatten)]
    _other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ComposerPackage {
    name: String,
    #[serde(rename = "direct-dependency")]
    direct_dependency: bool,
    version: String,
    #[serde(default)]
    latest: Option<String>,
    #[serde(rename = "latest-status", default)]
    latest_status: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(flatten)]
    _other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SearchPackage {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(flatten)]
    _other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComposerListing {
    installed: Vec<ComposerPackage>,
}

#[derive(Debug)]
struct InstalledPackage {
    name: String,
    version: String,
    constraint: String,
    description: Option<String>,
    homepage: Option<String>,
}

#[derive(Debug, Default)]
struct ComposerRequirements {
    runtime: HashMap<String, String>,
    dev: HashMap<String, String>,
}

impl InstalledPackage {
    fn target(&self, manager_id: &ManagerId, home: &Path) -> PackageTarget {
        let mut target = PackageTarget::new(manager_id.clone(), &self.name);
        target.scope = PackageScope::User;
        target.origin =
            Some(ComposerReference::installed(home, &self.name, &self.constraint).origin());
        target
    }

    fn info(self, manager_id: &ManagerId, home: &Path) -> PackageInfo {
        let mut info = PackageInfo::new(manager_id.clone(), &self.name, &self.version);
        info.description = self.description;
        info.homepage = self.homepage;
        info.scope = PackageScope::User;
        info.origin =
            Some(ComposerReference::installed(home, &self.name, &self.constraint).origin());
        info
    }
}

fn parse_requirements(value: &[u8]) -> ManagerResult<ComposerRequirements> {
    let manifest: RootManifest = serde_json::from_slice(value)
        .map_err(|error| protocol("Composer global manifest is malformed", &error.to_string()))?;
    Ok(ComposerRequirements {
        runtime: parse_requirement_map(manifest.require)?,
        dev: parse_requirement_map(manifest.require_dev)?,
    })
}

fn parse_requirement_map(
    entries: HashMap<String, serde_json::Value>,
) -> ManagerResult<HashMap<String, String>> {
    let mut requirements = HashMap::new();
    for (name, value) in entries {
        if is_platform_requirement(&name) {
            continue;
        }
        validate_package_name(&name)?;
        let constraint = value.as_str().ok_or_else(|| {
            protocol(
                "Composer global direct dependency constraint is not a string",
                &name,
            )
        })?;
        validate_constraint(constraint)?;
        requirements.insert(name, constraint.to_owned());
    }
    Ok(requirements)
}

fn parse_installed(
    value: &str,
    requirements: &ComposerRequirements,
) -> ManagerResult<Vec<InstalledPackage>> {
    let packages = parse_package_listing(value, "Composer global installed response is malformed")?;
    let mut installed = Vec::with_capacity(packages.len());
    let mut seen = HashMap::new();
    for package in packages {
        validate_package_name(&package.name)?;
        if !package.direct_dependency {
            return Err(protocol(
                "Composer global installed response contains a transitive dependency",
                &package.name,
            ));
        }
        if requirements.dev.contains_key(&package.name) {
            continue;
        }
        let constraint = requirements.runtime.get(&package.name).ok_or_else(|| {
            protocol(
                "Composer global installed package is absent from direct requirements",
                &package.name,
            )
        })?;
        validate_version(&package.version)?;
        if seen.insert(package.name.clone(), ()).is_some() {
            return Err(protocol(
                "Composer global installed response contains a duplicate package",
                &package.name,
            ));
        }
        installed.push(InstalledPackage {
            name: package.name,
            version: package.version,
            constraint: constraint.clone(),
            description: package.description,
            homepage: package.homepage.or(package.source),
        });
    }
    installed.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(installed)
}

fn parse_updates(
    value: &str,
    requirements: &ComposerRequirements,
    installed: &HashMap<String, InstalledPackage>,
    manager_id: &ManagerId,
    home: &Path,
) -> ManagerResult<Vec<PackageUpdate>> {
    let packages = parse_package_listing(value, "Composer global outdated response is malformed")?;
    let mut updates = Vec::with_capacity(packages.len());
    let mut seen = HashMap::new();
    for package in packages {
        validate_package_name(&package.name)?;
        if requirements.dev.contains_key(&package.name) {
            continue;
        }
        if !package.direct_dependency || !requirements.runtime.contains_key(&package.name) {
            return Err(protocol(
                "Composer global outdated response contains a non-direct dependency",
                &package.name,
            ));
        }
        let installed = installed.get(&package.name).ok_or_else(|| {
            protocol(
                "Composer global outdated package is absent from installed inventory",
                &package.name,
            )
        })?;
        if installed.version != package.version {
            return Err(protocol(
                "Composer global outdated version does not match installed inventory",
                &package.name,
            ));
        }
        let latest = package.latest.ok_or_else(|| {
            protocol(
                "Composer global outdated package is missing its latest version",
                &package.name,
            )
        })?;
        let status = package.latest_status.as_deref().ok_or_else(|| {
            protocol(
                "Composer global outdated package is missing its latest status",
                &package.name,
            )
        })?;
        if status == "up-to-date" {
            continue;
        }
        validate_version(&latest)?;
        if !matches!(status, "semver-safe-update" | "update-possible") || latest == package.version
        {
            return Err(protocol(
                "Composer global outdated package has an invalid update status",
                &package.name,
            ));
        }
        if seen.insert(package.name.clone(), ()).is_some() {
            return Err(protocol(
                "Composer global outdated response contains a duplicate package",
                &package.name,
            ));
        }
        updates.push(PackageUpdate::new(
            installed.target(manager_id, home),
            package.version,
            latest,
        ));
    }
    updates.sort_by(|left, right| left.target.name.cmp(&right.target.name));
    Ok(updates)
}

fn parse_package_listing(value: &str, message: &str) -> ManagerResult<Vec<ComposerPackage>> {
    let value: serde_json::Value =
        serde_json::from_str(value).map_err(|error| protocol(message, &error.to_string()))?;
    if let Some(packages) = value.as_array() {
        if packages.is_empty() {
            return Ok(Vec::new());
        }
        return Err(protocol(
            message,
            "non-empty Composer package listings must use the installed object",
        ));
    }
    let listing: ComposerListing =
        serde_json::from_value(value).map_err(|error| protocol(message, &error.to_string()))?;
    Ok(listing.installed)
}

fn parse_search(
    value: &str,
    manager_id: &ManagerId,
    home: &Path,
) -> ManagerResult<Vec<PackageInfo>> {
    let packages: Vec<SearchPackage> = serde_json::from_str(value).map_err(|error| {
        protocol(
            "Composer global search response is malformed",
            &error.to_string(),
        )
    })?;
    let mut found = Vec::new();
    let mut seen = HashMap::new();
    for package in packages {
        if is_platform_requirement(&package.name) {
            continue;
        }
        validate_package_name(&package.name)?;
        if seen.insert(package.name.clone(), ()).is_some() {
            return Err(protocol(
                "Composer global search response contains a duplicate package",
                &package.name,
            ));
        }
        let mut info = PackageInfo::new(manager_id.clone(), &package.name, "Not Installed");
        info.description = package.description;
        info.homepage = package.url.or(package.repository);
        info.scope = PackageScope::User;
        info.origin = Some(ComposerReference::search(home, &package.name).origin());
        found.push(info);
    }
    found.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(found)
}

fn composer_command(composer: &Path) -> CommandSpec {
    CommandSpec::new(composer).env_remove("COMPOSER")
}

async fn run_success(spec: &CommandSpec, timeout_message: &str) -> ManagerResult<Output> {
    let output = timeout(COMMAND_TIMEOUT, run_output(spec))
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, timeout_message)
                .with_detail(spec.program().to_string_lossy())
        })??;
    if output.status.success() {
        Ok(output)
    } else {
        let tail = String::from_utf8_lossy(&output.stderr);
        Err(command_status_error(spec, output.status, &tail))
    }
}

fn decode_utf8(value: &[u8], message: &str) -> ManagerResult<String> {
    String::from_utf8(value.to_vec()).map_err(|error| {
        ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(error.to_string())
    })
}

fn single_line<'a>(value: &'a str, message: &str) -> ManagerResult<&'a str> {
    let value = value.trim();
    if value.is_empty() || value.lines().count() != 1 {
        Err(protocol(message, value))
    } else {
        Ok(value)
    }
}

fn validate_home(value: &str) -> ManagerResult<PathBuf> {
    let home = PathBuf::from(value);
    validate_home_path(&home)?;
    Ok(home)
}

fn validate_home_path(home: &Path) -> ManagerResult<()> {
    if !home.is_absolute()
        || home.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(protocol(
            "Composer global home must be an absolute normalized path",
            &home.to_string_lossy(),
        ));
    }
    Ok(())
}

fn validate_package_name(name: &str) -> ManagerResult<()> {
    let mut parts = name.split('/');
    let vendor = parts.next().unwrap_or_default();
    let package = parts.next().unwrap_or_default();
    if parts.next().is_some() || !valid_name_part(vendor) || !valid_name_part(package) {
        return Err(protocol("Composer package name is malformed", name));
    }
    Ok(())
}

fn valid_name_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

fn is_platform_requirement(name: &str) -> bool {
    matches!(
        name,
        "php" | "hhvm" | "composer" | "composer-plugin-api" | "composer-runtime-api"
    ) || name.starts_with("php-")
        || name.starts_with("ext-")
        || name.starts_with("lib-")
}

fn validate_constraint(value: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value.len() > MAX_CONSTRAINT_LENGTH
        || value.starts_with('-')
        || value.chars().any(char::is_control)
    {
        return Err(protocol("Composer version constraint is malformed", value));
    }
    Ok(())
}

fn validate_version(value: &str) -> ManagerResult<()> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(protocol("Composer package version is malformed", value));
    }
    Ok(())
}

fn validate_search_term(value: &str) -> ManagerResult<()> {
    if value.starts_with('-') || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(protocol("Composer search term is malformed", value));
    }
    Ok(())
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    if matches!(
        action,
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall
    ) {
        Ok(())
    } else {
        Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "Composer Global action is not supported",
        ))
    }
}

fn protocol(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_keep_only_direct_package_names() {
        let requirements = parse_requirements(
            br#"{"require":{"php":"^8.4","ext-json":"*","vendor/tool":"^1.0 || ^2.0"},"require-dev":{"dev/tool":"^1"}}"#,
        )
        .expect("parse Composer requirements");
        assert_eq!(requirements.runtime.len(), 1);
        assert_eq!(requirements.runtime["vendor/tool"], "^1.0 || ^2.0");
        assert_eq!(requirements.dev["dev/tool"], "^1");
    }

    #[test]
    fn installed_rejects_transitive_and_unrequired_packages() {
        let requirements = ComposerRequirements {
            runtime: HashMap::from([("vendor/tool".to_owned(), "^1".to_owned())]),
            dev: HashMap::new(),
        };
        for value in [
            r#"{"installed":[{"name":"vendor/transitive","direct-dependency":false,"version":"1.0.0"}]}"#,
            r#"{"installed":[{"name":"vendor/other","direct-dependency":true,"version":"1.0.0"}]}"#,
        ] {
            assert_eq!(
                parse_installed(value, &requirements)
                    .expect_err("reject non-direct Composer package")
                    .kind(),
                ManagerErrorKind::Protocol
            );
        }
    }

    #[test]
    fn installed_accepts_current_composer_json_array_contract() {
        let requirements = ComposerRequirements {
            runtime: HashMap::from([("vendor/tool".to_owned(), "^1".to_owned())]),
            dev: HashMap::new(),
        };
        let installed = parse_installed(
            r#"{"installed":[{"name":"vendor/tool","direct-dependency":true,"homepage":"https://example.test","source":"https://source.test","version":"1.0.0","description":"Tool"}]}"#,
            &requirements,
        )
        .expect("parse installed Composer package");
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].constraint, "^1");
        assert_eq!(
            installed[0].homepage.as_deref(),
            Some("https://example.test")
        );
        assert_eq!(
            parse_installed(
                r#"[{"name":"vendor/tool","direct-dependency":true,"version":"1.0.0"}]"#,
                &requirements,
            )
            .expect_err("reject non-empty top-level Composer package array")
            .kind(),
            ManagerErrorKind::Protocol
        );
    }

    #[test]
    fn outdated_requires_known_current_version_and_update_status() {
        let requirements = ComposerRequirements {
            runtime: HashMap::from([("vendor/tool".to_owned(), "^1".to_owned())]),
            dev: HashMap::new(),
        };
        let installed = HashMap::from([(
            "vendor/tool".to_owned(),
            InstalledPackage {
                name: "vendor/tool".to_owned(),
                version: "1.0.0".to_owned(),
                constraint: "^1".to_owned(),
                description: None,
                homepage: None,
            },
        )]);
        let id = ManagerId::parse(COMPOSER_ID).expect("valid manager ID");
        let home = PathBuf::from("/tmp/composer");
        let updates = parse_updates(
            r#"{"installed":[{"name":"vendor/tool","direct-dependency":true,"version":"1.0.0","latest":"1.2.0","latest-status":"semver-safe-update"}]}"#,
            &requirements,
            &installed,
            &id,
            &home,
        )
        .expect("parse Composer update");
        assert_eq!(updates[0].available_version, "1.2.0");
        assert!(
            parse_updates(
                r#"{"installed":[{"name":"vendor/tool","direct-dependency":true,"version":"1.0.0","latest":"[none matched]","latest-status":"up-to-date"}]}"#,
                &requirements,
                &installed,
                &id,
                &home,
            )
            .expect("ignore Composer up-to-date metadata row")
            .is_empty()
        );
    }

    #[test]
    fn search_skips_platform_results_and_rejects_duplicates() {
        let id = ManagerId::parse(COMPOSER_ID).expect("valid manager ID");
        let home = PathBuf::from("/tmp/composer");
        let found = parse_search(
            r#"[{"name":"composer","description":"platform"},{"name":"vendor/tool","description":"Tool","url":"https://example.test"}]"#,
            &id,
            &home,
        )
        .expect("parse Composer search");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "vendor/tool");
        assert_eq!(
            parse_search(
                r#"[{"name":"vendor/tool"},{"name":"vendor/tool"}]"#,
                &id,
                &home,
            )
            .expect_err("reject duplicate Composer search result")
            .kind(),
            ManagerErrorKind::Protocol
        );
    }
}
