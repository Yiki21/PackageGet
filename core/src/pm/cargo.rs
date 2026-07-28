use std::{collections::HashMap, path::PathBuf, time::Duration};

use async_trait::async_trait;
use futures::future::join_all;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    pm::{
        common::{manager_command, manager_command_path},
        progress::{CommandProgressEvent, run_command_with_progress},
    },
};

#[derive(Debug, Clone)]
pub struct CargoManager;

#[derive(Debug)]
struct InstalledCrate {
    name: String,
    version: String,
    bins: Vec<String>,
    can_update_from_registry: bool,
}

#[derive(Debug, Default)]
struct CrateMetadata {
    latest_version: Option<String>,
    description: Option<String>,
    homepage: Option<String>,
}

fn command_path(config: &Config) -> String {
    manager_command_path(config, PackageManagerType::Cargo)
}

#[async_trait]
impl PackageManager for CargoManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        let path = command_path(config);

        let install_output = manager_command(&path)
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
        let client = Self::crates_io_client()?;

        let mut updates: Vec<PackageUpdate> = Vec::new();
        for inst in installed
            .into_iter()
            .filter(|package| package.can_update_from_registry)
        {
            let Some(metadata) = Self::get_crate_metadata(&client, &inst.name).await else {
                continue;
            };
            let Some(latest_version) = metadata.latest_version else {
                continue;
            };
            if latest_version == inst.version {
                continue;
            }

            updates.push(PackageUpdate {
                name: inst.name,
                current_version: inst.version,
                new_version: latest_version,
            });
        }

        Ok(updates)
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        let path = command_path(config);

        let install_output = manager_command(&path)
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
        let path = command_path(config);

        let install_output = manager_command(&path)
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
        let client = Self::crates_io_client()?;
        let registry_names: Vec<_> = installed
            .iter()
            .filter(|crate_info| crate_info.can_update_from_registry)
            .map(|crate_info| crate_info.name.clone())
            .collect();
        let mut metadata_by_name = HashMap::with_capacity(registry_names.len());

        for names in registry_names.chunks(6) {
            let responses = join_all(
                names
                    .iter()
                    .map(|name| Self::get_crate_metadata(&client, name)),
            )
            .await;
            for (name, metadata) in names.iter().zip(responses) {
                if let Some(metadata) = metadata {
                    metadata_by_name.insert(name.clone(), metadata);
                }
            }
        }

        let cargo_root = std::env::var_os("CARGO_INSTALL_ROOT")
            .or_else(|| std::env::var_os("CARGO_HOME"))
            .map(PathBuf::from)
            .or_else(|| {
                directories_next::BaseDirs::new().map(|dirs| dirs.home_dir().join(".cargo"))
            });
        let bin_dir = cargo_root.map(|root| root.join("bin"));
        let mut packages = Vec::with_capacity(installed.len());

        for crate_info in installed {
            let mut installed_size = 0_u64;
            let mut found_binary = false;
            if let Some(bin_dir) = &bin_dir {
                for binary in &crate_info.bins {
                    if let Ok(metadata) = tokio::fs::metadata(bin_dir.join(binary)).await {
                        installed_size = installed_size.saturating_add(metadata.len());
                        found_binary = true;
                    }
                }
            }

            let metadata = metadata_by_name
                .remove(&crate_info.name)
                .unwrap_or_default();
            packages.push(PackageInfo {
                name: crate_info.name,
                version: crate_info.version,
                source: PackageManagerType::Cargo,
                description: metadata.description,
                size: found_binary.then_some(installed_size),
                install_date: None,
                homepage: metadata.homepage,
            });
        }

        Ok(packages)
    }

    async fn count_installed(config: &Config) -> CoreResult<usize> {
        let path = command_path(config);

        let install_output = manager_command(&path)
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
        Ok(Self::parse_cargo_install_list(&stdout).len())
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        // 使用 crates.io API 搜索
        let encoded_name = package_name.replace(' ', "%20");
        let url = format!(
            "https://crates.io/api/v1/crates?page=1&per_page=10&q={}",
            encoded_name
        );

        log::debug!("Cargo search: querying URL: {}", url);

        // crates.io API 要求提供 User-Agent 头
        let client = Self::crates_io_client()?;

        let resp = client.get(&url).send().await?;

        log::debug!("Cargo search: got response status: {}", resp.status());

        if !resp.status().is_success() {
            log::warn!(
                "Cargo search: HTTP request failed with status {}",
                resp.status()
            );
            return Ok(Vec::new());
        }

        let search_result: serde_json::Value = resp.json().await?;
        let mut packages = Vec::new();
        let installed_versions = Self::get_installed_versions(config).await;

        log::debug!("Cargo search: parsing JSON response");

        if let Some(crates) = search_result["crates"].as_array() {
            log::debug!("Cargo search: found {} crates in response", crates.len());
            for crate_info in crates {
                if let Some(name) = crate_info["name"].as_str() {
                    let description = crate_info["description"].as_str().map(|s| s.to_string());
                    let homepage = crate_info["homepage"]
                        .as_str()
                        .or_else(|| crate_info["repository"].as_str())
                        .map(|s| s.to_string());

                    packages.push(PackageInfo {
                        name: name.to_string(),
                        version: installed_versions
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| "Not Installed".to_string()),
                        source: PackageManagerType::Cargo,
                        description,
                        size: None,
                        install_date: None,
                        homepage,
                    });
                }
            }
        } else {
            log::warn!("Cargo search: 'crates' field not found in JSON response");
        }

        log::debug!("Cargo search: returning {} packages", packages.len());
        Ok(packages)
    }
}

impl CargoManager {
    fn crates_io_client() -> CoreResult<reqwest::Client> {
        Ok(reqwest::Client::builder()
            .user_agent("updater/0.1.0 (https://github.com/Yiki21/updater)")
            .timeout(Duration::from_secs(20))
            .build()?)
    }

    async fn get_crate_metadata(
        client: &reqwest::Client,
        crate_name: &str,
    ) -> Option<CrateMetadata> {
        let response = client
            .get(format!("https://crates.io/api/v1/crates/{crate_name}"))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }

        let crate_info: serde_json::Value = response.json().await.ok()?;
        let details = &crate_info["crate"];

        Some(CrateMetadata {
            latest_version: details["max_stable_version"]
                .as_str()
                .or_else(|| details["max_version"].as_str())
                .map(str::to_owned),
            description: details["description"].as_str().map(str::to_owned),
            homepage: details["homepage"]
                .as_str()
                .or_else(|| details["repository"].as_str())
                .map(str::to_owned),
        })
    }

    async fn get_installed_versions(config: &Config) -> HashMap<String, String> {
        let path = command_path(config);

        let output = match manager_command(&path)
            .arg("install")
            .arg("--list")
            .output()
            .await
        {
            Ok(output) => output,
            Err(_) => return HashMap::new(),
        };

        if !output.status.success() {
            return HashMap::new();
        }

        let stdout = match String::from_utf8(output.stdout) {
            Ok(stdout) => stdout,
            Err(_) => return HashMap::new(),
        };

        Self::parse_cargo_install_list(&stdout)
            .into_iter()
            .map(|crate_info| (crate_info.name, crate_info.version))
            .collect()
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

        let args = vec![
            "install".to_string(),
            "--force".to_string(),
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

        let args = vec!["install".to_string(), package_name.to_owned()];

        run_command_with_progress(&path, &args, on_progress).await
    }

    fn parse_cargo_install_list(input: &str) -> Vec<InstalledCrate> {
        let mut installed = Vec::<InstalledCrate>::new();

        for line in input.lines() {
            if let Some(header) = line.trim().strip_suffix(':') {
                let mut parts = header.split_whitespace();
                let (Some(name), Some(version)) = (
                    parts.next(),
                    parts.next().and_then(|value| value.strip_prefix('v')),
                ) else {
                    continue;
                };

                installed.push(InstalledCrate {
                    name: name.to_owned(),
                    version: version.to_owned(),
                    bins: Vec::new(),
                    can_update_from_registry: parts.next().is_none(),
                });
            } else if let Some(crate_info) = installed.last_mut() {
                let binary = line.trim();
                if !binary.is_empty() {
                    crate_info.bins.push(binary.to_owned());
                }
            }
        }

        installed
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
        assert_eq!(crates.len(), 9);
        assert_eq!(crates[0].name, "bluetui");
        assert_eq!(crates[0].version, "0.8.0");
        assert_eq!(crates[0].bins, ["bluetui"]);
        assert!(crates[0].can_update_from_registry);

        assert_eq!(crates[4].name, "cargo-update");
        assert_eq!(crates[4].version, "18.0.0");
        assert_eq!(
            crates[4].bins,
            ["cargo-install-update", "cargo-install-update-config"]
        );
        assert!(crates[4].can_update_from_registry);

        assert_eq!(crates[6].name, "hyprshell");
        assert!(!crates[6].can_update_from_registry);
        assert_eq!(crates[7].name, "sea-orm-cli");
        assert_eq!(crates[7].version, "1.1.19");
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
        assert_eq!(crates[0].bins, ["cargo-watch"]);
    }

    #[test]
    fn test_parse_local_path_crate() {
        let input = r#"my-tool v1.0.0 (/home/user/projects/my-tool):
    my-tool
"#;
        let crates = CargoManager::parse_cargo_install_list(input);
        assert_eq!(crates.len(), 1);
        assert_eq!(crates[0].name, "my-tool");
        assert!(!crates[0].can_update_from_registry);
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
        assert_eq!(crates.len(), 3);
        assert_eq!(crates[0].name, "cargo-watch");
        assert_eq!(crates[1].name, "local-tool");
        assert!(!crates[1].can_update_from_registry);
        assert_eq!(crates[2].name, "ripgrep");
    }

    #[tokio::test]
    async fn test_search_package_yazi() {
        let _ = env_logger::builder().is_test(true).try_init();
        let config = crate::Config::default();
        match CargoManager::search_package(&config, "yazi").await {
            Ok(packages) => {
                println!("Found {} packages for 'yazi':", packages.len());
                assert!(!packages.is_empty(), "Should find at least one package");
                for (i, pkg) in packages.iter().take(5).enumerate() {
                    println!("  {}: {} - {}", i + 1, pkg.name, pkg.version);
                    if let Some(ref desc) = pkg.description {
                        println!("     {}", desc);
                    }
                }
                // 应该找到 yazi 包
                let has_yazi = packages.iter().any(|p| p.name == "yazi");
                if has_yazi {
                    println!("✓ Found 'yazi' package");
                }
            }
            Err(e) => {
                panic!("Search failed: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_search_package_eza() {
        let _ = env_logger::builder().is_test(true).try_init();
        let config = crate::Config::default();
        match CargoManager::search_package(&config, "eza").await {
            Ok(packages) => {
                println!("Found {} packages for 'eza':", packages.len());
                assert!(!packages.is_empty(), "Should find at least one package");
                for (i, pkg) in packages.iter().take(5).enumerate() {
                    println!("  {}: {} - {}", i + 1, pkg.name, pkg.version);
                    if let Some(ref desc) = pkg.description {
                        println!("     {}", desc);
                    }
                }
                // 应该找到 eza 包（精确匹配）
                let eza_pkg = packages.iter().find(|p| p.name == "eza");
                assert!(eza_pkg.is_some(), "Should find exact match for 'eza'");
                if let Some(pkg) = eza_pkg {
                    println!("✓ Found 'eza' package version {}", pkg.version);
                    assert!(!pkg.version.is_empty(), "Version should not be empty");
                }
            }
            Err(e) => {
                panic!("Search failed: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_search_package_returns_install_state() {
        let _ = env_logger::builder().is_test(true).try_init();
        let config = crate::Config::default();
        match CargoManager::search_package(&config, "serde").await {
            Ok(packages) => {
                println!("Found {} packages for 'serde':", packages.len());
                assert!(!packages.is_empty(), "Should find serde packages");

                // 检查第一个结果
                let first = &packages[0];
                println!("First result: {} - {}", first.name, first.version);

                assert!(!first.version.is_empty(), "Version should not be empty");
                assert_ne!(first.version, "unknown", "Version should not be unknown");

                println!("✓ Search returns install state for Cargo packages");
            }
            Err(e) => {
                panic!("Search failed: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_search_empty_query() {
        let config = crate::Config::default();
        // 空查询应该返回一些结果（crates.io 会返回流行的包）
        match CargoManager::search_package(&config, "").await {
            Ok(packages) => {
                println!("Empty query returned {} packages", packages.len());
                // crates.io API 对空查询会返回结果
                // 不强制要求有结果，但如果有结果应该是有效的
                for pkg in packages.iter().take(3) {
                    println!("  {} - {}", pkg.name, pkg.version);
                }
            }
            Err(e) => {
                println!("Empty query failed (this may be expected): {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_search_nonexistent_package() {
        let config = crate::Config::default();
        // 搜索一个不存在的包名
        match CargoManager::search_package(&config, "this-package-definitely-does-not-exist-12345")
            .await
        {
            Ok(packages) => {
                println!(
                    "Search for nonexistent package returned {} results",
                    packages.len()
                );
                // 应该返回空列表或者没有精确匹配
                if !packages.is_empty() {
                    println!("Got some fuzzy matches:");
                    for pkg in packages.iter().take(3) {
                        println!("  {} - {}", pkg.name, pkg.version);
                    }
                }
            }
            Err(e) => {
                println!("Search failed: {}", e);
            }
        }
    }
}
