use std::fmt::Debug;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::CoreError,
    pm::{
        cargo::CargoManager,
        dnf::DnfManager,
        flatpak::FlatpakManager,
        go::GoManager,
        homebrew::HomebrewManager,
        npm::{NpmManager, PnpmManager},
        progress::CommandProgressEvent,
    },
};

pub mod error;
mod pm;
mod storage;

pub use storage::{Config, PackageManagerConfig};

#[derive(Debug, Clone)]
pub struct PackageUpdate {
    pub name: String,
    pub current_version: String,
    pub new_version: String,
}

#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub source: PackageManagerType,
    pub description: Option<String>,
    pub size: Option<u64>,
    pub install_date: Option<String>,
    pub homepage: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InstallProgress {
    pub manager: PackageManagerType,
    pub current_package: String,
    pub completed: usize,
    pub total: usize,
    pub command_message: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum PackageAction {
    Uninstall,
    Update,
    Install,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum PackageManagerType {
    Dnf,
    Flatpak,
    Homebrew,
    Cargo,
    Go,
    Npm,
    Pnpm,
}

pub static ALL_SYSTEM_PACKAGE_MANAGERS: [PackageManagerType; 1] = [PackageManagerType::Dnf];
pub static ALL_APP_PACKAGE_MANAGERS: [PackageManagerType; 6] = [
    PackageManagerType::Flatpak,
    PackageManagerType::Homebrew,
    PackageManagerType::Cargo,
    PackageManagerType::Go,
    PackageManagerType::Npm,
    PackageManagerType::Pnpm,
];
pub static ALL_PACKAGE_MANAGERS: [PackageManagerType; 7] = [
    PackageManagerType::Dnf,
    PackageManagerType::Flatpak,
    PackageManagerType::Homebrew,
    PackageManagerType::Cargo,
    PackageManagerType::Go,
    PackageManagerType::Npm,
    PackageManagerType::Pnpm,
];

macro_rules! dispatch_manager_static {
    ($manager:expr, $method:ident ( $($arg:expr),* $(,)? )) => {
        match $manager {
            PackageManagerType::Dnf => DnfManager::$method($($arg),*).await,
            PackageManagerType::Flatpak => FlatpakManager::$method($($arg),*).await,
            PackageManagerType::Homebrew => HomebrewManager::$method($($arg),*).await,
            PackageManagerType::Cargo => CargoManager::$method($($arg),*).await,
            PackageManagerType::Go => GoManager::$method($($arg),*).await,
            PackageManagerType::Npm => NpmManager::$method($($arg),*).await,
            PackageManagerType::Pnpm => PnpmManager::$method($($arg),*).await,
        }
    };
}

macro_rules! dispatch_manager_instance {
    ($manager:expr, $method:ident ( $($arg:expr),* $(,)? )) => {
        match $manager {
            PackageManagerType::Dnf => DnfManager.$method($($arg),*).await,
            PackageManagerType::Flatpak => FlatpakManager.$method($($arg),*).await,
            PackageManagerType::Homebrew => HomebrewManager.$method($($arg),*).await,
            PackageManagerType::Cargo => CargoManager.$method($($arg),*).await,
            PackageManagerType::Go => GoManager.$method($($arg),*).await,
            PackageManagerType::Npm => NpmManager.$method($($arg),*).await,
            PackageManagerType::Pnpm => PnpmManager.$method($($arg),*).await,
        }
    };
}

macro_rules! dispatch_package_progress_method {
    ($manager:expr, $method:ident($config:expr, $package_name:expr, $report:expr)) => {
        match $manager {
            PackageManagerType::Dnf => DnfManager::$method($config, $package_name, $report).await,
            PackageManagerType::Flatpak => {
                FlatpakManager::$method($config, $package_name, $report).await
            }
            PackageManagerType::Homebrew => {
                HomebrewManager::$method($config, $package_name, $report).await
            }
            PackageManagerType::Cargo => {
                CargoManager::$method($config, $package_name, $report).await
            }
            PackageManagerType::Go => GoManager::$method($config, $package_name, $report).await,
            PackageManagerType::Npm => NpmManager::$method($config, $package_name, $report).await,
            PackageManagerType::Pnpm => PnpmManager::$method($config, $package_name, $report).await,
        }
    };
}

impl PackageManagerType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Dnf => "DNF",
            Self::Flatpak => "Flatpak",
            Self::Homebrew => "Homebrew",
            Self::Cargo => "Cargo",
            Self::Go => "Go",
            Self::Npm => "NPM",
            Self::Pnpm => "pnpm",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Dnf => "Fedora/RHEL 系统包管理器",
            Self::Flatpak => "跨平台应用沙箱管理器",
            Self::Homebrew => "macOS/Linux 包管理器",
            Self::Cargo => "Rust 编程语言的包管理器",
            Self::Go => "Go 编程语言的包管理器",
            Self::Npm => "Node.js 默认包管理器",
            Self::Pnpm => "Node.js 高性能包管理器",
        }
    }

    pub fn is_system_manager(&self) -> bool {
        matches!(self, Self::Dnf)
    }

    pub async fn is_available(&self) -> bool {
        let cmd = match self {
            Self::Dnf => "dnf",
            Self::Flatpak => "flatpak",
            Self::Homebrew => "brew",
            Self::Cargo => "cargo",
            Self::Go => "go",
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
        };

        tokio::process::Command::new("which")
            .arg(cmd)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub async fn list_updates(&self, config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        dispatch_manager_static!(self, list_updates(config))
    }

    pub async fn list_updates_with_refresh(
        &self,
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        if matches!(self, Self::Dnf) {
            DnfManager::list_updates_with_refresh(config, refresh).await
        } else {
            self.list_updates(config).await
        }
    }

    pub async fn get_current_version(
        &self,
        config: &Config,
        package_name: &str,
    ) -> CoreResult<String> {
        dispatch_manager_static!(self, get_current_version(config, package_name))
    }

    pub async fn list_installed(&self, config: &Config) -> CoreResult<Vec<PackageInfo>> {
        dispatch_manager_static!(self, list_installed(config))
    }

    pub async fn count_installed(&self, config: &Config) -> CoreResult<usize> {
        dispatch_manager_static!(self, count_installed(config))
    }

    pub async fn search_package(
        &self,
        config: &Config,
        package_name: &str,
    ) -> CoreResult<Vec<PackageInfo>> {
        dispatch_manager_static!(self, search_package(config, package_name))
    }

    pub async fn uninstall_package(&self, config: &Config, package_name: &str) -> CoreResult<()> {
        dispatch_manager_instance!(self, uninstall_package(config, package_name))
    }

    pub async fn uninstall_packages(
        &self,
        config: &Config,
        package_names: &[String],
    ) -> CoreResult<()> {
        dispatch_manager_instance!(self, uninstall_packages(config, package_names))
    }

    pub async fn uninstall_packages_with_progress(
        &self,
        config: &Config,
        package_names: &[String],
        mut on_progress: impl FnMut(InstallProgress),
    ) -> CoreResult<()> {
        self.run_packages_with_progress(
            PackageAction::Uninstall,
            config,
            package_names,
            &mut on_progress,
        )
        .await
    }

    pub async fn update_packages(
        &self,
        config: &Config,
        package_names: &[String],
    ) -> CoreResult<()> {
        dispatch_manager_instance!(self, update_packages(config, package_names))
    }

    pub async fn update_package(&self, config: &Config, package_name: &str) -> CoreResult<()> {
        dispatch_manager_instance!(self, update_package(config, package_name))
    }

    pub async fn update_packages_with_progress(
        &self,
        config: &Config,
        package_names: &[String],
        mut on_progress: impl FnMut(InstallProgress),
    ) -> CoreResult<()> {
        self.run_packages_with_progress(
            PackageAction::Update,
            config,
            package_names,
            &mut on_progress,
        )
        .await
    }

    pub async fn install_packages(
        &self,
        config: &Config,
        package_names: &[String],
    ) -> CoreResult<()> {
        dispatch_manager_instance!(self, install_packages(config, package_names))
    }

    pub async fn install_package(&self, config: &Config, package_name: &str) -> CoreResult<()> {
        dispatch_manager_instance!(self, install_package(config, package_name))
    }

    pub async fn install_packages_with_progress(
        &self,
        config: &Config,
        package_names: &[String],
        mut on_progress: impl FnMut(InstallProgress),
    ) -> CoreResult<()> {
        self.run_packages_with_progress(
            PackageAction::Install,
            config,
            package_names,
            &mut on_progress,
        )
        .await
    }

    async fn run_packages_with_progress(
        &self,
        action: PackageAction,
        config: &Config,
        package_names: &[String],
        on_progress: &mut impl FnMut(InstallProgress),
    ) -> CoreResult<()> {
        let total = package_names.len();
        if total == 0 {
            return Ok(());
        }

        if matches!(self, Self::Dnf) {
            let mut report = |event: CommandProgressEvent| {
                let progress = event.progress.clamp(0.0, 1.0);
                let completed = if progress >= 1.0 {
                    total
                } else {
                    ((progress * total as f32).floor() as usize).min(total)
                };

                on_progress(InstallProgress {
                    manager: *self,
                    current_package: String::new(),
                    completed,
                    total,
                    command_message: event.command_message,
                });
            };

            Self::run_dnf_batch_action_with_progress(action, config, package_names, &mut report)
                .await?;
            return Ok(());
        }

        for (index, package_name) in package_names.iter().enumerate() {
            let package_name = package_name.clone();
            let mut report = |event: CommandProgressEvent| {
                let completed = if event.progress.clamp(0.0, 1.0) >= 1.0 {
                    index + 1
                } else {
                    index
                };

                on_progress(InstallProgress {
                    manager: *self,
                    current_package: package_name.clone(),
                    completed,
                    total,
                    command_message: event.command_message,
                });
            };

            self.run_single_package_action_with_progress(
                action,
                config,
                &package_name,
                &mut report,
            )
            .await?;
        }

        Ok(())
    }

    async fn run_dnf_batch_action_with_progress(
        action: PackageAction,
        config: &Config,
        package_names: &[String],
        report: &mut impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        match action {
            PackageAction::Uninstall => {
                DnfManager::uninstall_packages_with_progress(config, package_names, report).await
            }
            PackageAction::Update => {
                DnfManager::update_packages_with_progress(config, package_names, report).await
            }
            PackageAction::Install => {
                DnfManager::install_packages_with_progress(config, package_names, report).await
            }
        }
    }

    async fn run_single_package_action_with_progress(
        &self,
        action: PackageAction,
        config: &Config,
        package_name: &str,
        report: &mut impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        match action {
            PackageAction::Uninstall => dispatch_package_progress_method!(
                self,
                uninstall_package_with_progress(config, package_name, report)
            ),
            PackageAction::Update => {
                dispatch_package_progress_method!(
                    self,
                    update_package_with_progress(config, package_name, report)
                )
            }
            PackageAction::Install => dispatch_package_progress_method!(
                self,
                install_package_with_progress(config, package_name, report)
            ),
        }
    }
}

type CoreResult<T> = Result<T, CoreError>;

#[async_trait]
pub trait PackageManager: Send + Sync {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>>;

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String>;

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>>;

    /// Get Installed package count
    /// Default implementation counts the length of the list_installed result
    async fn count_installed(config: &Config) -> CoreResult<usize> {
        Ok(Self::list_installed(config).await?.len())
    }

    async fn search_package(_config: &Config, _package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        Err(CoreError::UnknownError(
            "search_package not implemented".into(),
        ))
    }

    async fn uninstall_package(&self, _config: &Config, _package_name: &str) -> CoreResult<()> {
        self.uninstall_packages(_config, &[_package_name.to_owned()])
            .await
    }

    async fn uninstall_packages(
        &self,
        _config: &Config,
        _package_names: &[String],
    ) -> CoreResult<()> {
        Err(CoreError::UnknownError(
            "uninstall_packages not implemented".into(),
        ))
    }

    async fn update_packages(&self, _config: &Config, _package_names: &[String]) -> CoreResult<()> {
        Err(CoreError::UnknownError(
            "update_packages not implemented".into(),
        ))
    }

    async fn update_package(&self, _config: &Config, _package_name: &str) -> CoreResult<()> {
        self.update_packages(_config, &[_package_name.to_owned()])
            .await
    }

    async fn install_packages(
        &self,
        _config: &Config,
        _package_names: &[String],
    ) -> CoreResult<()> {
        Err(CoreError::UnknownError(
            "install_packages not implemented".into(),
        ))
    }

    async fn install_package(&self, config: &Config, package_name: &str) -> CoreResult<()> {
        self.install_packages(config, &[package_name.to_owned()])
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        ALL_APP_PACKAGE_MANAGERS, ALL_PACKAGE_MANAGERS, ALL_SYSTEM_PACKAGE_MANAGERS,
        PackageManagerType,
    };

    #[test]
    fn manager_sets_have_no_duplicates() {
        let all_unique: HashSet<PackageManagerType> = ALL_PACKAGE_MANAGERS.into_iter().collect();
        assert_eq!(all_unique.len(), ALL_PACKAGE_MANAGERS.len());

        let system_unique: HashSet<PackageManagerType> =
            ALL_SYSTEM_PACKAGE_MANAGERS.into_iter().collect();
        assert_eq!(system_unique.len(), ALL_SYSTEM_PACKAGE_MANAGERS.len());

        let app_unique: HashSet<PackageManagerType> =
            ALL_APP_PACKAGE_MANAGERS.into_iter().collect();
        assert_eq!(app_unique.len(), ALL_APP_PACKAGE_MANAGERS.len());
    }

    #[test]
    fn system_and_app_managers_cover_all_managers() {
        let mut union: HashSet<PackageManagerType> =
            ALL_SYSTEM_PACKAGE_MANAGERS.into_iter().collect();
        union.extend(ALL_APP_PACKAGE_MANAGERS);

        let all: HashSet<PackageManagerType> = ALL_PACKAGE_MANAGERS.into_iter().collect();
        assert_eq!(union, all);
    }
}
