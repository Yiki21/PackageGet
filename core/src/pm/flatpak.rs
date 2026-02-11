use std::collections::HashMap;

use async_trait::async_trait;

use crate::{Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate};

#[derive(Debug, Clone, Copy)]
pub struct FlatpakManager;

#[async_trait]
impl PackageManager for FlatpakManager {
    async fn list_updates(config: &Config) -> crate::CoreResult<Vec<crate::PackageUpdate>> {
        /*
         * flatpak list --updates --app --columns=application,version,branch --no-heading
         * org.fedoraproject.MediaWriter  5.2.9  stable
         * org.freedesktop.Platform       24.08  24.08
         */
        let path = config
            .get_package_path(crate::PackageManagerType::Flatpak)
            .unwrap_or_else(|| "flatpak".to_owned());

        let output = tokio::process::Command::new(&path)
            .arg("list")
            .arg("--updates")
            .arg("--app")
            .arg("--columns=application,version,branch")
            .arg("--no-heading")
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::UnknownError(
                "flatpak list --updates failed".into(),
            ));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let installed_info = Self::get_all_installed_info().await?;
        let mut updates: Vec<PackageUpdate> = Vec::new();

        for line in stdout.lines() {
            let line = line.trim();

            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('\t').collect();
            if parts.is_empty() {
                continue;
            }

            let app_id = parts[0];
            let new_version = parts.get(1).map(|s| s.trim()).unwrap_or("");
            let new_branch = parts.get(2).map(|s| s.trim()).unwrap_or("unknown");

            let current_version = installed_info
                .get(app_id)
                .map(|(v, b)| {
                    if v.is_empty() {
                        format!("branch: {}", b)
                    } else {
                        format!("{} ({})", v, b)
                    }
                })
                .unwrap_or_else(|| "unknown".to_string());

            let new_version_str = if new_version.is_empty() {
                format!("branch: {}", new_branch)
            } else {
                format!("{} ({})", new_version, new_branch)
            };

            updates.push(PackageUpdate {
                name: app_id.to_owned(),
                current_version,
                new_version: new_version_str,
            });
        }

        Ok(updates)
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        let path = config
            .get_package_path(crate::PackageManagerType::Flatpak)
            .unwrap_or_else(|| "flatpak".to_owned());

        let output = tokio::process::Command::new(&path)
            .arg("info")
            .arg("--show-version")
            .arg(package_name)
            .output()
            .await;

        let version = if let Ok(output) = output {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let output = tokio::process::Command::new(&path)
            .arg("info")
            .arg("--show-branch")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::UnknownError(format!(
                "Package {} not found",
                package_name
            )));
        }

        let branch = String::from_utf8(output.stdout)?.trim().to_string();

        if version.is_empty() {
            Ok(format!("branch: {}", branch))
        } else {
            Ok(format!("{} ({})", version, branch))
        }
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        let path = config
            .get_package_path(PackageManagerType::Flatpak)
            .unwrap_or_else(|| "flatpak".to_owned());

        let output = tokio::process::Command::new(&path)
            .arg("list")
            .arg("--app")
            .arg("--columns=application,name,version,branch,size,origin")
            .output()
            .await?;

        if !output.status.success() {
            let installed_info = Self::get_all_installed_info().await?;
            return Ok(installed_info
                .into_iter()
                .map(|(name, (version, branch))| {
                    let version_str = if version.is_empty() {
                        format!("branch: {}", branch)
                    } else {
                        format!("{} ({})", version, branch)
                    };
                    PackageInfo {
                        name,
                        version: version_str,
                        source: PackageManagerType::Flatpak,
                        description: None,
                        size: None,
                        install_date: None,
                        homepage: None,
                    }
                })
                .collect());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut packages = Vec::new();

        for (i, line) in stdout.lines().enumerate() {
            if i == 0 || line.trim().is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 4 {
                let app_id = parts[0].to_string();
                let description = parts
                    .get(1)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                let version_part = parts.get(2).map(|s| s.to_string()).unwrap_or_default();
                let branch = parts
                    .get(3)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "stable".to_string());

                let version_str = if version_part.is_empty() {
                    format!("branch: {}", branch)
                } else {
                    format!("{} ({})", version_part, branch)
                };

                let size = parts.get(4).and_then(|s| Self::parse_flatpak_size(s));

                packages.push(PackageInfo {
                    name: app_id,
                    version: version_str,
                    source: PackageManagerType::Flatpak,
                    description,
                    size,
                    install_date: None,
                    homepage: None,
                });
            }
        }

        Ok(packages)
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        let path = config
            .get_package_path(PackageManagerType::Flatpak)
            .unwrap_or_else(|| "flatpak".to_owned());

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

        // flatpak search 输出格式：
        // Name                     Description                         Application ID                   Version    Branch  Remotes
        for (i, line) in stdout.lines().enumerate() {
            let line = line.trim();

            // Skip header
            if i == 0 || line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                // Try to find application ID (usually contains a dot)
                let app_id = parts
                    .iter()
                    .find(|p| p.contains('.') && p.len() > 5)
                    .map(|s| s.to_string());

                if let Some(app_id) = app_id {
                    let mut version = parts
                        .get(parts.len().saturating_sub(2))
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    if version == "unknown" {
                        version.clear();
                    }

                    packages.push(PackageInfo {
                        name: app_id,
                        version,
                        source: PackageManagerType::Flatpak,
                        description: None,
                        size: None,
                        install_date: None,
                        homepage: None,
                    });
                }
            }
        }

        Ok(packages)
    }

    async fn uninstall_packages(
        &self,
        config: &Config,
        package_names: &[String],
    ) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Flatpak)
            .unwrap_or_else(|| "flatpak".to_owned());

        let mut args = vec!["uninstall".to_string(), "-y".to_string()];
        for name in package_names {
            args.push(name.clone());
        }

        let output = tokio::process::Command::new(&path)
            .args(&args)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::error::CoreError::UnknownError(format!(
                "flatpak uninstall failed: {}",
                stderr
            )));
        }

        Ok(())
    }

    async fn update_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Flatpak)
            .unwrap_or_else(|| "flatpak".to_owned());

        let mut args = vec!["update", "-y"];
        for name in package_names {
            args.push(name);
        }

        let output = tokio::process::Command::new(&path)
            .args(&args)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::error::CoreError::UnknownError(format!(
                "flatpak update failed: {}",
                stderr
            )));
        }

        Ok(())
    }

    async fn install_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Flatpak)
            .unwrap_or_else(|| "flatpak".to_owned());

        let mut args = vec!["install", "-y"];
        for name in package_names {
            args.push(name);
        }

        let output = tokio::process::Command::new(&path)
            .args(&args)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::error::CoreError::UnknownError(format!(
                "flatpak install failed: {}",
                stderr
            )));
        }

        Ok(())
    }
}

impl FlatpakManager {
    /// Parse Flatpak size string (e.g., "123.4 MB" or "1.2 GB")
    fn parse_flatpak_size(size_str: &str) -> Option<u64> {
        let parts: Vec<&str> = size_str.split_whitespace().collect();
        if parts.len() != 2 {
            return None;
        }

        let value: f64 = parts[0].parse().ok()?;
        let multiplier = match parts[1].to_uppercase().as_str() {
            "B" | "BYTES" => 1.0,
            "KB" | "KIB" => 1024.0,
            "MB" | "MIB" => 1024.0 * 1024.0,
            "GB" | "GIB" => 1024.0 * 1024.0 * 1024.0,
            _ => return None,
        };

        Some((value * multiplier) as u64)
    }

    #[allow(dead_code)]
    fn extract_version_and_branch(parts: &[&str]) -> (String, String) {
        if parts.is_empty() {
            return (String::new(), "unknown".to_string());
        }

        let first = parts[0];

        if first.matches('.').count() >= 2 {
            let version = first.to_string();
            let branch = parts
                .get(1)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "stable".to_string());
            (version, branch)
        } else {
            (String::new(), first.to_string())
        }
    }

    async fn get_all_installed_info() -> CoreResult<HashMap<String, (String, String)>> {
        let output = tokio::process::Command::new("flatpak")
            .arg("list")
            .arg("--columns=application,version,branch")
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::CommandError(
                "flatpak list failed".into(),
            ));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut info_map = HashMap::new();

        for (i, line) in stdout.lines().enumerate() {
            if i == 0 {
                continue;
            }

            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts.len() {
                3 => {
                    info_map.insert(
                        parts[0].to_string(),
                        (parts[1].to_string(), parts[2].to_string()),
                    );
                }
                2 => {
                    info_map.insert(parts[0].to_string(), (String::new(), parts[1].to_string()));
                }
                1 => {
                    info_map.insert(parts[0].to_string(), (String::new(), "unknown".to_string()));
                }
                _ => continue,
            }
        }
        Ok(info_map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_version_and_branch() {
        let (v, b) =
            FlatpakManager::extract_version_and_branch(&["5.2.9", "stable", "x86_64", "fedora"]);
        assert_eq!(v, "5.2.9");
        assert_eq!(b, "stable");

        let (v, b) = FlatpakManager::extract_version_and_branch(&[
            "freedesktop-sdk-24.08.28",
            "24.08",
            "x86_64",
            "flathub",
        ]);
        assert_eq!(v, "freedesktop-sdk-24.08.28");
        assert_eq!(b, "24.08");

        // 没有版本号的情况（少于3个点）
        let (v, b) = FlatpakManager::extract_version_and_branch(&["24.08", "x86_64", "flathub"]);
        assert_eq!(v, "");
        assert_eq!(b, "24.08");

        let (v, b) = FlatpakManager::extract_version_and_branch(&["f43", "x86_64", "fedora"]);
        assert_eq!(v, "");
        assert_eq!(b, "f43");

        let (v, b) = FlatpakManager::extract_version_and_branch(&["stable", "x86_64", "flathub"]);
        assert_eq!(v, "");
        assert_eq!(b, "stable");
    }

    #[tokio::test]
    async fn test_get_all_installed_info() {
        match FlatpakManager::get_all_installed_info().await {
            Ok(info) => {
                println!("Found {} installed packages:", info.len());
                for (app_id, (version, branch)) in info.iter().take(5) {
                    if version.is_empty() {
                        println!("  {}: branch {}", app_id, branch);
                    } else {
                        println!("  {}: {} ({})", app_id, version, branch);
                    }
                }
            }
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    #[tokio::test]
    async fn test_flatpak_list_updates() {
        let config = crate::Config::default();
        match FlatpakManager::list_updates(&config).await {
            Ok(updates) => {
                println!("\nFound {} Flatpak updates:", updates.len());
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

        match FlatpakManager::get_current_version(&config, "org.freedesktop.Platform").await {
            Ok(version) => println!("Version: {}", version),
            Err(e) => eprintln!("Error: {}", e),
        }
    }
}
