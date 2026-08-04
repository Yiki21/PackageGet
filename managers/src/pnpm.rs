use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

use async_trait::async_trait;
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

const PNPM_ID: &str = "builtin:pnpm";
const PNPM_COMMAND: &str = "pnpm";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const SEARCH_LIMIT: &str = "50";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Direct `updater-manager-api` implementation for global pnpm packages.
#[derive(Debug, Clone)]
pub struct PnpmManager {
    descriptor: ManagerDescriptor,
}

impl PnpmManager {
    /// Creates the built-in pnpm manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(PNPM_ID).expect("pnpm manager ID must remain valid"),
            "pnpm",
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
        .expect("pnpm descriptor must remain valid")
        .with_description("Global JavaScript package manager")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }

    /// Returns the installed version of one global package.
    ///
    /// # Errors
    ///
    /// Propagates command, JSON, identity, and filesystem evidence errors, or
    /// returns a protocol error when the package is absent.
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
            .ok_or_else(|| protocol("pnpm package version is unavailable", package_name))
    }

    /// Executes one validated pnpm target with normalized command progress.
    ///
    /// # Errors
    ///
    /// Returns a typed configuration, target, or command execution error.
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
        run_pnpm_command_with_progress(&command, on_progress).await
    }

    async fn installed_packages(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<Vec<InstalledPnpmPackage>> {
        self.validate_config(config)?;
        let pnpm = resolve_executable(config, PNPM_COMMAND);
        let spec =
            pnpm_command(&pnpm).args(["list", "--global", "--depth", "0", "--json", "--long"]);
        let output = run_success(&spec, "pnpm global listing timed out").await?;
        let roots: Vec<PnpmRoot> = decode_json(&output.stdout, "pnpm global listing is invalid")?;
        installed_from_roots(roots).await
    }

    async fn registry(&self, config: &ManagerConfig) -> ManagerResult<String> {
        let pnpm = resolve_executable(config, PNPM_COMMAND);
        let spec = pnpm_command(&pnpm).args(["config", "get", "registry"]);
        let output = run_success(&spec, "pnpm registry query timed out").await?;
        let registry = String::from_utf8(output.stdout).map_err(|error| {
            protocol(
                "pnpm registry output is not valid UTF-8",
                &error.to_string(),
            )
        })?;
        validate_registry(registry.trim())
    }

    async fn write_command(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
    ) -> ManagerResult<CommandSpec> {
        ensure_supported_action(action)?;
        if &target.manager_id != self.descriptor.id() {
            return Err(protocol(
                "pnpm package target belongs to another manager",
                &target.name,
            ));
        }
        validate_package_name(&target.name)?;
        let pnpm = resolve_executable(config, PNPM_COMMAND);
        if target.scope == PackageScope::Unknown && target.origin.is_none() {
            return legacy_write_command(&pnpm, action, target);
        }
        if target.scope != PackageScope::User {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "pnpm target scope is not supported",
            )
            .with_detail(&target.name));
        }
        let origin = target.origin.as_ref().ok_or_else(|| {
            protocol(
                "scoped pnpm target is missing its typed origin",
                &target.name,
            )
        })?;
        validate_origin(origin, &target.name)?;
        match action {
            PackageAction::Uninstall => {
                if origin.name != "pnpm global" || target.version.is_some() {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "pnpm uninstall requires an unpinned global package target",
                    )
                    .with_detail(&target.name));
                }
                Ok(pnpm_command(&pnpm).args(["remove", "-g", target.name.as_str()]))
            }
            PackageAction::Install | PackageAction::Update => {
                let expected_origin = self.registry(config).await?;
                if origin.name != expected_origin {
                    return Err(protocol(
                        "pnpm target origin does not match the requested action",
                        &origin.name,
                    ));
                }
                let version = target.version.as_deref().unwrap_or("latest");
                validate_version_or_tag(version)?;
                let spec = format!("{}@{version}", target.name);
                Ok(pnpm_command(&pnpm).args(["add", "-g", spec.as_str()]))
            }
            _ => Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "pnpm package action is unsupported",
            )),
        }
    }
}

impl Default for PnpmManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for PnpmManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(self.descriptor(), config, PNPM_COMMAND, &["--version"]).await)
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
        self.validate_config(config)?;
        let pnpm = resolve_executable(config, PNPM_COMMAND);
        let spec = pnpm_command(&pnpm).args(["outdated", "--global", "--format", "json"]);
        let output = timeout(COMMAND_TIMEOUT, run_output(&spec))
            .await
            .map_err(|_| {
                ManagerError::new(ManagerErrorKind::Timeout, "pnpm outdated query timed out")
                    .with_detail(spec.program().to_string_lossy())
            })??;
        if !matches!(output.status.code(), Some(0 | 1)) {
            let detail = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return Err(command_status_error(&spec, output.status, detail.trim()));
        }
        let response: BTreeMap<String, PnpmOutdated> =
            decode_json(&output.stdout, "pnpm outdated response is invalid")?;
        if output.status.code() == Some(1) && response.is_empty() {
            return Err(command_status_error(
                &spec,
                output.status,
                "pnpm outdated exited with status 1 without reporting updates",
            ));
        }
        let registry = self.registry(config).await?;
        let mut updates = Vec::with_capacity(response.len());
        for (name, detail) in response {
            validate_package_name(&name)?;
            detail.validated()?;
            if detail.current == detail.latest {
                continue;
            }
            let mut target = PackageTarget::new(self.descriptor.id().clone(), &name);
            target.version = Some(detail.latest.clone());
            target.scope = PackageScope::User;
            target.origin = Some(package_origin(&registry, &name));
            updates.push(PackageUpdate::new(target, detail.current, detail.latest));
        }
        Ok(updates)
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
        let pnpm = resolve_executable(config, PNPM_COMMAND);
        let spec =
            pnpm_command(&pnpm).args(["search", query, "--json", "--search-limit", SEARCH_LIMIT]);
        let output = run_success(&spec, "pnpm registry search timed out").await?;
        let results: Vec<PnpmSearchPackage> =
            decode_json(&output.stdout, "pnpm search response is invalid")?;
        let mut identities = HashSet::new();
        let mut packages = Vec::with_capacity(results.len());
        for result in results {
            if !identities.insert(result.name.clone()) {
                return Err(protocol(
                    "pnpm search response contains a duplicate package",
                    &result.name,
                ));
            }
            let installed_version = installed.get(&result.name);
            packages.push(result.info(self.descriptor.id(), &registry, installed_version)?);
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
        let mut commands = Vec::with_capacity(packages.len());
        for target in packages {
            commands.push(self.write_command(config, action, target).await?);
        }
        let total = packages.len();
        progress.emit(ProgressEvent::Started { action, total });
        for (index, (target, command)) in packages.iter().zip(&commands).enumerate() {
            run_cancellable_pnpm_command_with_progress(command, progress, |event| {
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
struct PnpmRoot {
    path: PathBuf,
    private: bool,
    dependencies: BTreeMap<String, PnpmInstalled>,
}

#[derive(Debug, Deserialize)]
struct PnpmInstalled {
    from: String,
    version: String,
    path: PathBuf,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<PnpmRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PnpmRepository {
    Text(String),
    Object { url: String },
}

impl PnpmRepository {
    fn url(self) -> String {
        match self {
            Self::Text(url) | Self::Object { url } => url,
        }
    }
}

#[derive(Debug)]
struct InstalledPnpmPackage {
    name: String,
    version: String,
    description: Option<String>,
    homepage: Option<String>,
    size: Option<u64>,
}

impl InstalledPnpmPackage {
    fn info(self, manager_id: &ManagerId) -> PackageInfo {
        let mut info = PackageInfo::new(manager_id.clone(), &self.name, self.version);
        info.description = self.description;
        info.homepage = self.homepage;
        info.size = self.size;
        info.scope = PackageScope::User;
        info.origin = Some(package_origin("pnpm global", &self.name));
        info
    }
}

#[derive(Debug, Deserialize)]
struct PnpmOutdated {
    current: String,
    latest: String,
    #[serde(default)]
    wanted: Option<String>,
}

impl PnpmOutdated {
    fn validated(&self) -> ManagerResult<()> {
        validate_version(&self.current)?;
        validate_version(&self.latest)?;
        if let Some(wanted) = &self.wanted {
            validate_version(wanted)?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct PnpmSearchPackage {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    links: PnpmLinks,
}

impl PnpmSearchPackage {
    fn info(
        self,
        manager_id: &ManagerId,
        registry: &str,
        installed_version: Option<&String>,
    ) -> ManagerResult<PackageInfo> {
        validate_package_name(&self.name)?;
        validate_version(&self.version)?;
        let mut info = PackageInfo::new(
            manager_id.clone(),
            &self.name,
            installed_version.map_or(NOT_INSTALLED_VERSION, String::as_str),
        );
        info.description = non_empty(self.description);
        info.homepage = non_empty(self.links.homepage).or(non_empty(self.links.npm));
        info.scope = PackageScope::User;
        info.origin = Some(package_origin(registry, &self.name));
        Ok(info)
    }
}

#[derive(Debug, Default, Deserialize)]
struct PnpmLinks {
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    npm: Option<String>,
}

async fn installed_from_roots(roots: Vec<PnpmRoot>) -> ManagerResult<Vec<InstalledPnpmPackage>> {
    let mut identities = HashSet::new();
    let mut packages = Vec::new();
    for root in roots {
        if !root.private {
            return Err(protocol(
                "pnpm global listing root is not private",
                &root.path.display().to_string(),
            ));
        }
        let canonical_root = canonical_directory(&root.path, "pnpm global root").await?;
        for (name, detail) in root.dependencies {
            validate_package_name(&name)?;
            if detail.from != name {
                return Err(protocol(
                    "pnpm installed package source does not match its name",
                    &name,
                ));
            }
            validate_version(&detail.version)?;
            if !identities.insert(name.clone()) {
                return Err(protocol(
                    "pnpm global listing contains a duplicate package",
                    &name,
                ));
            }
            if !detail.path.is_absolute()
                || !detail.path.starts_with(&root.path)
                || detail.path == root.path
            {
                return Err(ManagerError::new(
                    ManagerErrorKind::Permission,
                    "pnpm package path escapes its global root",
                )
                .with_detail(detail.path.display().to_string()));
            }
            let metadata = tokio::fs::symlink_metadata(&detail.path)
                .await
                .map_err(|error| fs_error("failed to inspect pnpm package path", error))?;
            let size = if metadata.file_type().is_symlink() {
                None
            } else if metadata.is_dir() {
                let canonical_package = tokio::fs::canonicalize(&detail.path)
                    .await
                    .map_err(|error| fs_error("failed to resolve pnpm package path", error))?;
                if !canonical_package.starts_with(&canonical_root)
                    || canonical_package == canonical_root
                {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Permission,
                        "pnpm package path escapes its global root",
                    )
                    .with_detail(canonical_package.display().to_string()));
                }
                Some(strict_directory_size(&canonical_package).await?)
            } else {
                return Err(protocol(
                    "pnpm package path is not a directory",
                    &detail.path.display().to_string(),
                ));
            };
            let repository = detail
                .repository
                .map(PnpmRepository::url)
                .and_then(|url| non_empty(Some(url)))
                .map(|url| url.strip_prefix("git+").unwrap_or(&url).to_owned());
            packages.push(InstalledPnpmPackage {
                name,
                version: detail.version,
                description: non_empty(detail.description),
                homepage: non_empty(detail.homepage).or(repository),
                size,
            });
        }
    }
    packages.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(packages)
}

async fn canonical_directory(path: &Path, kind: &str) -> ManagerResult<PathBuf> {
    if !path.is_absolute() {
        return Err(protocol(
            &format!("{kind} is not absolute"),
            &path.display().to_string(),
        ));
    }
    let link = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| fs_error(&format!("failed to inspect {kind}"), error))?;
    if link.file_type().is_symlink() || !link.is_dir() {
        return Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            format!("{kind} is not a regular directory"),
        )
        .with_detail(path.display().to_string()));
    }
    tokio::fs::canonicalize(path)
        .await
        .map_err(|error| fs_error(&format!("failed to resolve {kind}"), error))
}

async fn strict_directory_size(root: &Path) -> ManagerResult<u64> {
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&directory)
            .await
            .map_err(|error| fs_error("failed to read pnpm package directory", error))?;
        let mut children = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| fs_error("failed to read pnpm package directory entry", error))?
        {
            children.push(entry);
        }
        children.sort_by_key(tokio::fs::DirEntry::file_name);
        for entry in children.into_iter().rev() {
            let file_type = entry.file_type().await.map_err(|error| {
                fs_error("failed to inspect pnpm package directory entry", error)
            })?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                let length = entry
                    .metadata()
                    .await
                    .map_err(|error| fs_error("failed to inspect pnpm package file", error))?
                    .len();
                total = total.checked_add(length).ok_or_else(|| {
                    protocol(
                        "pnpm package size exceeds the supported range",
                        &root.display().to_string(),
                    )
                })?;
            }
        }
    }
    Ok(total)
}

fn package_origin(name: &str, package: &str) -> PackageOrigin {
    PackageOrigin::new(name).with_reference(format!("package:{package}"))
}

fn validate_origin(origin: &PackageOrigin, package: &str) -> ManagerResult<()> {
    let expected = format!("package:{package}");
    if origin.reference.as_deref() != Some(expected.as_str()) {
        return Err(protocol(
            "pnpm package origin reference is malformed",
            package,
        ));
    }
    if origin.name != "pnpm global" {
        validate_registry(&origin.name)?;
    }
    Ok(())
}

fn validate_registry(value: &str) -> ManagerResult<String> {
    let mut url = reqwest::Url::parse(value)
        .map_err(|error| protocol("pnpm registry URL is invalid", &error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(protocol("pnpm registry URL is unsupported", value));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url.to_string())
}

fn validate_package_name(name: &str) -> ManagerResult<()> {
    if name.is_empty() || name.len() > 214 || !name.is_ascii() || name.starts_with('-') {
        return Err(protocol("pnpm package name is malformed", name));
    }
    let valid = if let Some(scoped) = name.strip_prefix('@') {
        scoped.split_once('/').is_some_and(|(scope, package)| {
            valid_name_component(scope) && valid_name_component(package) && !package.contains('/')
        })
    } else {
        valid_name_component(name)
    };
    if valid {
        Ok(())
    } else {
        Err(protocol("pnpm package name is malformed", name))
    }
}

fn valid_name_component(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(['.', '_'])
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '.' | '_' | '~')
        })
}

fn validate_version(version: &str) -> ManagerResult<()> {
    Version::parse(version).map(|_| ()).map_err(|error| {
        protocol(
            "pnpm package version is not valid semver",
            &error.to_string(),
        )
    })
}

fn validate_version_or_tag(version: &str) -> ManagerResult<()> {
    if Version::parse(version).is_ok()
        || (!version.is_empty()
            && version.len() <= 128
            && !version.starts_with('-')
            && version.is_ascii()
            && version.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | '_' | '~')
            }))
    {
        Ok(())
    } else {
        Err(protocol(
            "pnpm package version or tag is malformed",
            version,
        ))
    }
}

fn validate_search_query(query: &str) -> ManagerResult<()> {
    if query.len() <= 512 && !query.starts_with('-') && !query.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(protocol("pnpm search query is malformed", query))
    }
}

fn legacy_write_command(
    pnpm: &Path,
    action: PackageAction,
    target: &PackageTarget,
) -> ManagerResult<CommandSpec> {
    match action {
        PackageAction::Install | PackageAction::Update => {
            let version = target.version.as_deref().unwrap_or("latest");
            validate_version_or_tag(version)?;
            let spec = format!("{}@{version}", target.name);
            Ok(pnpm_command(pnpm).args(["add", "-g", spec.as_str()]))
        }
        PackageAction::Uninstall if target.version.is_none() => {
            Ok(pnpm_command(pnpm).args(["remove", "-g", target.name.as_str()]))
        }
        PackageAction::Uninstall => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "version-pinned pnpm uninstall targets are not supported",
        )
        .with_detail(&target.name)),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "pnpm package action is unsupported",
        )),
    }
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "pnpm package action is unsupported",
        )),
    }
}

fn pnpm_command(path: &Path) -> CommandSpec {
    CommandSpec::new(path)
        .env("NO_COLOR", "1")
        .env("CI", "true")
}

#[allow(dead_code)]
async fn run_pnpm_command_with_progress(
    command: &CommandSpec,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    timeout(
        COMMAND_TIMEOUT,
        run_command_with_progress(command, on_progress),
    )
    .await
    .map_err(|_| {
        ManagerError::new(ManagerErrorKind::Timeout, "pnpm package command timed out")
            .with_detail(command.program().to_string_lossy())
    })?
}

async fn run_cancellable_pnpm_command_with_progress(
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
        ManagerError::new(ManagerErrorKind::Timeout, "pnpm package command timed out")
            .with_detail(command.program().to_string_lossy())
    })?
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
    let detail = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Err(command_status_error(spec, output.status, detail.trim()))
}

fn decode_json<T: for<'de> Deserialize<'de>>(bytes: &[u8], message: &str) -> ManagerResult<T> {
    serde_json::from_slice(bytes).map_err(|error| protocol(message, &error.to_string()))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
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
