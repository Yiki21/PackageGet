use async_trait::async_trait;
use updater_manager_api::{
    ManagerConfig, ManagerError, ManagerErrorKind, PackageAction as ApiPackageAction,
    PackageInfo as ApiPackageInfo, PackageManager as ApiPackageManager,
    PackageUpdate as ApiPackageUpdate,
};
use updater_managers::AptManager as DirectAptManager;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    error::CoreError, pm::progress::CommandProgressEvent,
};

#[derive(Debug, Clone, Copy)]
pub struct AptManager;

#[async_trait]
impl PackageManager for AptManager {
    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        DirectAptManager::new()
            .current_version(&manager_config(config), package_name)
            .await
            .map_err(convert_manager_error)
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        DirectAptManager::new()
            .installed(&manager_config(config))
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_manager_error)
    }

    async fn count_installed(config: &Config) -> CoreResult<usize> {
        DirectAptManager::new()
            .count_installed(&manager_config(config))
            .await
            .map_err(convert_manager_error)
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        DirectAptManager::new()
            .search(&manager_config(config), package_name)
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_manager_error)
    }
}

impl AptManager {
    pub async fn list_updates_with_refresh(
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        DirectAptManager::new()
            .updates(&manager_config(config), refresh)
            .await
            .map(|updates| updates.into_iter().map(convert_package_update).collect())
            .map_err(convert_manager_error)
    }

    pub async fn uninstall_packages_with_progress(
        config: &Config,
        package_names: &[String],
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_packages_with_progress(
            config,
            ApiPackageAction::Uninstall,
            package_names,
            on_progress,
        )
        .await
    }

    pub async fn update_packages_with_progress(
        config: &Config,
        package_names: &[String],
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_packages_with_progress(config, ApiPackageAction::Update, package_names, on_progress)
            .await
    }

    pub async fn install_packages_with_progress(
        config: &Config,
        package_names: &[String],
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_packages_with_progress(
            config,
            ApiPackageAction::Install,
            package_names,
            on_progress,
        )
        .await
    }
}

async fn run_packages_with_progress(
    config: &Config,
    action: ApiPackageAction,
    package_names: &[String],
    mut on_progress: impl FnMut(CommandProgressEvent),
) -> CoreResult<()> {
    DirectAptManager::new()
        .execute_packages_with_progress(&manager_config(config), action, package_names, |event| {
            let (progress, command_message) = event.into_parts();
            on_progress(CommandProgressEvent {
                progress,
                command_message,
            });
        })
        .await
        .map_err(convert_manager_error)
}

fn manager_config(config: &Config) -> ManagerConfig {
    let manager_config = ManagerConfig::new(PackageManagerType::Apt.manager_id());
    if let Some(path) = config.get_package_path(PackageManagerType::Apt) {
        manager_config.with_executable(path)
    } else {
        manager_config
    }
}

fn convert_package_info(package: ApiPackageInfo) -> PackageInfo {
    PackageInfo {
        name: package.name,
        version: package.version,
        source: PackageManagerType::Apt,
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
        | ManagerErrorKind::RebootRequired => CoreError::CommandError(detail),
        ManagerErrorKind::Unsupported | ManagerErrorKind::Cancelled | ManagerErrorKind::Other => {
            CoreError::UnknownError(detail)
        }
        _ => CoreError::UnknownError(detail),
    }
}
