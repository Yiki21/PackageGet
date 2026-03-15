use crate::{Config, PackageManagerType};

pub(crate) fn manager_default_command(manager_type: PackageManagerType) -> &'static str {
    match manager_type {
        PackageManagerType::Apt => "apt",
        PackageManagerType::Dnf => "dnf",
        PackageManagerType::Pacman => "pacman",
        PackageManagerType::Zypper => "zypper",
        PackageManagerType::Flatpak => "flatpak",
        PackageManagerType::Homebrew => "brew",
        PackageManagerType::Cargo => "cargo",
        PackageManagerType::Go => "go",
        PackageManagerType::Npm => "npm",
        PackageManagerType::Pnpm => "pnpm",
    }
}

pub(crate) fn manager_command_path(config: &Config, manager_type: PackageManagerType) -> String {
    config
        .get_package_path(manager_type)
        .unwrap_or_else(|| manager_default_command(manager_type).to_owned())
}
