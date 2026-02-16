use std::collections::HashMap;

use async_trait::async_trait;

use crate::{Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate};

#[derive(Debug, Clone, Copy)]
pub struct HomebrewManager;

#[async_trait]
impl PackageManager for HomebrewManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        /*
         * brew outdated --verbose
         * 输出格式：
         * package_name (current_version) < new_version
         * 例如：
         * git (2.43.0) < 2.44.0
         * node (20.11.0) < 20.11.1
         */
        let path = config
            .get_package_path(PackageManagerType::Homebrew)
            .unwrap_or_else(|| "brew".to_owned());

        let output = tokio::process::Command::new(&path)
            .arg("outdated")
            .arg("--verbose")
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::UnknownError(
                "brew outdated --verbose failed".into(),
            ));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut updates: Vec<PackageUpdate> = Vec::new();

        for line in stdout.lines() {
            let line = line.trim();

            if line.is_empty() {
                continue;
            }

            // 解析格式：package_name (current_version) < new_version
            if let Some((name_and_current, new_version)) = line.split_once('<') {
                let name_and_current = name_and_current.trim();
                let new_version = new_version.trim();

                // 提取包名和当前版本
                if let Some((name, current_version)) =
                    Self::parse_name_and_version(name_and_current)
                {
                    updates.push(PackageUpdate {
                        name: name.to_owned(),
                        current_version: current_version.to_owned(),
                        new_version: new_version.to_owned(),
                    });
                }
            }
        }

        Ok(updates)
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        let path = config
            .get_package_path(PackageManagerType::Homebrew)
            .unwrap_or_else(|| "brew".to_owned());

        let output = tokio::process::Command::new(&path)
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
        let path = config
            .get_package_path(PackageManagerType::Homebrew)
            .unwrap_or_else(|| "brew".to_owned());

        let output = tokio::process::Command::new(&path)
            .arg("info")
            .arg("--json=v2")
            .arg("--installed")
            .output()
            .await?;

        if !output.status.success() {
            // Fallback to basic info
            let installed_info = Self::get_all_installed_info().await?;
            return Ok(installed_info
                .into_iter()
                .map(|(name, version)| PackageInfo {
                    name,
                    version,
                    source: PackageManagerType::Homebrew,
                    description: None,
                    size: None,
                    install_date: None,
                    homepage: None,
                })
                .collect());
        }

        let json_str = String::from_utf8(output.stdout)?;
        let json: serde_json::Value = serde_json::from_str(&json_str)?;

        let mut packages = Vec::new();

        if let Some(formulae) = json["formulae"].as_array() {
            for formula in formulae {
                let name = formula["name"].as_str().unwrap_or("").to_string();
                let version = formula["versions"]["stable"]
                    .as_str()
                    .or_else(|| formula["version"].as_str())
                    .unwrap_or("unknown")
                    .to_string();
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
                let version = cask["version"].as_str().unwrap_or("unknown").to_string();
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
        let path = config
            .get_package_path(PackageManagerType::Homebrew)
            .unwrap_or_else(|| "brew".to_owned());

        let output = tokio::process::Command::new(&path)
            .arg("list")
            .output()
            .await?;

        if !output.status.success() {
            return Ok(0);
        }

        let stdout = String::from_utf8(output.stdout)?;
        Ok(stdout.lines().filter(|l| !l.trim().is_empty()).count())
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        let path = config
            .get_package_path(PackageManagerType::Homebrew)
            .unwrap_or_else(|| "brew".to_owned());

        let output = tokio::process::Command::new(&path)
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

            packages.push(PackageInfo {
                name,
                version: "Not Installed".to_string(),
                source: PackageManagerType::Homebrew,
                description: None,
                size: None,
                install_date: None,
                homepage: None,
            });
        }

        Ok(packages)
    }

    async fn uninstall_packages(
        &self,
        config: &Config,
        package_names: &[String],
    ) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Homebrew)
            .unwrap_or_else(|| "brew".to_owned());

        for name in package_names {
            let output = tokio::process::Command::new(&path)
                .arg("uninstall")
                .arg(name)
                .output()
                .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(crate::error::CoreError::UnknownError(format!(
                    "brew uninstall failed for {}: {}",
                    name, stderr
                )));
            }
        }

        Ok(())
    }

    async fn update_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Homebrew)
            .unwrap_or_else(|| "brew".to_owned());

        for name in package_names {
            let output = tokio::process::Command::new(&path)
                .arg("upgrade")
                .arg(name)
                .output()
                .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(crate::error::CoreError::UnknownError(format!(
                    "brew upgrade failed for {}: {}",
                    name, stderr
                )));
            }
        }

        Ok(())
    }

    async fn install_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Homebrew)
            .unwrap_or_else(|| "brew".to_owned());

        for name in package_names {
            let output = tokio::process::Command::new(&path)
                .arg("install")
                .arg(name)
                .output()
                .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(crate::error::CoreError::UnknownError(format!(
                    "brew install failed for {}: {}",
                    name, stderr
                )));
            }
        }

        Ok(())
    }
}

impl HomebrewManager {
    fn parse_name_and_version(s: &str) -> Option<(&str, &str)> {
        let open_paren = s.rfind('(')?;
        let close_paren = s.rfind(')')?;

        if open_paren >= close_paren {
            return None;
        }

        let name = s[..open_paren].trim();
        let version = s[open_paren + 1..close_paren].trim();

        Some((name, version))
    }

    async fn get_all_installed_info() -> CoreResult<HashMap<String, String>> {
        let output = tokio::process::Command::new("brew")
            .arg("list")
            .arg("--versions")
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::CommandError(
                "brew list --versions failed".into(),
            ));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut info_map = HashMap::new();

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // 格式：package_name version1 version2 ...
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let package_name = parts[0];
                // Take the last version
                let version = parts.last().unwrap();
                info_map.insert(package_name.to_string(), version.to_string());
            }
        }

        Ok(info_map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_name_and_version() {
        assert_eq!(
            HomebrewManager::parse_name_and_version("git (2.43.0)"),
            Some(("git", "2.43.0"))
        );

        assert_eq!(
            HomebrewManager::parse_name_and_version("node (20.11.0)"),
            Some(("node", "20.11.0"))
        );

        assert_eq!(
            HomebrewManager::parse_name_and_version("python@3.12 (3.12.1)"),
            Some(("python@3.12", "3.12.1"))
        );

        assert_eq!(
            HomebrewManager::parse_name_and_version("my package (1.0.0)"),
            Some(("my package", "1.0.0"))
        );

        assert_eq!(HomebrewManager::parse_name_and_version("invalid"), None);
        assert_eq!(HomebrewManager::parse_name_and_version("git 2.43.0"), None);
    }

    #[tokio::test]
    async fn test_get_all_installed_info() {
        match HomebrewManager::get_all_installed_info().await {
            Ok(info) => {
                println!("Found {} installed packages:", info.len());
                for (package, version) in info.iter().take(5) {
                    println!("  {}: {}", package, version);
                }
            }
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    #[tokio::test]
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
    async fn test_get_current_version() {
        let config = crate::Config::default();
        match HomebrewManager::get_current_version(&config, "git").await {
            Ok(version) => println!("Git version: {}", version),
            Err(e) => eprintln!("Error: {}", e),
        }
    }
}
