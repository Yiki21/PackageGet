use std::{
    collections::{HashMap, HashSet},
    env,
    path::Path,
    process::Output,
    time::Duration,
};

use async_trait::async_trait;
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use tokio::time::{sleep, timeout};
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
    progress::{CommandProgress, run_command_with_progress},
};

const CARGO_ID: &str = "builtin:cargo";
const CARGO_COMMAND: &str = "cargo";
const CRATES_IO_API: &str = "https://crates.io/api/v1/";
const CRATES_IO_INDEX: &str = "https://github.com/rust-lang/crates.io-index";
const NOT_INSTALLED_VERSION: &str = "Not Installed";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const TRANSIENT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Direct `updater-manager-api` implementation for Cargo-installed binaries.
#[derive(Debug, Clone)]
pub struct CargoManager {
    descriptor: ManagerDescriptor,
}

impl CargoManager {
    /// Creates the built-in Cargo manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(CARGO_ID).expect("Cargo manager ID must remain valid"),
            "Cargo",
            ManagerCategory::Development,
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
        .expect("Cargo descriptor must remain valid")
        .with_description("Rust crate 二进制包管理器")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }

    /// Returns the installed version of one unambiguous Cargo package.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the package is absent or ambiguous, and
    /// propagates typed command and parser errors from the installed inventory.
    pub async fn current_version(
        &self,
        config: &ManagerConfig,
        package_name: &str,
    ) -> ManagerResult<String> {
        let installed = self.installed_packages(config).await?;
        let matches = installed
            .iter()
            .filter(|package| package.name == package_name)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [package] => Ok(package.version.clone()),
            [] => Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo package version is unavailable",
            )
            .with_detail(package_name)),
            _ => Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo package version is ambiguous",
            )
            .with_detail(package_name)),
        }
    }

    /// Executes one Cargo target while exposing normalized command progress.
    ///
    /// # Errors
    ///
    /// Returns a typed target-validation or command execution error.
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

    async fn installed_packages(
        &self,
        config: &ManagerConfig,
    ) -> ManagerResult<Vec<InstalledCargoPackage>> {
        self.validate_config(config)?;
        let cargo_path = resolve_executable(config, CARGO_COMMAND);
        let spec = cargo_command(&cargo_path).args(["install", "--list"]);
        let output =
            run_success(&spec, COMMAND_TIMEOUT, "cargo installed listing timed out").await?;
        let stdout = String::from_utf8(output.stdout).map_err(|error| {
            ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo installed listing is not valid UTF-8",
            )
            .with_detail(error.to_string())
        })?;
        parse_installed(&stdout)
    }

    async fn registry_client(&self, config: &ManagerConfig) -> ManagerResult<CargoRegistryClient> {
        let settings: CargoSettings =
            serde_json::from_value(config.settings.clone()).map_err(|error| {
                ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "cargo manager settings are invalid",
                )
                .with_detail(error.to_string())
            })?;
        CargoRegistryClient::new(settings.api_base_url.as_deref().unwrap_or(CRATES_IO_API))
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        if &config.id == self.descriptor.id() {
            return Ok(());
        }
        Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "cargo configuration ID does not match the manager",
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
                "cargo package target belongs to another manager",
            )
            .with_detail(&target.name));
        }
        validate_package_name(&target.name)?;
        let cargo_path = resolve_executable(config, CARGO_COMMAND);

        if target.scope == PackageScope::Unknown && target.origin.is_none() {
            return write_legacy_command(&cargo_path, action, target);
        }
        if target.scope != PackageScope::User {
            return Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "cargo target scope is not supported",
            )
            .with_detail(&target.name));
        }
        let origin = target.origin.as_ref().ok_or_else(|| {
            ManagerError::new(
                ManagerErrorKind::Protocol,
                "scoped cargo target is missing its typed origin",
            )
            .with_detail(&target.name)
        })?;
        let source = CargoSource::from_origin(origin, &target.name)?;
        match (&source, action) {
            (CargoSource::CratesIo, _) => write_registry_command(&cargo_path, action, target),
            (CargoSource::Path(_) | CargoSource::Git(_), PackageAction::Uninstall) => {
                if target.version.is_some() {
                    return Err(unsupported_version(&target.name));
                }
                Ok(cargo_command(&cargo_path).args(["uninstall", target.name.as_str()]))
            }
            (CargoSource::Path(_) | CargoSource::Git(_), _) => Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "local or git Cargo targets cannot be installed or updated as registry crates",
            )
            .with_detail(&target.name)),
            (CargoSource::OtherRegistry(_), _) => Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "non-crates.io Cargo registry writes are not supported",
            )
            .with_detail(&target.name)),
            (CargoSource::Other(_), _) => Err(ManagerError::new(
                ManagerErrorKind::Unsupported,
                "Cargo targets with an unknown source are read-only",
            )
            .with_detail(&target.name)),
        }
    }
}

impl Default for CargoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for CargoManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        self.validate_config(config)?;
        Ok(manager_availability(config, CARGO_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        self.installed_packages(config)
            .await?
            .into_iter()
            .map(|package| package.info(self.descriptor.id(), config))
            .collect()
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
        let client = self.registry_client(config).await?;
        let registry_packages = installed
            .into_iter()
            .filter(|package| package.source == CargoSource::CratesIo)
            .collect::<Vec<_>>();
        let mut updates = Vec::new();
        for package in registry_packages {
            let metadata = client.crate_metadata(&package.name).await?;
            let available_version = metadata.available_version()?.to_owned();
            if available_version != package.version {
                updates.push(PackageUpdate::new(
                    package.target(self.descriptor.id()),
                    package.version,
                    available_version,
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
        let installed = self
            .installed_packages(config)
            .await?
            .into_iter()
            .filter(|package| package.source == CargoSource::CratesIo)
            .map(|package| (package.name, package.version))
            .collect::<HashMap<_, _>>();
        let response = self.registry_client(config).await?.search(query).await?;
        response
            .crates
            .into_iter()
            .map(|metadata| {
                let installed_version = installed.get(&metadata.name).map(String::as_str);
                metadata.info(self.descriptor.id(), installed_version)
            })
            .collect()
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
            run_command_with_progress(command, |event| {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CargoSource {
    CratesIo,
    OtherRegistry(String),
    Path(String),
    Git(String),
    Other(String),
}

impl CargoSource {
    fn parse(marker: Option<&str>) -> ManagerResult<Self> {
        let Some(marker) = marker.map(str::trim).filter(|marker| !marker.is_empty()) else {
            return Ok(Self::CratesIo);
        };
        if let Some(value) = marker.strip_prefix("registry+") {
            return if value == CRATES_IO_INDEX {
                Ok(Self::CratesIo)
            } else {
                Ok(Self::OtherRegistry(required_source(value, marker)?))
            };
        }
        if let Some(value) = marker.strip_prefix("sparse+") {
            return Ok(Self::OtherRegistry(required_source(value, marker)?));
        }
        if let Some(value) = marker.strip_prefix("path+") {
            return Ok(Self::Path(required_source(value, marker)?));
        }
        if let Some(value) = marker.strip_prefix("git+") {
            return Ok(Self::Git(required_source(value, marker)?));
        }
        if marker.starts_with('/') || marker.starts_with("file://") {
            return Ok(Self::Path(marker.to_owned()));
        }
        if marker.starts_with("http://")
            || marker.starts_with("https://")
            || marker.starts_with("ssh://")
            || marker.starts_with("git@")
        {
            return Ok(Self::Git(marker.to_owned()));
        }
        Ok(Self::Other(marker.to_owned()))
    }

    fn origin(&self, package_name: &str) -> PackageOrigin {
        match self {
            Self::CratesIo => PackageOrigin::new("crates.io")
                .with_reference(format!("registry:crates.io/{package_name}")),
            Self::OtherRegistry(url) => {
                PackageOrigin::new("cargo registry").with_reference(format!("registry:{url}"))
            }
            Self::Path(path) => PackageOrigin::new("path").with_reference(format!("path:{path}")),
            Self::Git(url) => PackageOrigin::new("git").with_reference(format!("git:{url}")),
            Self::Other(marker) => {
                PackageOrigin::new("cargo source").with_reference(format!("other:{marker}"))
            }
        }
    }

    fn from_origin(origin: &PackageOrigin, package_name: &str) -> ManagerResult<Self> {
        let reference = origin.reference.as_deref().ok_or_else(|| {
            ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo package origin is missing its typed reference",
            )
            .with_detail(package_name)
        })?;
        let source = if reference == format!("registry:crates.io/{package_name}") {
            Self::CratesIo
        } else if let Some(value) = reference.strip_prefix("registry:") {
            Self::OtherRegistry(required_source(value, reference)?)
        } else if let Some(value) = reference.strip_prefix("path:") {
            Self::Path(required_source(value, reference)?)
        } else if let Some(value) = reference.strip_prefix("git:") {
            Self::Git(required_source(value, reference)?)
        } else if let Some(value) = reference.strip_prefix("other:") {
            Self::Other(required_source(value, reference)?)
        } else {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo package origin reference is malformed",
            )
            .with_detail(reference));
        };
        let expected_name = source.origin(package_name).name;
        if origin.name != expected_name {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo package origin name does not match its typed reference",
            )
            .with_detail(format!("{} != {expected_name}", origin.name)));
        }
        Ok(source)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledCargoPackage {
    name: String,
    version: String,
    source: CargoSource,
    binaries: Vec<String>,
}

impl InstalledCargoPackage {
    fn target(&self, manager_id: &ManagerId) -> PackageTarget {
        let mut target = PackageTarget::new(manager_id.clone(), &self.name);
        target.scope = PackageScope::User;
        target.origin = Some(self.source.origin(&self.name));
        target
    }

    fn info(self, manager_id: &ManagerId, config: &ManagerConfig) -> ManagerResult<PackageInfo> {
        let mut info = PackageInfo::new(manager_id.clone(), &self.name, self.version);
        info.scope = PackageScope::User;
        info.origin = Some(self.source.origin(&self.name));
        info.size = installed_binary_size(config, &self.binaries)?;
        Ok(info)
    }
}

fn parse_installed(stdout: &str) -> ManagerResult<Vec<InstalledCargoPackage>> {
    let mut packages = Vec::new();
    let mut current: Option<InstalledCargoPackage> = None;
    let mut identities = HashSet::new();
    for raw_line in stdout.lines() {
        if raw_line.trim().is_empty() {
            continue;
        }
        if raw_line.starts_with(char::is_whitespace) {
            let binary = raw_line.trim();
            if binary.is_empty() || binary.chars().any(char::is_whitespace) {
                return Err(installed_protocol_error(
                    "cargo installed binary name is malformed",
                    raw_line,
                ));
            }
            current
                .as_mut()
                .ok_or_else(|| {
                    installed_protocol_error(
                        "cargo installed binary has no package header",
                        raw_line,
                    )
                })?
                .binaries
                .push(binary.to_owned());
            continue;
        }
        if let Some(package) = current.take() {
            packages.push(package);
        }
        let line = raw_line.strip_suffix(':').ok_or_else(|| {
            installed_protocol_error("cargo installed package header is malformed", raw_line)
        })?;
        let (identity, marker) = split_source_marker(line)?;
        let (name, version) = identity.split_once(' ').ok_or_else(|| {
            installed_protocol_error("cargo installed package identity is malformed", raw_line)
        })?;
        validate_package_name(name)?;
        let version = version.strip_prefix('v').unwrap_or(version).trim();
        if version.is_empty() || version.chars().any(char::is_whitespace) {
            return Err(installed_protocol_error(
                "cargo installed package version is malformed",
                raw_line,
            ));
        }
        let source = CargoSource::parse(marker)?;
        if !identities.insert((name.to_owned(), source.clone())) {
            return Err(installed_protocol_error(
                "cargo installed inventory contains a duplicate identity",
                raw_line,
            ));
        }
        current = Some(InstalledCargoPackage {
            name: name.to_owned(),
            version: version.to_owned(),
            source,
            binaries: Vec::new(),
        });
    }
    if let Some(package) = current {
        packages.push(package);
    }
    Ok(packages)
}

fn split_source_marker(line: &str) -> ManagerResult<(&str, Option<&str>)> {
    if !line.ends_with(')') {
        return Ok((line, None));
    }
    let start = line.rfind(" (").ok_or_else(|| {
        installed_protocol_error("cargo installed source marker is malformed", line)
    })?;
    Ok((&line[..start], Some(&line[start + 2..line.len() - 1])))
}

#[derive(Debug, Default, Deserialize)]
struct CargoSettings {
    #[serde(default)]
    api_base_url: Option<String>,
    #[serde(default)]
    install_root: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone)]
struct CargoRegistryClient {
    client: Client,
    base_url: Url,
}

impl CargoRegistryClient {
    fn new(base_url: &str) -> ManagerResult<Self> {
        let mut base_url = Url::parse(base_url).map_err(|error| {
            ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo registry API URL is invalid",
            )
            .with_detail(error.to_string())
        })?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo registry API URL must use HTTP or HTTPS",
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
            .map_err(|error| network_error("failed to create cargo registry client", error))?;
        Ok(Self { client, base_url })
    }

    async fn search(&self, query: &str) -> ManagerResult<CratesResponse> {
        let url = self.endpoint(&["crates"])?;
        self.request(
            self.client
                .get(url)
                .query(&[("q", query), ("page", "1"), ("per_page", "50")]),
        )
        .await
    }

    async fn crate_metadata(&self, name: &str) -> ManagerResult<CrateMetadata> {
        let url = self.endpoint(&["crates", name])?;
        let response: CrateResponse = self.request(self.client.get(url)).await?;
        response.krate.validated()
    }

    fn endpoint(&self, segments: &[&str]) -> ManagerResult<Url> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| {
                ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "cargo registry API URL cannot contain path segments",
                )
            })?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    async fn request<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> ManagerResult<T> {
        let retry = request.try_clone();
        match self.request_once(request).await {
            Err(error)
                if matches!(
                    error.kind(),
                    ManagerErrorKind::Network | ManagerErrorKind::Timeout
                ) && retry.is_some() =>
            {
                sleep(TRANSIENT_RETRY_DELAY).await;
                self.request_once(retry.expect("retry request was checked above"))
                    .await
            }
            result => result,
        }
    }

    async fn request_once<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> ManagerResult<T> {
        let response = request
            .send()
            .await
            .map_err(|error| network_error("cargo registry request failed", error))?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned);
            let detail = response.text().await.unwrap_or_default();
            let kind = if matches!(
                status,
                StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
            ) {
                ManagerErrorKind::Busy
            } else if status.is_server_error() {
                ManagerErrorKind::Network
            } else {
                ManagerErrorKind::Protocol
            };
            let retry =
                retry_after.map_or_else(String::new, |value| format!("; retry-after={value}"));
            return Err(
                ManagerError::new(kind, "cargo registry returned an unsuccessful status")
                    .with_detail(format!("HTTP {status}{retry}: {}", bounded_detail(&detail))),
            );
        }
        let url = response.url().clone();
        let body = response
            .bytes()
            .await
            .map_err(|error| network_error("failed to read cargo registry response body", error))?;
        serde_json::from_slice(&body).map_err(|error| {
            ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo registry response JSON is invalid",
            )
            .with_detail(format!("{url}: {error}"))
        })
    }
}

#[derive(Debug, Deserialize)]
struct CratesResponse {
    #[serde(default)]
    crates: Vec<CrateMetadata>,
}

#[derive(Debug, Deserialize)]
struct CrateResponse {
    #[serde(rename = "crate")]
    krate: CrateMetadata,
}

#[derive(Debug, Deserialize)]
struct CrateMetadata {
    name: String,
    max_version: String,
    #[serde(default)]
    max_stable_version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    repository: Option<String>,
}

impl CrateMetadata {
    fn validated(self) -> ManagerResult<Self> {
        validate_package_name(&self.name)?;
        if self.available_version().is_err() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo registry crate version is missing",
            )
            .with_detail(self.name));
        }
        Ok(self)
    }

    fn available_version(&self) -> ManagerResult<&str> {
        self.max_stable_version
            .as_deref()
            .filter(|version| !version.trim().is_empty())
            .or_else(|| (!self.max_version.trim().is_empty()).then_some(self.max_version.as_str()))
            .ok_or_else(|| {
                ManagerError::new(
                    ManagerErrorKind::Protocol,
                    "cargo registry crate version is missing",
                )
                .with_detail(&self.name)
            })
    }

    fn info(
        self,
        manager_id: &ManagerId,
        installed_version: Option<&str>,
    ) -> ManagerResult<PackageInfo> {
        let metadata = self.validated()?;
        metadata.available_version()?;
        let mut info = PackageInfo::new(
            manager_id.clone(),
            &metadata.name,
            installed_version.unwrap_or(NOT_INSTALLED_VERSION),
        );
        info.description = metadata.description;
        info.homepage = metadata.homepage.or(metadata.repository);
        info.scope = PackageScope::User;
        info.origin = Some(CargoSource::CratesIo.origin(&metadata.name));
        Ok(info)
    }
}

fn cargo_command(cargo_path: &Path) -> CommandSpec {
    CommandSpec::new(cargo_path).env("CARGO_TERM_COLOR", "never")
}

async fn run_success(
    spec: &CommandSpec,
    duration: Duration,
    timeout_message: &str,
) -> ManagerResult<Output> {
    let output = timeout(duration, run_output(spec)).await.map_err(|_| {
        ManagerError::new(ManagerErrorKind::Timeout, timeout_message)
            .with_detail(spec.program().to_string_lossy())
    })??;
    if output.status.success() {
        return Ok(output);
    }
    let tail = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Err(command_status_error(spec, output.status, tail.trim()))
}

fn write_legacy_command(
    cargo_path: &Path,
    action: PackageAction,
    target: &PackageTarget,
) -> ManagerResult<CommandSpec> {
    match action {
        PackageAction::Install => install_command(cargo_path, target, false),
        PackageAction::Update => install_command(cargo_path, target, true),
        PackageAction::Uninstall => {
            if target.version.is_some() {
                Err(unsupported_version(&target.name))
            } else {
                Ok(cargo_command(cargo_path).args(["uninstall", target.name.as_str()]))
            }
        }
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "cargo package action is unsupported",
        )),
    }
}

fn write_registry_command(
    cargo_path: &Path,
    action: PackageAction,
    target: &PackageTarget,
) -> ManagerResult<CommandSpec> {
    write_legacy_command(cargo_path, action, target)
}

fn install_command(
    cargo_path: &Path,
    target: &PackageTarget,
    force: bool,
) -> ManagerResult<CommandSpec> {
    let mut command = cargo_command(cargo_path).arg("install");
    if force {
        command = command.arg("--force");
    }
    if let Some(version) = target.version.as_deref() {
        if version.trim().is_empty() || version.starts_with('-') {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo target version is malformed",
            )
            .with_detail(&target.name));
        }
        command = command.args(["--version", version]);
    }
    Ok(command.arg(&target.name))
}

fn installed_binary_size(
    config: &ManagerConfig,
    binaries: &[String],
) -> ManagerResult<Option<u64>> {
    let settings =
        serde_json::from_value::<CargoSettings>(config.settings.clone()).map_err(|error| {
            ManagerError::new(
                ManagerErrorKind::Protocol,
                "cargo manager settings are invalid",
            )
            .with_detail(error.to_string())
        })?;
    let root = env::var_os("CARGO_INSTALL_ROOT")
        .map(std::path::PathBuf::from)
        .or(settings.install_root)
        .or_else(|| env::var_os("CARGO_HOME").map(std::path::PathBuf::from))
        .or_else(|| directories_next::UserDirs::new().map(|dirs| dirs.home_dir().join(".cargo")));
    let Some(root) = root else {
        return Ok(None);
    };
    let bin_dir = root.join("bin");
    let total = binaries
        .iter()
        .filter_map(|binary| {
            let path = bin_dir.join(binary);
            std::fs::metadata(&path)
                .or_else(|_| std::fs::metadata(path.with_extension("exe")))
                .ok()
                .map(|metadata| metadata.len())
        })
        .sum::<u64>();
    Ok((total > 0).then_some(total))
}

fn validate_package_name(name: &str) -> ManagerResult<()> {
    if name.trim().is_empty()
        || name.starts_with('-')
        || name.chars().any(char::is_whitespace)
        || name
            .chars()
            .any(|character| !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_')))
    {
        return Err(ManagerError::new(
            ManagerErrorKind::Protocol,
            "cargo package name is malformed",
        )
        .with_detail(name));
    }
    Ok(())
}

fn required_source(value: &str, marker: &str) -> ManagerResult<String> {
    if value.trim().is_empty() {
        Err(installed_protocol_error(
            "cargo package source is empty",
            marker,
        ))
    } else {
        Ok(value.to_owned())
    }
}

fn ensure_supported_action(action: PackageAction) -> ManagerResult<()> {
    match action {
        PackageAction::Install | PackageAction::Update | PackageAction::Uninstall => Ok(()),
        _ => Err(ManagerError::new(
            ManagerErrorKind::Unsupported,
            "cargo package action is unsupported",
        )),
    }
}

fn unsupported_version(name: &str) -> ManagerError {
    ManagerError::new(
        ManagerErrorKind::Unsupported,
        "version-pinned Cargo uninstall targets are not supported",
    )
    .with_detail(name)
}

fn installed_protocol_error(message: &str, detail: &str) -> ManagerError {
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

fn bounded_detail(detail: &str) -> String {
    detail.chars().take(2_048).collect()
}
