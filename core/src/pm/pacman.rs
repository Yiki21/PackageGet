use async_trait::async_trait;
use updater_manager_api::{
    ManagerConfig, ManagerError, ManagerErrorKind, PackageAction as ApiPackageAction,
    PackageInfo as ApiPackageInfo, PackageManager as ApiPackageManager,
    PackageUpdate as ApiPackageUpdate,
};
use updater_managers::PacmanManager as DirectPacmanManager;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    error::CoreError, pm::progress::CommandProgressEvent,
};

#[derive(Debug, Clone, Copy)]
pub struct PacmanManager;

#[async_trait]
impl PackageManager for PacmanManager {
    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        DirectPacmanManager::new()
            .current_version(&manager_config(config), package_name)
            .await
            .map_err(convert_manager_error)
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        DirectPacmanManager::new()
            .installed(&manager_config(config))
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_manager_error)
    }

    async fn count_installed(config: &Config) -> CoreResult<usize> {
        DirectPacmanManager::new()
            .count_installed(&manager_config(config))
            .await
            .map_err(convert_manager_error)
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        DirectPacmanManager::new()
            .search(&manager_config(config), package_name)
            .await
            .map(|packages| packages.into_iter().map(convert_package_info).collect())
            .map_err(convert_manager_error)
    }
}

impl PacmanManager {
    pub async fn list_updates_with_refresh(
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        DirectPacmanManager::new()
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
    DirectPacmanManager::new()
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
    let id = PackageManagerType::Pacman.manager_id();
    config
        .manager(&id)
        .cloned()
        .unwrap_or_else(|| ManagerConfig::new(id))
}

fn convert_package_info(package: ApiPackageInfo) -> PackageInfo {
    PackageInfo {
        name: package.name,
        version: package.version,
        source: PackageManagerType::Pacman,
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
    #[test]
    fn config_preserves_custom_pacman_path() {
        let config = Config {
            managers: vec![
                ManagerConfig::new(PackageManagerType::Pacman.manager_id())
                    .with_executable("/custom/pacman"),
            ],
            ..Config::default()
        };

        let converted = manager_config(&config);
        assert_eq!(converted.id, PackageManagerType::Pacman.manager_id());
        assert_eq!(
            converted.executable(),
            Some(std::path::Path::new("/custom/pacman"))
        );
    }

    #[test]
    fn direct_model_conversion_preserves_pacman_metadata() {
        let id = ManagerId::parse("builtin:pacman").expect("valid Pacman ID");
        let mut package = ApiPackageInfo::new(id.clone(), "bash", "5.2.037-1");
        package.description = Some("GNU shell".to_owned());
        package.size = Some(42);
        package.install_date = Some("2026-07-29".to_owned());
        package.homepage = Some("https://www.gnu.org/software/bash/".to_owned());

        let converted = convert_package_info(package);
        assert_eq!(converted.name, "bash");
        assert_eq!(converted.version, "5.2.037-1");
        assert_eq!(converted.source, PackageManagerType::Pacman);
        assert_eq!(converted.description.as_deref(), Some("GNU shell"));
        assert_eq!(converted.size, Some(42));
        assert_eq!(converted.install_date.as_deref(), Some("2026-07-29"));
        assert_eq!(
            converted.homepage.as_deref(),
            Some("https://www.gnu.org/software/bash/")
        );

        let update = ApiPackageUpdate::new(PackageTarget::new(id, "bash"), "5.1", "5.2");
        let converted = convert_package_update(update);
        assert_eq!(converted.name, "bash");
        assert_eq!(converted.current_version, "5.1");
        assert_eq!(converted.new_version, "5.2");
    }

    #[test]
    fn typed_manager_errors_map_back_to_legacy_categories() {
        for (kind, expected) in [
            (ManagerErrorKind::Network, "request"),
            (ManagerErrorKind::Protocol, "parse"),
            (ManagerErrorKind::Permission, "command"),
            (ManagerErrorKind::Busy, "command"),
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
                CoreError::Utf8Error(_)
                | CoreError::SerializationError(_)
                | CoreError::ConfigError(_) => "unexpected",
            };
            assert_eq!(category, expected);
        }
    }

    #[test]
    fn command_errors_preserve_legacy_lock_and_cancellation_guidance() {
        let lock = convert_manager_error(
            ManagerError::new(ManagerErrorKind::Busy, "pacman failed")
                .with_detail("failed to init transaction (unable to lock database)"),
        );
        assert!(matches!(
            lock,
            CoreError::CommandError(message) if message.contains("package database is locked")
        ));

        let cancellation = convert_manager_error(
            ManagerError::new(ManagerErrorKind::Cancelled, "pkexec failed")
                .with_detail("operation was cancelled"),
        );
        assert!(matches!(
            cancellation,
            CoreError::CommandError(message) if message.contains("authorization was cancelled")
        ));
    }

    #[tokio::test]
    async fn empty_legacy_execution_does_not_run_pacman() {
        let mut events = Vec::new();
        PacmanManager::install_packages_with_progress(&Config::default(), &[], |event| {
            events.push(event);
        })
        .await
        .expect("execute empty legacy Pacman group");

        assert!(events.is_empty());
    }
}
