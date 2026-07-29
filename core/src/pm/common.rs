use std::{env, ffi::OsStr, path::PathBuf};

use directories_next::UserDirs;
use tokio::process::Command;

use crate::{Config, PackageManagerType};

pub(crate) fn manager_default_command(manager_type: PackageManagerType) -> &'static str {
    manager_type.command()
}

fn push_search_directory(directories: &mut Vec<PathBuf>, directory: impl Into<PathBuf>) {
    let directory = directory.into();
    if !directory.as_os_str().is_empty() && !directories.contains(&directory) {
        directories.push(directory);
    }
}

fn manager_search_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();

    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            push_search_directory(&mut directories, directory);
        }
    }

    if let Some(asdf_data_dir) = env::var_os("ASDF_DATA_DIR") {
        push_search_directory(&mut directories, PathBuf::from(asdf_data_dir).join("shims"));
    }
    if let Some(pnpm_home) = env::var_os("PNPM_HOME") {
        push_search_directory(&mut directories, pnpm_home);
    }
    if let Some(homebrew_prefix) = env::var_os("HOMEBREW_PREFIX") {
        push_search_directory(&mut directories, PathBuf::from(homebrew_prefix).join("bin"));
    }

    if let Some(user_dirs) = UserDirs::new() {
        let home = user_dirs.home_dir();
        for directory in [
            home.join(".asdf/shims"),
            home.join(".local/share/pnpm"),
            home.join(".local/share/pnpm/bin"),
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join("go/bin"),
            home.join(".linuxbrew/bin"),
        ] {
            push_search_directory(&mut directories, directory);
        }
    }

    for directory in [
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ] {
        push_search_directory(&mut directories, directory);
    }

    directories
}

pub(crate) fn manager_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    if let Ok(path) = env::join_paths(manager_search_directories()) {
        command.env("PATH", path);
    }
    command
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

fn find_executable(command: &str, directories: &[PathBuf]) -> Option<PathBuf> {
    directories.iter().find_map(|directory| {
        let candidate = directory.join(command);
        let metadata = std::fs::metadata(&candidate).ok()?;
        (metadata.is_file() && is_executable(&metadata)).then_some(candidate)
    })
}

fn resolve_default_manager_command(manager_type: PackageManagerType) -> String {
    let command = manager_default_command(manager_type);
    find_executable(command, &manager_search_directories())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| command.to_owned())
}

pub(crate) fn manager_command_path(config: &Config, manager_type: PackageManagerType) -> String {
    let id = manager_type.manager_id();
    config
        .manager_executable(&id)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| resolve_default_manager_command(manager_type))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn find_executable_uses_the_first_executable_candidate() {
        let first = tempdir().expect("create first temporary directory");
        let second = tempdir().expect("create second temporary directory");
        let first_command = first.path().join("pnpm");
        let second_command = second.path().join("pnpm");
        fs::write(&first_command, "#!/bin/sh\n").expect("write first executable");
        fs::write(&second_command, "#!/bin/sh\n").expect("write second executable");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&first_command, fs::Permissions::from_mode(0o755))
                .expect("mark first candidate executable");
            fs::set_permissions(&second_command, fs::Permissions::from_mode(0o755))
                .expect("mark second candidate executable");
        }

        let directories = vec![first.path().to_path_buf(), second.path().to_path_buf()];
        assert_eq!(find_executable("pnpm", &directories), Some(first_command));
    }

    #[cfg(unix)]
    #[test]
    fn find_executable_ignores_non_executable_files() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().expect("create temporary directory");
        let command = directory.path().join("brew");
        fs::write(&command, "#!/bin/sh\n").expect("write non-executable candidate");
        fs::set_permissions(&command, fs::Permissions::from_mode(0o644))
            .expect("mark candidate non-executable");

        assert_eq!(
            find_executable("brew", &[directory.path().to_path_buf()]),
            None
        );
    }
}
