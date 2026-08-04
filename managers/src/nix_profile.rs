use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::timeout;
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId,
    ManagerResult, PackageAction, PackageInfo, PackageManager, PackageOrigin, PackageScope,
    PackageTarget, Platform, ProgressEvent, ProgressSink, SupportedPlatforms,
};

use crate::{
    command::{
        CommandSpec, manager_availability, resolve_executable, run_output, unsupported_platform,
    },
    progress::run_cancellable_command_with_progress,
};

const NIX_ID: &str = "builtin:nix-profile";
const NIX_COMMAND: &str = "nix";
const ORIGIN_NAME: &str = "Nix profile";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_ELEMENT_NAME_LENGTH: usize = 512;
const MAX_INSTALLABLE_LENGTH: usize = 2_048;

/// Direct manager for one explicitly configured current-user Nix profile.
#[derive(Debug, Clone)]
pub struct NixProfileManager {
    descriptor: ManagerDescriptor,
}

impl NixProfileManager {
    /// Creates the built-in Nix profile manager.
    #[must_use]
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(NIX_ID).expect("Nix profile manager ID must remain valid"),
            "Nix profile",
            ManagerCategory::Development,
            SupportedPlatforms::from([Platform::Linux, Platform::MacOs]),
            ManagerCapabilities::from([
                ManagerCapability::Installed,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ]),
        )
        .expect("Nix profile descriptor must remain valid")
        .with_description("One explicitly selected current-user Nix profile")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }

    fn settings(&self, config: &ManagerConfig) -> ManagerResult<NixSettings> {
        config.validate_for(self.descriptor())?;
        configured_nix_profile(config).map(|profile| NixSettings { profile })
    }

    async fn inventory(&self, config: &ManagerConfig) -> ManagerResult<(PathBuf, Vec<NixElement>)> {
        let settings = self.settings(config)?;
        let nix = resolve_executable(config, NIX_COMMAND);
        let spec = CommandSpec::new(nix).args([
            "profile".into(),
            "list".into(),
            "--json".into(),
            "--profile".into(),
            settings.profile.as_os_str().to_owned(),
        ]);
        let output = timeout(Duration::from_secs(90), run_output(&spec))
            .await
            .map_err(|_| {
                ManagerError::new(ManagerErrorKind::Timeout, "Nix profile listing timed out")
                    .with_detail(settings.profile.to_string_lossy())
            })??;
        if !output.status.success() {
            return Err(crate::command::command_status_error(
                &spec,
                output.status,
                &String::from_utf8_lossy(&output.stderr),
            ));
        }
        let elements = parse_manifest(&output.stdout, &settings.profile)?;
        Ok((settings.profile, elements))
    }

    async fn write_commands(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        targets: &[PackageTarget],
    ) -> ManagerResult<Vec<CommandSpec>> {
        let settings = self.settings(config)?;
        let nix = resolve_executable(config, NIX_COMMAND);
        let installed = if matches!(action, PackageAction::Update | PackageAction::Uninstall) {
            let (_, elements) = self.inventory(config).await?;
            elements
                .into_iter()
                .map(|element| (element.name.clone(), element))
                .collect::<HashMap<_, _>>()
        } else {
            HashMap::new()
        };

        targets
            .iter()
            .map(|target| {
                validate_target_header(self.descriptor.id(), target)?;
                let reference = NixReference::parse(target.origin.as_ref().ok_or_else(|| {
                    protocol(
                        "Nix profile target is missing its typed origin",
                        &target.name,
                    )
                })?)?;
                if reference.profile() != settings.profile {
                    return Err(protocol(
                        "Nix profile target belongs to another profile",
                        &target.name,
                    ));
                }

                let mut spec = CommandSpec::new(&nix);
                match action {
                    PackageAction::Install => {
                        if target.version.is_some() {
                            return Err(protocol(
                                "Nix install version must be part of the installable",
                                &target.name,
                            ));
                        }
                        let installable = reference.installable(&target.name)?;
                        spec = spec.args([
                            "profile".into(),
                            "install".into(),
                            installable.into(),
                            "--profile".into(),
                            settings.profile.as_os_str().to_owned(),
                        ]);
                    }
                    PackageAction::Update | PackageAction::Uninstall => {
                        if target.version.is_some() {
                            return Err(protocol(
                                "Nix installed target must not override its version",
                                &target.name,
                            ));
                        }
                        let current = installed.get(&target.name).ok_or_else(|| {
                            protocol("Nix profile target is no longer installed", &target.name)
                        })?;
                        let expected = current.reference(&settings.profile);
                        if reference != expected {
                            return Err(protocol(
                                "Nix profile target origin is stale or forged",
                                &target.name,
                            ));
                        }
                        if action == PackageAction::Update && !current.is_updatable() {
                            return Err(protocol(
                                "Nix profile element has no unlocked flake source",
                                &target.name,
                            ));
                        }
                        let operation = if action == PackageAction::Update {
                            "upgrade"
                        } else {
                            "remove"
                        };
                        spec = spec.args([
                            "profile".into(),
                            operation.into(),
                            target.name.clone().into(),
                            "--profile".into(),
                            settings.profile.as_os_str().to_owned(),
                        ]);
                    }
                    _ => {
                        return Err(ManagerError::unsupported(action.capability()));
                    }
                }
                Ok(spec)
            })
            .collect()
    }
}

impl Default for NixProfileManager {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PackageManager for NixProfileManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    fn validate_config(&self, config: &ManagerConfig) -> ManagerResult<()> {
        self.settings(config).map(|_| ())
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        if let Some(availability) = unsupported_platform(self.descriptor()) {
            return Ok(availability);
        }
        self.settings(config)?;
        Ok(manager_availability(config, NIX_COMMAND, &["--version"]).await)
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<PackageInfo>> {
        let (profile, elements) = self.inventory(config).await?;
        Ok(elements
            .into_iter()
            .map(|element| element.info(self.descriptor.id(), &profile))
            .collect())
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        Ok(self.inventory(config).await?.1.len())
    }

    async fn execute(
        &self,
        config: &ManagerConfig,
        action: PackageAction,
        packages: &[PackageTarget],
        progress: &dyn ProgressSink,
    ) -> ManagerResult<()> {
        let commands = self.write_commands(config, action, packages).await?;
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
                    "Nix profile write command timed out",
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NixSettings {
    profile: PathBuf,
}

/// Returns the configured current-user Nix profile.
///
/// # Errors
///
/// Returns a protocol error when the configuration ID, settings schema, or
/// profile path is invalid.
pub fn configured_nix_profile(config: &ManagerConfig) -> ManagerResult<PathBuf> {
    if config.id.as_str() != NIX_ID {
        return Err(protocol(
            "Nix profile configuration ID does not match the manager",
            config.id.as_str(),
        ));
    }
    let settings: NixSettings = serde_json::from_value(config.settings.clone())
        .map_err(|error| protocol("Nix profile settings are invalid", &error.to_string()))?;
    validate_profile_path(&settings.profile)?;
    Ok(settings.profile)
}

/// Sets the configured current-user Nix profile.
///
/// # Errors
///
/// Returns a protocol error when the configuration does not belong to the Nix
/// profile manager, its settings are malformed, or `profile` is outside the
/// supported current-user profile scope.
pub fn set_configured_nix_profile(
    config: &mut ManagerConfig,
    profile: PathBuf,
) -> ManagerResult<()> {
    if config.id.as_str() != NIX_ID {
        return Err(protocol(
            "Nix profile configuration ID does not match the manager",
            config.id.as_str(),
        ));
    }
    validate_profile_path(&profile)?;
    let settings = config.settings.as_object_mut().ok_or_else(|| {
        protocol(
            "Nix profile settings must be a JSON object",
            config.id.as_str(),
        )
    })?;
    let value = serde_json::to_value(profile).map_err(|error| {
        protocol(
            "Nix profile path could not be serialized",
            &error.to_string(),
        )
    })?;
    settings.insert("profile".to_owned(), value);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileManifest {
    version: u8,
    elements: ProfileElements,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ProfileElements {
    Named(BTreeMap<String, ManifestElement>),
    Indexed(Vec<ManifestElement>),
}

#[derive(Debug, Deserialize)]
struct ManifestElement {
    active: bool,
    #[serde(rename = "storePaths")]
    store_paths: Vec<String>,
    #[serde(default)]
    priority: Option<i64>,
    #[serde(rename = "originalUrl", alias = "originalUri", default)]
    original_url: Option<String>,
    #[serde(rename = "url", alias = "uri", default)]
    locked_url: Option<String>,
    #[serde(rename = "attrPath", default)]
    attr_path: Option<String>,
    #[serde(default)]
    outputs: Value,
    #[serde(flatten)]
    _other: HashMap<String, Value>,
}

#[derive(Debug)]
struct NixElement {
    name: String,
    version: String,
    store_paths: Vec<String>,
    priority: Option<i64>,
    original_url: Option<String>,
    locked_url: Option<String>,
    attr_path: Option<String>,
    outputs: Value,
}

impl NixElement {
    fn reference(&self, profile: &Path) -> NixReference {
        NixReference::Installed {
            profile: profile.to_path_buf(),
            element: self.name.clone(),
            original_url: self.original_url.clone(),
            locked_url: self.locked_url.clone(),
            attr_path: self.attr_path.clone(),
            outputs: self.outputs.clone(),
            store_paths: self.store_paths.clone(),
            priority: self.priority,
        }
    }

    fn info(self, manager_id: &ManagerId, profile: &Path) -> PackageInfo {
        let description = self.original_url.as_ref().map(|source| {
            self.attr_path
                .as_ref()
                .map_or_else(|| source.clone(), |attr| format!("{source}#{attr}"))
        });
        let origin = self.reference(profile).origin();
        let mut info = PackageInfo::new(manager_id.clone(), self.name, self.version);
        info.description = description;
        info.scope = PackageScope::User;
        info.origin = Some(origin);
        info
    }

    fn is_updatable(&self) -> bool {
        self.original_url
            .as_deref()
            .zip(self.locked_url.as_deref())
            .is_some_and(|(original, locked)| is_unlocked_flake_reference(original, locked))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum NixReference {
    Installed {
        profile: PathBuf,
        element: String,
        original_url: Option<String>,
        locked_url: Option<String>,
        attr_path: Option<String>,
        outputs: Value,
        store_paths: Vec<String>,
        priority: Option<i64>,
    },
    Installable {
        profile: PathBuf,
        installable: String,
    },
}

impl NixReference {
    fn parse(origin: &PackageOrigin) -> ManagerResult<Self> {
        if origin.name != ORIGIN_NAME {
            return Err(protocol(
                "Nix profile target has an invalid origin type",
                &origin.name,
            ));
        }
        let reference = origin
            .reference
            .as_deref()
            .ok_or_else(|| protocol("Nix profile target origin has no reference", &origin.name))?;
        serde_json::from_str(reference)
            .map_err(|error| protocol("Nix profile target origin is malformed", &error.to_string()))
    }

    fn profile(&self) -> &Path {
        match self {
            Self::Installed { profile, .. } | Self::Installable { profile, .. } => profile,
        }
    }

    fn installable(&self, target_name: &str) -> ManagerResult<&str> {
        let Self::Installable { installable, .. } = self else {
            return Err(protocol(
                "Nix install target does not carry an installable origin",
                target_name,
            ));
        };
        validate_installable(installable)?;
        if installable != target_name {
            return Err(protocol(
                "Nix install target name does not match its installable",
                target_name,
            ));
        }
        Ok(installable)
    }

    fn origin(&self) -> PackageOrigin {
        PackageOrigin::new(ORIGIN_NAME).with_reference(
            serde_json::to_string(self)
                .expect("Nix reference serialization must remain infallible"),
        )
    }
}

fn parse_manifest(value: &[u8], profile: &Path) -> ManagerResult<Vec<NixElement>> {
    let manifest: ProfileManifest = serde_json::from_slice(value)
        .map_err(|error| protocol("Nix profile list response is malformed", &error.to_string()))?;
    if !(1..=3).contains(&manifest.version) {
        return Err(protocol(
            "Nix profile manifest version is unsupported",
            &manifest.version.to_string(),
        ));
    }
    let entries: Vec<(String, ManifestElement)> = match manifest.elements {
        ProfileElements::Named(elements) => elements.into_iter().collect(),
        ProfileElements::Indexed(elements) => elements
            .into_iter()
            .enumerate()
            .map(|(index, element)| (index.to_string(), element))
            .collect(),
    };
    let mut names = BTreeSet::new();
    entries
        .into_iter()
        .filter(|(_, element)| element.active)
        .map(|(name, element)| {
            validate_element_name(&name)?;
            if !names.insert(name.clone()) {
                return Err(protocol(
                    "Nix profile contains duplicate element names",
                    &name,
                ));
            }
            if element.store_paths.is_empty()
                || element
                    .store_paths
                    .iter()
                    .any(|path| !valid_store_path(path))
            {
                return Err(protocol(
                    "Nix profile element has invalid store paths",
                    &name,
                ));
            }
            let source_fields = [
                element.original_url.is_some(),
                element.locked_url.is_some(),
                element.attr_path.is_some(),
            ];
            if source_fields.iter().any(|present| *present)
                && !source_fields.iter().all(|present| *present)
            {
                return Err(protocol(
                    "Nix profile element has partial flake identity",
                    &name,
                ));
            }
            for value in [
                element.original_url.as_deref(),
                element.locked_url.as_deref(),
                element.attr_path.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                validate_identity_text(value, "Nix profile element identity is invalid")?;
            }
            let version = versions_from_store_paths(&element.store_paths);
            Ok(NixElement {
                name,
                version,
                store_paths: element.store_paths,
                priority: element.priority,
                original_url: element.original_url,
                locked_url: element.locked_url,
                attr_path: element.attr_path,
                outputs: element.outputs,
            })
        })
        .collect::<ManagerResult<Vec<_>>>()
        .map_err(|error| {
            let detail = error.detail().unwrap_or(error.message()).to_owned();
            error.with_detail(format!("{}: {}", profile.display(), detail))
        })
}

fn versions_from_store_paths(paths: &[String]) -> String {
    let versions = paths
        .iter()
        .filter_map(|path| version_from_store_path(path))
        .collect::<BTreeSet<_>>();
    if versions.is_empty() {
        "Unknown".to_owned()
    } else {
        versions.into_iter().collect::<Vec<_>>().join(", ")
    }
}

fn version_from_store_path(path: &str) -> Option<String> {
    let name = Path::new(path).file_name()?.to_str()?;
    let package = name.get(33..)?;
    package.match_indices('-').find_map(|(index, _)| {
        let candidate = package.get(index + 1..)?;
        candidate
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
            .then(|| candidate.to_owned())
    })
}

fn validate_profile_path(profile: &Path) -> ManagerResult<()> {
    let Some(profile_text) = profile.to_str() else {
        return Err(protocol(
            "Nix profile path must be valid UTF-8",
            &profile.to_string_lossy(),
        ));
    };
    if !profile.is_absolute()
        || profile_text.trim() != profile_text
        || profile_text.is_empty()
        || profile_text.chars().any(char::is_control)
    {
        return Err(protocol("Nix profile path must be absolute", profile_text));
    }
    let normalized = profile_text.replace('\\', "/");
    if normalized == "/nix/var/nix/profiles/system"
        || normalized.starts_with("/nix/var/nix/profiles/system-")
        || normalized == "/nix/var/nix/profiles/default"
    {
        return Err(protocol(
            "Nix system profiles are outside this manager's scope",
            &normalized,
        ));
    }
    Ok(())
}

fn validate_target_header(manager_id: &ManagerId, target: &PackageTarget) -> ManagerResult<()> {
    if &target.manager_id != manager_id {
        return Err(protocol(
            "Nix profile target belongs to another manager",
            &target.name,
        ));
    }
    if target.scope != PackageScope::User {
        return Err(protocol(
            "Nix profile target scope must be user",
            &target.name,
        ));
    }
    validate_element_name(&target.name)
}

fn validate_element_name(name: &str) -> ManagerResult<()> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > MAX_ELEMENT_NAME_LENGTH
        || name.starts_with('-')
        || name.chars().any(char::is_control)
    {
        return Err(protocol("Nix profile element name is invalid", name));
    }
    Ok(())
}

fn validate_installable(installable: &str) -> ManagerResult<()> {
    let value = installable.trim();
    if value.is_empty()
        || value != installable
        || value.len() > MAX_INSTALLABLE_LENGTH
        || value.starts_with('-')
        || value.chars().any(char::is_control)
    {
        return Err(protocol("Nix installable is invalid", installable));
    }
    Ok(())
}

fn validate_identity_text(value: &str, message: &str) -> ManagerResult<()> {
    if value.trim().is_empty()
        || value.len() > MAX_INSTALLABLE_LENGTH
        || value.chars().any(char::is_control)
    {
        return Err(protocol(message, value));
    }
    Ok(())
}

fn valid_store_path(value: &str) -> bool {
    value.starts_with("/nix/store/")
        && value.len() > "/nix/store/".len() + 32
        && !value.chars().any(char::is_control)
}

fn is_unlocked_flake_reference(original: &str, locked: &str) -> bool {
    if original == locked || original.contains("?rev=") || original.contains("&rev=") {
        return false;
    }
    let without_query = original.split('?').next().unwrap_or(original);
    let last_segment = without_query.rsplit('/').next().unwrap_or_default();
    !(last_segment.len() >= 32
        && last_segment
            .chars()
            .all(|character| character.is_ascii_hexdigit()))
}

fn protocol(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = "/home/test/.local/state/nix/profiles/profile";

    #[test]
    fn parses_current_manifest_and_preserves_flake_identity() {
        let elements = parse_manifest(
            br#"{"version":3,"elements":{"hello":{"active":true,"priority":5,"storePaths":["/nix/store/0123456789abcdfghijklmnpqrsvwxyz-hello-2.12.2"],"originalUrl":"flake:nixpkgs","url":"github:NixOS/nixpkgs/7b38b03d76ab71bdc8dc325e3f6338d984cc35ca","attrPath":"legacyPackages.x86_64-linux.hello","outputs":["out"]},"disabled":{"active":false,"storePaths":["/nix/store/0123456789abcdfghijklmnpqrsvwxyz-disabled-1.0"]}}}"#,
            Path::new(PROFILE),
        )
        .expect("parse Nix v3 manifest");
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].name, "hello");
        assert_eq!(elements[0].version, "2.12.2");
        assert!(elements[0].is_updatable());
        let NixReference::Installed {
            original_url,
            locked_url,
            attr_path,
            ..
        } = elements[0].reference(Path::new(PROFILE))
        else {
            panic!("expected installed Nix reference");
        };
        assert_eq!(original_url.as_deref(), Some("flake:nixpkgs"));
        assert!(locked_url.unwrap().contains("7b38b03d"));
        assert_eq!(
            attr_path.as_deref(),
            Some("legacyPackages.x86_64-linux.hello")
        );
    }

    #[test]
    fn parses_legacy_manifest_keys_and_rejects_partial_sources() {
        let legacy = parse_manifest(
            br#"{"version":1,"elements":[{"active":true,"storePaths":["/nix/store/0123456789abcdfghijklmnpqrsvwxyz-ripgrep-14.1.1"],"originalUri":"flake:nixpkgs","uri":"github:NixOS/nixpkgs/0123456789abcdef0123456789abcdef01234567","attrPath":"legacyPackages.x86_64-linux.ripgrep"}]}"#,
            Path::new(PROFILE),
        )
        .expect("parse Nix v1 manifest");
        assert_eq!(legacy[0].name, "0");
        assert_eq!(legacy[0].version, "14.1.1");

        let error = parse_manifest(
            br#"{"version":3,"elements":{"bad":{"active":true,"storePaths":["/nix/store/0123456789abcdfghijklmnpqrsvwxyz-bad-1.0"],"originalUrl":"flake:nixpkgs"}}}"#,
            Path::new(PROFILE),
        )
        .expect_err("partial flake identity must fail");
        assert_eq!(error.kind(), ManagerErrorKind::Protocol);
    }

    #[test]
    fn distinguishes_unlocked_and_locked_flake_references() {
        assert!(is_unlocked_flake_reference(
            "github:NixOS/nixpkgs/nixos-unstable",
            "github:NixOS/nixpkgs/0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!is_unlocked_flake_reference(
            "github:NixOS/nixpkgs/0123456789abcdef0123456789abcdef01234567",
            "github:NixOS/nixpkgs/0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!is_unlocked_flake_reference(
            "git+https://example.test/repo?rev=0123456789abcdef0123456789abcdef01234567",
            "git+https://example.test/repo?rev=0123456789abcdef0123456789abcdef01234567"
        ));
    }
}
