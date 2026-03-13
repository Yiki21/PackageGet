use async_trait::async_trait;
use log::debug;
use std::{collections::HashSet, process::ExitStatus};
use tokio::process::Command;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    error::CoreError,
    pm::progress::{CommandProgressEvent, run_command_with_progress},
};

#[derive(Debug, Clone, Copy)]
pub struct DnfManager;

#[async_trait]
impl PackageManager for DnfManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        Self::list_updates_with_refresh(config, false).await
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
        let mut seen_packages = HashSet::new();

        debug!("Starting dnf search_package");
        debug!("dnf search output size: {} bytes", stdout.len());

        // dnf search 输出格式：
        // Matched fields: name, summary
        // package-name.arch<TAB>Summary description
        for line in stdout.lines() {
            let line = line.trim();

            // 跳过头部行和空行
            if line.is_empty() || line.starts_with("Matched fields:") {
                continue;
            }

            // DNF 使用 tab 分隔包名和描述
            if let Some((name_part, _summary)) = line.split_once('\t') {
                let name_part = name_part.trim();

                // 移除架构后缀 (如 .x86_64, .noarch)
                let name = name_part
                    .rsplit_once('.')
                    .map(|(n, _)| n)
                    .unwrap_or(name_part)
                    .to_string();

                if !seen_packages.insert(name.clone()) {
                    continue;
                }

                let version = Self::get_current_version(config, &name)
                    .await
                    .unwrap_or_else(|_| "Not Installed".to_string());

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

        packages.sort_by(|a, b| a.name.cmp(&b.name));

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

impl DnfManager {
    pub async fn list_updates_with_refresh(
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        debug!("Starting dnf list_updates (refresh={})", refresh);
        let path = config
            .get_package_path(crate::PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let (program, args) = build_check_upgrade_command(&path, refresh);
        let output = Command::new(&program).args(&args).output().await?;

        if !is_check_upgrade_status_ok(&output.status) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            let detail = if stderr.is_empty() {
                "no stderr output".to_string()
            } else {
                stderr.to_string()
            };
            return Err(CoreError::CommandError(format!(
                "dnf check-upgrade failed with status {:?}: {}",
                output.status.code(),
                detail
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        debug!("dnf check-upgrade exited: {}", output.status);
        debug!("dnf output size: {} bytes", stdout.len());

        let mut updates: Vec<PackageUpdate> = Vec::new();
        let mut seen_packages: HashSet<String> = HashSet::new();

        for raw_line in stdout.lines() {
            let Some((name, new_version)) = parse_check_upgrade_entry(raw_line) else {
                continue;
            };

            // Skip if we've already processed this package (handle duplicates)
            if seen_packages.contains(name) {
                debug!("Skipping duplicate package: {}", name);
                continue;
            }
            seen_packages.insert(name.to_string());

            // Get current version, but don't fail entire function if one package fails
            let current_version = Self::get_current_version(config, name)
                .await
                .unwrap_or_else(|e| {
                    debug!("Failed to get current version for {}: {}", name, e);
                    "unknown".to_string()
                });

            debug!(
                "Found update: {}: {} -> {}",
                name, current_version, new_version
            );

            updates.push(PackageUpdate {
                name: name.to_owned(),
                current_version,
                new_version: new_version.to_owned(),
            });
        }

        debug!("Total updates found: {}", updates.len());
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

        let path = config
            .get_package_path(PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let mut args = vec![path, "remove".to_string(), "-y".to_string()];
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

        let path = config
            .get_package_path(PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let mut args = vec![
            path,
            "upgrade".to_string(),
            "-y".to_string(),
            "--skip-unavailable".to_string(),
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

        let path = config
            .get_package_path(PackageManagerType::Dnf)
            .unwrap_or_else(|| "dnf".to_owned());

        let mut args = vec![path, "install".to_string(), "-y".to_string()];
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
}

fn build_check_upgrade_command(path: &str, refresh: bool) -> (String, Vec<String>) {
    if refresh {
        return (
            "pkexec".to_string(),
            vec![
                path.to_string(),
                "check-upgrade".to_string(),
                "--refresh".to_string(),
            ],
        );
    }

    (path.to_string(), vec!["check-upgrade".to_string()])
}

fn is_check_upgrade_status_ok(status: &ExitStatus) -> bool {
    // dnf check-upgrade returns code 100 when updates are available.
    status.success() || status.code() == Some(100)
}

fn parse_check_upgrade_entry(raw_line: &str) -> Option<(&str, &str)> {
    // Obsoleted package rows are indented and should not be treated as direct upgrades.
    if raw_line
        .chars()
        .next()
        .is_some_and(|first| first.is_whitespace())
    {
        return None;
    }

    let line = raw_line.trim();
    if line.is_empty()
        || line.starts_with("Updating and loading repositories:")
        || line.starts_with("Repositories loaded.")
        || line.starts_with("Available upgrades")
        || line.starts_with("Obsoleting packages")
    {
        return None;
    }

    // Expected format: name.arch version repository
    let mut parts = line.split_whitespace();
    let package_with_arch = parts.next()?;
    let new_version = parts.next()?;
    let _repo = parts.next()?;

    let (name, arch) = package_with_arch.rsplit_once('.')?;
    if name.is_empty() || arch.is_empty() {
        return None;
    }

    Some((name, new_version))
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

    #[test]
    fn test_parse_check_upgrade_entry_parses_normal_line() {
        let line = "akonadi-calendar.x86_64 25.12.3-1.fc43 updates";
        let parsed = parse_check_upgrade_entry(line);

        assert_eq!(parsed, Some(("akonadi-calendar", "25.12.3-1.fc43")));
    }

    #[test]
    fn test_parse_check_upgrade_entry_skips_headers() {
        assert!(parse_check_upgrade_entry("Repositories loaded.").is_none());
        assert!(parse_check_upgrade_entry("Available upgrades").is_none());
        assert!(parse_check_upgrade_entry("Obsoleting packages").is_none());
    }

    #[test]
    fn test_parse_check_upgrade_entry_skips_indented_obsoleted_rows() {
        let line = "    kernel-headers.x86_64 6.18.3-200.fc43 updates";
        assert!(parse_check_upgrade_entry(line).is_none());
    }

    #[test]
    fn test_build_check_upgrade_command_without_refresh() {
        let (program, args) = build_check_upgrade_command("/usr/bin/dnf", false);
        assert_eq!(program, "/usr/bin/dnf");
        assert_eq!(args, vec!["check-upgrade"]);
    }

    #[test]
    fn test_build_check_upgrade_command_with_refresh_uses_pkexec() {
        let (program, args) = build_check_upgrade_command("/usr/bin/dnf", true);
        assert_eq!(program, "pkexec");
        assert_eq!(args, vec!["/usr/bin/dnf", "check-upgrade", "--refresh"]);
    }
}
