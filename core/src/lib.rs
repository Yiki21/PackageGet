use std::{fmt::Debug, path::Path, time::Duration};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
pub use updater_manager_api::ManagerConfig;
use updater_manager_api::{
    AuthorizationHint, ManagerCapabilities, ManagerCapability, ManagerCategory, ManagerDescriptor,
    ManagerId, Platform, SupportedPlatforms,
};

use crate::{
    error::CoreError,
    pm::{
        apt::AptManager,
        cargo::CargoManager,
        common::{manager_command, manager_command_path},
        dnf::DnfManager,
        flatpak::FlatpakManager,
        go::GoManager,
        homebrew::HomebrewManager,
        npm::{NpmManager, PnpmManager},
        pacman::PacmanManager,
        pipx::PipxManager,
        progress::CommandProgressEvent,
        zypper::ZypperManager,
    },
};

mod builtin_managers;
pub mod error;
mod pm;
mod registry;
mod storage;

pub use builtin_managers::register_builtin_managers;
pub use registry::{ManagerRegistry, RegistryError};
pub use storage::Config;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageManagerAvailability {
    Available,
    NotFound { command: String },
    NotExecutable { path: String },
    VersionCheckFailed { command: String, detail: String },
}

impl PackageManagerAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub fn message(&self) -> String {
        match self {
            Self::Available => "Available".to_owned(),
            Self::NotFound { command } => format!("Not found: {}", command),
            Self::NotExecutable { path } => format!("Not executable: {}", path),
            Self::VersionCheckFailed { command, detail } => {
                format!("Version check failed for {}: {}", command, detail)
            }
        }
    }
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
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

            pub(crate) fn command(&self) -> &'static str {
                self.metadata().command
            }

            pub fn is_system_manager(&self) -> bool {
                self.metadata().kind == PackageManagerKind::System
            }

            fn version_args(&self) -> &'static [&'static str] {
                match self {
                    Self::Go => &["version"],
                    _ => &["--version"],
                }
            }

            pub async fn is_available(&self) -> bool {
                self.availability_with_config(&Config::default())
                    .await
                    .is_available()
            }

            pub async fn availability_with_config(
                &self,
                config: &Config,
            ) -> PackageManagerAvailability {
                let manager_id = self.manager_id();
                let custom_path = config
                    .manager(&manager_id)
                    .is_some_and(|manager| manager.executable().is_some());
                let command = manager_command_path(config, *self);
                let validate_path = custom_path || Path::new(&command).components().count() > 1;

                if validate_path {
                    let path = Path::new(&command);
                    let metadata = match tokio::fs::metadata(path).await {
                        Ok(metadata) => metadata,
                        Err(_) => {
                            return PackageManagerAvailability::NotFound {
                                command,
                            };
                        }
                    };

                    if !metadata.is_file() || !is_executable(&metadata) {
                        return PackageManagerAvailability::NotExecutable { path: command };
                    }
                }

                let version_args = self.version_args();
                let version_command = version_args.join(" ");
                let output = tokio::time::timeout(
                    Duration::from_secs(5),
                    manager_command(&command).args(version_args).output(),
                )
                .await;

                match output {
                    Ok(Ok(output)) if output.status.success() => PackageManagerAvailability::Available,
                    Ok(Ok(output)) => {
                        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                        PackageManagerAvailability::VersionCheckFailed {
                            command,
                            detail: if detail.is_empty() {
                                format!("exited with {}", output.status)
                            } else {
                                detail
                            },
                        }
                    }
                    Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                        PackageManagerAvailability::NotFound { command }
                    }
                    Ok(Err(error)) => PackageManagerAvailability::VersionCheckFailed {
                        command,
                        detail: error.to_string(),
                    },
                    Err(_) => PackageManagerAvailability::VersionCheckFailed {
                        command,
                        detail: format!("timed out while running {}", version_command),
                    },
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

                    match self {
                        Self::Apt => match action {
                            PackageAction::Uninstall => AptManager::uninstall_packages_with_progress(config, package_names, &mut report).await,
                            PackageAction::Update => AptManager::update_packages_with_progress(config, package_names, &mut report).await,
                            PackageAction::Install => AptManager::install_packages_with_progress(config, package_names, &mut report).await,
                        },
                        Self::Dnf => match action {
                            PackageAction::Uninstall => DnfManager::uninstall_packages_with_progress(config, package_names, &mut report).await,
                            PackageAction::Update => DnfManager::update_packages_with_progress(config, package_names, &mut report).await,
                            PackageAction::Install => DnfManager::install_packages_with_progress(config, package_names, &mut report).await,
                        },
                        Self::Pacman => match action {
                            PackageAction::Uninstall => PacmanManager::uninstall_packages_with_progress(config, package_names, &mut report).await,
                            PackageAction::Update => PacmanManager::update_packages_with_progress(config, package_names, &mut report).await,
                            PackageAction::Install => PacmanManager::install_packages_with_progress(config, package_names, &mut report).await,
                        },
                        Self::Zypper => match action {
                            PackageAction::Uninstall => ZypperManager::uninstall_packages_with_progress(config, package_names, &mut report).await,
                            PackageAction::Update => ZypperManager::update_packages_with_progress(config, package_names, &mut report).await,
                            PackageAction::Install => ZypperManager::install_packages_with_progress(config, package_names, &mut report).await,
                        },
                        _ => Err(CoreError::UnknownError(
                            "batch action is only supported for system package managers".to_owned(),
                        )),
                    }?;
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

                    match action {
                        PackageAction::Uninstall => match self {
                            $(Self::$app_variant => $app_manager::uninstall_package_with_progress(config, &package_name, &mut report).await,)*
                            _ => Err(CoreError::UnknownError("single-package action is only supported for app package managers".to_owned())),
                        },
                        PackageAction::Update => match self {
                            $(Self::$app_variant => $app_manager::update_package_with_progress(config, &package_name, &mut report).await,)*
                            _ => Err(CoreError::UnknownError("single-package action is only supported for app package managers".to_owned())),
                        },
                        PackageAction::Install => match self {
                            $(Self::$app_variant => $app_manager::install_package_with_progress(config, &package_name, &mut report).await,)*
                            _ => Err(CoreError::UnknownError("single-package action is only supported for app package managers".to_owned())),
                        },
                    }?;
                }

                Ok(())
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
        Pipx: PipxManager => ("pipx", "Python CLI 应用隔离安装管理器", "pipx"),
    }
}

impl PackageManagerType {
    /// Returns the stable built-in manager ID.
    #[must_use]
    pub fn manager_id(self) -> ManagerId {
        ManagerId::parse(match self {
            Self::Apt => "builtin:apt",
            Self::Dnf => "builtin:dnf",
            Self::Pacman => "builtin:pacman",
            Self::Zypper => "builtin:zypper",
            Self::Flatpak => "builtin:flatpak",
            Self::Homebrew => "builtin:homebrew",
            Self::Cargo => "builtin:cargo",
            Self::Go => "builtin:go",
            Self::Npm => "builtin:npm",
            Self::Pnpm => "builtin:pnpm",
            Self::Pipx => "builtin:pipx",
        })
        .expect("built-in manager IDs must be valid")
    }

    /// Resolves a built-in runtime manager from its stable ID.
    #[must_use]
    pub fn from_manager_id(id: &ManagerId) -> Option<Self> {
        match id.as_str() {
            "builtin:apt" => Some(Self::Apt),
            "builtin:dnf" => Some(Self::Dnf),
            "builtin:pacman" => Some(Self::Pacman),
            "builtin:zypper" => Some(Self::Zypper),
            "builtin:flatpak" => Some(Self::Flatpak),
            "builtin:homebrew" => Some(Self::Homebrew),
            "builtin:cargo" => Some(Self::Cargo),
            "builtin:go" => Some(Self::Go),
            "builtin:npm" => Some(Self::Npm),
            "builtin:pnpm" => Some(Self::Pnpm),
            "builtin:pipx" => Some(Self::Pipx),
            _ => None,
        }
    }

    /// Returns the object-safe API descriptor for this built-in manager.
    #[must_use]
    pub fn manager_descriptor(self) -> ManagerDescriptor {
        let category = match self {
            Self::Apt | Self::Dnf | Self::Pacman | Self::Zypper => ManagerCategory::System,
            Self::Flatpak | Self::Homebrew => ManagerCategory::Application,
            Self::Cargo | Self::Go | Self::Npm | Self::Pnpm | Self::Pipx => {
                ManagerCategory::Development
            }
        };

        let platforms = match self {
            Self::Apt | Self::Dnf | Self::Pacman | Self::Zypper | Self::Flatpak => {
                SupportedPlatforms::from([Platform::Linux])
            }
            Self::Homebrew | Self::Cargo | Self::Go | Self::Npm | Self::Pnpm | Self::Pipx => {
                SupportedPlatforms::from([Platform::Linux, Platform::MacOs])
            }
        };

        let capabilities = ManagerCapabilities::from([
            ManagerCapability::Installed,
            ManagerCapability::Updates,
            ManagerCapability::Search,
            ManagerCapability::Install,
            ManagerCapability::Update,
            ManagerCapability::Uninstall,
        ]);

        let descriptor = ManagerDescriptor::new(
            self.manager_id(),
            self.name(),
            category,
            platforms,
            capabilities,
        )
        .expect("built-in manager descriptors must be valid")
        .with_description(self.description());

        if self.is_system_manager() {
            descriptor.with_authorization(AuthorizationHint::RequiresElevation {
                message: Some("System package changes require administrator approval.".to_owned()),
            })
        } else {
            descriptor
        }
    }

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
            Self::Flatpak => FlatpakManager::list_updates_with_refresh(config, refresh).await,
            Self::Homebrew => HomebrewManager::list_updates_with_refresh(config, refresh).await,
            Self::Cargo => CargoManager::list_updates_with_refresh(config, refresh).await,
            Self::Go => GoManager::list_updates_with_refresh(config, refresh).await,
            Self::Npm => NpmManager::list_updates_with_refresh(config, refresh).await,
            Self::Pnpm => PnpmManager::list_updates_with_refresh(config, refresh).await,
            Self::Pipx => PipxManager::list_updates_with_refresh(config, refresh).await,
        }
    }
}

type CoreResult<T> = Result<T, CoreError>;

#[async_trait]
pub trait PackageManager: Send + Sync {
    async fn list_updates(_config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        Err(CoreError::UnknownError(
            "list_updates not implemented".into(),
        ))
    }

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
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        ALL_APP_PACKAGE_MANAGERS, ALL_PACKAGE_MANAGERS, ALL_SYSTEM_PACKAGE_MANAGERS,
        ManagerCapability, PackageManagerType,
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

    #[test]
    fn go_uses_go_version_for_availability_check() {
        assert_eq!(PackageManagerType::Go.version_args(), &["version"]);
    }

    #[test]
    fn other_managers_use_dash_dash_version_for_availability_check() {
        for manager in ALL_PACKAGE_MANAGERS {
            if *manager != PackageManagerType::Go {
                assert_eq!(manager.version_args(), &["--version"]);
            }
        }
    }

    #[test]
    fn built_in_manager_ids_are_unique_and_namespaced() {
        let ids: HashSet<_> = ALL_PACKAGE_MANAGERS
            .iter()
            .map(|manager| manager.manager_id())
            .collect();

        assert_eq!(ids.len(), ALL_PACKAGE_MANAGERS.len());
        assert!(ids.iter().all(|id| id.as_str().starts_with("builtin:")));
    }

    #[test]
    fn built_in_descriptors_advertise_current_capabilities() {
        for manager in ALL_PACKAGE_MANAGERS {
            let descriptor = manager.manager_descriptor();
            assert_eq!(descriptor.id(), &manager.manager_id());

            for capability in [
                ManagerCapability::Installed,
                ManagerCapability::Updates,
                ManagerCapability::Search,
                ManagerCapability::Install,
                ManagerCapability::Update,
                ManagerCapability::Uninstall,
            ] {
                assert!(descriptor.capabilities().contains(capability));
            }
        }
    }
}
