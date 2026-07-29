use std::collections::{HashMap, HashSet};

use async_trait::async_trait;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    pm::{
        common::manager_command_path,
        progress::{CommandProgressEvent, run_command_with_progress},
    },
};

#[derive(Debug, Clone, Copy)]
pub struct FlatpakManager;

fn command_path(config: &Config) -> String {
    manager_command_path(config, PackageManagerType::Flatpak)
}

#[async_trait]
impl PackageManager for FlatpakManager {
    async fn list_updates(config: &Config) -> crate::CoreResult<Vec<crate::PackageUpdate>> {
        let installed_info = Self::get_all_installed_info(config).await?;
        let path = command_path(config);
        let output = tokio::process::Command::new(&path)
            .arg("remote-ls")
            .arg("--updates")
            .arg("--columns=application,branch,version,commit")
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .output()
            .await?;

        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            return Err(crate::error::CoreError::from_command_failure(format!(
                "flatpak remote-ls --updates failed: {}",
                detail.trim()
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut updates = Vec::new();
        let mut seen = HashSet::new();

        for line in stdout.lines() {
            let parts: Vec<_> = line.split('\t').map(str::trim).collect();
            let Some(app_id) = parts.first().filter(|value| !value.is_empty()) else {
                continue;
            };
            if app_id.eq_ignore_ascii_case("application") || !seen.insert((*app_id).to_owned()) {
                continue;
            }

            let Some((installed_version, installed_branch)) = installed_info.get(*app_id) else {
                continue;
            };
            let branch = parts.get(1).copied().unwrap_or("");
            let new_version = parts.get(2).copied().unwrap_or("");

            let current_version = Self::display_version(installed_version, installed_branch);
            let mut display_new_version = if new_version.is_empty() {
                format!("update available ({branch})")
            } else {
                Self::display_version(new_version, branch)
            };
            if display_new_version == current_version {
                display_new_version = format!("new build ({display_new_version})");
            }

            updates.push(PackageUpdate {
                name: (*app_id).to_owned(),
                current_version,
                new_version: display_new_version,
            });
        }

        Ok(updates)
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        let path = command_path(config);

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

        Ok(Self::display_version(&version, &branch))
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        let path = command_path(config);

        let output = tokio::process::Command::new(&path)
            .arg("list")
            .arg("--app")
            .arg("--columns=application,name,version,branch,size,origin")
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::from_command_failure(format!(
                "flatpak list --app failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut packages = Vec::new();

        for line in stdout.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 4 {
                if parts[0].trim().eq_ignore_ascii_case("application") {
                    continue;
                }

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

                let version_str = Self::display_version(&version_part, &branch);
                let size = parts.get(4).and_then(|size| {
                    let mut parts = size.split_whitespace();
                    let value = parts.next()?.parse::<f64>().ok()?;
                    let multiplier = match parts.next()?.to_ascii_uppercase().as_str() {
                        "B" | "BYTES" => 1.0,
                        "KB" | "KIB" => 1024.0,
                        "MB" | "MIB" => 1024.0 * 1024.0,
                        "GB" | "GIB" => 1024.0 * 1024.0 * 1024.0,
                        _ => return None,
                    };
                    Some((value * multiplier) as u64)
                });

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
        let path = command_path(config);

        let output = tokio::process::Command::new(&path)
            .arg("search")
            .arg("--columns=application,name,description,version,branch")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let installed_info = Self::get_all_installed_info(config).await?;
        let mut packages = Vec::new();

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<_> = line.split('\t').map(str::trim).collect();
            let Some(app_id) = parts.first().filter(|value| !value.is_empty()) else {
                continue;
            };
            if app_id.eq_ignore_ascii_case("application") {
                continue;
            }

            let version = installed_info
                .get(*app_id)
                .map(|(version, branch)| Self::display_version(version, branch))
                .unwrap_or_else(|| "Not Installed".to_owned());

            packages.push(PackageInfo {
                name: (*app_id).to_owned(),
                version,
                source: PackageManagerType::Flatpak,
                description: parts
                    .get(2)
                    .filter(|value| !value.is_empty())
                    .map(|value| (*value).to_owned()),
                size: None,
                install_date: None,
                homepage: None,
            });
        }

        Ok(packages)
    }
}

impl FlatpakManager {
    fn display_version(version: &str, branch: &str) -> String {
        if version.is_empty() {
            format!("branch: {branch}")
        } else if branch.is_empty() {
            version.to_owned()
        } else {
            format!("{version} ({branch})")
        }
    }

    pub async fn uninstall_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);

        let args = vec![
            "uninstall".to_string(),
            "-y".to_string(),
            package_name.to_owned(),
        ];

        run_command_with_progress(&path, &args, on_progress).await
    }

    pub async fn update_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);

        let args = vec![
            "update".to_string(),
            "-y".to_string(),
            package_name.to_owned(),
        ];

        run_command_with_progress(&path, &args, on_progress).await
    }

    pub async fn install_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);

        let args = vec![
            "install".to_string(),
            "-y".to_string(),
            package_name.to_owned(),
        ];

        run_command_with_progress(&path, &args, on_progress).await
    }
    async fn get_all_installed_info(
        config: &Config,
    ) -> CoreResult<HashMap<String, (String, String)>> {
        let path = command_path(config);

        let output = tokio::process::Command::new(&path)
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

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('\t').collect();
            if parts.is_empty() {
                continue;
            }

            let app_id = parts[0].trim();
            if app_id.eq_ignore_ascii_case("application") {
                continue;
            }
            if app_id.is_empty() {
                continue;
            }

            let version = parts.get(1).map(|s| s.trim()).unwrap_or("").to_string();
            let branch = parts
                .get(2)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown")
                .to_string();

            info_map.insert(app_id.to_string(), (version, branch));
        }
        Ok(info_map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a local Flatpak installation"]
    async fn test_get_all_installed_info() {
        let config = crate::Config::default();
        match FlatpakManager::get_all_installed_info(&config).await {
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
    #[ignore = "requires a local Flatpak installation"]
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
    #[ignore = "requires a local Flatpak installation"]
    async fn test_get_current_version() {
        let config = crate::Config::default();

        match FlatpakManager::get_current_version(&config, "org.freedesktop.Platform").await {
            Ok(version) => println!("Version: {}", version),
            Err(e) => eprintln!("Error: {}", e),
        }
    }
}
