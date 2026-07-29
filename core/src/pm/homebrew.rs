use async_trait::async_trait;
use updater_manager_api::{
    ManagerConfig, ManagerError, ManagerErrorKind, PackageAction as ApiPackageAction,
    PackageInfo as ApiPackageInfo, PackageManager as ApiPackageManager, PackageTarget,
    PackageUpdate as ApiPackageUpdate,
};
use updater_managers::HomebrewManager as DirectHomebrewManager;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    error::CoreError, pm::progress::CommandProgressEvent,
};

#[derive(Debug, Clone, Copy)]
pub struct HomebrewManager;

#[async_trait]
impl PackageManager for HomebrewManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        Self::list_updates_with_refresh(config, false).await
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        DirectHomebrewManager::new()
            .current_version(&manager_config(config), package_name)
            .await
            .map_err(convert_manager_error)
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        DirectHomebrewManager::new()
            .installed(&manager_config(config))
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_manager_error)
    }

    async fn count_installed(config: &Config) -> CoreResult<usize> {
        DirectHomebrewManager::new()
            .count_installed(&manager_config(config))
            .await
            .map_err(convert_manager_error)
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        DirectHomebrewManager::new()
            .search(&manager_config(config), package_name)
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_manager_error)
    }
}

impl HomebrewManager {
    pub async fn list_updates_with_refresh(
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        DirectHomebrewManager::new()
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
    let target = PackageTarget::new(PackageManagerType::Homebrew.manager_id(), package_name);
    DirectHomebrewManager::new()
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
    let manager_config = ManagerConfig::new(PackageManagerType::Homebrew.manager_id());
    if let Some(path) = config.get_package_path(PackageManagerType::Homebrew) {
        manager_config.with_executable(path)
    } else {
        manager_config
    }
}

fn convert_package_info(package: ApiPackageInfo) -> PackageInfo {
    PackageInfo {
        name: package.name,
        version: package.version,
        source: PackageManagerType::Homebrew,
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
    use updater_manager_api::{ManagerId, PackageInfo as ApiPackageInfo, PackageTarget};

    use super::*;
    use crate::PackageManagerConfig;

    #[test]
    fn legacy_config_bridge_preserves_custom_homebrew_path() {
        let config = Config {
            app_managers: vec![PackageManagerConfig {
                manager_type: PackageManagerType::Homebrew,
                custom_path: Some("/custom/brew".to_owned()),
            }],
            ..Config::default()
        };

        let converted = manager_config(&config);
        assert_eq!(converted.id, PackageManagerType::Homebrew.manager_id());
        assert_eq!(
            converted.executable(),
            Some(std::path::Path::new("/custom/brew"))
        );
    }

    #[test]
    fn legacy_model_bridge_preserves_homebrew_metadata() {
        let id = ManagerId::parse("builtin:homebrew").expect("valid Homebrew ID");
        let mut package = ApiPackageInfo::new(id.clone(), "jq", "1.8.0");
        package.description = Some("JSON processor".to_owned());
        package.homepage = Some("https://jqlang.github.io/jq/".to_owned());

        let converted = convert_package_info(package);
        assert_eq!(converted.name, "jq");
        assert_eq!(converted.version, "1.8.0");
        assert_eq!(converted.source, PackageManagerType::Homebrew);
        assert_eq!(converted.description.as_deref(), Some("JSON processor"));
        assert_eq!(
            converted.homepage.as_deref(),
            Some("https://jqlang.github.io/jq/")
        );

        let update = ApiPackageUpdate::new(PackageTarget::new(id, "jq"), "1.8.0", "1.9.0");
        let converted = convert_package_update(update);
        assert_eq!(converted.name, "jq");
        assert_eq!(converted.current_version, "1.8.0");
        assert_eq!(converted.new_version, "1.9.0");
    }

    #[test]
    fn typed_manager_errors_map_back_to_legacy_categories() {
        for (kind, expected) in [
            (ManagerErrorKind::Network, "request"),
            (ManagerErrorKind::Protocol, "parse"),
            (ManagerErrorKind::Permission, "command"),
            (ManagerErrorKind::Busy, "command"),
            (ManagerErrorKind::Timeout, "command"),
            (ManagerErrorKind::Cancelled, "command"),
            (ManagerErrorKind::Other, "unknown"),
        ] {
            let error = convert_manager_error(
                ManagerError::new(kind, "operation failed").with_detail("diagnostic"),
            );
            let category = match error {
                CoreError::RequestError(_) => "request",
                CoreError::ParseError(_) => "parse",
                CoreError::CommandError(_) => "command",
                CoreError::UnknownError(_) => "unknown",
                CoreError::Utf8Error(_) | CoreError::SerializationError(_) => "unexpected",
            };
            assert_eq!(category, expected);
        }
    }
}
