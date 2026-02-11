use std::fmt::Debug;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::CoreError,
    pm::{
        cargo::CargoManager, dnf::DnfManager, flatpak::FlatpakManager, go::GoManager,
        homebrew::HomebrewManager,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum PackageManagerType {
    Dnf,
    Flatpak,
    Homebrew,
    Cargo,
    Go,
}

pub static ALL_SYSTEM_PACKAGE_MANAGERS: [PackageManagerType; 1] = [PackageManagerType::Dnf];
pub static ALL_APP_PACKAGE_MANAGERS: [PackageManagerType; 4] = [
    PackageManagerType::Flatpak,
    PackageManagerType::Homebrew,
    PackageManagerType::Cargo,
    PackageManagerType::Go,
];

impl PackageManagerType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Dnf => "DNF",
            Self::Flatpak => "Flatpak",
            Self::Homebrew => "Homebrew",
            Self::Cargo => "Cargo",
            Self::Go => "Go",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Dnf => "Fedora/RHEL 系统包管理器",
            Self::Flatpak => "跨平台应用沙箱管理器",
            Self::Homebrew => "macOS/Linux 包管理器",
            Self::Cargo => "Rust 编程语言的包管理器",
            Self::Go => "Go 编程语言的包管理器",
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
        };

        tokio::process::Command::new("which")
            .arg(cmd)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub async fn list_updates(&self, config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        match self {
            Self::Dnf => DnfManager::list_updates(config).await,
            Self::Flatpak => FlatpakManager::list_updates(config).await,
            Self::Homebrew => HomebrewManager::list_updates(config).await,
            Self::Cargo => CargoManager::list_updates(config).await,
            Self::Go => GoManager::list_updates(config).await,
        }
    }

    pub async fn get_current_version(
        &self,
        config: &Config,
        package_name: &str,
    ) -> CoreResult<String> {
        match self {
            Self::Dnf => DnfManager::get_current_version(config, package_name).await,
            Self::Flatpak => FlatpakManager::get_current_version(config, package_name).await,
            Self::Homebrew => HomebrewManager::get_current_version(config, package_name).await,
            Self::Cargo => CargoManager::get_current_version(config, package_name).await,
            Self::Go => GoManager::get_current_version(config, package_name).await,
        }
    }

    pub async fn list_installed(&self, config: &Config) -> CoreResult<Vec<PackageInfo>> {
        match self {
            Self::Dnf => DnfManager::list_installed(config).await,
            Self::Flatpak => FlatpakManager::list_installed(config).await,
            Self::Homebrew => HomebrewManager::list_installed(config).await,
            Self::Cargo => CargoManager::list_installed(config).await,
            Self::Go => GoManager::list_installed(config).await,
        }
    }

    pub async fn count_installed(&self, config: &Config) -> CoreResult<usize> {
        match self {
            Self::Dnf => DnfManager::count_installed(config).await,
            Self::Flatpak => FlatpakManager::count_installed(config).await,
            Self::Homebrew => HomebrewManager::count_installed(config).await,
            Self::Cargo => CargoManager::count_installed(config).await,
            Self::Go => GoManager::count_installed(config).await,
        }
    }

    pub async fn search_package(
        &self,
        config: &Config,
        package_name: &str,
    ) -> CoreResult<Vec<PackageInfo>> {
        match self {
            Self::Dnf => DnfManager::search_package(config, package_name).await,
            Self::Flatpak => FlatpakManager::search_package(config, package_name).await,
            Self::Homebrew => HomebrewManager::search_package(config, package_name).await,
            Self::Cargo => CargoManager::search_package(config, package_name).await,
            Self::Go => GoManager::search_package(config, package_name).await,
        }
    }

    pub async fn uninstall_package(&self, config: &Config, package_name: &str) -> CoreResult<()> {
        match self {
            Self::Dnf => DnfManager.uninstall_package(config, package_name).await,
            Self::Flatpak => FlatpakManager.uninstall_package(config, package_name).await,
            Self::Homebrew => {
                HomebrewManager
                    .uninstall_package(config, package_name)
                    .await
            }
            Self::Cargo => CargoManager.uninstall_package(config, package_name).await,
            Self::Go => GoManager.uninstall_package(config, package_name).await,
        }
    }

    pub async fn uninstall_packages(
        &self,
        config: &Config,
        package_names: &[String],
    ) -> CoreResult<()> {
        match self {
            Self::Dnf => DnfManager.uninstall_packages(config, package_names).await,
            Self::Flatpak => {
                FlatpakManager
                    .uninstall_packages(config, package_names)
                    .await
            }
            Self::Homebrew => {
                HomebrewManager
                    .uninstall_packages(config, package_names)
                    .await
            }
            Self::Cargo => CargoManager.uninstall_packages(config, package_names).await,
            Self::Go => GoManager.uninstall_packages(config, package_names).await,
        }
    }

    pub async fn update_packages(
        &self,
        config: &Config,
        package_names: &[String],
    ) -> CoreResult<()> {
        match self {
            Self::Dnf => DnfManager.update_packages(config, package_names).await,
            Self::Flatpak => FlatpakManager.update_packages(config, package_names).await,
            Self::Homebrew => HomebrewManager.update_packages(config, package_names).await,
            Self::Cargo => CargoManager.update_packages(config, package_names).await,
            Self::Go => GoManager.update_packages(config, package_names).await,
        }
    }

    pub async fn install_packages(
        &self,
        config: &Config,
        package_names: &[String],
    ) -> CoreResult<()> {
        match self {
            Self::Dnf => DnfManager.install_packages(config, package_names).await,
            Self::Flatpak => FlatpakManager.install_packages(config, package_names).await,
            Self::Homebrew => {
                HomebrewManager
                    .install_packages(config, package_names)
                    .await
            }
            Self::Cargo => CargoManager.install_packages(config, package_names).await,
            Self::Go => GoManager.install_packages(config, package_names).await,
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
        Err(CoreError::UnknownError(
            "uninstall_package not implemented".into(),
        ))
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

    async fn install_packages(
        &self,
        _config: &Config,
        _package_names: &[String],
    ) -> CoreResult<()> {
        Err(CoreError::UnknownError(
            "install_packages not implemented".into(),
        ))
    }
}
