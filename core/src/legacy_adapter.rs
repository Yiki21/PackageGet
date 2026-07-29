use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde::Deserialize;
use updater_manager_api::{
    AvailabilityReason, ManagerAvailability, ManagerConfig, ManagerDescriptor, ManagerError,
    ManagerErrorKind, ManagerResult, PackageAction as ApiPackageAction,
    PackageInfo as ApiPackageInfo, PackageManager as ApiPackageManager, PackageScope,
    PackageTarget, PackageUpdate as ApiPackageUpdate, Platform, ProgressEvent, ProgressSink,
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
