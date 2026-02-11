use async_trait::async_trait;
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
        let path = config
            .get_package_path(crate::PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let output = Command::new(&path)
            .arg("upgrade")
            .arg("--assumeno")
            .arg("--refresh")
            .arg("--setopt=best=True")
            .arg("--setopt=clean_requirements_on_remove=True")
            .output()
            .await?;

        if !output.status.success() {
            return Err(CoreError::UnknownError(
                "dnf upgrade --assumeno failed".into(),
            ));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut updates: Vec<PackageUpdate> = Vec::new();

        /*
         * dnf upgrade --assumeno --refresh
         * ...
         * Upgrading:
         *  bash.x86_64  5.2.26-1.fc43  updates  1.6 M
         * ...
         */
        let mut in_upgrading = false;
        for line in stdout.lines() {
            let line = line.trim();

            if line.is_empty() {
                if in_upgrading {
                    in_upgrading = false;
                }
                continue;
            }

            if line.starts_with("Upgrading:") {
                in_upgrading = true;
                continue;
            }

            if line.starts_with("Transaction Summary") {
                in_upgrading = false;
                continue;
            }

            if !in_upgrading {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();

            if parts.len() < 3 {
                continue;
            }

            let name = parts[0]
                .rsplit_once('.')
                .map(|(n, _)| n)
                .unwrap_or(parts[0]);

            let new_version = parts[2];

            let current_version = Self::get_current_version(config, name).await?;

            updates.push(PackageUpdate {
                name: name.to_owned(),
                current_version,
                new_version: new_version.to_owned(),
            });
        }

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
