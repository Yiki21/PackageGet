use async_trait::async_trait;
use updater_manager_api::{
    ManagerConfig, ManagerError, ManagerErrorKind, PackageAction as ApiPackageAction,
    PackageInfo as ApiPackageInfo, PackageManager as ApiPackageManager, PackageTarget,
    PackageUpdate as ApiPackageUpdate,
};
use updater_managers::GoManager as DirectGoManager;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    error::CoreError, pm::progress::CommandProgressEvent,
};

#[derive(Debug, Clone, Copy)]
pub struct GoManager;

#[async_trait]
impl PackageManager for GoManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        Self::list_updates_with_refresh(config, false).await
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        DirectGoManager::new()
            .current_version(&manager_config(config), package_name)
            .await
            .map_err(convert_manager_error)
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        DirectGoManager::new()
            .installed(&manager_config(config))
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_manager_error)
    }

    async fn count_installed(config: &Config) -> CoreResult<usize> {
        DirectGoManager::new()
            .count_installed(&manager_config(config))
            .await
            .map_err(convert_manager_error)
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        DirectGoManager::new()
            .search(&manager_config(config), package_name)
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_manager_error)
    }
}

impl GoManager {
    pub async fn list_updates_with_refresh(
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        DirectGoManager::new()
            .updates(&manager_config(config), refresh)
            .await
            .map(|updates| updates.into_iter().map(convert_package_update).collect())
            .map_err(convert_manager_error)
    }

    pub async fn uninstall_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_package_with_progress(
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
        run_package_with_progress(config, ApiPackageAction::Update, package_name, on_progress).await
    }

    pub async fn install_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_package_with_progress(config, ApiPackageAction::Install, package_name, on_progress)
            .await
    }
}

async fn run_package_with_progress(
    config: &Config,
    action: ApiPackageAction,
    package_name: &str,
    mut on_progress: impl FnMut(CommandProgressEvent),
) -> CoreResult<()> {
    let target = PackageTarget::new(PackageManagerType::Go.manager_id(), package_name);
    DirectGoManager::new()
        .execute_target_with_progress(&manager_config(config), action, &target, |event| {
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
    let mut manager_config = ManagerConfig::new(PackageManagerType::Go.manager_id());
    if let Some(path) = config.get_package_path(PackageManagerType::Go) {
        manager_config = manager_config.with_executable(path);
    }
    if let Some(go_bin_dir) = &config.go_bin_dir {
        manager_config.settings = serde_json::json!({ "go_bin_dir": go_bin_dir });
    }
    manager_config
}

fn convert_package_info(package: ApiPackageInfo) -> PackageInfo {
    PackageInfo {
        name: package.name,
        version: package.version,
        source: PackageManagerType::Go,
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
    use crate::PackageManagerConfig;

    #[test]
    fn legacy_config_bridge_preserves_go_paths_and_bin_dir() {
        let config = Config {
            app_managers: vec![PackageManagerConfig {
                manager_type: PackageManagerType::Go,
                custom_path: Some("/custom/go".to_owned()),
            }],
            go_bin_dir: Some("/custom/gobin".to_owned()),
            ..Config::default()
        };
        let converted = manager_config(&config);
        assert_eq!(
            converted.executable(),
            Some(std::path::Path::new("/custom/go"))
        );
        assert_eq!(converted.settings["go_bin_dir"], "/custom/gobin");
    }

    #[test]
    fn legacy_model_bridge_preserves_go_metadata() {
        let id = PackageManagerType::Go.manager_id();
        let mut package = ApiPackageInfo::new(id.clone(), "stringer", "v0.30.0");
        package.size = Some(1024);
        let converted = convert_package_info(package);
        assert_eq!(converted.name, "stringer");
        assert_eq!(converted.source, PackageManagerType::Go);
        assert_eq!(converted.size, Some(1024));

        let converted = convert_package_update(ApiPackageUpdate::new(
            PackageTarget::new(id, "stringer"),
            "v0.30.0",
            "v0.31.0",
        ));
        assert_eq!(converted.new_version, "v0.31.0");
    }
}
