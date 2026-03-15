use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use tokio::process::Command;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    error::CoreError,
    pm::{
        common::manager_command_path,
        progress::{CommandProgressEvent, run_command_with_progress},
    },
};

#[derive(Debug, Clone, Copy)]
pub struct PacmanManager;

fn command_path(config: &Config) -> String {
    manager_command_path(config, PackageManagerType::Pacman)
}

#[async_trait]
impl PackageManager for PacmanManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        Self::list_updates_with_refresh(config, false).await
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        let path = command_path(config);

        let output = Command::new(&path)
            .arg("-Q")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Err(CoreError::ParseError(format!(
                "Package {} not found",
                package_name
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut parts = stdout.split_whitespace();
        let _name = parts.next();
        let version = parts.next();

        version
            .map(ToOwned::to_owned)
            .ok_or_else(|| CoreError::ParseError("Failed to parse pacman version".to_owned()))
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        let path = command_path(config);

        let output = Command::new(&path).arg("-Q").output().await?;
        if !output.status.success() {
            return Err(CoreError::UnknownError("pacman -Q failed".to_owned()));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let packages = stdout
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let name = parts.next()?;
                let version = parts.next()?;

                Some(PackageInfo {
                    name: name.to_owned(),
                    version: version.to_owned(),
                    source: PackageManagerType::Pacman,
                    description: None,
                    size: None,
                    install_date: None,
                    homepage: None,
                })
            })
            .collect();

        Ok(packages)
    }

    async fn count_installed(config: &Config) -> CoreResult<usize> {
        let path = command_path(config);

        let output = Command::new(&path).arg("-Qq").output().await?;
        if !output.status.success() {
            return Ok(Self::list_installed(config).await?.len());
        }

        let stdout = String::from_utf8(output.stdout)?;
        Ok(stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count())
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        let path = command_path(config);

        let output = Command::new(&path)
            .arg("-Ss")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let installed_versions = Self::installed_version_map(config).await?;

        let packages = parse_search_results(&stdout)
            .into_iter()
            .map(|(name, _available_version, description)| PackageInfo {
                version: installed_versions
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| "Not Installed".to_owned()),
                name,
                source: PackageManagerType::Pacman,
                description,
                size: None,
                install_date: None,
                homepage: None,
            })
            .collect();

        Ok(packages)
    }

    async fn uninstall_packages(
        &self,
        config: &Config,
        package_names: &[String],
    ) -> CoreResult<()> {
        Self::uninstall_packages_with_progress(config, package_names, |_| {}).await
    }

    async fn update_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        Self::update_packages_with_progress(config, package_names, |_| {}).await
    }

    async fn install_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        Self::install_packages_with_progress(config, package_names, |_| {}).await
    }
}

impl PacmanManager {
    pub async fn list_updates_with_refresh(
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        let path = command_path(config);

        if refresh {
            let args = vec![path.clone(), "-Sy".to_owned(), "--noconfirm".to_owned()];
            run_command_with_progress("pkexec", &args, |_| {}).await?;
        }

        let output = Command::new(&path).arg("-Qu").output().await?;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stdout.trim().is_empty() && stderr.trim().is_empty() {
                return Ok(Vec::new());
            }

            return Err(CoreError::CommandError(format!(
                "pacman -Qu failed: {}",
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let updates = stdout
            .lines()
            .filter_map(parse_update_line)
            .collect::<Vec<_>>();

        Ok(updates)
    }

    pub async fn uninstall_packages_with_progress(
        config: &Config,
        package_names: &[String],
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        if package_names.is_empty() {
            return Ok(());
        }

        let path = command_path(config);

        let mut args = vec![path, "-R".to_owned(), "--noconfirm".to_owned()];
        args.extend(package_names.iter().cloned());

        run_command_with_progress("pkexec", &args, on_progress).await
    }

    pub async fn uninstall_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        Self::uninstall_packages_with_progress(config, &[package_name.to_owned()], on_progress)
            .await
    }

    pub async fn update_packages_with_progress(
        config: &Config,
        package_names: &[String],
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        if package_names.is_empty() {
            return Ok(());
        }

        let path = command_path(config);

        let mut args = vec![
            path,
            "-S".to_owned(),
            "--needed".to_owned(),
            "--noconfirm".to_owned(),
        ];
        args.extend(package_names.iter().cloned());

        run_command_with_progress("pkexec", &args, on_progress).await
    }

    pub async fn update_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        Self::update_packages_with_progress(config, &[package_name.to_owned()], on_progress).await
    }

    pub async fn install_packages_with_progress(
        config: &Config,
        package_names: &[String],
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        if package_names.is_empty() {
            return Ok(());
        }

        let path = command_path(config);

        let mut args = vec![
            path,
            "-S".to_owned(),
            "--needed".to_owned(),
            "--noconfirm".to_owned(),
        ];
        args.extend(package_names.iter().cloned());

        run_command_with_progress("pkexec", &args, on_progress).await
    }

    pub async fn install_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        Self::install_packages_with_progress(config, &[package_name.to_owned()], on_progress).await
    }

    async fn installed_version_map(config: &Config) -> CoreResult<HashMap<String, String>> {
        let path = command_path(config);

        let output = Command::new(&path).arg("-Q").output().await?;
        if !output.status.success() {
            return Ok(HashMap::new());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut map = HashMap::new();

        for line in stdout.lines() {
            let mut parts = line.split_whitespace();
            let Some(name) = parts.next() else {
                continue;
            };
            let Some(version) = parts.next() else {
                continue;
            };

            map.insert(name.to_owned(), version.to_owned());
        }

        Ok(map)
    }
}

fn parse_update_line(line: &str) -> Option<PackageUpdate> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 || parts[2] != "->" {
        return None;
    }

    Some(PackageUpdate {
        name: parts[0].to_owned(),
        current_version: parts[1].to_owned(),
        new_version: parts[3].to_owned(),
    })
}

fn parse_search_results(output: &str) -> Vec<(String, String, Option<String>)> {
    let mut packages = Vec::new();
    let mut seen = HashSet::new();
    let mut lines = output.lines().peekable();

    while let Some(line) = lines.next() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let Some(repo_and_name) = parts.next() else {
            continue;
        };
        let Some(available_version) = parts.next() else {
            continue;
        };

        let Some((_, name)) = repo_and_name.split_once('/') else {
            continue;
        };

        if !seen.insert(name.to_owned()) {
            continue;
        }

        let description = lines
            .next_if(|next| next.starts_with(' ') || next.starts_with('\t'))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        packages.push((name.to_owned(), available_version.to_owned(), description));
    }

    packages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_update_line_accepts_standard_pacman_format() {
        let line = "linux 6.8.9.arch1-1 -> 6.8.10.arch1-1";
        let parsed = parse_update_line(line).expect("line should parse");

        assert_eq!(parsed.name, "linux");
        assert_eq!(parsed.current_version, "6.8.9.arch1-1");
        assert_eq!(parsed.new_version, "6.8.10.arch1-1");
    }

    #[test]
    fn parse_update_line_rejects_unexpected_lines() {
        assert!(parse_update_line("warning: database file for 'core' does not exist").is_none());
        assert!(parse_update_line("linux 6.8.9.arch1-1 6.8.10.arch1-1").is_none());
    }

    #[test]
    fn parse_search_results_reads_description_line() {
        let output = "core/bash 5.2.026-2\n    The GNU Bourne Again shell\nextra/fzf 0.58.0-1\n    Command-line fuzzy finder\n";

        let parsed = parse_search_results(output);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "bash");
        assert_eq!(parsed[0].1, "5.2.026-2");
        assert_eq!(parsed[0].2.as_deref(), Some("The GNU Bourne Again shell"));
        assert_eq!(parsed[1].0, "fzf");
    }
}
