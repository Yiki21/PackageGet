use std::{collections::HashMap, path::PathBuf, time::Duration};

use async_trait::async_trait;
use futures::future::join_all;
use serde::Deserialize;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    pm::{
        common::{directory_size, manager_command, manager_command_path},
        progress::{CommandProgressEvent, run_command_with_progress},
    },
};

#[derive(Debug, Clone, Copy)]
pub struct PipxManager;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PipxPackage {
    venv_name: String,
    name: String,
    version: String,
    package_or_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PipxList {
    #[serde(default)]
    venvs: HashMap<String, PipxVenv>,
}

#[derive(Debug, Deserialize)]
struct PipxVenv {
    #[serde(default)]
    metadata: PipxMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct PipxMetadata {
    main_package: Option<PipxMainPackage>,
}

#[derive(Debug, Deserialize)]
struct PipxMainPackage {
    package: Option<String>,
    package_or_url: Option<String>,
    package_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PypiPackageResponse {
    info: PypiPackageInfo,
}

#[derive(Debug, Deserialize)]
struct PypiPackageInfo {
    name: String,
    version: String,
    summary: Option<String>,
    home_page: Option<String>,
    project_url: Option<String>,
    project_urls: Option<HashMap<String, String>>,
}

impl PypiPackageInfo {
    fn homepage(&self) -> Option<String> {
        self.home_page
            .as_ref()
            .filter(|url| !url.trim().is_empty())
            .cloned()
            .or_else(|| {
                let urls = self.project_urls.as_ref()?;
                ["Homepage", "Source", "Source Code", "Repository"]
                    .into_iter()
                    .find_map(|key| urls.get(key).filter(|url| !url.trim().is_empty()).cloned())
            })
            .or_else(|| {
                self.project_url
                    .as_ref()
                    .filter(|url| !url.trim().is_empty())
                    .cloned()
            })
    }
}

fn command_path(config: &Config) -> String {
    manager_command_path(config, PackageManagerType::Pipx)
}

#[async_trait]
impl PackageManager for PipxManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        let installed = Self::installed_packages(config).await?;
        let client = Self::pypi_client()?;
        let mut updates = Vec::new();

        for package in installed {
            let Ok(info) = Self::get_pypi_package_info(&client, &package.name).await else {
                continue;
            };

            if info.version != package.version {
                updates.push(PackageUpdate {
                    name: package.name,
                    current_version: package.version,
                    new_version: info.version,
                });
            }
        }

        Ok(updates)
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        Self::installed_packages(config)
            .await?
            .into_iter()
            .find(|package| package.name == package_name)
            .map(|package| package.version)
            .ok_or_else(|| {
                crate::error::CoreError::UnknownError(format!(
                    "Package {} not installed",
                    package_name
                ))
            })
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        let installed = Self::installed_packages(config).await?;
        let client = Self::pypi_client()?;
        let mut metadata_by_name = HashMap::with_capacity(installed.len());

        for packages in installed.chunks(6) {
            let responses = join_all(
                packages
                    .iter()
                    .map(|package| Self::get_pypi_package_info(&client, &package.name)),
            )
            .await;
            for (package, metadata) in packages.iter().zip(responses) {
                if let Ok(metadata) = metadata {
                    metadata_by_name.insert(package.name.clone(), metadata);
                }
            }
        }

        let path = command_path(config);
        let pipx_home = match manager_command(&path)
            .arg("environment")
            .arg("--value")
            .arg("PIPX_HOME")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let home = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                (!home.is_empty()).then(|| PathBuf::from(home).join("venvs"))
            }
            _ => None,
        };

        let mut packages = Vec::with_capacity(installed.len());
        for package in installed {
            let metadata = metadata_by_name.remove(&package.name);
            let homepage = metadata
                .as_ref()
                .and_then(PypiPackageInfo::homepage)
                .or_else(|| {
                    let source = package.package_or_url.as_deref()?;
                    let source = source.strip_prefix("git+").unwrap_or(source);
                    (source.starts_with("https://") || source.starts_with("http://"))
                        .then(|| source.to_owned())
                });
            let size = if let Some(venvs_dir) = &pipx_home {
                directory_size(&venvs_dir.join(&package.venv_name)).await
            } else {
                None
            };

            packages.push(PackageInfo {
                name: package.name,
                version: package.version,
                source: PackageManagerType::Pipx,
                description: metadata.and_then(|info| info.summary),
                size,
                install_date: None,
                homepage,
            });
        }

        Ok(packages)
    }

    async fn count_installed(config: &Config) -> CoreResult<usize> {
        Ok(Self::installed_packages(config).await?.len())
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        let client = Self::pypi_client()?;
        let Ok(info) = Self::get_pypi_package_info(&client, package_name).await else {
            return Ok(Vec::new());
        };

        let installed_versions = Self::installed_packages(config)
            .await?
            .into_iter()
            .map(|package| (package.name.to_ascii_lowercase(), package.version))
            .collect::<std::collections::HashMap<_, _>>();

        let version = installed_versions
            .get(&info.name.to_ascii_lowercase())
            .cloned()
            .unwrap_or_else(|| "Not Installed".to_owned());

        let homepage = info.homepage();
        Ok(vec![PackageInfo {
            name: info.name,
            version,
            source: PackageManagerType::Pipx,
            description: info.summary,
            size: None,
            install_date: None,
            homepage,
        }])
    }
}

impl PipxManager {
    pub async fn uninstall_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);
        let args = vec!["uninstall".to_owned(), package_name.to_owned()];
        run_command_with_progress(&path, &args, on_progress).await
    }

    pub async fn update_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);
        let args = vec!["upgrade".to_owned(), package_name.to_owned()];
        run_command_with_progress(&path, &args, on_progress).await
    }

    pub async fn install_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);
        let args = vec!["install".to_owned(), package_name.to_owned()];
        run_command_with_progress(&path, &args, on_progress).await
    }

    async fn installed_packages(config: &Config) -> CoreResult<Vec<PipxPackage>> {
        let path = command_path(config);
        let output = manager_command(&path)
            .arg("list")
            .arg("--json")
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::from_command_failure(format!(
                "pipx list --json failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        Self::parse_installed_packages(&stdout)
    }

    fn parse_installed_packages(stdout: &str) -> CoreResult<Vec<PipxPackage>> {
        let list: PipxList = serde_json::from_str(stdout)?;
        let mut packages = Vec::new();

        for (venv_name, venv) in list.venvs {
            let Some(main_package) = venv.metadata.main_package else {
                continue;
            };

            let name = main_package.package.unwrap_or_else(|| venv_name.clone());
            let version = main_package
                .package_version
                .unwrap_or_else(|| "unknown".to_owned());

            packages.push(PipxPackage {
                venv_name,
                name,
                version,
                package_or_url: main_package.package_or_url,
            });
        }

        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(packages)
    }

    async fn get_pypi_package_info(
        client: &reqwest::Client,
        package_name: &str,
    ) -> CoreResult<PypiPackageInfo> {
        let resp = client
            .get(format!("https://pypi.org/pypi/{}/json", package_name))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(crate::error::CoreError::RequestError(format!(
                "PyPI request failed with status {}",
                resp.status()
            )));
        }

        let response: PypiPackageResponse = resp.json().await?;
        Ok(response.info)
    }

    fn pypi_client() -> CoreResult<reqwest::Client> {
        Ok(reqwest::Client::builder()
            .user_agent("updater/0.1.0 (https://github.com/Yiki21/updater)")
            .timeout(Duration::from_secs(20))
            .build()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_installed_packages_reads_pipx_json() {
        let stdout = r#"{
            "venvs": {
                "black": {
                    "metadata": {
                        "main_package": {
                            "package": "black",
                            "package_version": "24.10.0"
                        }
                    }
                },
                "httpie": {
                    "metadata": {
                        "main_package": {
                            "package": "httpie",
                            "package_version": "3.2.4"
                        }
                    }
                }
            }
        }"#;

        let packages = PipxManager::parse_installed_packages(stdout).unwrap();

        assert_eq!(
            packages,
            vec![
                PipxPackage {
                    venv_name: "black".to_owned(),
                    name: "black".to_owned(),
                    version: "24.10.0".to_owned(),
                    package_or_url: None,
                },
                PipxPackage {
                    venv_name: "httpie".to_owned(),
                    name: "httpie".to_owned(),
                    version: "3.2.4".to_owned(),
                    package_or_url: None,
                },
            ]
        );
    }

    #[test]
    fn parse_installed_packages_uses_venv_name_and_unknown_version_as_fallback() {
        let stdout = r#"{
            "venvs": {
                "ruff": {
                    "metadata": {
                        "main_package": {}
                    }
                }
            }
        }"#;

        let packages = PipxManager::parse_installed_packages(stdout).unwrap();

        assert_eq!(
            packages,
            vec![PipxPackage {
                venv_name: "ruff".to_owned(),
                name: "ruff".to_owned(),
                version: "unknown".to_owned(),
                package_or_url: None,
            }]
        );
    }
}
