use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde::Deserialize;
use updater_manager_api::{
    AvailabilityReason, ManagerAvailability, ManagerConfig, ManagerDescriptor, ManagerError,
    ManagerErrorKind, ManagerResult, PackageAction as ApiPackageAction,
    PackageInfo as ApiPackageInfo, PackageManager as ApiPackageManager, PackageScope,
    PackageTarget, PackageUpdate as ApiPackageUpdate, Platform, ProgressEvent, ProgressSink,
};
use updater_managers::{
    AptManager as DirectAptManager, DnfManager as DirectDnfManager,
    PacmanManager as DirectPacmanManager, ZypperManager as DirectZypperManager,
};

use crate::{
    ALL_PACKAGE_MANAGERS, Config, InstallProgress, ManagerRegistry, PackageInfo,
    PackageManagerAvailability, PackageManagerConfig, PackageManagerType, PackageUpdate,
    RegistryError, error::CoreError,
};

/// Temporary object-safe wrapper around the legacy built-in manager dispatcher.
///
/// This adapter keeps the current commands and Config V1 behavior intact while
/// callers migrate to `updater-manager-api`. It can be removed after the
/// concrete managers move behind the new trait directly.
#[derive(Debug, Clone)]
pub struct LegacyPackageManagerAdapter {
    manager_type: PackageManagerType,
    descriptor: ManagerDescriptor,
}

impl LegacyPackageManagerAdapter {
    /// Creates an adapter for one built-in manager.
    #[must_use]
    pub fn new(manager_type: PackageManagerType) -> Self {
        Self {
            manager_type,
            descriptor: manager_type.manager_descriptor(),
        }
    }

    /// Returns the wrapped legacy manager identity.
    #[must_use]
    pub fn manager_type(&self) -> PackageManagerType {
        self.manager_type
    }

    fn legacy_config(&self, config: &ManagerConfig) -> ManagerResult<Config> {
        if config.id != *self.descriptor.id() {
            return Err(ManagerError::new(
                ManagerErrorKind::Protocol,
                "manager configuration ID does not match the adapter",
            )
            .with_detail(format!(
                "expected {}, received {}",
                self.descriptor.id(),
                config.id
            )));
        }

        let custom_path = config
            .executable()
            .map(|path| {
                path.to_str().map(str::to_owned).ok_or_else(|| {
                    ManagerError::new(
                        ManagerErrorKind::Protocol,
                        "manager executable path is not valid UTF-8",
                    )
                    .with_detail(path.to_string_lossy())
                })
            })
            .transpose()?;

        let package_config = PackageManagerConfig {
            manager_type: self.manager_type,
            custom_path,
        };
        let (system_manager, app_managers) = if self.manager_type.is_system_manager() {
            (Some(package_config), Vec::new())
        } else {
            (None, vec![package_config])
        };

        let go_bin_dir = if self.manager_type == PackageManagerType::Go {
            serde_json::from_value::<GoManagerSettings>(config.settings.clone())
                .map_err(|error| {
                    ManagerError::new(
                        ManagerErrorKind::Protocol,
                        "Go manager settings are invalid",
                    )
                    .with_detail(error.to_string())
                })?
                .go_bin_dir
        } else {
            None
        };

        Ok(Config {
            system_manager,
            app_managers,
            go_bin_dir,
            ..Config::default()
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct GoManagerSettings {
    #[serde(default)]
    go_bin_dir: Option<String>,
}

#[async_trait]
impl ApiPackageManager for LegacyPackageManagerAdapter {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        let legacy_config = self.legacy_config(config)?;

        if let Some(platform) = current_platform()
            && !self.descriptor.platforms().contains(platform)
        {
            return Ok(ManagerAvailability::Unavailable {
                reason: AvailabilityReason::UnsupportedPlatform {
                    platform: Some(platform),
                },
            });
        }

        Ok(convert_availability(
            self.manager_type
                .availability_with_config(&legacy_config)
                .await,
        ))
    }

    async fn installed(&self, config: &ManagerConfig) -> ManagerResult<Vec<ApiPackageInfo>> {
        let legacy_config = self.legacy_config(config)?;
        self.manager_type
            .list_installed(&legacy_config)
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_core_error)
    }

    async fn count_installed(&self, config: &ManagerConfig) -> ManagerResult<usize> {
        let legacy_config = self.legacy_config(config)?;
        self.manager_type
            .count_installed(&legacy_config)
            .await
            .map_err(convert_core_error)
    }

    async fn updates(
        &self,
        config: &ManagerConfig,
        refresh: bool,
    ) -> ManagerResult<Vec<ApiPackageUpdate>> {
        let legacy_config = self.legacy_config(config)?;
        self.manager_type
            .list_updates_with_refresh(&legacy_config, refresh)
            .await
            .map(|updates| {
                updates
                    .into_iter()
                    .map(|update| convert_package_update(self.manager_type, update))
                    .collect()
            })
            .map_err(convert_core_error)
    }

    async fn search(
        &self,
        config: &ManagerConfig,
        query: &str,
    ) -> ManagerResult<Vec<ApiPackageInfo>> {
        let legacy_config = self.legacy_config(config)?;
        self.manager_type
            .search_package(&legacy_config, query)
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_core_error)
    }

    async fn execute(
        &self,
        config: &ManagerConfig,
        action: ApiPackageAction,
        packages: &[PackageTarget],
        progress: &dyn ProgressSink,
    ) -> ManagerResult<()> {
        let legacy_config = self.legacy_config(config)?;
        let package_names = packages
            .iter()
            .map(|package| {
                if package.manager_id != *self.descriptor.id() {
                    return Err(ManagerError::new(
                        ManagerErrorKind::Protocol,
                        "package target belongs to a different manager",
                    )
                    .with_detail(format!(
                        "expected {}, received {} for package {}",
                        self.descriptor.id(),
                        package.manager_id,
                        package.name
                    )));
                }

                Ok(package.name.clone())
            })
            .collect::<ManagerResult<Vec<_>>>()?;

        ensure_supported_action(action)?;
        let total = package_names.len();
        progress.emit(ProgressEvent::Started { action, total });

        let result = match action {
            ApiPackageAction::Install => {
                self.manager_type
                    .install_packages_with_progress(&legacy_config, &package_names, |event| {
                        emit_progress(progress, event);
                    })
                    .await
            }
            ApiPackageAction::Update => {
                self.manager_type
                    .update_packages_with_progress(&legacy_config, &package_names, |event| {
                        emit_progress(progress, event);
                    })
                    .await
            }
            ApiPackageAction::Uninstall => {
                self.manager_type
                    .uninstall_packages_with_progress(&legacy_config, &package_names, |event| {
                        emit_progress(progress, event);
                    })
                    .await
            }
            _ => return Err(unsupported_action_error()),
        };

        result.map_err(convert_core_error)?;
        progress.emit(ProgressEvent::Finished {
            completed: total,
            total,
        });
        Ok(())
    }
}

/// Registers adapters for every existing built-in manager.
///
/// # Errors
///
/// Returns [`RegistryError::DuplicateManager`] if the registry already
/// contains one of the stable built-in IDs.
pub fn register_legacy_managers(registry: &mut ManagerRegistry) -> Result<(), RegistryError> {
    for manager_type in ALL_PACKAGE_MANAGERS {
        registry.register(Arc::new(LegacyPackageManagerAdapter::new(*manager_type)))?;
    }

    Ok(())
}

/// Registers the current mixed set of direct and legacy built-in managers.
///
/// APT, DNF, Pacman, and Zypper are registered through their direct
/// `updater-managers` implementations. Managers that have not migrated yet
/// remain wrapped by [`LegacyPackageManagerAdapter`].
///
/// # Errors
///
/// Returns [`RegistryError::DuplicateManager`] if the registry already
/// contains any stable built-in ID.
pub fn register_builtin_managers(registry: &mut ManagerRegistry) -> Result<(), RegistryError> {
    registry.register(Arc::new(DirectAptManager::new()))?;
    registry.register(Arc::new(DirectDnfManager::new()))?;
    registry.register(Arc::new(DirectPacmanManager::new()))?;
    registry.register(Arc::new(DirectZypperManager::new()))?;

    for manager_type in ALL_PACKAGE_MANAGERS {
        if !matches!(
            manager_type,
            PackageManagerType::Apt
                | PackageManagerType::Dnf
                | PackageManagerType::Pacman
                | PackageManagerType::Zypper
        ) {
            registry.register(Arc::new(LegacyPackageManagerAdapter::new(*manager_type)))?;
        }
    }

    Ok(())
}

fn current_platform() -> Option<Platform> {
    if cfg!(target_os = "linux") {
        Some(Platform::Linux)
    } else if cfg!(target_os = "windows") {
        Some(Platform::Windows)
    } else if cfg!(target_os = "macos") {
        Some(Platform::MacOs)
    } else {
        None
    }
}

fn convert_availability(availability: PackageManagerAvailability) -> ManagerAvailability {
    match availability {
        PackageManagerAvailability::Available => ManagerAvailability::Available { version: None },
        PackageManagerAvailability::NotFound { command } => ManagerAvailability::Unavailable {
            reason: AvailabilityReason::CommandMissing { command },
        },
        PackageManagerAvailability::NotExecutable { path } => ManagerAvailability::Unavailable {
            reason: AvailabilityReason::NotExecutable {
                path: PathBuf::from(path),
            },
        },
        PackageManagerAvailability::VersionCheckFailed { command, detail } => {
            ManagerAvailability::Unavailable {
                reason: AvailabilityReason::VersionCheckFailed {
                    detail: format!("{command}: {detail}"),
                },
            }
        }
    }
}

fn convert_package_info(package: PackageInfo) -> ApiPackageInfo {
    let mut converted =
        ApiPackageInfo::new(package.source.manager_id(), package.name, package.version);
    converted.description = package.description;
    converted.homepage = package.homepage;
    converted.size = package.size;
    converted.install_date = package.install_date;
    converted.scope = PackageScope::Unknown;
    converted
}

fn convert_package_update(
    manager_type: PackageManagerType,
    update: PackageUpdate,
) -> ApiPackageUpdate {
    ApiPackageUpdate::new(
        PackageTarget::new(manager_type.manager_id(), update.name),
        update.current_version,
        update.new_version,
    )
}

fn emit_progress(progress: &dyn ProgressSink, event: InstallProgress) {
    if let Some(message) = event.command_message {
        progress.emit(ProgressEvent::Message { message });
    }

    progress.emit(ProgressEvent::Advanced {
        completed: event.completed,
        total: event.total,
        current_package: (!event.current_package.is_empty()).then_some(event.current_package),
    });
}

fn ensure_supported_action(action: ApiPackageAction) -> ManagerResult<()> {
    match action {
        ApiPackageAction::Install | ApiPackageAction::Update | ApiPackageAction::Uninstall => {
            Ok(())
        }
        _ => Err(unsupported_action_error()),
    }
}

fn unsupported_action_error() -> ManagerError {
    ManagerError::new(
        ManagerErrorKind::Unsupported,
        "package action is not supported by the legacy adapter",
    )
}

fn convert_core_error(error: CoreError) -> ManagerError {
    match error {
        CoreError::CommandError(detail) => {
            let kind = classify_command_error(&detail);
            let message = match kind {
                ManagerErrorKind::CommandMissing => "package manager command is unavailable",
                ManagerErrorKind::Permission => "permission or elevation was denied",
                ManagerErrorKind::Busy => "package manager is busy",
                ManagerErrorKind::Timeout => "package manager command timed out",
                ManagerErrorKind::RebootRequired => "installer requires a reboot",
                _ => "package manager command failed",
            };
            ManagerError::new(kind, message).with_detail(detail)
        }
        CoreError::Utf8Error(detail)
        | CoreError::ParseError(detail)
        | CoreError::SerializationError(detail) => ManagerError::new(
            ManagerErrorKind::Protocol,
            "package manager output could not be parsed",
        )
        .with_detail(detail),
        CoreError::RequestError(detail) => {
            ManagerError::new(ManagerErrorKind::Network, "network request failed")
                .with_detail(detail)
        }
        CoreError::UnknownError(detail) => {
            ManagerError::new(ManagerErrorKind::Other, "package manager operation failed")
                .with_detail(detail)
        }
    }
}

fn classify_command_error(detail: &str) -> ManagerErrorKind {
    let detail = detail.to_ascii_lowercase();

    if contains_any(
        &detail,
        &[
            "command not found",
            "executable file not found",
            "failed to spawn",
            "no such file or directory",
            "not found in path",
            "os error 2",
        ],
    ) {
        ManagerErrorKind::CommandMissing
    } else if contains_any(
        &detail,
        &[
            "permission denied",
            "not authorized",
            "authorization failed",
            "authentication failure",
            "no authentication agent",
            "polkit",
            "pkexec",
            "must be root",
            "requires root",
            "access is denied",
            "eacces",
        ],
    ) {
        ManagerErrorKind::Permission
    } else if contains_any(
        &detail,
        &[
            "could not get lock",
            "could not acquire lock",
            "unable to acquire",
            "unable to lock",
            "database is locked",
            "holding the yum lock",
            "holding the dnf lock",
            "system management is locked",
        ],
    ) {
        ManagerErrorKind::Busy
    } else if contains_any(&detail, &["timed out", "timeout", "deadline exceeded"]) {
        ManagerErrorKind::Timeout
    } else if contains_any(
        &detail,
        &[
            "reboot required",
            "restart required",
            "restart the computer",
        ],
    ) {
        ManagerErrorKind::RebootRequired
    } else {
        ManagerErrorKind::Other
    }
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;
    use updater_manager_api::{ManagerCapability, ManagerId};

    use super::*;

    #[test]
    fn bridges_system_manager_config_and_custom_executable() {
        let adapter = LegacyPackageManagerAdapter::new(PackageManagerType::Apt);
        let config =
            ManagerConfig::new(PackageManagerType::Apt.manager_id()).with_executable("custom-apt");

        let legacy = adapter.legacy_config(&config).expect("bridge APT config");

        assert_eq!(
            legacy.system_manager,
            Some(PackageManagerConfig {
                manager_type: PackageManagerType::Apt,
                custom_path: Some("custom-apt".to_owned()),
            })
        );
        assert!(legacy.app_managers.is_empty());
    }

    #[test]
    fn bridges_app_manager_and_typed_go_settings() {
        let adapter = LegacyPackageManagerAdapter::new(PackageManagerType::Go);
        let mut config = ManagerConfig::new(PackageManagerType::Go.manager_id());
        config.settings = json!({"go_bin_dir": "/tmp/updater-go-bin"});

        let legacy = adapter.legacy_config(&config).expect("bridge Go config");

        assert!(legacy.system_manager.is_none());
        assert_eq!(
            legacy.app_managers,
            vec![PackageManagerConfig {
                manager_type: PackageManagerType::Go,
                custom_path: None,
            }]
        );
        assert_eq!(legacy.go_bin_dir.as_deref(), Some("/tmp/updater-go-bin"));
    }

    #[test]
    fn rejects_mismatched_ids_and_invalid_go_settings() {
        let adapter = LegacyPackageManagerAdapter::new(PackageManagerType::Go);
        let wrong_id = ManagerConfig::new(PackageManagerType::Cargo.manager_id());
        let mismatch = adapter
            .legacy_config(&wrong_id)
            .expect_err("reject mismatched manager ID");
        assert_eq!(mismatch.kind(), ManagerErrorKind::Protocol);

        let mut invalid = ManagerConfig::new(PackageManagerType::Go.manager_id());
        invalid.settings = json!({"go_bin_dir": 42});
        let error = adapter
            .legacy_config(&invalid)
            .expect_err("reject invalid Go settings");
        assert_eq!(error.kind(), ManagerErrorKind::Protocol);
        assert_eq!(error.message(), "Go manager settings are invalid");
    }

    #[test]
    fn converts_legacy_package_and_update_metadata_without_loss() {
        let package = convert_package_info(PackageInfo {
            name: "ripgrep".to_owned(),
            version: "14.1.1".to_owned(),
            source: PackageManagerType::Cargo,
            description: Some("Fast recursive search".to_owned()),
            size: Some(12_345),
            install_date: Some("2026-07-29".to_owned()),
            homepage: Some("https://github.com/BurntSushi/ripgrep".to_owned()),
        });

        assert_eq!(package.manager_id, PackageManagerType::Cargo.manager_id());
        assert_eq!(package.name, "ripgrep");
        assert_eq!(package.version, "14.1.1");
        assert_eq!(
            package.description.as_deref(),
            Some("Fast recursive search")
        );
        assert_eq!(package.size, Some(12_345));
        assert_eq!(package.install_date.as_deref(), Some("2026-07-29"));
        assert_eq!(
            package.homepage.as_deref(),
            Some("https://github.com/BurntSushi/ripgrep")
        );
        assert_eq!(package.scope, PackageScope::Unknown);
        assert!(package.origin.is_none());

        let update = convert_package_update(
            PackageManagerType::Cargo,
            PackageUpdate {
                name: "ripgrep".to_owned(),
                current_version: "14.1.0".to_owned(),
                new_version: "14.1.1".to_owned(),
            },
        );
        assert_eq!(
            update.target.manager_id,
            PackageManagerType::Cargo.manager_id()
        );
        assert_eq!(update.target.name, "ripgrep");
        assert_eq!(update.current_version, "14.1.0");
        assert_eq!(update.available_version, "14.1.1");
    }

    #[test]
    fn converts_all_legacy_availability_states() {
        assert_eq!(
            convert_availability(PackageManagerAvailability::Available),
            ManagerAvailability::Available { version: None }
        );
        assert_eq!(
            convert_availability(PackageManagerAvailability::NotFound {
                command: "missing-manager".to_owned(),
            }),
            ManagerAvailability::Unavailable {
                reason: AvailabilityReason::CommandMissing {
                    command: "missing-manager".to_owned(),
                },
            }
        );
        assert_eq!(
            convert_availability(PackageManagerAvailability::NotExecutable {
                path: "/tmp/manager".to_owned(),
            }),
            ManagerAvailability::Unavailable {
                reason: AvailabilityReason::NotExecutable {
                    path: PathBuf::from("/tmp/manager"),
                },
            }
        );
        assert_eq!(
            convert_availability(PackageManagerAvailability::VersionCheckFailed {
                command: "manager".to_owned(),
                detail: "exit status 1".to_owned(),
            }),
            ManagerAvailability::Unavailable {
                reason: AvailabilityReason::VersionCheckFailed {
                    detail: "manager: exit status 1".to_owned(),
                },
            }
        );
    }

    #[test]
    fn classifies_legacy_errors_and_preserves_diagnostic_detail() {
        let cases = [
            (
                CoreError::CommandError("command not found".to_owned()),
                ManagerErrorKind::CommandMissing,
            ),
            (
                CoreError::CommandError("pkexec: not authorized".to_owned()),
                ManagerErrorKind::Permission,
            ),
            (
                CoreError::CommandError("could not get lock".to_owned()),
                ManagerErrorKind::Busy,
            ),
            (
                CoreError::CommandError("operation timed out".to_owned()),
                ManagerErrorKind::Timeout,
            ),
            (
                CoreError::CommandError("reboot required".to_owned()),
                ManagerErrorKind::RebootRequired,
            ),
            (
                CoreError::ParseError("unexpected column".to_owned()),
                ManagerErrorKind::Protocol,
            ),
            (
                CoreError::RequestError("offline".to_owned()),
                ManagerErrorKind::Network,
            ),
            (
                CoreError::UnknownError("unexpected".to_owned()),
                ManagerErrorKind::Other,
            ),
        ];

        for (legacy, expected_kind) in cases {
            let expected_detail = match &legacy {
                CoreError::CommandError(detail)
                | CoreError::Utf8Error(detail)
                | CoreError::ParseError(detail)
                | CoreError::UnknownError(detail)
                | CoreError::SerializationError(detail)
                | CoreError::RequestError(detail) => detail.clone(),
            };
            let converted = convert_core_error(legacy);
            assert_eq!(converted.kind(), expected_kind);
            assert_eq!(converted.detail(), Some(expected_detail.as_str()));
        }
    }

    #[test]
    fn maps_legacy_progress_to_message_and_advanced_events() {
        let events = Mutex::new(Vec::new());
        let sink = |event| events.lock().expect("progress lock").push(event);

        emit_progress(
            &sink,
            InstallProgress {
                manager: PackageManagerType::Cargo,
                current_package: "ripgrep".to_owned(),
                completed: 1,
                total: 2,
                command_message: Some("downloading crate".to_owned()),
            },
        );

        assert_eq!(
            *events.lock().expect("progress lock"),
            vec![
                ProgressEvent::Message {
                    message: "downloading crate".to_owned(),
                },
                ProgressEvent::Advanced {
                    completed: 1,
                    total: 2,
                    current_package: Some("ripgrep".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn registers_every_legacy_manager_with_all_current_capabilities() {
        let mut registry = ManagerRegistry::new();
        register_legacy_managers(&mut registry).expect("register legacy managers");

        assert_eq!(registry.len(), ALL_PACKAGE_MANAGERS.len());
        for manager_type in ALL_PACKAGE_MANAGERS {
            let manager = registry
                .get(&manager_type.manager_id())
                .expect("registered built-in manager");
            assert_eq!(manager.descriptor().id(), &manager_type.manager_id());
            for capability in [
                ManagerCapability::Installed,
                ManagerCapability::Updates,
                ManagerCapability::Search,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ] {
                assert!(manager.descriptor().capabilities().contains(capability));
            }
        }
    }

    #[test]
    fn duplicate_legacy_registration_returns_the_stable_id() {
        let mut registry = ManagerRegistry::new();
        registry
            .register(Arc::new(LegacyPackageManagerAdapter::new(
                PackageManagerType::Apt,
            )))
            .expect("register initial APT adapter");

        assert!(matches!(
            register_legacy_managers(&mut registry),
            Err(RegistryError::DuplicateManager { id })
                if id == PackageManagerType::Apt.manager_id()
        ));
    }

    #[tokio::test]
    async fn empty_execute_emits_started_and_finished_without_running_a_command() {
        let adapter = LegacyPackageManagerAdapter::new(PackageManagerType::Cargo);
        let config = ManagerConfig::new(PackageManagerType::Cargo.manager_id());
        let events = Mutex::new(Vec::new());
        let sink = |event| events.lock().expect("progress lock").push(event);

        adapter
            .execute(&config, ApiPackageAction::Install, &[], &sink)
            .await
            .expect("execute empty group");

        assert_eq!(
            *events.lock().expect("progress lock"),
            vec![
                ProgressEvent::Started {
                    action: ApiPackageAction::Install,
                    total: 0,
                },
                ProgressEvent::Finished {
                    completed: 0,
                    total: 0,
                },
            ]
        );
    }

    #[tokio::test]
    async fn execute_rejects_targets_from_another_manager_before_progress() {
        let adapter = LegacyPackageManagerAdapter::new(PackageManagerType::Cargo);
        let config = ManagerConfig::new(PackageManagerType::Cargo.manager_id());
        let target = PackageTarget::new(
            ManagerId::parse("org.example:other").expect("valid external manager ID"),
            "ripgrep",
        );
        let events = Mutex::new(Vec::new());
        let sink = |event| events.lock().expect("progress lock").push(event);

        let error = adapter
            .execute(
                &config,
                ApiPackageAction::Install,
                std::slice::from_ref(&target),
                &sink,
            )
            .await
            .expect_err("reject mismatched package target");

        assert_eq!(error.kind(), ManagerErrorKind::Protocol);
        assert!(events.lock().expect("progress lock").is_empty());
    }
}
