use std::collections::HashMap;

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
pub struct AptManager;

fn command_path(config: &Config) -> String {
    manager_command_path(config, PackageManagerType::Apt)
}

#[async_trait]
impl PackageManager for AptManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        Self::list_updates_with_refresh(config, false).await
    }

    async fn get_current_version(_config: &Config, package_name: &str) -> CoreResult<String> {
        let output = Command::new("dpkg-query")
            .arg("-W")
            .arg("-f=${Version}")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Err(CoreError::ParseError(format!(
                "Package {} not found",
                package_name
            )));
        }

        let version = String::from_utf8(output.stdout)?.trim().to_owned();
        if version.is_empty() {
            return Err(CoreError::ParseError(format!(
                "Package {} has empty version",
                package_name
            )));
        }

        Ok(version)
    }

    async fn list_installed(_config: &Config) -> CoreResult<Vec<PackageInfo>> {
        let output = Command::new("dpkg-query")
            .arg("-W")
            .arg("-f=${binary:Package}\t${Version}\t${binary:Summary}\n")
            .output()
            .await?;

        if !output.status.success() {
            return Err(CoreError::UnknownError("dpkg-query -W failed".to_string()));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let packages = stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() < 2 {
                    return None;
                }

                let description = parts
                    .get(2)
                    .map(|desc| desc.trim())
                    .filter(|desc| !desc.is_empty())
                    .map(ToOwned::to_owned);

                Some(PackageInfo {
                    name: parts[0].trim().to_owned(),
                    version: parts[1].trim().to_owned(),
                    source: PackageManagerType::Apt,
                    description,
                    size: None,
                    install_date: None,
                    homepage: None,
                })
            })
            .collect();

        Ok(packages)
    }

    async fn count_installed(_config: &Config) -> CoreResult<usize> {
        let output = Command::new("dpkg-query")
            .arg("-W")
            .arg("-f=${binary:Package}\n")
            .output()
            .await?;

        if !output.status.success() {
            return Ok(Self::list_installed(_config).await?.len());
        }

        let stdout = String::from_utf8(output.stdout)?;
        Ok(stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count())
    }

    async fn search_package(_config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        let output = Command::new("apt-cache")
            .arg("search")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let installed_versions = Self::installed_version_map().await?;

        let packages = stdout
            .lines()
            .filter_map(|line| {
                let (name, description) = line.split_once(" - ")?;
                let name = name.trim();
                if name.is_empty() {
                    return None;
                }

                Some(PackageInfo {
                    name: name.to_owned(),
                    version: installed_versions
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| "Not Installed".to_owned()),
                    source: PackageManagerType::Apt,
                    description: Some(description.trim().to_owned()),
                    size: None,
                    install_date: None,
                    homepage: None,
                })
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

impl AptManager {
    pub async fn list_updates_with_refresh(
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        let path = command_path(config);

        if refresh {
            let args = vec![path.clone(), "update".to_owned()];
            run_command_with_progress("pkexec", &args, |_| {}).await?;
        }

        let output = Command::new(&path)
            .arg("list")
            .arg("--upgradable")
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CoreError::CommandError(format!(
                "apt list --upgradable failed: {}",
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut updates = Vec::new();

        for line in stdout.lines() {
            let Some((name, mut current_version, new_version)) = parse_upgradable_line(line) else {
                continue;
            };

            if current_version == "unknown" {
                current_version = Self::get_current_version(config, &name)
                    .await
                    .unwrap_or_else(|_| "unknown".to_owned());
            }

            updates.push(PackageUpdate {
                name,
                current_version,
                new_version,
            });
        }

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
        let mut args = vec![path, "remove".to_owned(), "-y".to_owned()];
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
            "install".to_owned(),
            "-y".to_owned(),
            "--only-upgrade".to_owned(),
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
        let mut args = vec![path, "install".to_owned(), "-y".to_owned()];
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

    async fn installed_version_map() -> CoreResult<HashMap<String, String>> {
        let output = Command::new("dpkg-query")
            .arg("-W")
            .arg("-f=${binary:Package}\t${Version}\n")
            .output()
            .await?;

        if !output.status.success() {
            return Ok(HashMap::new());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut map = HashMap::new();

        for line in stdout.lines() {
            let Some((name, version)) = line.split_once('\t') else {
                continue;
            };
            let name = name.trim();
            let version = version.trim();
            if !name.is_empty() && !version.is_empty() {
                map.insert(name.to_owned(), version.to_owned());
            }
        }

        Ok(map)
    }
}

fn parse_upgradable_line(line: &str) -> Option<(String, String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("Listing...") {
        return None;
    }

    let (name, rest) = line.split_once('/')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let mut parts = rest.split_whitespace();
    let _distribution = parts.next()?;
    let new_version = parts.next()?.to_owned();

    let current_version = parse_upgradable_from(line).unwrap_or_else(|| "unknown".to_owned());

    Some((name.to_owned(), current_version, new_version))
}

fn parse_upgradable_from(line: &str) -> Option<String> {
    let marker = "[upgradable from: ";
    let start = line.find(marker)? + marker.len();
    let end = line[start..].find(']')? + start;
    let value = line[start..end].trim();
    if value.is_empty() {
        return None;
    }

    Some(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_upgradable_line_extracts_name_and_versions() {
        let line = "bash/jammy-updates 5.1-6ubuntu1.1 amd64 [upgradable from: 5.1-6ubuntu1]";
        let parsed = parse_upgradable_line(line).expect("line should parse");

        assert_eq!(parsed.0, "bash");
        assert_eq!(parsed.1, "5.1-6ubuntu1");
        assert_eq!(parsed.2, "5.1-6ubuntu1.1");
    }

    #[test]
    fn parse_upgradable_line_handles_missing_current_version_marker() {
        let line = "vim/stable 2:9.1.1234 amd64";
        let parsed = parse_upgradable_line(line).expect("line should parse");

        assert_eq!(parsed.0, "vim");
        assert_eq!(parsed.1, "unknown");
        assert_eq!(parsed.2, "2:9.1.1234");
    }

    #[test]
    fn parse_upgradable_line_skips_listing_header() {
        assert!(parse_upgradable_line("Listing...").is_none());
    }
}
