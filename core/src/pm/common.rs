use std::path::Path;

use crate::{Config, PackageManagerType};

pub(crate) async fn directory_size(path: &Path) -> Option<u64> {
    let root = tokio::fs::canonicalize(path).await.ok()?;
    let mut pending = vec![root];
    let mut total = 0_u64;

    while let Some(directory) = pending.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(directory).await else {
            continue;
        };

        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) | Err(_) => break,
            };
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };

            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file()
                && let Ok(metadata) = entry.metadata().await
            {
                total = total.saturating_add(metadata.len());
            }
        }
    }

    Some(total)
}

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
        PackageManagerType::Pipx => "pipx",
    }
}

pub(crate) fn manager_command_path(config: &Config, manager_type: PackageManagerType) -> String {
    config
        .get_package_path(manager_type)
        .unwrap_or_else(|| manager_default_command(manager_type).to_owned())
}
