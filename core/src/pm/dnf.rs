use async_trait::async_trait;
use log::debug;
use std::collections::HashSet;
use tokio::process::Command;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    error::CoreError,
};

#[derive(Debug, Clone, Copy)]
pub struct DnfManager;

#[async_trait]
impl PackageManager for DnfManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        debug!("Starting dnf list_updates");
        let path = config
            .get_package_path(crate::PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let output = Command::new(&path)
            .arg("check-upgrade")
            .output()
            .await?;

        let stdout = String::from_utf8(output.stdout)?;
        debug!("dnf check-upgrade exited: {}", output.status);
        debug!("dnf output size: {} bytes", stdout.len());

        let mut updates: Vec<PackageUpdate> = Vec::new();
        let mut seen_packages: HashSet<String> = HashSet::new();

        /*
         * dnf check-upgrade output format:
         * package-name.arch  version  repository
         * cosmic-app-library.x86_64  1.0.6^git20260209.a9da1de-1.fc43  copr:...
         */
        for line in stdout.lines() {
            let line = line.trim();

            // Skip empty lines and header lines
            if line.is_empty() 
                || line.starts_with("Updating and loading repositories:")
                || line.starts_with("Repositories loaded.")
            {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            
            // Expected format: name.arch version repository
            if parts.len() < 2 {
                continue;
            }

            let name = parts[0]
                .rsplit_once('.')
                .map(|(n, _)| n)
                .unwrap_or(parts[0]);

            // Skip if we've already processed this package (handle duplicates)
            if seen_packages.contains(name) {
                debug!("Skipping duplicate package: {}", name);
                continue;
            }
            seen_packages.insert(name.to_string());

            let new_version = parts[1];

            // Get current version, but don't fail entire function if one package fails
            let current_version = Self::get_current_version(config, name)
                .await
                .unwrap_or_else(|e| {
                    debug!("Failed to get current version for {}: {}", name, e);
                    "unknown".to_string()
                });

            debug!("Found update: {}: {} -> {}", name, current_version, new_version);

            updates.push(PackageUpdate {
                name: name.to_owned(),
                current_version,
                new_version: new_version.to_owned(),
            });
        }

        debug!("Total updates found: {}", updates.len());
        Ok(updates)
    }

    async fn get_current_version(_config: &Config, package_name: &str) -> CoreResult<String> {
        let output = Command::new("rpm")
            .arg("-q")
            .arg("--queryformat")
            .arg("%{VERSION}-%{RELEASE}")
            .arg(package_name)
            .output()
            .await?;

        if output.status.success() {
            Ok(String::from_utf8(output.stdout)?.trim().to_string())
        } else {
            Err(CoreError::ParseError(format!(
                "Package {} not found",
                package_name
            )))
        }
    }

    async fn list_installed(_config: &Config) -> CoreResult<Vec<PackageInfo>> {
        // use rpm -qa to list installed packages
        let output = Command::new("rpm")
            .arg("-qa")
            .arg("--queryformat")
            .arg("%{NAME}\t%{VERSION}-%{RELEASE}\t%{SUMMARY}\t%{SIZE}\t%{INSTALLTIME}\t%{URL}\n")
            .output()
            .await?;

        if !output.status.success() {
            return Err(CoreError::UnknownError("rpm -qa failed".into()));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let packages = stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('\t').collect();
                if parts.len() >= 2 {
                    let description = parts
                        .get(2)
                        .filter(|s| !s.is_empty() && **s != "(none)")
                        .map(|s| s.to_string());

                    let size = parts.get(3).and_then(|s| s.parse::<u64>().ok());

                    let install_date = parts
                        .get(4)
                        .filter(|s| !s.is_empty() && **s != "(none)")
                        .and_then(|timestamp| {
                            timestamp.parse::<i64>().ok().and_then(|ts| {
                                let datetime = chrono::DateTime::from_timestamp(ts, 0)?;
                                Some(datetime.format("%Y-%m-%d %H:%M:%S").to_string())
                            })
                        });

                    let homepage = parts
                        .get(5)
                        .filter(|s| !s.is_empty() && **s != "(none)")
                        .map(|s| s.to_string());

                    Some(PackageInfo {
                        name: parts[0].to_string(),
                        version: parts[1].to_string(),
                        source: PackageManagerType::Dnf,
                        description,
                        size,
                        install_date,
                        homepage,
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(packages)
    }

    async fn count_installed(_config: &Config) -> CoreResult<usize> {
        let output = Command::new("sh")
            .arg("-c")
            .arg("rpm -qa | wc -l")
            .output()
            .await?;

        if !output.status.success() {
            return Ok(Self::list_installed(_config).await?.len());
        }

        let count_str = String::from_utf8(output.stdout)?.trim().to_string();

        count_str
            .parse::<usize>()
            .map_err(|e| CoreError::ParseError(format!("Failed to parse count: {}", e)))
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        let path = config
            .get_package_path(PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let output = Command::new(&path)
            .arg("search")
            .arg("--quiet")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut packages = Vec::new();

        debug!("dnf search output size: {} bytes", stdout.len());

        // dnf search 输出格式：
        // package-name.arch : Summary
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() || !line.contains(':') {
                continue;
            }

            if let Some((name_part, _summary)) = line.split_once(':') {
                let name_part = name_part.trim();
                if name_part.contains(' ') || name_part.starts_with('=') {
                    continue;
                }

                let name = name_part
                    .rsplit_once('.')
                    .map(|(n, _)| n)
                    .unwrap_or(name_part)
                    .to_string();

                // 尝试获取版本信息
                let mut version = Self::get_current_version(config, &name)
                    .await
                    .unwrap_or_default();
                if version == "unknown" {
                    version.clear();
                }

                packages.push(PackageInfo {
                    name,
                    version,
                    source: PackageManagerType::Dnf,
                    description: None,
                    size: None,
                    install_date: None,
                    homepage: None,
                });
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
            .get_package_path(PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let mut args = vec![path.clone(), "remove".to_string(), "-y".to_string()];
        for name in package_names {
            args.push(name.clone());
        }

        let output = Command::new("pkexec").args(&args).output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CoreError::UnknownError(format!(
                "pkexec dnf remove failed: {}",
                stderr
            )));
        }

        Ok(())
    }

    async fn update_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let mut args = vec![path.clone(), "upgrade".to_string(), "-y".to_string()];
        for name in package_names {
            args.push(name.clone());
        }

        let output = Command::new("pkexec").args(&args).output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CoreError::UnknownError(format!(
                "pkexec dnf upgrade failed: {}",
                stderr
            )));
        }

        Ok(())
    }

    async fn install_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let mut args = vec![path.clone(), "install".to_string(), "-y".to_string()];
        for name in package_names {
            args.push(name.clone());
        }

        let output = Command::new("pkexec").args(&args).output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CoreError::UnknownError(format!(
                "pkexec dnf install failed: {}",
                stderr
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dnf_list_updates() {
        let config = crate::Config::default();
        match DnfManager::list_updates(&config).await {
            Ok(updates) => {
                println!("Found {} updates:", updates.len());
                for update in updates.iter().take(5) {
                    println!(
                        "  {}: {} -> {}",
                        update.name, update.current_version, update.new_version
                    );
                }
            }
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    #[tokio::test]
    async fn test_dnf_get_current_version() {
        let config = crate::Config::default();
        let package_name = "bash"; // Common package
        match DnfManager::get_current_version(&config, package_name).await {
            Ok(version) => println!("Current version of {}: {}", package_name, version),
            Err(e) => eprintln!("Error: {}", e),
        }
    }
}
