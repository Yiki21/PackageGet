use async_trait::async_trait;
use updater_manager_api::{
    ManagerConfig, ManagerError, ManagerErrorKind, PackageAction as ApiPackageAction,
    PackageInfo as ApiPackageInfo, PackageManager as ApiPackageManager, PackageTarget,
    PackageUpdate as ApiPackageUpdate,
};
use updater_managers::{NpmManager as DirectNpmManager, PnpmManager as DirectPnpmManager};

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    error::CoreError, pm::progress::CommandProgressEvent,
};

#[derive(Debug, Clone, Copy)]
pub struct NpmManager;

#[derive(Debug, Clone, Copy)]
pub struct PnpmManager;

macro_rules! impl_js_manager_bridge {
    ($manager:ty, $direct:ty, $manager_type:expr) => {
        #[async_trait]
        impl PackageManager for $manager {
            async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
                Self::list_updates_with_refresh(config, false).await
            }

            async fn get_current_version(
                config: &Config,
                package_name: &str,
            ) -> CoreResult<String> {
                <$direct>::new()
                    .current_version(&manager_config(config, $manager_type), package_name)
                    .await
                    .map_err(convert_manager_error)
            }

            async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
                <$direct>::new()
                    .installed(&manager_config(config, $manager_type))
                    .await
                    .map(|packages| {
                        packages
                            .into_iter()
                            .map(|package| convert_package_info(package, $manager_type))
                            .collect()
                    })
                    .map_err(convert_manager_error)
            }

            async fn count_installed(config: &Config) -> CoreResult<usize> {
                <$direct>::new()
                    .count_installed(&manager_config(config, $manager_type))
                    .await
                    .map_err(convert_manager_error)
            }

            async fn search_package(
                config: &Config,
                package_name: &str,
            ) -> CoreResult<Vec<PackageInfo>> {
                <$direct>::new()
                    .search(&manager_config(config, $manager_type), package_name)
                    .await
                    .map(|packages| {
                        packages
                            .into_iter()
                            .map(|package| convert_package_info(package, $manager_type))
                            .collect()
                    })
                    .map_err(convert_manager_error)
            }
        }

        impl $manager {
            async fn run_package_with_progress(
                config: &Config,
                action: ApiPackageAction,
                package_name: &str,
                mut on_progress: impl FnMut(CommandProgressEvent),
            ) -> CoreResult<()> {
                let target = PackageTarget::new($manager_type.manager_id(), package_name);
                <$direct>::new()
                    .execute_target_with_progress(
                        &manager_config(config, $manager_type),
                        action,
                        &target,
                        |event| {
                            let (progress, command_message) = event.into_parts();
                            on_progress(CommandProgressEvent {
                                progress,
                                command_message,
                            });
                        },
                    )
                    .await
                    .map_err(convert_manager_error)
            }

            pub async fn list_updates_with_refresh(
                config: &Config,
                refresh: bool,
            ) -> CoreResult<Vec<PackageUpdate>> {
                <$direct>::new()
                    .updates(&manager_config(config, $manager_type), refresh)
                    .await
                    .map(|updates| updates.into_iter().map(convert_package_update).collect())
                    .map_err(convert_manager_error)
            }

            pub async fn uninstall_package_with_progress(
                config: &Config,
                package_name: &str,
                on_progress: impl FnMut(CommandProgressEvent),
            ) -> CoreResult<()> {
                Self::run_package_with_progress(
                    config,
                    ApiPackageAction::Uninstall,
                    package_name,
                    on_progress,
                )
                .await
            }

            pub async fn update_package_with_progress(
                config: &Config,
                package_name: &str,
                on_progress: impl FnMut(CommandProgressEvent),
            ) -> CoreResult<()> {
                Self::run_package_with_progress(
                    config,
                    ApiPackageAction::Update,
                    package_name,
                    on_progress,
                )
                .await
            }

            pub async fn install_package_with_progress(
                config: &Config,
                package_name: &str,
                on_progress: impl FnMut(CommandProgressEvent),
            ) -> CoreResult<()> {
                Self::run_package_with_progress(
                    config,
                    ApiPackageAction::Install,
                    package_name,
                    on_progress,
                )
                .await
            }
        }
    };
}

impl_js_manager_bridge!(NpmManager, DirectNpmManager, PackageManagerType::Npm);
impl_js_manager_bridge!(PnpmManager, DirectPnpmManager, PackageManagerType::Pnpm);

fn manager_config(config: &Config, manager_type: PackageManagerType) -> ManagerConfig {
    let id = manager_type.manager_id();
    config
        .manager(&id)
        .cloned()
        .unwrap_or_else(|| ManagerConfig::new(id))
}

fn convert_package_info(package: ApiPackageInfo, source: PackageManagerType) -> PackageInfo {
    PackageInfo {
        name: package.name,
        version: package.version,
        source,
        description: package.description,
        size: package.size,
        install_date: package.install_date,
        homepage: package.homepage,
    }
}

fn convert_package_update(update: ApiPackageUpdate) -> PackageUpdate {
    PackageUpdate {
        name: update.target.name,
        current_version: update.current_version,
        new_version: update.available_version,
    }
}

fn convert_manager_error(error: ManagerError) -> CoreError {
    let detail = error.detail().map_or_else(
        || error.message().to_owned(),
        |detail| format!("{}: {detail}", error.message()),
    );
    match error.kind() {
        ManagerErrorKind::Network => CoreError::RequestError(detail),
        ManagerErrorKind::Protocol => CoreError::ParseError(detail),
        ManagerErrorKind::CommandMissing
        | ManagerErrorKind::Permission
        | ManagerErrorKind::Busy
        | ManagerErrorKind::Timeout
        | ManagerErrorKind::RebootRequired
        | ManagerErrorKind::Cancelled => CoreError::from_command_failure(detail),
        ManagerErrorKind::Unsupported | ManagerErrorKind::Other => CoreError::UnknownError(detail),
        _ => CoreError::UnknownError(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_preserves_custom_js_manager_paths() {
        for (manager_type, path) in [
            (PackageManagerType::Npm, "/custom/npm"),
            (PackageManagerType::Pnpm, "/custom/pnpm"),
        ] {
            let config = Config {
                managers: vec![ManagerConfig::new(manager_type.manager_id()).with_executable(path)],
                ..Config::default()
            };
            assert_eq!(
                manager_config(&config, manager_type).executable(),
                Some(std::path::Path::new(path))
            );
        }
    }

    #[test]
    fn direct_model_conversion_preserves_js_manager_identity() {
        for manager_type in [PackageManagerType::Npm, PackageManagerType::Pnpm] {
            let id = manager_type.manager_id();
            let mut package = ApiPackageInfo::new(id.clone(), "typescript", "5.9.0");
            package.size = Some(1024);
            let converted = convert_package_info(package, manager_type);
            assert_eq!(converted.source, manager_type);
            assert_eq!(converted.size, Some(1024));

            let converted = convert_package_update(ApiPackageUpdate::new(
                PackageTarget::new(id, "typescript"),
                "5.9.0",
                "5.10.0",
            ));
            assert_eq!(converted.new_version, "5.10.0");
        }
    }
}
