use std::fmt::Debug;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::CoreError,
    pm::{
        apt::AptManager,
        cargo::CargoManager,
        dnf::DnfManager,
        flatpak::FlatpakManager,
        go::GoManager,
        homebrew::HomebrewManager,
        npm::{NpmManager, PnpmManager},
        pacman::PacmanManager,
        progress::CommandProgressEvent,
        zypper::ZypperManager,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManagerKind {
    System,
    App,
}

#[derive(Debug, Clone, Copy)]
struct PackageManagerMetadata {
    name: &'static str,
    description: &'static str,
    command: &'static str,
    kind: PackageManagerKind,
}

macro_rules! define_package_managers {
    (
        system {
            $( $system_variant:ident : $system_manager:ident => ($system_name:expr, $system_description:expr, $system_command:expr), )*
        }
        app {
            $( $app_variant:ident : $app_manager:ident => ($app_name:expr, $app_description:expr, $app_command:expr), )*
        }
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
        pub enum PackageManagerType {
            $($system_variant,)*
            $($app_variant,)*
        }

        pub const ALL_SYSTEM_PACKAGE_MANAGERS: &[PackageManagerType] = &[
            $(PackageManagerType::$system_variant,)*
        ];

        pub const ALL_APP_PACKAGE_MANAGERS: &[PackageManagerType] = &[
            $(PackageManagerType::$app_variant,)*
        ];

        pub const ALL_PACKAGE_MANAGERS: &[PackageManagerType] = &[
            $(PackageManagerType::$system_variant,)*
            $(PackageManagerType::$app_variant,)*
        ];

        impl PackageManagerType {
            fn metadata(self) -> PackageManagerMetadata {
                match self {
                    $(
                        Self::$system_variant => PackageManagerMetadata {
                            name: $system_name,
                            description: $system_description,
                            command: $system_command,
                            kind: PackageManagerKind::System,
                        },
                    )*
                    $(
                        Self::$app_variant => PackageManagerMetadata {
                            name: $app_name,
                            description: $app_description,
                            command: $app_command,
                            kind: PackageManagerKind::App,
                        },
                    )*
                }
            }

            pub fn name(&self) -> &'static str {
                self.metadata().name
            }

            pub fn description(&self) -> &'static str {
                self.metadata().description
            }

            pub fn is_system_manager(&self) -> bool {
                self.metadata().kind == PackageManagerKind::System
            }

            pub async fn is_available(&self) -> bool {
                tokio::process::Command::new("which")
                    .arg(self.metadata().command)
                    .output()
                    .await
                    .map(|output| output.status.success())
                    .unwrap_or(false)
            }

            pub async fn list_updates(&self, config: &Config) -> CoreResult<Vec<PackageUpdate>> {
                match self {
                    $(Self::$system_variant => $system_manager::list_updates(config).await,)*
                    $(Self::$app_variant => $app_manager::list_updates(config).await,)*
                }
            }

            pub async fn get_current_version(
                &self,
                config: &Config,
                package_name: &str,
            ) -> CoreResult<String> {
                match self {
                    $(Self::$system_variant => $system_manager::get_current_version(config, package_name).await,)*
                    $(Self::$app_variant => $app_manager::get_current_version(config, package_name).await,)*
                }
            }

            pub async fn list_installed(&self, config: &Config) -> CoreResult<Vec<PackageInfo>> {
                match self {
                    $(Self::$system_variant => $system_manager::list_installed(config).await,)*
                    $(Self::$app_variant => $app_manager::list_installed(config).await,)*
                }
            }

            pub async fn count_installed(&self, config: &Config) -> CoreResult<usize> {
                match self {
                    $(Self::$system_variant => $system_manager::count_installed(config).await,)*
                    $(Self::$app_variant => $app_manager::count_installed(config).await,)*
                }
            }

            pub async fn search_package(
                &self,
                config: &Config,
                package_name: &str,
            ) -> CoreResult<Vec<PackageInfo>> {
                match self {
                    $(Self::$system_variant => $system_manager::search_package(config, package_name).await,)*
                    $(Self::$app_variant => $app_manager::search_package(config, package_name).await,)*
                }
            }

            pub async fn uninstall_package(
                &self,
                config: &Config,
                package_name: &str,
            ) -> CoreResult<()> {
                match self {
                    $(Self::$system_variant => $system_manager.uninstall_package(config, package_name).await,)*
                    $(Self::$app_variant => $app_manager.uninstall_package(config, package_name).await,)*
                }
            }

            pub async fn uninstall_packages(
                &self,
                config: &Config,
                package_names: &[String],
            ) -> CoreResult<()> {
                match self {
                    $(Self::$system_variant => $system_manager.uninstall_packages(config, package_names).await,)*
                    $(Self::$app_variant => $app_manager.uninstall_packages(config, package_names).await,)*
                }
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
                match self {
                    $(Self::$system_variant => $system_manager.update_packages(config, package_names).await,)*
                    $(Self::$app_variant => $app_manager.update_packages(config, package_names).await,)*
                }
            }

            pub async fn update_package(
                &self,
                config: &Config,
                package_name: &str,
            ) -> CoreResult<()> {
                match self {
                    $(Self::$system_variant => $system_manager.update_package(config, package_name).await,)*
                    $(Self::$app_variant => $app_manager.update_package(config, package_name).await,)*
                }
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
                match self {
                    $(Self::$system_variant => $system_manager.install_packages(config, package_names).await,)*
                    $(Self::$app_variant => $app_manager.install_packages(config, package_names).await,)*
                }
            }

            pub async fn install_package(
                &self,
                config: &Config,
                package_name: &str,
            ) -> CoreResult<()> {
                match self {
                    $(Self::$system_variant => $system_manager.install_package(config, package_name).await,)*
                    $(Self::$app_variant => $app_manager.install_package(config, package_name).await,)*
                }
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

                if self.is_system_manager() {
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

                    Self::run_system_batch_action_with_progress(
                        *self,
                        action,
                        config,
                        package_names,
                        &mut report,
                    )
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

            async fn run_single_package_action_with_progress(
                &self,
                action: PackageAction,
                config: &Config,
                package_name: &str,
                report: &mut impl FnMut(CommandProgressEvent),
            ) -> CoreResult<()> {
                match action {
                    PackageAction::Uninstall => match self {
                        $(Self::$system_variant => $system_manager::uninstall_package_with_progress(config, package_name, report).await,)*
                        $(Self::$app_variant => $app_manager::uninstall_package_with_progress(config, package_name, report).await,)*
                    },
                    PackageAction::Update => match self {
                        $(Self::$system_variant => $system_manager::update_package_with_progress(config, package_name, report).await,)*
                        $(Self::$app_variant => $app_manager::update_package_with_progress(config, package_name, report).await,)*
                    },
                    PackageAction::Install => match self {
                        $(Self::$system_variant => $system_manager::install_package_with_progress(config, package_name, report).await,)*
                        $(Self::$app_variant => $app_manager::install_package_with_progress(config, package_name, report).await,)*
                    },
                }
            }
        }
    };
}

define_package_managers! {
    system {
        Apt: AptManager => ("APT", "Debian/Ubuntu 系统包管理器", "apt"),
        Dnf: DnfManager => ("DNF", "Fedora/RHEL 系统包管理器", "dnf"),
        Pacman: PacmanManager => ("Pacman", "Arch Linux 系统包管理器", "pacman"),
        Zypper: ZypperManager => ("Zypper", "openSUSE/SUSE 系统包管理器", "zypper"),
    }
    app {
        Flatpak: FlatpakManager => ("Flatpak", "跨平台应用沙箱管理器", "flatpak"),
        Homebrew: HomebrewManager => ("Homebrew", "macOS/Linux 包管理器", "brew"),
        Cargo: CargoManager => ("Cargo", "Rust 编程语言的包管理器", "cargo"),
        Go: GoManager => ("Go", "Go 编程语言的包管理器", "go"),
        Npm: NpmManager => ("NPM", "Node.js 默认包管理器", "npm"),
        Pnpm: PnpmManager => ("pnpm", "Node.js 高性能包管理器", "pnpm"),
    }
}

impl PackageManagerType {
    pub async fn list_updates_with_refresh(
        &self,
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        match self {
            Self::Apt => AptManager::list_updates_with_refresh(config, refresh).await,
            Self::Dnf => DnfManager::list_updates_with_refresh(config, refresh).await,
            Self::Pacman => PacmanManager::list_updates_with_refresh(config, refresh).await,
            Self::Zypper => ZypperManager::list_updates_with_refresh(config, refresh).await,
            _ => self.list_updates(config).await,
        }
    }

    async fn run_system_batch_action_with_progress(
        manager: PackageManagerType,
        action: PackageAction,
        config: &Config,
        package_names: &[String],
        report: &mut impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        match manager {
            Self::Apt => match action {
                PackageAction::Uninstall => {
                    AptManager::uninstall_packages_with_progress(config, package_names, report)
                        .await
                }
                PackageAction::Update => {
                    AptManager::update_packages_with_progress(config, package_names, report).await
                }
                PackageAction::Install => {
                    AptManager::install_packages_with_progress(config, package_names, report).await
                }
            },
            Self::Dnf => match action {
                PackageAction::Uninstall => {
                    DnfManager::uninstall_packages_with_progress(config, package_names, report)
                        .await
                }
                PackageAction::Update => {
                    DnfManager::update_packages_with_progress(config, package_names, report).await
                }
                PackageAction::Install => {
                    DnfManager::install_packages_with_progress(config, package_names, report).await
                }
            },
            Self::Pacman => match action {
                PackageAction::Uninstall => {
                    PacmanManager::uninstall_packages_with_progress(config, package_names, report)
                        .await
                }
                PackageAction::Update => {
                    PacmanManager::update_packages_with_progress(config, package_names, report)
                        .await
                }
                PackageAction::Install => {
                    PacmanManager::install_packages_with_progress(config, package_names, report)
                        .await
                }
            },
            Self::Zypper => match action {
                PackageAction::Uninstall => {
                    ZypperManager::uninstall_packages_with_progress(config, package_names, report)
                        .await
                }
                PackageAction::Update => {
                    ZypperManager::update_packages_with_progress(config, package_names, report)
                        .await
                }
                PackageAction::Install => {
                    ZypperManager::install_packages_with_progress(config, package_names, report)
                        .await
                }
            },
            _ => Err(CoreError::UnknownError(
                "batch action is only supported for system package managers".to_owned(),
            )),
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
        let all_unique: HashSet<PackageManagerType> =
            ALL_PACKAGE_MANAGERS.iter().copied().collect();
        assert_eq!(all_unique.len(), ALL_PACKAGE_MANAGERS.len());

        let system_unique: HashSet<PackageManagerType> =
            ALL_SYSTEM_PACKAGE_MANAGERS.iter().copied().collect();
        assert_eq!(system_unique.len(), ALL_SYSTEM_PACKAGE_MANAGERS.len());

        let app_unique: HashSet<PackageManagerType> =
            ALL_APP_PACKAGE_MANAGERS.iter().copied().collect();
        assert_eq!(app_unique.len(), ALL_APP_PACKAGE_MANAGERS.len());
    }

    #[test]
    fn system_and_app_managers_cover_all_managers() {
        let mut union: HashSet<PackageManagerType> =
            ALL_SYSTEM_PACKAGE_MANAGERS.iter().copied().collect();
        union.extend(ALL_APP_PACKAGE_MANAGERS.iter().copied());

        let all: HashSet<PackageManagerType> = ALL_PACKAGE_MANAGERS.iter().copied().collect();
        assert_eq!(union, all);
    }
}
