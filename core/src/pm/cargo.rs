use async_trait::async_trait;
use regex::Regex;
use tokio::process::Command;

use crate::{Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate};

#[derive(Debug, Clone)]
pub struct CargoManager;

#[derive(Debug)]
struct InstalledCrate {
    name: String,
    version: String,
    bins: Vec<String>,
}

#[async_trait]
impl PackageManager for CargoManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        let path = config
            .get_package_path(PackageManagerType::Cargo)
            .unwrap_or_else(|| "cargo".to_owned());

        let install_output = Command::new(&path)
            .arg("install")
            .arg("--list")
            .output()
            .await?;

        if !install_output.status.success() {
            return Err(crate::error::CoreError::UnknownError(
                "cargo install --list failed".into(),
            ));
        }

        let stdout = String::from_utf8(install_output.stdout)?;

        let installed = Self::parse_cargo_install_list(&stdout);

        let mut updates: Vec<PackageUpdate> = Vec::new();
        for inst in installed {
            if let Ok(latest_version) = Self::get_latest_version(&inst.name).await
                && latest_version != inst.version
            {
                updates.push(PackageUpdate {
                    name: inst.name,
                    current_version: inst.version,
                    new_version: latest_version,
                });
            }
        }

        Ok(updates)
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        let path = config
            .get_package_path(PackageManagerType::Cargo)
            .unwrap_or_else(|| "cargo".to_owned());

        let install_output = Command::new(&path)
            .arg("install")
            .arg("--list")
            .output()
            .await?;

        if !install_output.status.success() {
            return Err(crate::error::CoreError::UnknownError(
                "cargo install --list failed".into(),
            ));
        }

        let stdout = String::from_utf8(install_output.stdout)?;
        let installed = Self::parse_cargo_install_list(&stdout);

        for crate_info in installed {
            if crate_info.name == package_name {
                return Ok(crate_info.version);
            }
        }

        Err(crate::error::CoreError::UnknownError(format!(
            "Package {} not installed",
            package_name
        )))
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        let path = config
            .get_package_path(PackageManagerType::Cargo)
            .unwrap_or_else(|| "cargo".to_owned());

        let install_output = Command::new(&path)
            .arg("install")
            .arg("--list")
            .output()
            .await?;

        if !install_output.status.success() {
            return Err(crate::error::CoreError::UnknownError(
                "cargo install --list failed".into(),
            ));
        }

        let stdout = String::from_utf8(install_output.stdout)?;
        let installed = Self::parse_cargo_install_list(&stdout);

        // Batch fetch crate info from crates.io
        let mut packages = Vec::new();
        for crate_info in installed {
            let (description, homepage) = match Self::get_crate_info(&crate_info.name).await {
                Ok((desc, home)) => (desc, home),
                Err(_) => (None, None),
            };

            packages.push(PackageInfo {
                name: crate_info.name,
                version: crate_info.version,
                source: PackageManagerType::Cargo,
                description,
                size: None,
                install_date: None,
                homepage,
            });
        }

        Ok(packages)
    }

    async fn search_package(_config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        // 使用 crates.io API 搜索
        let encoded_name = package_name.replace(' ', "%20");
        let url = format!(
            "https://crates.io/api/v1/crates?page=1&per_page=10&q={}",
            encoded_name
        );

        let resp = reqwest::get(&url).await?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        let search_result: serde_json::Value = resp.json().await?;
        let mut packages = Vec::new();

        if let Some(crates) = search_result["crates"].as_array() {
            for crate_info in crates {
                if let (Some(name), Some(version)) = (
                    crate_info["name"].as_str(),
                    crate_info["max_version"].as_str(),
                ) {
                    let description = crate_info["description"].as_str().map(|s| s.to_string());
                    let homepage = crate_info["homepage"]
                        .as_str()
                        .or_else(|| crate_info["repository"].as_str())
                        .map(|s| s.to_string());

                    packages.push(PackageInfo {
                        name: name.to_string(),
                        version: version.to_string(),
                        source: PackageManagerType::Cargo,
                        description,
                        size: None,
                        install_date: None,
                        homepage,
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
            .get_package_path(PackageManagerType::Cargo)
            .unwrap_or_else(|| "cargo".to_owned());

        for name in package_names {
            let output = Command::new(&path)
                .arg("uninstall")
                .arg(name)
                .output()
                .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(crate::error::CoreError::UnknownError(format!(
                    "cargo uninstall failed for {}: {}",
                    name, stderr
                )));
            }
        }

        Ok(())
    }

    async fn update_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Cargo)
            .unwrap_or_else(|| "cargo".to_owned());

        for name in package_names {
            // Cargo doesn't have a direct update command, so we use install --force
            let output = Command::new(&path)
                .arg("install")
                .arg("--force")
                .arg(name)
                .output()
                .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(crate::error::CoreError::UnknownError(format!(
                    "cargo install --force failed for {}: {}",
                    name, stderr
                )));
            }
        }

        Ok(())
    }

    async fn install_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        let path = config
            .get_package_path(PackageManagerType::Cargo)
            .unwrap_or_else(|| "cargo".to_owned());

        for name in package_names {
            let output = Command::new(&path)
                .arg("install")
                .arg(name)
                .output()
                .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(crate::error::CoreError::UnknownError(format!(
                    "cargo install failed for {}: {}",
                    name, stderr
                )));
            }
        }

        Ok(())
    }
}

impl CargoManager {
    /// get crate info from crates.io API
    async fn get_crate_info(crate_name: &str) -> CoreResult<(Option<String>, Option<String>)> {
        let resp = reqwest::get(format!("https://crates.io/api/v1/crates/{}", crate_name)).await?;

        if !resp.status().is_success() {
            return Ok((None, None));
        }

        let crate_info: serde_json::Value = resp.json().await?;

        let description = crate_info["crate"]["description"]
            .as_str()
            .map(|s| s.to_string());

        let homepage = crate_info["crate"]["homepage"]
            .as_str()
            .or_else(|| crate_info["crate"]["repository"].as_str())
            .map(|s| s.to_string());

        Ok((description, homepage))
    }

    /// Get latest version of a crate from crates.io
    async fn get_latest_version(package_name: &str) -> CoreResult<String> {
        let resp =
            reqwest::get(format!("https://crates.io/api/v1/crates/{}", package_name)).await?;

        if !resp.status().is_success() {
            return Err(crate::error::CoreError::UnknownError(format!(
                "Failed to fetch crate info for {}",
                package_name
            )));
        }

        let crate_info: serde_json::Value = resp.json().await?;
        if let Some(version) = crate_info["crate"]["max_version"].as_str() {
            Ok(version.to_owned())
        } else {
            Err(crate::error::CoreError::UnknownError(format!(
                "Version info not found for crate {}",
                package_name
            )))
        }
    }

    fn parse_cargo_install_list(input: &str) -> Vec<InstalledCrate> {
        let crate_line = Regex::new(r"^(\S+)\s+v([\d\.]+):$").unwrap();
        let bin_line = Regex::new(r"^\s+(\S+)").unwrap();

        let mut result = Vec::new();
        let mut current_crate: Option<InstalledCrate> = None;

        for line in input.lines() {
            if let Some(caps) = crate_line.captures(line) {
                if let Some(c) = current_crate.take() {
                    result.push(c);
                }
                current_crate = Some(InstalledCrate {
                    name: caps[1].to_string(),
                    version: caps[2].to_string(),
                    bins: Vec::new(),
                });
            } else if let Some(caps) = bin_line.captures(line)
                && let Some(ref mut c) = current_crate
            {
                c.bins.push(caps[1].to_string());
            }
        }

        if let Some(c) = current_crate.take() {
            result.push(c);
        }

        result
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_cargo_install_list() {
        let input = r#"
bluetui v0.8.0:
    bluetui
cargo-chef v0.1.73:
    cargo-chef
cargo-deb v3.6.2:
    cargo-deb
cargo-generate-rpm v0.20.0:
    cargo-generate-rpm
cargo-update v18.0.0:
    cargo-install-update
    cargo-install-update-config
fnm v1.38.1:
    fnm
hyprshell v4.8.1 (/home/ayi/Downloads/hyprshell):
    hyprshell
sea-orm-cli v1.1.19:
    sea
    sea-orm-cli
starship v1.24.2:
    starship
"#;

        let crates = CargoManager::parse_cargo_install_list(input);
        assert_eq!(crates.len(), 8);
        assert_eq!(crates[0].name, "bluetui");
        assert_eq!(crates[0].version, "0.8.0");
        assert_eq!(crates[0].bins, vec!["bluetui"]);

        assert_eq!(crates[4].name, "cargo-update");
        assert_eq!(crates[4].version, "18.0.0");
        assert_eq!(
            crates[4].bins,
            vec!["cargo-install-update", "cargo-install-update-config"]
        );

        assert_eq!(crates[6].name, "sea-orm-cli");
        assert_eq!(crates[6].version, "1.1.19");
        assert_eq!(crates[6].bins, vec!["sea", "sea-orm-cli"]);
    }

    #[test]
    fn test_parse_empty_list() {
        let input = "";
        let crates = CargoManager::parse_cargo_install_list(input);
        assert_eq!(crates.len(), 0);
    }

    #[test]
    fn test_parse_single_crate() {
        let input = r#"cargo-watch v8.5.2:
    cargo-watch
"#;
        let crates = CargoManager::parse_cargo_install_list(input);
        assert_eq!(crates.len(), 1);
        assert_eq!(crates[0].name, "cargo-watch");
        assert_eq!(crates[0].version, "8.5.2");
        assert_eq!(crates[0].bins, vec!["cargo-watch"]);
    }

    #[test]
    fn test_parse_crate_with_multiple_bins() {
        let input = r#"tokio-console v0.1.12:
    tokio-console
    tokio-console-subscriber
    tokio-console-recorder
"#;
        let crates = CargoManager::parse_cargo_install_list(input);
        assert_eq!(crates.len(), 1);
        assert_eq!(crates[0].name, "tokio-console");
        assert_eq!(crates[0].version, "0.1.12");
        assert_eq!(
            crates[0].bins,
            vec![
                "tokio-console",
                "tokio-console-subscriber",
                "tokio-console-recorder"
            ]
        );
    }

    #[test]
    fn test_parse_local_path_crate_ignored() {
        let input = r#"my-tool v1.0.0 (/home/user/projects/my-tool):
    my-tool
"#;
        let crates = CargoManager::parse_cargo_install_list(input);
        assert_eq!(crates.len(), 0, "本地路径安装的包应该被忽略");
    }

    #[test]
    fn test_parse_mixed_crates() {
        let input = r#"cargo-watch v8.5.2:
    cargo-watch
local-tool v1.0.0 (/home/user/local-tool):
    local-tool
ripgrep v14.1.0:
    rg
"#;
        let crates = CargoManager::parse_cargo_install_list(input);
        assert_eq!(crates.len(), 2, "只应该解析非本地路径的包");
        assert_eq!(crates[0].name, "cargo-watch");
        assert_eq!(crates[1].name, "ripgrep");
    }
}
