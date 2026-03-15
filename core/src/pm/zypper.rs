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
pub struct ZypperManager;

fn command_path(config: &Config) -> String {
    manager_command_path(config, PackageManagerType::Zypper)
}

#[async_trait]
impl PackageManager for ZypperManager {
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
            Ok(String::from_utf8(output.stdout)?.trim().to_owned())
        } else {
            Err(CoreError::ParseError(format!(
                "Package {} not found",
                package_name
            )))
        }
    }

    async fn list_installed(_config: &Config) -> CoreResult<Vec<PackageInfo>> {
        let output = Command::new("rpm")
            .arg("-qa")
            .arg("--queryformat")
            .arg("%{NAME}\t%{VERSION}-%{RELEASE}\t%{SUMMARY}\t%{SIZE}\t%{INSTALLTIME}\t%{URL}\n")
            .output()
            .await?;

        if !output.status.success() {
            return Err(CoreError::UnknownError("rpm -qa failed".to_owned()));
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
                    .filter(|value| !value.trim().is_empty() && value.trim() != "(none)")
                    .map(|value| value.trim().to_owned());

                let size = parts
                    .get(3)
                    .map(|value| value.trim())
                    .and_then(|value| value.parse::<u64>().ok());

                let install_date = parts
                    .get(4)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty() && *value != "(none)")
                    .and_then(|timestamp| {
                        timestamp.parse::<i64>().ok().and_then(|ts| {
                            let datetime = chrono::DateTime::from_timestamp(ts, 0)?;
                            Some(datetime.format("%Y-%m-%d %H:%M:%S").to_string())
                        })
                    });

                let homepage = parts
                    .get(5)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty() && *value != "(none)")
                    .map(ToOwned::to_owned);

                Some(PackageInfo {
                    name: parts[0].trim().to_owned(),
                    version: parts[1].trim().to_owned(),
                    source: PackageManagerType::Zypper,
                    description,
                    size,
                    install_date,
                    homepage,
                })
            })
            .collect();

        Ok(packages)
    }

    async fn count_installed(_config: &Config) -> CoreResult<usize> {
        let output = Command::new("rpm").arg("-qa").output().await?;
        if !output.status.success() {
            return Ok(Self::list_installed(_config).await?.len());
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
            .arg("--non-interactive")
            .arg("search")
            .arg("--details")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let installed_versions = Self::installed_version_map().await?;
        let search_rows = parse_table_rows(&stdout, &search_headers());

        let mut packages = Vec::new();
        let mut seen = HashSet::new();

        for row in search_rows {
            let Some(name) = row.get("name") else {
                continue;
            };

            if !seen.insert(name.clone()) {
                continue;
            }

            let version = installed_versions
                .get(name)
                .cloned()
                .unwrap_or_else(|| "Not Installed".to_owned());

            packages.push(PackageInfo {
                name: name.clone(),
                version,
                source: PackageManagerType::Zypper,
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
        Self::uninstall_packages_with_progress(config, package_names, |_| {}).await
    }

    async fn update_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        Self::update_packages_with_progress(config, package_names, |_| {}).await
    }

    async fn install_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        Self::install_packages_with_progress(config, package_names, |_| {}).await
    }
}

impl ZypperManager {
    pub async fn list_updates_with_refresh(
        config: &Config,
        refresh: bool,
    ) -> CoreResult<Vec<PackageUpdate>> {
        let path = command_path(config);

        if refresh {
            let args = vec![
                path.clone(),
                "--non-interactive".to_owned(),
                "refresh".to_owned(),
            ];
            run_command_with_progress("pkexec", &args, |_| {}).await?;
        }

        let output = Command::new(&path)
            .arg("--non-interactive")
            .arg("list-updates")
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CoreError::CommandError(format!(
                "zypper list-updates failed: {}",
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        let rows = parse_table_rows(&stdout, &update_headers());

        let mut updates = Vec::new();
        let mut seen = HashSet::new();

        for row in rows {
            let Some(name) = row.get("name") else {
                continue;
            };
            let Some(current_version) = row.get("current_version") else {
                continue;
            };
            let Some(new_version) = row.get("available_version") else {
                continue;
            };

            if name.is_empty()
                || current_version.is_empty()
                || new_version.is_empty()
                || !seen.insert(name.clone())
            {
                continue;
            }

            updates.push(PackageUpdate {
                name: name.clone(),
                current_version: current_version.clone(),
                new_version: new_version.clone(),
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
        let mut args = vec![
            path,
            "--non-interactive".to_owned(),
            "remove".to_owned(),
            "-y".to_owned(),
        ];
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
            "--non-interactive".to_owned(),
            "update".to_owned(),
            "-y".to_owned(),
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
            "--non-interactive".to_owned(),
            "install".to_owned(),
            "-y".to_owned(),
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

    async fn installed_version_map() -> CoreResult<HashMap<String, String>> {
        let output = Command::new("rpm")
            .arg("-qa")
            .arg("--queryformat")
            .arg("%{NAME}\t%{VERSION}-%{RELEASE}\n")
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

fn update_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("name", "name"),
        ("currentversion", "current_version"),
        ("availableversion", "available_version"),
    ]
}

fn search_headers() -> Vec<(&'static str, &'static str)> {
    vec![("name", "name"), ("version", "version")]
}

fn parse_table_rows(
    output: &str,
    required_headers: &[(&'static str, &'static str)],
) -> Vec<HashMap<String, String>> {
    let mut rows = Vec::new();
    let mut header_mapping: Option<HashMap<usize, String>> = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("Loading repository data")
            || trimmed.starts_with("Reading installed packages")
            || is_separator_line(trimmed)
            || !trimmed.contains('|')
        {
            continue;
        }

        let columns = split_table_row(trimmed);
        if columns.is_empty() {
            continue;
        }

        if header_mapping.is_none() {
            let normalized_headers = columns
                .iter()
                .enumerate()
                .map(|(idx, value)| (idx, normalize_header(value)))
                .collect::<Vec<_>>();

            let mut mapping = HashMap::new();
            for (header_name, key_name) in required_headers {
                if let Some((idx, _)) = normalized_headers
                    .iter()
                    .find(|(_, normalized)| normalized == header_name)
                {
                    mapping.insert(*idx, (*key_name).to_owned());
                }
            }

            if mapping.len() == required_headers.len() {
                header_mapping = Some(mapping);
                continue;
            }
        }

        let Some(mapping) = header_mapping.as_ref() else {
            continue;
        };

        let mut row = HashMap::new();
        for (idx, key_name) in mapping {
            let value = columns
                .get(*idx)
                .map(|value| value.trim().to_owned())
                .unwrap_or_default();
            row.insert(key_name.clone(), value);
        }

        if row.values().all(|value| value.is_empty()) {
            continue;
        }

        rows.push(row);
    }

    rows
}

fn split_table_row(line: &str) -> Vec<String> {
    line.split('|').map(|part| part.trim().to_owned()).collect()
}

fn normalize_header(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_separator_line(line: &str) -> bool {
    line.chars()
        .all(|ch| ch == '-' || ch == '+' || ch == '=' || ch == '|')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_table_rows_extracts_updates_from_list_updates_table() {
        let output = r#"
S | Repository      | Name  | Current Version | Available Version | Arch
--+-----------------+-------+-----------------+-------------------+------
v | repo-update     | bash  | 5.2-3.1         | 5.2-4.1           | x86_64
v | repo-update     | vim   | 9.1-2.1         | 9.1-3.1           | x86_64
"#;

        let rows = parse_table_rows(output, &update_headers());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("name"), Some(&"bash".to_owned()));
        assert_eq!(rows[0].get("current_version"), Some(&"5.2-3.1".to_owned()));
        assert_eq!(
            rows[0].get("available_version"),
            Some(&"5.2-4.1".to_owned())
        );
    }

    #[test]
    fn parse_table_rows_extracts_search_rows() {
        let output = r#"
S | Name      | Type    | Version  | Arch   | Repository
--+-----------+---------+----------+--------+-----------
  | ripgrep   | package | 14.1.0-1 | x86_64 | repo-oss
  | fd        | package | 9.0.0-1  | x86_64 | repo-oss
"#;

        let rows = parse_table_rows(output, &search_headers());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].get("name"), Some(&"fd".to_owned()));
        assert_eq!(rows[1].get("version"), Some(&"9.0.0-1".to_owned()));
    }

    #[test]
    fn normalize_header_removes_spaces_and_hyphens() {
        assert_eq!(normalize_header("Current Version"), "currentversion");
        assert_eq!(normalize_header("Available-Version"), "availableversion");
    }
}
