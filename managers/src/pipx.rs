use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Output,
    time::Duration,
};

use async_trait::async_trait;
use reqwest::{Client, StatusCode, Url};
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

const PIPX_ID: &str = "builtin:pipx";
const PIPX_COMMAND: &str = "pipx";
const PYPI_API: &str = "https://pypi.org/pypi/";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PYPI_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Direct `updater-manager-api` implementation for pipx applications.
#[derive(Debug, Clone)]
pub struct PipxManager {
    descriptor: ManagerDescriptor,
}

impl PipxManager {
    /// Creates the built-in pipx manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(PIPX_ID).expect("pipx manager ID must remain valid"),
            "pipx",
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
        .expect("pipx descriptor must remain valid")
        .with_description("Isolated Python application manager")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }

    /// Returns the version for one unambiguous distribution or venv identity.
    ///
    /// # Errors
    ///
    /// Propagates command, JSON, environment, and filesystem errors. Missing
    /// and ambiguous identities are protocol errors.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        identity: &str,
    ) -> ManagerResult<String> {
        validate_identity(identity, "pipx package identity is malformed")?;
        let installed = self.installed_packages(config).await?;
        if let Some(package) = installed.iter().find(|package| package.venv == identity) {
            return Ok(package.version.clone());
        }
        let normalized = normalize_distribution(identity);
        let matches = installed
            .into_iter()
            .filter(|package| normalize_distribution(&package.distribution) == normalized)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [package] => Ok(package.version.clone()),
            [] => Err(protocol("pipx package version is unavailable", identity)),
            _ => Err(protocol("pipx package version is ambiguous", identity)),
        }
    }

    /// Executes one validated target with bounded command progress.
    ///
    /// # Errors
    ///
    /// Returns a typed target-validation, timeout, or command error.
    pub async fn execute_target_with_progress(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        target: &PackageTarget,
        on_progress: impl FnMut(CommandProgress),
    ) -> ManagerResult<()> {
        self.validate_config(config)?;
        let command = self.write_command(config, action, target)?;
        timeout(
            COMMAND_TIMEOUT,
            run_command_with_progress(&command, on_progress),
        )
        .await
        .map_err(|_| {
            ManagerError::new(ManagerErrorKind::Timeout, "pipx write command timed out")
                .with_detail(command.program().to_string_lossy())
        })?
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            Ok(())
        } else {
            Err(protocol(
                "pipx configuration ID does not match the manager",
                &format!("expected {}, received {}", self.descriptor.id(), config.id),
            ))
        }
    }

    async fn venvs_root(&self, config: &ManagerConfig) -> ManagerResult<PathBuf> {
        self.validate_config(config)?;
        let pipx = resolve_executable(config, PIPX_COMMAND);
        let spec = pipx_command(&pipx).args(["environment", "--value", "PIPX_HOME"]);
        let output = run_success(&spec, "pipx environment query timed out").await?;
        let value = decode_utf8(
            &output.stdout,
            "pipx environment response is not valid UTF-8",
        )?;
        let home = value.trim();
        if home.is_empty() || value.lines().count() != 1 {
            return Err(protocol("pipx environment response is malformed", &value));
        }
        let path = PathBuf::from(home);
        if !path.is_absolute() {
            return Err(protocol("PIPX_HOME must be an absolute path", home));
        }
        Ok(path.join("venvs"))
    }

    async fn installed_packages(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<Vec<InstalledPipxPackage>> {
        self.validate_config(config)?;
        let root = self.venvs_root(config).await?;
        let canonical_root = canonical_directory(&root, "pipx venv root").await?;
        let pipx = resolve_executable(config, PIPX_COMMAND);
        let spec = pipx_command(&pipx).args(["list", "--json"]);
        let output = run_success(&spec, "pipx installed listing timed out").await?;
        let response: PipxList = decode_json(&output.stdout, "pipx installed listing is invalid")?;
        response.validated(&canonical_root).await
    }

    fn pypi_client(&self, config: &ManagerConfig) -> ManagerResult<PypiClient> {
        let settings: PipxSettings = serde_json::from_value(config.settings.clone())
            .map_err(|error| protocol("pipx manager settings are invalid", &error.to_string()))?;
        PypiClient::new(settings.pypi_api_base_url.as_deref().unwrap_or(PYPI_API))
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
                "pipx package target belongs to another manager",
                &target.name,
            ));
        }
        let pipx = resolve_executable(config, PIPX_COMMAND);
        if target.scope == PackageScope::Unknown && target.origin.is_none() {
            return legacy_write_command(&pipx, action, target);
        }
        if target.scope != PackageScope::User {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "pipx target scope is not supported",
            )
            .with_detail(&target.name));
        }
        let origin = target.origin.as_ref().ok_or_else(|| {
            protocol(
                "scoped pipx target is missing its typed origin",
                &target.name,
            )
        })?;
        let reference = PipxReference::parse(origin, &target.name)?;
        match action {
            PackageAction::Install => {
                if reference.source != PipxSourceKind::Registry || reference.venv.is_some() {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "pipx install requires a PyPI distribution target",
                    )
                    .with_detail(&target.name));
                }
                let version = target.version.as_deref();
                if let Some(version) = version {
                    validate_version(version)?;
                }
                let spec = version.map_or_else(
                    || reference.distribution.clone(),
                    |version| format!("{}=={version}", reference.distribution),
                );
                Ok(pipx_command(&pipx).args(["install", spec.as_str()]))
            }
            PackageAction::Update | PackageAction::Uninstall => {
                if target.version.is_some() {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Unsupported,
                        "version-pinned pipx venv operations are not supported",
                    )
                    .with_detail(&target.name));
                }
                let venv = reference.venv.ok_or_else(|| {
                    protocol(
                        "pipx installed target is missing its venv identity",
                        &target.name,
                    )
                })?;
                let verb = if action == PackageAction::Update {
                    if reference.source != PipxSourceKind::Registry {
                        return Err(ManagerError::new(
                            ManagerErrorKind::Unsupported,
                            "non-PyPI pipx sources are read-only",
                        )
                        .with_detail(&target.name));
                    }
                    "upgrade"
                } else {
                    "uninstall"
                };
                Ok(pipx_command(&pipx).args([verb, venv.as_str()]))
            }
            _ => Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "pipx package action is unsupported",
            )),
        }
    }
}

impl Default for PipxManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for PipxManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(config, PIPX_COMMAND, &["--version"]).await)
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
        let installed = self.installed_packages(config).await?;
        let client = self.pypi_client(config)?;
        let mut updates = Vec::new();
        for package in installed
            .into_iter()
            .filter(|package| package.source.kind() == PipxSourceKind::Registry && !package.pinned)
        {
            let metadata = client
                .package(&package.distribution)
                .await?
                .ok_or_else(|| {
                    protocol(
                        "installed PyPI distribution was not found",
                        &package.distribution,
                    )
                })?;
            if normalize_distribution(&metadata.name)
                != normalize_distribution(&package.distribution)
            {
                return Err(protocol(
                    "PyPI response identity does not match installed distribution",
                    &metadata.name,
                ));
            }
            if metadata.version != package.version {
                updates.push(PackageUpdate::new(
                    package.target(self.descriptor.id()),
                    package.version,
                    metadata.version,
                ));
            }
        }
        Ok(updates)
    }

    async fn search(&self, config: &ManagerConfig, query: &str) -> ManagerResult<Vec<PackageInfo>> {
        self.validate_config(config)?;
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        validate_distribution(query)?;
        let Some(metadata) = self.pypi_client(config)?.package(query).await? else {
            return Ok(Vec::new());
        };
        let normalized = normalize_distribution(&metadata.name);
        if normalized != normalize_distribution(query) {
            return Err(protocol(
                "PyPI response identity does not match the exact query",
                &metadata.name,
            ));
        }
        let matches = self
            .installed_packages(config)
            .await?
            .into_iter()
            .filter(|package| {
                package.source.kind() == PipxSourceKind::Registry
                    && normalize_distribution(&package.distribution) == normalized
            })
            .collect::<Vec<_>>();
        let installed_version = match matches.as_slice() {
            [] => NOT_INSTALLED_VERSION,
            [package] => package.version.as_str(),
            _ => {
                return Err(protocol(
                    "pipx installed distribution identity is ambiguous",
                    query,
                ));
            }
        };
        Ok(vec![
            metadata.info(self.descriptor.id(), installed_version)?,
        ])
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
                ManagerError::new(ManagerErrorKind::Timeout, "pipx write command timed out")
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

#[derive(Debug, Default, Deserialize)]
struct PipxSettings {
    #[serde(default)]
    pypi_api_base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PipxList {
    pipx_spec_version: String,
    venvs: BTreeMap<String, PipxVenv>,
}

impl PipxList {
    async fn validated(self, venvs_root: &Path) -> ManagerResult<Vec<InstalledPipxPackage>> {
        if self.pipx_spec_version.trim().is_empty() {
            return Err(protocol(
                "pipx spec version is missing",
                "pipx_spec_version",
            ));
        }
        let mut packages = Vec::with_capacity(self.venvs.len());
        for (venv_name, venv) in self.venvs {
            validate_venv(&venv_name)?;
            let main = venv.metadata.main_package;
            validate_distribution(&main.package)?;
            validate_version(&main.package_version)?;
            let source = PipxSource::parse(&main.package_or_url, &main.package, &main.pip_args)?;
            let venv_path = venvs_root.join(&venv_name);
            let canonical_venv = canonical_directory(&venv_path, "pipx venv").await?;
            if canonical_venv.parent() != Some(venvs_root) {
                return Err(ManagerError::new(
                    ManagerErrorKind::Permission,
                    "pipx venv escapes the configured venv root",
                )
                .with_detail(canonical_venv.display().to_string()));
            }
            let size = strict_directory_size(&canonical_venv).await?;
            packages.push(InstalledPipxPackage {
                venv: venv_name,
                distribution: main.package,
                version: main.package_version,
                source,
                size,
                pinned: main.pinned,
            });
        }
        packages.sort_by(|left, right| left.distribution.cmp(&right.distribution));
        Ok(packages)
    }
}

#[derive(Debug, Deserialize)]
struct PipxVenv {
    metadata: PipxMetadata,
}

#[derive(Debug, Deserialize)]
struct PipxMetadata {
    main_package: PipxMainPackage,
}

#[derive(Debug, Deserialize)]
struct PipxMainPackage {
    package: String,
    package_or_url: String,
    package_version: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    pip_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PipxSource {
    Registry,
    Git(String),
    Url(String),
    Path(String),
    Editable(String),
    Unknown(String),
}

impl PipxSource {
    fn parse(value: &str, distribution: &str, pip_args: &[String]) -> ManagerResult<Self> {
        let value = value.trim();
        if value.is_empty() {
            return Err(protocol("pipx package source is missing", distribution));
        }
        if pip_args.iter().any(|argument| {
            argument == "-e" || argument == "--editable" || argument.starts_with("--editable=")
        }) {
            return Ok(Self::Editable(value.to_owned()));
        }
        if is_registry_requirement(value, distribution) {
            return Ok(Self::Registry);
        }
        if value.starts_with("git+") {
            return Ok(Self::Git(value.to_owned()));
        }
        if value.starts_with("http://") || value.starts_with("https://") {
            return Ok(Self::Url(value.to_owned()));
        }
        if value.starts_with("file://") || Path::new(value).is_absolute() || value.starts_with("./")
        {
            return Ok(Self::Path(value.to_owned()));
        }
        Ok(Self::Unknown(value.to_owned()))
    }

    fn kind(&self) -> PipxSourceKind {
        match self {
            Self::Registry => PipxSourceKind::Registry,
            Self::Git(_) => PipxSourceKind::Git,
            Self::Url(_) => PipxSourceKind::Url,
            Self::Path(_) => PipxSourceKind::Path,
            Self::Editable(_) => PipxSourceKind::Editable,
            Self::Unknown(_) => PipxSourceKind::Unknown,
        }
    }

    fn homepage(&self) -> Option<String> {
        match self {
            Self::Git(url) => Some(url.strip_prefix("git+").unwrap_or(url).to_owned()),
            Self::Url(url) => Some(url.clone()),
            Self::Registry | Self::Path(_) | Self::Editable(_) | Self::Unknown(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipxSourceKind {
    Registry,
    Git,
    Url,
    Path,
    Editable,
    Unknown,
}

impl PipxSourceKind {
    fn label(self) -> &'static str {
        match self {
            Self::Registry => "registry",
            Self::Git => "git",
            Self::Url => "url",
            Self::Path => "path",
            Self::Editable => "editable",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> ManagerResult<Self> {
        match value {
            "registry" => Ok(Self::Registry),
            "git" => Ok(Self::Git),
            "url" => Ok(Self::Url),
            "path" => Ok(Self::Path),
            "editable" => Ok(Self::Editable),
            "unknown" => Ok(Self::Unknown),
            _ => Err(protocol("pipx origin source kind is malformed", value)),
        }
    }
}

#[derive(Debug)]
struct InstalledPipxPackage {
    venv: String,
    distribution: String,
    version: String,
    source: PipxSource,
    size: u64,
    pinned: bool,
}

impl InstalledPipxPackage {
    fn reference(&self) -> PipxReference {
        PipxReference {
            source: self.source.kind(),
            venv: Some(self.venv.clone()),
            distribution: self.distribution.clone(),
        }
    }

    fn target(&self, manager_id: &ManagerId) -> PackageTarget {
        let mut target = PackageTarget::new(manager_id.clone(), &self.venv);
        target.scope = PackageScope::User;
        target.origin = Some(self.reference().origin());
        target
    }

    fn info(self, manager_id: &ManagerId) -> PackageInfo {
        let origin = self.reference().origin();
        let mut info = PackageInfo::new(manager_id.clone(), self.venv, self.version);
        info.homepage = self.source.homepage();
        info.size = Some(self.size);
        info.scope = PackageScope::User;
        info.origin = Some(origin);
        info
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PipxReference {
    source: PipxSourceKind,
    venv: Option<String>,
    distribution: String,
}

impl PipxReference {
    fn registry_search(distribution: &str) -> Self {
        Self {
            source: PipxSourceKind::Registry,
            venv: None,
            distribution: distribution.to_owned(),
        }
    }

    fn origin(&self) -> PackageOrigin {
        let name = match self.source {
            PipxSourceKind::Registry => "PyPI",
            PipxSourceKind::Git => "git",
            PipxSourceKind::Url => "URL",
            PipxSourceKind::Path => "path",
            PipxSourceKind::Editable => "editable",
            PipxSourceKind::Unknown => "pipx source",
        };
        let reference = self.venv.as_ref().map_or_else(
            || format!("{}:distribution={}", self.source.label(), self.distribution),
            |venv| {
                format!(
                    "{}:venv={venv};distribution={}",
                    self.source.label(),
                    self.distribution
                )
            },
        );
        PackageOrigin::new(name).with_reference(reference)
    }

    fn parse(origin: &PackageOrigin, target_name: &str) -> ManagerResult<Self> {
        let reference = origin.reference.as_deref().ok_or_else(|| {
            protocol(
                "pipx package origin is missing its typed reference",
                target_name,
            )
        })?;
        let (kind, fields) = reference
            .split_once(':')
            .ok_or_else(|| protocol("pipx package origin reference is malformed", reference))?;
        let source = PipxSourceKind::parse(kind)?;
        let mut venv = None;
        let mut distribution = None;
        for field in fields.split(';') {
            let (key, value) = field
                .split_once('=')
                .ok_or_else(|| protocol("pipx package origin field is malformed", field))?;
            match key {
                "venv" if venv.is_none() => {
                    validate_venv(value)?;
                    venv = Some(value.to_owned());
                }
                "distribution" if distribution.is_none() => {
                    validate_distribution(value)?;
                    distribution = Some(value.to_owned());
                }
                _ => return Err(protocol("pipx package origin field is duplicated", field)),
            }
        }
        let distribution = distribution.ok_or_else(|| {
            protocol("pipx package origin is missing its distribution", reference)
        })?;
        let expected_name = venv.as_deref().unwrap_or(&distribution);
        if expected_name != target_name {
            return Err(protocol(
                "pipx target name does not match its write identity",
                target_name,
            ));
        }
        let parsed = Self {
            source,
            venv,
            distribution,
        };
        if parsed.origin().name != origin.name {
            return Err(protocol(
                "pipx origin name does not match its source kind",
                &origin.name,
            ));
        }
        Ok(parsed)
    }
}

#[derive(Debug, Clone)]
struct PypiClient {
    client: Client,
    base_url: Url,
}

impl PypiClient {
    fn new(base_url: &str) -> ManagerResult<Self> {
        let mut base_url = Url::parse(base_url)
            .map_err(|error| protocol("PyPI API URL is invalid", &error.to_string()))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(protocol(
                "PyPI API URL must use HTTP or HTTPS",
                base_url.as_str(),
            ));
        }
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        let client = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent(concat!("updater/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| network_error("failed to create PyPI client", error))?;
        Ok(Self { client, base_url })
    }

    async fn package(&self, distribution: &str) -> ManagerResult<Option<PypiInfo>> {
        validate_distribution(distribution)?;
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| protocol("PyPI API URL cannot contain path segments", distribution))?
            .pop_if_empty()
            .extend([distribution, "json"]);
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| network_error("PyPI request failed", error))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            let status = response.status();
            let detail =
                bounded_response_body(&mut response, "failed to read PyPI error response").await?;
            let kind = if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                ManagerErrorKind::Permission
            } else if status.is_server_error() {
                ManagerErrorKind::Network
            } else if status == StatusCode::TOO_MANY_REQUESTS {
                ManagerErrorKind::Busy
            } else {
                ManagerErrorKind::Protocol
            };
            return Err(
                ManagerError::new(kind, "PyPI returned an unsuccessful status").with_detail(
                    format!(
                        "HTTP {status}: {}",
                        bounded(&String::from_utf8_lossy(&detail))
                    ),
                ),
            );
        }
        let body = bounded_response_body(&mut response, "failed to read PyPI response").await?;
        let response: PypiResponse = serde_json::from_slice(&body)
            .map_err(|error| protocol("PyPI response is invalid", &error.to_string()))?;
        Ok(Some(response.info.validated()?))
    }
}

async fn bounded_response_body(
    response: &mut reqwest::Response,
    message: &str,
) -> ManagerResult<Vec<u8>> {
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(MAX_PYPI_RESPONSE_BYTES),
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| network_error(message, error))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_PYPI_RESPONSE_BYTES {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "PyPI response exceeds the supported size",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Debug, Deserialize)]
struct PypiResponse {
    info: PypiInfo,
}

#[derive(Debug, Deserialize)]
struct PypiInfo {
    name: String,
    version: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    home_page: Option<String>,
    #[serde(default)]
    project_url: Option<String>,
    #[serde(default)]
    project_urls: Option<BTreeMap<String, String>>,
}

impl PypiInfo {
    fn validated(self) -> ManagerResult<Self> {
        validate_distribution(&self.name)?;
        validate_version(&self.version)?;
        Ok(self)
    }

    fn homepage(&self) -> Option<String> {
        non_empty(self.home_page.clone())
            .or_else(|| {
                let urls = self.project_urls.as_ref()?;
                ["Homepage", "Source", "Source Code", "Repository"]
                    .into_iter()
                    .find_map(|key| urls.get(key).cloned().and_then(|url| non_empty(Some(url))))
            })
            .or_else(|| non_empty(self.project_url.clone()))
    }

    fn info(self, manager_id: &ManagerId, installed_version: &str) -> ManagerResult<PackageInfo> {
        let homepage = self.homepage();
        let reference = PipxReference::registry_search(&self.name);
        let mut info = PackageInfo::new(manager_id.clone(), &self.name, installed_version);
        info.description = non_empty(self.summary);
        info.homepage = homepage;
        info.scope = PackageScope::User;
        info.origin = Some(reference.origin());
        Ok(info)
    }
}

async fn canonical_directory(path: &Path, kind: &str) -> ManagerResult<PathBuf> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| fs_error(&format!("failed to inspect {kind}"), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
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
        let mut entries = tokio::fs::read_dir(directory)
            .await
            .map_err(|error| fs_error("failed to read pipx venv directory", error))?;
        let mut children = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| fs_error("failed to read pipx venv directory entry", error))?
        {
            children.push(entry);
        }
        children.sort_by_key(tokio::fs::DirEntry::file_name);
        for entry in children.into_iter().rev() {
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| fs_error("failed to inspect pipx venv directory entry", error))?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                let size = entry
                    .metadata()
                    .await
                    .map_err(|error| fs_error("failed to inspect pipx venv file", error))?
                    .len();
                total = total.checked_add(size).ok_or_else(|| {
                    ManagerError::new(
                        ManagerErrorKind::Other,
                        "pipx venv size exceeds the supported range",
                    )
                    .with_detail(root.display().to_string())
                })?;
            }
        }
    }
    Ok(total)
}

fn legacy_write_command(
    pipx: &Path,
    action: PackageAction,
    target: &PackageTarget,
) -> ManagerResult<CommandSpec> {
    match action {
        PackageAction::Install => {
            validate_distribution(&target.name)?;
            let spec = target.version.as_deref().map_or_else(
                || target.name.clone(),
                |version| format!("{}=={version}", target.name),
            );
            if let Some(version) = target.version.as_deref() {
                validate_version(version)?;
            }
            Ok(pipx_command(pipx).args(["install", spec.as_str()]))
        }
        PackageAction::Update | PackageAction::Uninstall => {
            validate_venv(&target.name)?;
            if target.version.is_some() {
                return Err(ManagerError::new(
                    ManagerErrorKind::Unsupported,
                    "version-pinned pipx venv operations are not supported",
                )
                .with_detail(&target.name));
            }
            let verb = if action == PackageAction::Update {
                "upgrade"
            } else {
                "uninstall"
            };
            Ok(pipx_command(pipx).args([verb, target.name.as_str()]))
        }
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "pipx package action is unsupported",
        )),
    }
}

fn validate_distribution(value: &str) -> ManagerResult<()> {
    validate_identity(value, "Python distribution name is malformed")
}

fn validate_venv(value: &str) -> ManagerResult<()> {
    validate_identity(value, "pipx venv name is malformed")
}

fn validate_identity(value: &str, message: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value.starts_with(['-', '.'])
        || value.ends_with('.')
        || value.chars().any(char::is_whitespace)
        || value.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
        })
        || value.contains([';', '='])
    {
        return Err(protocol(message, value));
    }
    Ok(())
}

fn validate_version(value: &str) -> ManagerResult<()> {
    if value.is_empty()
        || value.starts_with('-')
        || value.chars().any(char::is_whitespace)
        || value.contains(['/', '\\', ';', '='])
    {
        return Err(protocol("Python package version is malformed", value));
    }
    Ok(())
}

fn is_registry_requirement(value: &str, distribution: &str) -> bool {
    let (name, version) = value
        .split_once("==")
        .map_or((value, None), |(name, version)| (name, Some(version)));
    validate_distribution(name).is_ok()
        && normalize_distribution(name) == normalize_distribution(distribution)
        && version.is_none_or(|version| validate_version(version).is_ok())
}

fn normalize_distribution(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut separator = false;
    for character in value.chars() {
        if matches!(character, '-' | '_' | '.') {
            if !separator {
                normalized.push('-');
                separator = true;
            }
        } else {
            normalized.push(character.to_ascii_lowercase());
            separator = false;
        }
    }
    normalized
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "pipx package action is unsupported",
        )),
    }
}

fn pipx_command(path: &Path) -> CommandSpec {
    CommandSpec::new(path).env("NO_COLOR", "1")
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

fn decode_json<T: for<'de> Deserialize<'de>>(bytes: &[u8], message: &str) -> ManagerResult<T> {
    serde_json::from_slice(bytes).map_err(|error| protocol(message, &error.to_string()))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn protocol(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(detail)
}

fn network_error(message: &str, error: reqwest::Error) -> ManagerError {
    let kind = if error.is_timeout() {
        ManagerErrorKind::Timeout
    } else {
        ManagerErrorKind::Network
    };
    ManagerError::new(kind, message).with_detail(error.to_string())
}

fn fs_error(message: &str, error: std::io::Error) -> ManagerError {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied => ManagerErrorKind::Permission,
        std::io::ErrorKind::TimedOut => ManagerErrorKind::Timeout,
        _ => ManagerErrorKind::Other,
    };
    ManagerError::new(kind, message).with_detail(error.to_string())
}

fn bounded(value: &str) -> String {
    value.chars().take(2_048).collect()
}
