use std::time::Duration;

use async_trait::async_trait;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    pm::{
        common::{manager_command, manager_command_path},
        progress::{CommandProgressEvent, run_command_with_progress},
    },
};

#[derive(Debug, Clone, Copy)]
pub struct HomebrewManager;

fn command_path(config: &Config) -> String {
    manager_command_path(config, PackageManagerType::Homebrew)
}

#[async_trait]
impl PackageManager for HomebrewManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        Self::list_updates_with_refresh(config, false).await
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        let path = command_path(config);

        let output = manager_command(&path)
            .arg("list")
            .arg("--versions")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::UnknownError(format!(
                "brew list --versions {} failed",
                package_name
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let line = stdout.trim();

        if line.is_empty() {
            return Err(crate::error::CoreError::UnknownError(format!(
                "Package {} not found",
                package_name
            )));
        }

        // 输出格式：package_name version1 version2 ...
        // 取最后一个版本（最新安装的版本）
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            Ok(parts.last().unwrap().to_string())
        } else {
            Err(crate::error::CoreError::UnknownError(
                "Failed to parse version".into(),
            ))
        }
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        let path = command_path(config);

        let output = manager_command(&path)
            .arg("info")
            .arg("--json=v2")
            .arg("--installed")
            .output()
            .await?;

        if !output.status.success() {
            let fallback = manager_command(&path)
                .arg("list")
                .arg("--versions")
                .output()
                .await?;
            if !fallback.status.success() {
                return Err(crate::error::CoreError::from_command_failure(format!(
                    "brew list --versions failed: {}",
                    String::from_utf8_lossy(&fallback.stderr).trim()
                )));
            }

            let stdout = String::from_utf8(fallback.stdout)?;
            return Ok(stdout
                .lines()
                .filter_map(|line| {
                    let mut parts = line.split_whitespace();
                    let name = parts.next()?.to_owned();
                    let version = parts.collect::<Vec<_>>().join(" ");
                    (!version.is_empty()).then_some(PackageInfo {
                        name,
                        version,
                        source: PackageManagerType::Homebrew,
                        description: None,
                        size: None,
                        install_date: None,
                        homepage: None,
                    })
                })
                .collect());
        }

        let json_str = String::from_utf8(output.stdout)?;
        let json: serde_json::Value = serde_json::from_str(&json_str)?;

        let mut packages = Vec::new();

        if let Some(formulae) = json["formulae"].as_array() {
            for formula in formulae {
                let name = formula["name"].as_str().unwrap_or("").to_string();
                let installed_versions = formula["installed"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|installed| installed["version"].as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let version = if installed_versions.is_empty() {
                    formula["versions"]["stable"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_owned()
                } else {
                    installed_versions
                };
                let description = formula["desc"].as_str().map(|s| s.to_string());
                let homepage = formula["homepage"].as_str().map(|s| s.to_string());

                packages.push(PackageInfo {
                    name,
                    version,
                    source: PackageManagerType::Homebrew,
                    description,
                    size: None,
                    install_date: None,
                    homepage,
                });
            }
        }

        if let Some(casks) = json["casks"].as_array() {
            for cask in casks {
                let name = cask["token"].as_str().unwrap_or("").to_string();
                let version = cask["installed"]
                    .as_str()
                    .or_else(|| cask["version"].as_str())
                    .unwrap_or("unknown")
                    .to_owned();
                let description = cask["desc"].as_str().map(|s| s.to_string());
                let homepage = cask["homepage"].as_str().map(|s| s.to_string());

                packages.push(PackageInfo {
                    name,
                    version,
                    source: PackageManagerType::Homebrew,
                    description,
                    size: None,
                    install_date: None,
                    homepage,
                });
            }
        }

        Ok(packages)
    }

    async fn count_installed(config: &Config) -> CoreResult<usize> {
        let path = command_path(config);

        let output = manager_command(&path).arg("list").output().await?;

        if !output.status.success() {
            return Ok(0);
        }

        let stdout = String::from_utf8(output.stdout)?;
        Ok(stdout.lines().filter(|l| !l.trim().is_empty()).count())
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        let path = command_path(config);

        let output = manager_command(&path)
            .arg("search")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut packages = Vec::new();

        // brew search 输出格式：每行一个包名
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('=') {
                continue;
            }

            let name = line.to_string();
            let version = Self::get_current_version(config, &name)
                .await
                .unwrap_or_else(|_| "Not Installed".to_string());

            packages.push(PackageInfo {
                name,
                version,
                source: PackageManagerType::Homebrew,
                description: None,
                size: None,
                install_date: None,
                homepage: None,
            });
        }

        Ok(packages)
    }
}

impl HomebrewManager {
    pub async fn list_updates_with_refresh(
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        let path = command_path(config);

        if refresh {
            let output = tokio::time::timeout(
                Duration::from_secs(180),
                manager_command(&path).arg("update").output(),
            )
            .await
            .map_err(|_| {
                crate::error::CoreError::CommandError(
                    "brew update timed out after 180 seconds".to_owned(),
                )
            })??;
            if !output.status.success() {
                return Err(crate::error::CoreError::from_command_failure(format!(
                    "brew update failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }

        let output = tokio::time::timeout(
            Duration::from_secs(90),
            manager_command(&path)
                .arg("outdated")
                .arg("--verbose")
                .env("HOMEBREW_NO_AUTO_UPDATE", "1")
                .output(),
        )
        .await
        .map_err(|_| {
            crate::error::CoreError::CommandError(
                "brew outdated timed out after 90 seconds".to_owned(),
            )
        })??;

        if !output.status.success() {
            return Err(crate::error::CoreError::from_command_failure(format!(
                "brew outdated --verbose failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut updates = Vec::new();

        for line in stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let Some((name_and_current, new_version)) =
                line.split_once(" < ").or_else(|| line.split_once(" != "))
            else {
                continue;
            };
            let Some((name, current_version)) = name_and_current.rsplit_once(" (") else {
                continue;
            };
            let Some(current_version) = current_version.strip_suffix(')') else {
                continue;
            };

            updates.push(PackageUpdate {
                name: name.to_owned(),
                current_version: current_version.to_owned(),
                new_version: new_version.to_owned(),
            });
        }

        Ok(updates)
    }

    pub async fn uninstall_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);

        let args = vec!["uninstall".to_string(), package_name.to_owned()];

        run_command_with_progress(&path, &args, on_progress).await
    }

    pub async fn update_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);

        let args = vec!["upgrade".to_string(), package_name.to_owned()];

        run_command_with_progress(&path, &args, on_progress).await
    }

    pub async fn install_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);

        let args = vec!["install".to_string(), package_name.to_owned()];

        run_command_with_progress(&path, &args, on_progress).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a local Homebrew installation"]
    async fn test_homebrew_list_updates() {
        let config = crate::Config::default();
        match HomebrewManager::list_updates(&config).await {
            Ok(updates) => {
                println!("\nFound {} Homebrew updates:", updates.len());
                for update in updates {
                    println!("  {}", update.name);
                    println!("    Current: {}", update.current_version);
                    println!("    New:     {}", update.new_version);
                }
            }
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    #[tokio::test]
    #[ignore = "requires a local Homebrew installation"]
    async fn test_get_current_version() {
        let config = crate::Config::default();
        match HomebrewManager::get_current_version(&config, "git").await {
            Ok(version) => println!("Git version: {}", version),
            Err(e) => eprintln!("Error: {}", e),
        }
    }
}
