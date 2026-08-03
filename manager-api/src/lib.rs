//! Public extension contracts for package managers used by Updater.
//!
//! This crate intentionally contains no GUI types, async runtime, or concrete
//! command execution. Third-party crates can implement [`PackageManager`] and
//! register an instance with `updater_core` at compile time.

#![deny(missing_docs)]

use std::{
    collections::BTreeSet,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::Value;
use thiserror::Error;

const MAX_MANAGER_ID_LEN: usize = 128;

/// A validated, namespaced package manager identifier.
///
/// IDs use the form `namespace:name`, for example `builtin:apt` or
/// `org.example:manager`. Components must be lowercase ASCII and may contain
/// digits, `.`, `_`, or `-` between alphanumeric characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ManagerId(String);

impl ManagerId {
    /// Parses and validates a manager ID.
    ///
    /// # Errors
    ///
    /// Returns [`ManagerIdError`] when the value is empty, too long, lacks a
    /// single namespace separator, or contains an invalid component.
    pub fn parse(value: impl Into<String>) -> Result<Self, ManagerIdError> {
        let value = value.into();

        if value.is_empty() {
            return Err(ManagerIdError::Empty);
        }
        if value.len() > MAX_MANAGER_ID_LEN {
            return Err(ManagerIdError::TooLong {
                max: MAX_MANAGER_ID_LEN,
            });
        }

        let Some((namespace, name)) = value.split_once(':') else {
            return Err(ManagerIdError::MissingSeparator);
        };
        if name.contains(':') {
            return Err(ManagerIdError::MultipleSeparators);
        }
        if !is_valid_id_component(namespace) {
            return Err(ManagerIdError::InvalidNamespace(namespace.to_owned()));
        }
        if !is_valid_id_component(name) {
            return Err(ManagerIdError::InvalidName(name.to_owned()));
        }

        Ok(Self(value))
    }

    /// Returns the serialized ID value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the ID and returns its owned string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

fn is_valid_id_component(component: &str) -> bool {
    let mut chars = component.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let last = component.chars().next_back().unwrap_or(first);

    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && (last.is_ascii_lowercase() || last.is_ascii_digit())
        && component.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
}

impl fmt::Display for ManagerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for ManagerId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for ManagerId {
    type Err = ManagerIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl TryFrom<String> for ManagerId {
    type Error = ManagerIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for ManagerId {
    type Error = ManagerIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl Serialize for ManagerId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ManagerId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(D::Error::custom)
    }
}

/// Validation failures for [`ManagerId`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ManagerIdError {
    /// The ID was empty.
    #[error("manager ID is empty")]
    Empty,
    /// The ID exceeded the supported length.
    #[error("manager ID exceeds {max} bytes")]
    TooLong {
        /// Maximum accepted byte length.
        max: usize,
    },
    /// The ID did not contain `:`.
    #[error("manager ID must contain one namespace separator")]
    MissingSeparator,
    /// The ID contained more than one `:`.
    #[error("manager ID must contain only one namespace separator")]
    MultipleSeparators,
    /// The namespace component was invalid.
    #[error("invalid manager namespace: {0}")]
    InvalidNamespace(String),
    /// The name component was invalid.
    #[error("invalid manager name: {0}")]
    InvalidName(String),
}

/// Operating systems a manager can support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Platform {
    /// Linux desktop and server environments.
    Linux,
    /// Microsoft Windows.
    Windows,
    /// Apple macOS.
    #[serde(rename = "macos")]
    MacOs,
}

impl Platform {
    /// Returns the platform represented by the current compilation target.
    #[must_use]
    pub const fn current() -> Option<Self> {
        if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else {
            None
        }
    }
}

/// A deterministic set of supported platforms.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SupportedPlatforms(BTreeSet<Platform>);

impl SupportedPlatforms {
    /// Creates a platform set from an iterator.
    #[must_use]
    pub fn new(platforms: impl IntoIterator<Item = Platform>) -> Self {
        Self(platforms.into_iter().collect())
    }

    /// Returns whether the set contains `platform`.
    #[must_use]
    pub fn contains(&self, platform: Platform) -> bool {
        self.0.contains(&platform)
    }

    /// Returns whether no platform is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterates over platforms in stable order.
    pub fn iter(&self) -> impl Iterator<Item = &Platform> {
        self.0.iter()
    }
}

impl<const N: usize> From<[Platform; N]> for SupportedPlatforms {
    fn from(platforms: [Platform; N]) -> Self {
        Self::new(platforms)
    }
}

/// Broad manager category used for catalog grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ManagerCategory {
    /// Distribution or operating-system package manager.
    System,
    /// Desktop application or sandbox manager.
    Application,
    /// Language or developer-tool package manager.
    Development,
    /// Manager that does not fit another category.
    Other,
}

/// A single operation supported by a manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ManagerCapability {
    /// Enumerate installed packages and counts.
    Installed,
    /// Discover available updates.
    Updates,
    /// Search for packages.
    Search,
    /// Install packages.
    Install,
    /// Update packages.
    Update,
    /// Uninstall packages.
    Uninstall,
}

impl fmt::Display for ManagerCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Installed => "installed package listing",
            Self::Updates => "update discovery",
            Self::Search => "package search",
            Self::Install => "package installation",
            Self::Update => "package updates",
            Self::Uninstall => "package removal",
        })
    }
}

/// A deterministic set of manager capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ManagerCapabilities(BTreeSet<ManagerCapability>);

impl ManagerCapabilities {
    /// Creates a capability set from an iterator.
    #[must_use]
    pub fn new(capabilities: impl IntoIterator<Item = ManagerCapability>) -> Self {
        Self(capabilities.into_iter().collect())
    }

    /// Returns whether the set contains `capability`.
    #[must_use]
    pub fn contains(&self, capability: ManagerCapability) -> bool {
        self.0.contains(&capability)
    }

    /// Returns whether no capability is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterates over capabilities in stable order.
    pub fn iter(&self) -> impl Iterator<Item = &ManagerCapability> {
        self.0.iter()
    }
}

impl<const N: usize> From<[ManagerCapability; N]> for ManagerCapabilities {
    fn from(capabilities: [ManagerCapability; N]) -> Self {
        Self::new(capabilities)
    }
}

/// User-facing authorization behavior for write operations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum AuthorizationHint {
    /// The manager does not normally require elevation.
    #[default]
    None,
    /// Some operations may require elevation.
    MayRequireElevation {
        /// Optional guidance shown before execution.
        message: Option<String>,
    },
    /// Write operations require elevation.
    RequiresElevation {
        /// Optional guidance shown before execution.
        message: Option<String>,
    },
}

/// Stable metadata describing a manager implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ManagerDescriptor {
    id: ManagerId,
    display_name: String,
    description: String,
    category: ManagerCategory,
    platforms: SupportedPlatforms,
    capabilities: ManagerCapabilities,
    authorization: AuthorizationHint,
}

impl ManagerDescriptor {
    /// Creates validated manager metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ManagerDescriptorError`] when the display name, supported
    /// platforms, or capability set is empty.
    pub fn new(
        id: ManagerId,
        display_name: impl Into<String>,
        category: ManagerCategory,
        platforms: impl Into<SupportedPlatforms>,
        capabilities: impl Into<ManagerCapabilities>,
    ) -> Result<Self, ManagerDescriptorError> {
        let display_name = display_name.into().trim().to_owned();
        let platforms = platforms.into();
        let capabilities = capabilities.into();

        if display_name.is_empty() {
            return Err(ManagerDescriptorError::EmptyDisplayName);
        }
        if platforms.is_empty() {
            return Err(ManagerDescriptorError::NoPlatforms);
        }
        if capabilities.is_empty() {
            return Err(ManagerDescriptorError::NoCapabilities);
        }

        Ok(Self {
            id,
            display_name,
            description: String::new(),
            category,
            platforms,
            capabilities,
            authorization: AuthorizationHint::None,
        })
    }

    /// Sets the user-facing description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Sets the authorization guidance.
    #[must_use]
    pub fn with_authorization(mut self, authorization: AuthorizationHint) -> Self {
        self.authorization = authorization;
        self
    }

    /// Returns the stable manager ID.
    #[must_use]
    pub fn id(&self) -> &ManagerId {
        &self.id
    }

    /// Returns the display name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the user-facing description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the catalog category.
    #[must_use]
    pub fn category(&self) -> ManagerCategory {
        self.category
    }

    /// Returns the supported platform set.
    #[must_use]
    pub fn platforms(&self) -> &SupportedPlatforms {
        &self.platforms
    }

    /// Returns the supported capability set.
    #[must_use]
    pub fn capabilities(&self) -> &ManagerCapabilities {
        &self.capabilities
    }

    /// Returns authorization guidance for write operations.
    #[must_use]
    pub fn authorization(&self) -> &AuthorizationHint {
        &self.authorization
    }
}

/// Validation failures for [`ManagerDescriptor`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ManagerDescriptorError {
    /// The display name contained only whitespace.
    #[error("manager display name is empty")]
    EmptyDisplayName,
    /// The descriptor did not declare any supported platform.
    #[error("manager must support at least one platform")]
    NoPlatforms,
    /// The descriptor did not declare any capability.
    #[error("manager must declare at least one capability")]
    NoCapabilities,
}

/// Persisted configuration for a manager instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ManagerConfig {
    /// Stable manager ID.
    pub id: ManagerId,
    /// Optional custom executable path.
    pub executable: Option<PathBuf>,
    /// Manager-private JSON settings.
    pub settings: Value,
}

impl ManagerConfig {
    /// Creates a configuration with default private settings.
    #[must_use]
    pub fn new(id: ManagerId) -> Self {
        Self {
            id,
            executable: None,
            settings: Value::Object(Default::default()),
        }
    }

    /// Sets a custom executable path.
    #[must_use]
    pub fn with_executable(mut self, executable: impl Into<PathBuf>) -> Self {
        self.executable = Some(executable.into());
        self
    }

    /// Returns the custom executable path, when configured.
    #[must_use]
    pub fn executable(&self) -> Option<&Path> {
        self.executable.as_deref()
    }
}

/// Installation scope reported by a manager.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PackageScope {
    /// System-wide installation.
    System,
    /// Current-user installation.
    User,
    /// Project-local installation.
    Project,
    /// Manager-specific or unknown scope.
    #[default]
    Unknown,
}

/// Repository, channel, or other source metadata for a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PackageOrigin {
    /// Human-readable source name.
    pub name: String,
    /// Optional manager-specific reference such as a URL or channel.
    pub reference: Option<String>,
}

impl PackageOrigin {
    /// Creates source metadata with no additional reference.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reference: None,
        }
    }

    /// Sets a manager-specific source reference.
    #[must_use]
    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }
}

/// Installed or searchable package metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PackageInfo {
    /// Manager that produced the package.
    pub manager_id: ManagerId,
    /// Manager-specific package identifier.
    pub name: String,
    /// Installed or advertised version.
    pub version: String,
    /// Optional user-facing description.
    pub description: Option<String>,
    /// Optional package homepage.
    pub homepage: Option<String>,
    /// Optional installed size in bytes.
    pub size: Option<u64>,
    /// Optional manager-reported installation date.
    pub install_date: Option<String>,
    /// Installation scope.
    pub scope: PackageScope,
    /// Optional repository or channel metadata.
    pub origin: Option<PackageOrigin>,
}

impl PackageInfo {
    /// Creates package metadata with optional fields unset.
    #[must_use]
    pub fn new(manager_id: ManagerId, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            manager_id,
            name: name.into(),
            version: version.into(),
            description: None,
            homepage: None,
            size: None,
            install_date: None,
            scope: PackageScope::Unknown,
            origin: None,
        }
    }

    /// Freezes this package's manager-owned identity for a write operation.
    #[must_use]
    pub fn target(&self) -> PackageTarget {
        PackageTarget {
            manager_id: self.manager_id.clone(),
            name: self.name.clone(),
            version: None,
            scope: self.scope,
            origin: self.origin.clone(),
        }
    }
}

/// A package target frozen for a write operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PackageTarget {
    /// Manager responsible for the target.
    pub manager_id: ManagerId,
    /// Manager-specific package identifier.
    pub name: String,
    /// Optional requested version or channel.
    pub version: Option<String>,
    /// Requested installation scope.
    pub scope: PackageScope,
    /// Optional repository or channel metadata.
    pub origin: Option<PackageOrigin>,
}

impl PackageTarget {
    /// Creates a package target with manager defaults.
    #[must_use]
    pub fn new(manager_id: ManagerId, name: impl Into<String>) -> Self {
        Self {
            manager_id,
            name: name.into(),
            version: None,
            scope: PackageScope::Unknown,
            origin: None,
        }
    }
}

/// An available update for an installed package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PackageUpdate {
    /// Frozen package target.
    pub target: PackageTarget,
    /// Currently installed version.
    pub current_version: String,
    /// Available version.
    pub available_version: String,
}

impl PackageUpdate {
    /// Creates an update record.
    #[must_use]
    pub fn new(
        target: PackageTarget,
        current_version: impl Into<String>,
        available_version: impl Into<String>,
    ) -> Self {
        Self {
            target,
            current_version: current_version.into(),
            available_version: available_version.into(),
        }
    }
}

/// A write operation requested from a manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PackageAction {
    /// Install the requested targets.
    Install,
    /// Update the requested targets.
    Update,
    /// Uninstall the requested targets.
    Uninstall,
}

impl PackageAction {
    /// Returns the capability required for this action.
    #[must_use]
    pub const fn capability(self) -> ManagerCapability {
        match self {
            Self::Install => ManagerCapability::Install,
            Self::Update => ManagerCapability::Update,
            Self::Uninstall => ManagerCapability::Uninstall,
        }
    }
}

impl fmt::Display for PackageAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::Uninstall => "uninstall",
        })
    }
}

/// Progress emitted while executing one manager group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum ProgressEvent {
    /// The manager group started.
    Started {
        /// Requested action.
        action: PackageAction,
        /// Number of package targets in the group.
        total: usize,
    },
    /// Package-level progress changed.
    Advanced {
        /// Completed package targets.
        completed: usize,
        /// Total package targets.
        total: usize,
        /// Current package identifier, when known.
        current_package: Option<String>,
    },
    /// Manager-specific diagnostic output.
    Message {
        /// Bounded user-facing or diagnostic message.
        message: String,
    },
    /// The manager group finished successfully.
    Finished {
        /// Number of completed package targets.
        completed: usize,
        /// Total package targets.
        total: usize,
    },
}

/// Runtime-neutral destination for manager progress events.
pub trait ProgressSink: Send + Sync {
    /// Emits a progress event.
    fn emit(&self, event: ProgressEvent);

    /// Returns whether the caller requested cancellation.
    ///
    /// The default preserves source compatibility for managers that only emit
    /// progress. Implementations that support active cancellation should check
    /// this value before starting work and while waiting for external commands.
    fn is_cancelled(&self) -> bool {
        false
    }
}

impl<F> ProgressSink for F
where
    F: Fn(ProgressEvent) + Send + Sync,
{
    fn emit(&self, event: ProgressEvent) {
        self(event);
    }
}

/// Progress sink that discards all events.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopProgressSink;

impl ProgressSink for NoopProgressSink {
    fn emit(&self, _event: ProgressEvent) {}
}

/// Reason a manager is unavailable on the current system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum AvailabilityReason {
    /// The current operating system is unsupported.
    UnsupportedPlatform {
        /// Current platform when it can be represented by [`Platform`].
        platform: Option<Platform>,
    },
    /// The configured command could not be found.
    CommandMissing {
        /// Command or path that was checked.
        command: String,
    },
    /// The configured command exists but is not executable.
    NotExecutable {
        /// Path that failed validation.
        path: PathBuf,
    },
    /// The command failed its health or version check.
    VersionCheckFailed {
        /// Bounded failure detail.
        detail: String,
    },
}

/// Availability state returned by a manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
#[non_exhaustive]
pub enum ManagerAvailability {
    /// The manager is ready for use.
    Available {
        /// Optional detected manager version.
        version: Option<String>,
    },
    /// The manager is not currently usable.
    Unavailable {
        /// Structured unavailability reason.
        reason: AvailabilityReason,
    },
}

impl ManagerAvailability {
    /// Returns whether the manager is available.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }
}

/// Stable error classification used by core and UI guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ManagerErrorKind {
    /// Required command is missing.
    CommandMissing,
    /// Permission or elevation was denied.
    Permission,
    /// A network request failed.
    Network,
    /// A package database or manager lock is busy.
    Busy,
    /// An operation exceeded its deadline.
    Timeout,
    /// An installer requires a reboot.
    RebootRequired,
    /// Manager output did not match the expected protocol.
    Protocol,
    /// The requested operation is unsupported.
    Unsupported,
    /// The operation was cancelled.
    Cancelled,
    /// Unclassified manager failure.
    Other,
}

/// Structured package manager failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("{message}")]
#[non_exhaustive]
pub struct ManagerError {
    kind: ManagerErrorKind,
    message: String,
    detail: Option<String>,
}

impl ManagerError {
    /// Creates a classified manager error.
    #[must_use]
    pub fn new(kind: ManagerErrorKind, message: impl Into<String>) -> Self {
        let message = message.into();
        let message = if message.trim().is_empty() {
            "manager operation failed".to_owned()
        } else {
            message
        };

        Self {
            kind,
            message,
            detail: None,
        }
    }

    /// Creates an unsupported-capability error.
    #[must_use]
    pub fn unsupported(capability: ManagerCapability) -> Self {
        Self::new(
            ManagerErrorKind::Unsupported,
            format!("manager does not support {capability}"),
        )
    }

    /// Attaches bounded diagnostic detail.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Returns the stable error kind.
    #[must_use]
    pub fn kind(&self) -> ManagerErrorKind {
        self.kind
    }

    /// Returns the user-facing error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns optional diagnostic detail.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

/// Result type returned by package manager implementations.
pub type ManagerResult<T> = Result<T, ManagerError>;

/// Object-safe compile-time extension interface for package managers.
#[async_trait]
pub trait PackageManager: Send + Sync {
    /// Returns stable metadata for this implementation.
    fn descriptor(&self) -> &ManagerDescriptor;

    /// Checks whether the manager is usable with `config`.
    ///
    /// # Errors
    ///
    /// Returns a classified error when the availability check itself cannot be
    /// completed. Normal absence is represented by [`ManagerAvailability`].
    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability>;

    /// Lists installed packages.
    ///
    /// # Errors
    ///
    /// Returns [`ManagerErrorKind::Unsupported`] by default. Implementations
    /// advertising [`ManagerCapability::Installed`] must override this method.
    async fn installed(&self, _config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        Err(ManagerError::unsupported(ManagerCapability::Installed))
    }

    /// Counts installed packages.
    ///
    /// # Errors
    ///
    /// Propagates errors returned by [`PackageManager::installed`].
    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.installed(config).await?.len())
    }

    /// Lists available package updates.
    ///
    /// # Errors
    ///
    /// Returns [`ManagerErrorKind::Unsupported`] by default. Implementations
    /// advertising [`ManagerCapability::Updates`] must override this method.
    async fn updates(
        &self,
        _config: &ManagerConfig,
        _refresh: bool,
    ) -> ManagerResult<Vec<PackageUpdate>> {
        Err(ManagerError::unsupported(ManagerCapability::Updates))
    }

    /// Searches for packages matching `query`.
    ///
    /// # Errors
    ///
    /// Returns [`ManagerErrorKind::Unsupported`] by default. Implementations
    /// advertising [`ManagerCapability::Search`] must override this method.
    async fn search(
        &self,
        _config: &ManagerConfig,
        _query: &str,
    ) -> ManagerResult<Vec<PackageInfo>> {
        Err(ManagerError::unsupported(ManagerCapability::Search))
    }

    /// Executes one complete manager package group.
    ///
    /// Implementations may batch the group into one command or process targets
    /// internally in sequence. Cross-manager ordering remains the responsibility
    /// of `updater_core`.
    ///
    /// # Errors
    ///
    /// Returns [`ManagerErrorKind::Unsupported`] by default. Implementations
    /// advertising the capability for `action` must override this method.
    async fn execute(
        &self,
        _config: &ManagerConfig,
        action: PackageAction,
        _packages: &[PackageTarget],
        _progress: &dyn ProgressSink,
    ) -> ManagerResult<()> {
        Err(ManagerError::unsupported(action.capability()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_platform_matches_the_compilation_target() {
        #[cfg(target_os = "linux")]
        assert_eq!(Platform::current(), Some(Platform::Linux));
        #[cfg(target_os = "windows")]
        assert_eq!(Platform::current(), Some(Platform::Windows));
        #[cfg(target_os = "macos")]
        assert_eq!(Platform::current(), Some(Platform::MacOs));
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        assert_eq!(Platform::current(), None);
    }

    #[test]
    fn manager_id_accepts_namespaced_lowercase_values() {
        for value in ["builtin:apt", "org.example:manager", "dev-tools:cargo_2"] {
            let id = ManagerId::parse(value).expect("valid manager ID");
            assert_eq!(id.as_str(), value);
        }
    }

    #[test]
    fn manager_id_rejects_invalid_values() {
        for value in [
            "",
            "apt",
            "builtin:apt:extra",
            "Builtin:apt",
            "builtin:APT",
            "-builtin:apt",
            "builtin:apt-",
            "builtin:apt manager",
        ] {
            assert!(
                ManagerId::parse(value).is_err(),
                "accepted invalid ID: {value}"
            );
        }
    }

    #[test]
    fn manager_id_deserialization_preserves_validation() {
        let id = ManagerId::parse("builtin:apt").expect("valid ID");
        let json = serde_json::to_string(&id).expect("serialize manager ID");
        assert_eq!(json, "\"builtin:apt\"");
        assert_eq!(
            serde_json::from_str::<ManagerId>(&json).expect("deserialize manager ID"),
            id
        );
        assert!(serde_json::from_str::<ManagerId>("\"Builtin:apt\"").is_err());
    }

    #[test]
    fn descriptor_requires_display_platform_and_capability() {
        let id = ManagerId::parse("builtin:apt").expect("valid ID");
        let platforms = SupportedPlatforms::from([Platform::Linux]);
        let capabilities = ManagerCapabilities::from([ManagerCapability::Installed]);

        assert!(
            ManagerDescriptor::new(
                id.clone(),
                " ",
                ManagerCategory::System,
                platforms.clone(),
                capabilities.clone(),
            )
            .is_err()
        );
        assert!(
            ManagerDescriptor::new(
                id.clone(),
                "APT",
                ManagerCategory::System,
                SupportedPlatforms::default(),
                capabilities,
            )
            .is_err()
        );
        assert!(
            ManagerDescriptor::new(
                id,
                "APT",
                ManagerCategory::System,
                platforms,
                ManagerCapabilities::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn package_info_target_preserves_manager_owned_identity() {
        let manager = ManagerId::parse("builtin:winget").expect("valid manager ID");
        let mut package = PackageInfo::new(manager.clone(), "Contoso.App", "1.2.3");
        package.scope = PackageScope::User;
        package.origin = Some(
            PackageOrigin::new("winget").with_reference("Microsoft.Winget.Source_8wekyb3d8bbwe"),
        );

        assert_eq!(
            package.target(),
            PackageTarget {
                manager_id: manager,
                name: "Contoso.App".to_owned(),
                version: None,
                scope: PackageScope::User,
                origin: package.origin,
            }
        );
    }
}
