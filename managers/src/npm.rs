use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

use async_trait::async_trait;
use reqwest::Url;
use semver::Version;
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

const NPM_ID: &str = "builtin:npm";
const NPM_COMMAND: &str = "npm";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

/// Direct `updater-manager-api` implementation for npm global packages.
#[derive(Debug, Clone)]
pub struct NpmManager {
    descriptor: ManagerDescriptor,
}

impl NpmManager {
    /// Creates the built-in npm manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(NPM_ID).expect("npm manager ID must remain valid"),
            "npm",
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
        .expect("npm descriptor must remain valid")
        .with_description("npm global package manager")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }

    /// Returns the installed version of one global npm package.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the package is absent and propagates
    /// command, JSON, path, and filesystem errors from the global inventory.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        package_name: &str,
    ) -> ManagerResult<String> {
        validate_package_name(package_name)?;
        self.installed_packages(config)
            .await?
            .into_iter()
            .find(|package| package.name == package_name)
            .map(|package| package.version)
            .ok_or_else(|| protocol("npm package version is unavailable", package_name))
    }

    /// Executes one npm target while exposing normalized command progress.
    ///
    /// # Errors
    ///
    /// Returns a typed target-validation, registry, timeout, or command error.
    #[allow(dead_code)]
    async fn execute_target_with_progress(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
        on_progress: impl FnMut(CommandProgress),
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        let command = self.write_command(config, action, target).await?;
        run_npm_command_with_progress(&command, on_progress).await
    }

    async fn global_root(&self, config: &ManagerConfig) -> ManagerResult<PathBuf> {
        self.validate_config(config)?;
        let npm = resolve_executable(config, NPM_COMMAND);
        let spec = npm_command(&npm).args(["root", "-g"]);
        let output = run_success(&spec, "npm global root query timed out").await?;
        let stdout = decode_utf8(&output.stdout, "npm global root is not valid UTF-8")?;
        let root = stdout.trim();
        if root.is_empty() || stdout.lines().count() != 1 {
            return Err(protocol("npm global root response is malformed", &stdout));
        }
        let path = PathBuf::from(root);
        if !path.is_absolute() {
            return Err(protocol("npm global root must be an absolute path", root));
        }
        Ok(path)
    }

    async fn registry(&self, config: &ManagerConfig) -> ManagerResult<String> {
        self.validate_config(config)?;
        let npm = resolve_executable(config, NPM_COMMAND);
        let spec = npm_command(&npm).args(["config", "get", "registry"]);
        let output = run_success(&spec, "npm registry query timed out").await?;
        let stdout = decode_utf8(&output.stdout, "npm registry response is not valid UTF-8")?;
        let registry = stdout.trim();
        if registry.is_empty() || stdout.lines().count() != 1 {
            return Err(protocol("npm registry response is malformed", &stdout));
        }
        normalize_registry(registry)
    }

    async fn installed_packages(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<Vec<InstalledNpmPackage>> {
        self.validate_config(config)?;
        let root = self.global_root(config).await?;
        let npm = resolve_executable(config, NPM_COMMAND);
        let spec = npm_command(&npm).args(["ls", "-g", "--depth=0", "--json", "--long"]);
        let output = run_success(&spec, "npm global package listing timed out").await?;
        let response: NpmInstalledRoot =
            decode_json(&output.stdout, "npm global package listing is invalid")?;
        response.validated(&root).await
    }

    async fn outdated_packages(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<Vec<NpmOutdatedPackage>> {
        self.validate_config(config)?;
        let root = self.global_root(config).await?;
        let npm = resolve_executable(config, NPM_COMMAND);
        let spec = npm_command(&npm).args(["outdated", "-g", "--json"]);
        let output = run_with_timeout(&spec, "npm global outdated query timed out").await?;
        if !matches!(output.status.code(), Some(0 | 1)) {
            return Err(command_status_error(
                &spec,
                output.status,
                &output_text(&output),
            ));
        }
        let response: NpmOutdatedResponse = match serde_json::from_slice(&output.stdout) {
            Ok(response) => response,
            Err(error) if output.status.success() => {
                return Err(protocol(
                    "npm outdated response is invalid",
                    &error.to_string(),
                ));
            }
            Err(_) if !output_text(&output).is_empty() => {
                return Err(command_status_error(
                    &spec,
                    output.status,
                    &output_text(&output),
                ));
            }
            Err(error) => {
                return Err(protocol(
                    "npm outdated response is invalid",
                    &error.to_string(),
                ));
            }
        };
        let response = match response {
            NpmOutdatedResponse::Updates(response) => response,
            NpmOutdatedResponse::Error { error } if !output.status.success() => {
                return Err(command_status_error(&spec, output.status, &error.detail()));
            }
            NpmOutdatedResponse::Error { error } => {
                return Err(protocol(
                    "npm outdated reported an error with a success status",
                    &error.detail(),
                ));
            }
        };

        match (output.status.code(), response.is_empty()) {
            (Some(0), true) => return Ok(Vec::new()),
            (Some(1), false) => {}
            (Some(0), false) => {
                return Err(protocol(
                    "npm outdated returned updates with a success status",
                    "expected exit status 1",
                ));
            }
            (Some(1), true) => {
                return Err(command_status_error(
                    &spec,
                    output.status,
                    &output_text(&output),
                ));
            }
            _ => {
                return Err(command_status_error(
                    &spec,
                    output.status,
                    &output_text(&output),
                ));
            }
        }

        let mut packages = Vec::with_capacity(response.len());
        for (name, entries) in response {
            validate_package_name(&name)?;
            let entries = entries.into_vec();
            if entries.len() != 1 {
                return Err(protocol("npm global outdated identity is ambiguous", &name));
            }
            let entry = entries.into_iter().next().ok_or_else(|| {
                protocol("npm global outdated entry is unexpectedly empty", &name)
            })?;
            packages.push(entry.validated(name, &root)?);
        }
        Ok(packages)
    }

    async fn write_command(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
    ) -> ManagerResult<CommandSpec> {
        if &target.manager_id != self.descriptor.id() {
            return Err(protocol(
                "npm package target belongs to another manager",
                &target.name,
            ));
        }
        validate_package_name(&target.name)?;
        let npm = resolve_executable(config, NPM_COMMAND);

        if target.scope == PackageScope::Unknown && target.origin.is_none() {
            return legacy_write_command(&npm, action, target);
        }
        if target.scope != PackageScope::User {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "npm target scope is not supported",
            )
            .with_detail(&target.name));
        }
        let origin = target.origin.as_ref().ok_or_else(|| {
            protocol(
                "scoped npm target is missing its typed origin",
                &target.name,
            )
        })?;
        validate_origin_reference(origin, &target.name)?;

        if origin.name == "npm global" {
            if action != PackageAction::Uninstall {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "installed npm global targets are read-only except for uninstall",
                )
                .with_detail(&target.name));
            }
            if target.version.is_some() {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "version-pinned npm uninstall targets are not supported",
                )
                .with_detail(&target.name));
            }
            return Ok(npm_command(&npm).args(["uninstall", "-g", target.name.as_str()]));
        }

        let requested_registry = normalize_registry(&origin.name)?;
        let configured_registry = self.registry(config).await?;
        if requested_registry != configured_registry {
            return Err(protocol(
                "npm target registry does not match the configured registry",
                &origin.name,
            ));
        }
        registry_write_command(&npm, action, target)
    }
}

impl Default for NpmManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for NpmManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(self.descriptor(), config, NPM_COMMAND, &["--version"]).await)
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
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        let registry = self.registry(config).await?;
        self.outdated_packages(config)
            .await?
            .into_iter()
            .filter(|package| package.current != package.latest)
            .map(|package| package.update(self.descriptor.id(), &registry))
            .collect()
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        validate_search_query(query)?;
        let registry = self.registry(config).await?;
        let installed = self
            .installed_packages(config)
            .await?
            .into_iter()
            .map(|package| (package.name, package.version))
            .collect::<BTreeMap<_, _>>();
        let npm = resolve_executable(config, NPM_COMMAND);
        let spec = npm_command(&npm).args(["search", "--json", "--", query]);
        let output = run_success(&spec, "npm registry search timed out").await?;
        let response: Vec<NpmSearchEntry> =
            decode_json(&output.stdout, "npm search response is invalid")?;
        let mut packages = Vec::with_capacity(response.len());
        for entry in response {
            let installed_version = installed.get(&entry.name).map(String::as_str);
            packages.push(entry.info(self.descriptor.id(), &registry, installed_version)?);
        }
        packages.sort_by(|left, right| left.name.cmp(&right.name));
        if packages
            .windows(2)
            .any(|window| window[0].name == window[1].name)
        {
            return Err(protocol(
                "npm search response contains a duplicate package identity",
                query,
            ));
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
        let mut commands = Vec::with_capacity(packages.len());
        for target in packages {
            commands.push(self.write_command(config, action, target).await?);
        }
        let total = packages.len();
        progress.emit(ProgressEvent::Started { action, total });
        for (index, (target, command)) in packages.iter().zip(&commands).enumerate() {
            run_cancellable_npm_command_with_progress(command, progress, |event| {
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

#[derive(Debug, Deserialize)]
struct NpmInstalledRoot {
    #[serde(default)]
    dependencies: BTreeMap<String, NpmInstalledEntry>,
    #[serde(default)]
    problems: Vec<String>,
    #[serde(default)]
    error: Option<NpmCliError>,
}

impl NpmInstalledRoot {
    async fn validated(self, root: &Path) -> ManagerResult<Vec<InstalledNpmPackage>> {
        if let Some(error) = self.error {
            return Err(protocol(
                "npm global package listing reported an error",
                &error.detail(),
            ));
        }
        if !self.problems.is_empty() {
            return Err(protocol(
                "npm global package listing reported invalid dependencies",
                &self.problems.join("\n"),
            ));
        }
        let mut packages = Vec::with_capacity(self.dependencies.len());
        for (key, entry) in self.dependencies {
            packages.push(entry.validated(key, root).await?);
        }
        Ok(packages)
    }
}

#[derive(Debug, Deserialize)]
struct NpmCliError {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

impl NpmCliError {
    fn detail(self) -> String {
        [self.code, self.summary, self.detail]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(": ")
    }
}

#[derive(Debug, Deserialize)]
struct NpmInstalledEntry {
    name: String,
    version: String,
    #[serde(rename = "_id", default)]
    id: Option<String>,
    path: PathBuf,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<NpmRepository>,
    #[serde(default)]
    invalid: Option<BoolOrString>,
    #[serde(default)]
    missing: Option<BoolOrString>,
}

impl NpmInstalledEntry {
    async fn validated(self, key: String, root: &Path) -> ManagerResult<InstalledNpmPackage> {
        validate_package_name(&key)?;
        if self.name != key {
            return Err(protocol(
                "npm installed package key and name do not match",
                &key,
            ));
        }
        validate_version(&self.version)?;
        if let Some(id) = self.id
            && id != format!("{}@{}", self.name, self.version)
        {
            return Err(protocol("npm installed package ID is inconsistent", &id));
        }
        if self.invalid.as_ref().is_some_and(BoolOrString::is_problem)
            || self.missing.as_ref().is_some_and(BoolOrString::is_problem)
        {
            return Err(protocol(
                "npm installed package is invalid or missing",
                &self.name,
            ));
        }
        validate_package_path(root, &self.name, &self.path)?;
        let metadata = tokio::fs::symlink_metadata(&self.path)
            .await
            .map_err(|error| fs_error("failed to inspect npm package path", error))?;
        let size = if metadata.file_type().is_symlink() {
            None
        } else if metadata.is_dir() {
            validate_canonical_package_path(root, &self.path).await?;
            Some(directory_size(&self.path).await?)
        } else {
            return Err(protocol(
                "npm installed package path is not a directory",
                &self.path.display().to_string(),
            ));
        };
        let homepage = self
            .homepage
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                self.repository
                    .and_then(NpmRepository::url)
                    .map(|value| value.strip_prefix("git+").unwrap_or(&value).to_owned())
            });
        Ok(InstalledNpmPackage {
            name: self.name,
            version: self.version,
            description: self.description.filter(|value| !value.trim().is_empty()),
            homepage,
            size,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NpmRepository {
    String(String),
    Object { url: String },
}

impl NpmRepository {
    fn url(self) -> Option<String> {
        let value = match self {
            Self::String(value) | Self::Object { url: value } => value,
        };
        (!value.trim().is_empty()).then_some(value)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BoolOrString {
    Bool(bool),
    String(String),
}

impl BoolOrString {
    fn is_problem(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::String(value) => !value.trim().is_empty(),
        }
    }
}

#[derive(Debug)]
struct InstalledNpmPackage {
    name: String,
    version: String,
    description: Option<String>,
    homepage: Option<String>,
    size: Option<u64>,
}

impl InstalledNpmPackage {
    fn info(self, manager_id: &ManagerId) -> PackageInfo {
        let name = self.name;
        let mut info = PackageInfo::new(manager_id.clone(), &name, self.version);
        info.description = self.description;
        info.homepage = self.homepage;
        info.size = self.size;
        info.scope = PackageScope::User;
        info.origin = Some(global_origin(&name));
        info
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NpmOutdatedResponse {
    Error { error: NpmCliError },
    Updates(BTreeMap<String, OneOrMany<NpmOutdatedEntry>>),
}

impl<T> OneOrMany<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Deserialize)]
struct NpmOutdatedEntry {
    current: Option<String>,
    wanted: String,
    latest: String,
    dependent: String,
    location: PathBuf,
}

impl NpmOutdatedEntry {
    fn validated(self, name: String, root: &Path) -> ManagerResult<NpmOutdatedPackage> {
        let current = self
            .current
            .ok_or_else(|| protocol("npm global outdated entry has no installed version", &name))?;
        validate_version(&current)?;
        validate_version(&self.wanted)?;
        validate_version(&self.latest)?;
        if self.dependent.trim().is_empty() {
            return Err(protocol("npm outdated dependent is empty", &name));
        }
        validate_package_path(root, &name, &self.location)?;
        Ok(NpmOutdatedPackage {
            name,
            current,
            latest: self.latest,
        })
    }
}

#[derive(Debug)]
struct NpmOutdatedPackage {
    name: String,
    current: String,
    latest: String,
}

impl NpmOutdatedPackage {
    fn update(self, manager_id: &ManagerId, registry: &str) -> ManagerResult<PackageUpdate> {
        let mut target = PackageTarget::new(manager_id.clone(), &self.name);
        target.version = Some(self.latest.clone());
        target.scope = PackageScope::User;
        target.origin = Some(registry_origin(registry, &self.name)?);
        Ok(PackageUpdate::new(target, self.current, self.latest))
    }
}

#[derive(Debug, Deserialize)]
struct NpmSearchEntry {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    links: Option<NpmLinks>,
}

impl NpmSearchEntry {
    fn info(
        self,
        manager_id: &ManagerId,
        registry: &str,
        installed_version: Option<&str>,
    ) -> ManagerResult<PackageInfo> {
        validate_package_name(&self.name)?;
        validate_version(&self.version)?;
        let mut info = PackageInfo::new(
            manager_id.clone(),
            &self.name,
            installed_version.unwrap_or(NOT_INSTALLED_VERSION),
        );
        info.description = self.description.filter(|value| !value.trim().is_empty());
        info.homepage = self.links.and_then(NpmLinks::homepage);
        info.scope = PackageScope::User;
        info.origin = Some(registry_origin(registry, &self.name)?);
        Ok(info)
    }
}

#[derive(Debug, Deserialize)]
struct NpmLinks {
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    npm: Option<String>,
}

impl NpmLinks {
    fn homepage(self) -> Option<String> {
        self.homepage
            .filter(|value| !value.trim().is_empty())
            .or_else(|| self.npm.filter(|value| !value.trim().is_empty()))
    }
}

fn npm_command(path: &Path) -> CommandSpec {
    CommandSpec::new(path)
        .env("NO_COLOR", "1")
        .env("NPM_CONFIG_COLOR", "false")
        .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
}

#[allow(dead_code)]
async fn run_npm_command_with_progress(
    command: &CommandSpec,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    timeout(
        COMMAND_TIMEOUT,
        run_command_with_progress(command, on_progress),
    )
    .await
    .map_err(|_| {
        ManagerError::new(ManagerErrorKind::Timeout, "npm package command timed out")
            .with_detail(command.program().to_string_lossy())
    })?
}

async fn run_cancellable_npm_command_with_progress(
    command: &CommandSpec,
    cancellation: &dyn ProgressSink,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    timeout(
        COMMAND_TIMEOUT,
        run_cancellable_command_with_progress(command, cancellation, on_progress),
    )
    .await
    .map_err(|_| {
        ManagerError::new(ManagerErrorKind::Timeout, "npm package command timed out")
            .with_detail(command.program().to_string_lossy())
    })?
}

fn global_origin(name: &str) -> PackageOrigin {
    PackageOrigin::new("npm global").with_reference(format!("package:{name}"))
}

fn registry_origin(registry: &str, name: &str) -> ManagerResult<PackageOrigin> {
    Ok(PackageOrigin::new(normalize_registry(registry)?).with_reference(format!("package:{name}")))
}

fn validate_origin_reference(origin: &PackageOrigin, name: &str) -> ManagerResult<()> {
    let expected = format!("package:{name}");
    if origin.reference.as_deref() != Some(expected.as_str()) {
        return Err(protocol(
            "npm package origin reference is malformed",
            origin.reference.as_deref().unwrap_or("missing reference"),
        ));
    }
    Ok(())
}

fn normalize_registry(value: &str) -> ManagerResult<String> {
    let mut url = Url::parse(value)
        .map_err(|error| protocol("npm registry URL is malformed", &error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(protocol("npm registry URL is unsupported", value));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url.to_string())
}

fn validate_package_name(value: &str) -> ManagerResult<()> {
    if value.is_empty() || value.len() > 214 || !value.is_ascii() || value.starts_with('-') {
        return Err(protocol("npm package name is malformed", value));
    }
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && !segment.starts_with(['.', '_'])
            && segment.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '-' | '.' | '_' | '~')
            })
    };
    let valid = if let Some(scoped) = value.strip_prefix('@') {
        let mut segments = scoped.split('/');
        matches!((segments.next(), segments.next(), segments.next()), (Some(scope), Some(name), None) if valid_segment(scope) && valid_segment(name))
    } else {
        !value.contains(['@', '/']) && valid_segment(value)
    };
    if !valid {
        return Err(protocol("npm package name is malformed", value));
    }
    Ok(())
}

fn validate_version(value: &str) -> ManagerResult<()> {
    if value.is_empty() || value.starts_with('-') || value.contains(char::is_whitespace) {
        return Err(protocol("npm package version is malformed", value));
    }
    Version::parse(value).map(|_| ()).map_err(|error| {
        protocol(
            "npm package version is not valid semver",
            &error.to_string(),
        )
    })
}

fn validate_version_or_tag(value: &str) -> ManagerResult<()> {
    if Version::parse(value).is_ok()
        || (!value.is_empty()
            && value.len() <= 128
            && !value.starts_with('-')
            && value.is_ascii()
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | '_' | '~')
            }))
    {
        Ok(())
    } else {
        Err(protocol("npm package version or tag is malformed", value))
    }
}

fn validate_search_query(value: &str) -> ManagerResult<()> {
    if value.len() > 512 || value.chars().any(char::is_control) {
        return Err(protocol("npm search query is malformed", value));
    }
    Ok(())
}

fn expected_package_path(root: &Path, name: &str) -> PathBuf {
    if let Some(scoped) = name.strip_prefix('@')
        && let Some((scope, package)) = scoped.split_once('/')
    {
        return root.join(format!("@{scope}")).join(package);
    }
    root.join(name)
}

fn validate_package_path(root: &Path, name: &str, path: &Path) -> ManagerResult<()> {
    if !root.is_absolute() || !path.is_absolute() || path != expected_package_path(root, name) {
        return Err(ManagerError::new(
            ManagerErrorKind::Permission,
            "npm package path escapes the global package root",
        )
        .with_detail(path.display().to_string()));
    }
    Ok(())
}

async fn validate_canonical_package_path(root: &Path, path: &Path) -> ManagerResult<()> {
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .map_err(|error| fs_error("failed to resolve npm global package root", error))?;
    let canonical_path = tokio::fs::canonicalize(path)
        .await
        .map_err(|error| fs_error("failed to resolve npm package path", error))?;
    if !canonical_path.starts_with(&canonical_root) || canonical_path == canonical_root {
        return Err(ManagerError::new(
            ManagerErrorKind::Permission,
            "npm package path escapes the global package root",
        )
        .with_detail(canonical_path.display().to_string()));
    }
    Ok(())
}

async fn directory_size(root: &Path) -> ManagerResult<u64> {
    let mut total = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&directory)
            .await
            .map_err(|error| fs_error("failed to read npm package directory", error))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| fs_error("failed to read an npm package directory entry", error))?
        {
            let metadata = tokio::fs::symlink_metadata(entry.path())
                .await
                .map_err(|error| fs_error("failed to inspect an npm package entry", error))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total.checked_add(metadata.len()).ok_or_else(|| {
                    protocol(
                        "npm package size exceeds the supported range",
                        &root.display().to_string(),
                    )
                })?;
            }
        }
    }
    Ok(total)
}

fn legacy_write_command(
    npm: &Path,
    action: PackageAction,
    target: &PackageTarget,
) -> ManagerResult<CommandSpec> {
    match action {
        PackageAction::Install => {
            let spec = package_spec(&target.name, target.version.as_deref())?;
            Ok(npm_command(npm).args(["install", "-g", spec.as_str()]))
        }
        PackageAction::Update => {
            let version = target.version.as_deref().unwrap_or("latest");
            let spec = if version == "latest" {
                format!("{}@latest", target.name)
            } else {
                package_spec(&target.name, Some(version))?
            };
            Ok(npm_command(npm).args(["install", "-g", spec.as_str()]))
        }
        PackageAction::Uninstall => {
            if target.version.is_some() {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "version-pinned npm uninstall targets are not supported",
                )
                .with_detail(&target.name));
            }
            Ok(npm_command(npm).args(["uninstall", "-g", target.name.as_str()]))
        }
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "npm package action is unsupported",
        )),
    }
}

fn registry_write_command(
    npm: &Path,
    action: PackageAction,
    target: &PackageTarget,
) -> ManagerResult<CommandSpec> {
    match action {
        PackageAction::Install => {
            let spec = package_spec(&target.name, target.version.as_deref())?;
            Ok(npm_command(npm).args(["install", "-g", spec.as_str()]))
        }
        PackageAction::Update => {
            let version = target.version.as_deref().ok_or_else(|| {
                protocol(
                    "typed npm update target is missing an exact version",
                    &target.name,
                )
            })?;
            let spec = package_spec(&target.name, Some(version))?;
            Ok(npm_command(npm).args(["install", "-g", spec.as_str()]))
        }
        PackageAction::Uninstall => {
            if target.version.is_some() {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "version-pinned npm uninstall targets are not supported",
                )
                .with_detail(&target.name));
            }
            Ok(npm_command(npm).args(["uninstall", "-g", target.name.as_str()]))
        }
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "npm package action is unsupported",
        )),
    }
}

fn package_spec(name: &str, version: Option<&str>) -> ManagerResult<String> {
    validate_package_name(name)?;
    if let Some(version) = version {
        validate_version_or_tag(version)?;
        Ok(format!("{name}@{version}"))
    } else {
        Ok(name.to_owned())
    }
}

async fn run_with_timeout(spec: &CommandSpec, message: &str) -> ManagerResult<Output> {
    timeout(COMMAND_TIMEOUT, run_output(spec))
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, message)
                .with_detail(spec.program().to_string_lossy())
        })?
}

async fn run_success(spec: &CommandSpec, timeout_message: &str) -> ManagerResult<Output> {
    let output = run_with_timeout(spec, timeout_message).await?;
    if output.status.success() {
        return Ok(output);
    }
    Err(command_status_error(
        spec,
        output.status,
        &output_text(&output),
    ))
}

fn output_text(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.trim().is_empty() {
        stdout.trim().to_owned()
    } else {
        stderr.trim().to_owned()
    }
}

fn decode_utf8(bytes: &[u8], message: &str) -> ManagerResult<String> {
    String::from_utf8(bytes.to_vec()).map_err(|error| protocol(message, &error.to_string()))
}

fn decode_json<T: for<'de> Deserialize<'de>>(bytes: &[u8], message: &str) -> ManagerResult<T> {
    serde_json::from_slice(bytes).map_err(|error| protocol(message, &error.to_string()))
}

fn protocol(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(detail)
}

fn fs_error(message: &str, error: std::io::Error) -> ManagerError {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied => ManagerErrorKind::Permission,
        std::io::ErrorKind::TimedOut => ManagerErrorKind::Timeout,
        _ => ManagerErrorKind::Other,
    };
    ManagerError::new(kind, message).with_detail(error.to_string())
}
