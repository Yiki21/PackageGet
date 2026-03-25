use std::{
    collections::{HashMap, HashSet},
    process::Stdio,
};

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
        Self::list_updates_via_update(config, &installed_info).await
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

        if version.is_empty() {
            Ok(format!("branch: {}", branch))
        } else {
            Ok(format!("{} ({})", version, branch))
        }
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
            let installed_info = Self::get_all_installed_info(config).await?;
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
        let path = command_path(config);

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
                    let version = Self::get_current_version(config, &app_id)
                        .await
                        .unwrap_or_else(|_| "Not Installed".to_string());

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
        for package_name in package_names {
            Self::uninstall_package_with_progress(config, package_name, |_| {}).await?;
        }
        Ok(())
    }

    async fn update_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        for package_name in package_names {
            Self::update_package_with_progress(config, package_name, |_| {}).await?;
        }
        Ok(())
    }

    async fn install_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        for package_name in package_names {
            Self::install_package_with_progress(config, package_name, |_| {}).await?;
        }
        Ok(())
    }
}

impl FlatpakManager {
    async fn list_updates_via_update(
        config: &Config,
        installed_info: &HashMap<String, (String, String)>,
    ) -> CoreResult<Vec<PackageUpdate>> {
        let path = command_path(config);
        let output = tokio::process::Command::new(&path)
            .arg("update")
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let combined_output = match (stdout.trim(), stderr.trim()) {
            ("", "") => String::new(),
            ("", _) => stderr.clone(),
            (_, "") => stdout.clone(),
            _ => format!("{stdout}\n{stderr}"),
        };

        let updates = Self::parse_updates_from_update_output(&combined_output, installed_info);
        if !updates.is_empty() || output.status.success() {
            return Ok(updates);
        }

        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        Err(crate::error::CoreError::UnknownError(
            if detail.is_empty() {
                "flatpak update failed".to_string()
            } else {
                format!("flatpak update failed: {}", detail)
            },
        ))
    }

    fn parse_updates_from_update_output(
        output: &str,
        installed_info: &HashMap<String, (String, String)>,
    ) -> Vec<PackageUpdate> {
        let mut updates = Vec::new();
        let mut seen = HashSet::new();

        for line in output.lines() {
            let Some((app_id, branch)) = Self::parse_update_listing_line(line) else {
                continue;
            };

            if !seen.insert(app_id.to_string()) {
                continue;
            }

            if let Some(update) =
                Self::build_package_update(app_id, "", branch, None, installed_info)
            {
                updates.push(update);
            }
        }

        updates
    }

    fn parse_update_listing_line(line: &str) -> Option<(&str, &str)> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 || !Self::is_numbered_update_row(parts[0]) {
            return None;
        }

        let mut index = 1;
        if parts
            .get(index)
            .is_some_and(|part| part.starts_with('[') && part.ends_with(']'))
        {
            index += 1;
        }

        let app_id = *parts.get(index)?;
        if !app_id.contains('.') {
            return None;
        }

        let branch = *parts.get(index + 1)?;
        let op = *parts.get(index + 2)?;
        if op.len() != 1 || !op.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }

        Some((app_id, branch))
    }

    fn is_numbered_update_row(part: &str) -> bool {
        let Some(number) = part.strip_suffix('.') else {
            return false;
        };

        !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
    }

    fn build_package_update(
        app_id: &str,
        new_version: &str,
        new_branch: &str,
        new_commit: Option<&str>,
        installed_info: &HashMap<String, (String, String)>,
    ) -> Option<PackageUpdate> {
        let (installed_version, installed_branch) = installed_info.get(app_id)?;
        let current_version = if installed_version.is_empty() {
            format!("branch: {}", installed_branch)
        } else {
            format!("{} ({})", installed_version, installed_branch)
        };

        let new_version_str = if !new_version.is_empty() {
            if new_branch.is_empty() {
                new_version.to_string()
            } else {
                format!("{} ({})", new_version, new_branch)
            }
        } else if !new_branch.is_empty() {
            format!("update available ({})", new_branch)
        } else if let Some(commit) = new_commit.filter(|c| !c.is_empty()) {
            format!("commit: {}", commit)
        } else {
            "unknown".to_string()
        };

        Some(PackageUpdate {
            name: app_id.to_owned(),
            current_version,
            new_version: new_version_str,
        })
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

        for (index, line) in stdout.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split('\t').collect();
            if parts.is_empty() {
                continue;
            }

            let app_id = parts[0].trim();
            if index == 0 && app_id.eq_ignore_ascii_case("application") {
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

    #[test]
    fn test_parse_update_listing_line_handles_terminal_output() {
        let parsed = FlatpakManager::parse_update_listing_line(
            " 1. [✓] org.gnome.Platform.Locale 49 u flathub 18.5 kB / 385.5 MB",
        );
        assert_eq!(parsed, Some(("org.gnome.Platform.Locale", "49")));
    }

    #[test]
    fn test_parse_update_listing_line_handles_non_tty_output() {
        let parsed = FlatpakManager::parse_update_listing_line(
            " 3.\t\torg.freedesktop.Platform.Locale\t25.08\tu\tflathub\t< 378.7 MB",
        );
        assert_eq!(parsed, Some(("org.freedesktop.Platform.Locale", "25.08")));
    }

    #[tokio::test]
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
