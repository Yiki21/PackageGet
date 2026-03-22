use async_trait::async_trait;
use regex::Regex;
use tokio::{fs, process::Command};

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    pm::{
        common::manager_command_path,
        progress::{CommandProgressEvent, run_command_with_progress},
    },
};

#[derive(Debug, Clone)]
pub struct GoManager;

#[derive(Debug)]
struct InstalledBinary {
    name: String,
    path: String,
}

fn command_path(config: &Config) -> String {
    manager_command_path(config, PackageManagerType::Go)
}

#[async_trait]
impl PackageManager for GoManager {
    async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
        let path = command_path(config);
        let binaries = Self::list_installed_binaries(config).await?;
        let mut updates = Vec::new();

        for binary in binaries {
            // Try to get version info from the binary
            if let Ok(local_info) = Self::get_binary_info(&path, &binary.path).await {
                // Extract module path, e.g., github.com/user/repo
                if let Some(module) = Self::extract_module_path(&local_info) {
                    // Get latest version
                    if let Ok(latest_version) = Self::get_latest_version(&path, &module).await {
                        // Extract local version
                        if let Some(local_version) = Self::extract_version(&local_info)
                            && local_version != latest_version
                            && !latest_version.is_empty()
                        {
                            updates.push(PackageUpdate {
                                name: binary.name.clone(),
                                current_version: local_version,
                                new_version: latest_version,
                            });
                        }
                    }
                }
            }
        }

        Ok(updates)
    }

    async fn get_current_version(config: &Config, package_name: &str) -> CoreResult<String> {
        let path = command_path(config);
        let binaries = Self::list_installed_binaries(config).await?;

        // Try to find matching binary
        for binary in binaries {
            if let Ok(info) = Self::get_binary_info(&path, &binary.path).await
                && let Some(module) = Self::extract_module_path(&info)
                && (module == package_name || binary.name == package_name)
                && let Some(version) = Self::extract_version(&info)
            {
                return Ok(version);
            }
        }

        Err(crate::error::CoreError::UnknownError(format!(
            "Package {} not installed",
            package_name
        )))
    }

    async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
        let path = command_path(config);
        let binaries = Self::list_installed_binaries(config).await?;

        let mut packages = Vec::new();
        for binary in binaries {
            if let Ok(info) = Self::get_binary_info(&path, &binary.path).await
                && let Some(_module) = Self::extract_module_path(&info)
            {
                let version = Self::extract_version(&info).unwrap_or_else(|| "unknown".to_string());
                packages.push(PackageInfo {
                    name: binary.name,
                    version,
                    source: PackageManagerType::Go,
                    description: None,
                    size: None,
                    install_date: None,
                    homepage: None,
                });
            }
        }

        Ok(packages)
    }

    async fn search_package(config: &Config, package_name: &str) -> CoreResult<Vec<PackageInfo>> {
        let path = command_path(config);

        let output = Command::new(&path)
            .arg("list")
            .arg("-m")
            .arg("-versions")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut packages = Vec::new();

        // go list output format: module_path version1 version2 ...
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let module_name = parts[0].to_string();
            let version = Self::get_current_version(config, &module_name)
                .await
                .unwrap_or_else(|_| "Not Installed".to_string());

            packages.push(PackageInfo {
                name: module_name,
                version,
                source: PackageManagerType::Go,
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
        for name in package_names {
            Self::uninstall_package_with_progress(config, name, |_| {}).await?;
        }

        Ok(())
    }

    async fn update_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        for name in package_names {
            Self::update_package_with_progress(config, name, |_| {}).await?;
        }

        Ok(())
    }

    async fn install_packages(&self, config: &Config, package_names: &[String]) -> CoreResult<()> {
        for name in package_names {
            Self::install_package_with_progress(config, name, |_| {}).await?;
        }

        Ok(())
    }
}

impl GoManager {
    pub async fn uninstall_package_with_progress(
        _config: &Config,
        package_name: &str,
        mut on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        // Go uninstall is file removal only, no native streamed percentage.
        on_progress(CommandProgressEvent {
            progress: 0.0,
            command_message: Some(format!("Removing Go binary for {}", package_name)),
        });

        let gobin = std::env::var("GOBIN")
            .or_else(|_| std::env::var("GOPATH").map(|p| format!("{}/bin", p)))
            .unwrap_or_else(|_| format!("{}/go/bin", std::env::var("HOME").unwrap_or_default()));

        let binary_name = package_name.split('/').next_back().unwrap_or(package_name);
        let binary_path = format!("{}/{}", gobin, binary_name);

        if let Err(e) = tokio::fs::remove_file(&binary_path).await {
            return Err(crate::error::CoreError::UnknownError(format!(
                "Failed to remove Go binary {}: {}",
                binary_name, e
            )));
        }

        on_progress(CommandProgressEvent {
            progress: 1.0,
            command_message: Some(format!("Removed {}", binary_name)),
        });
        Ok(())
    }

    pub async fn update_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);
        let install_path = Self::resolve_install_path(config, package_name).await?;

        let args = vec!["install".to_string(), install_path];
        run_command_with_progress(&path, &args, on_progress).await
    }

    pub async fn install_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        let path = command_path(config);
        let install_path = Self::resolve_install_path(config, package_name).await?;

        let args = vec!["install".to_string(), install_path];
        run_command_with_progress(&path, &args, on_progress).await
    }

    /// Get latest version using go list
    async fn get_latest_version(path: &str, package_name: &str) -> CoreResult<String> {
        let output = Command::new(path)
            .arg("list")
            .arg("-m")
            .arg("-versions")
            .arg(package_name)
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::UnknownError(format!(
                "Failed to get versions for package: {}",
                package_name
            )));
        }

        let stdout = String::from_utf8(output.stdout)?;
        if let Some(version) = Self::parse_latest_version_from_list_output(&stdout) {
            return Ok(version);
        }

        Err(crate::error::CoreError::UnknownError(format!(
            "No valid version found for package: {}",
            package_name
        )))
    }

    /// List all installed Go binaries
    async fn list_installed_binaries(config: &Config) -> CoreResult<Vec<InstalledBinary>> {
        let bin_dir = config.get_go_bin_dir();
        let mut bins = Vec::new();

        if let Ok(mut entries) = fs::read_dir(&bin_dir).await {
            while let Some(entry) = entries.next_entry().await.unwrap_or(None) {
                if let Ok(file_type) = entry.file_type().await
                    && file_type.is_file()
                    && let Some(name) = entry.file_name().to_str()
                {
                    let path = entry.path().to_string_lossy().to_string();
                    bins.push(InstalledBinary {
                        name: name.to_owned(),
                        path,
                    });
                }
            }
        }
        Ok(bins)
    }

    /// Get build info of a binary (using go version -m)
    async fn get_binary_info(path: &str, binary_path: &str) -> CoreResult<String> {
        let output = Command::new(path)
            .arg("version")
            .arg("-m")
            .arg(binary_path)
            .output()
            .await?;

        if !output.status.success() {
            return Err(crate::error::CoreError::UnknownError(format!(
                "Failed to get info for binary: {}",
                binary_path
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn resolve_install_path(config: &Config, package_name: &str) -> CoreResult<String> {
        if package_name.contains('@') {
            return Ok(package_name.to_string());
        }

        if Self::looks_like_module_path(package_name) {
            return Ok(format!("{package_name}@latest"));
        }

        let path = command_path(config);
        let binaries = Self::list_installed_binaries(config).await?;

        for binary in binaries {
            if binary.name != package_name {
                continue;
            }

            if let Ok(info) = Self::get_binary_info(&path, &binary.path).await
                && let Some(module_path) = Self::extract_module_path(&info)
            {
                return Ok(Self::build_install_path(&module_path));
            }
        }

        Ok(Self::build_install_path(package_name))
    }

    fn build_install_path(package_name: &str) -> String {
        if package_name.contains('@') {
            package_name.to_string()
        } else {
            format!("{package_name}@latest")
        }
    }

    fn looks_like_module_path(package_name: &str) -> bool {
        package_name.contains('/')
            || package_name
                .split('/')
                .next()
                .is_some_and(|segment| segment.contains('.'))
    }

    /// Extract module path from go version -m output
    /// Example: "path\tgithub.com/user/repo" -> "github.com/user/repo"
    fn extract_module_path(info: &str) -> Option<String> {
        let re = Regex::new(r"path\s+([^\s]+)").ok()?;
        re.captures(info)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
    }

    /// Extract version from go version -m output
    /// Example: "mod\tgithub.com/user/repo\tv1.2.3" -> "v1.2.3"
    fn extract_version(info: &str) -> Option<String> {
        let re = Regex::new(r"mod\s+[^\s]+\s+(v[0-9]+\.[0-9]+\.[0-9]+[^\s]*)").ok()?;
        re.captures(info)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
    }

    /// Parse latest version from `go list -m -versions` output.
    /// Output format: `module/path v0.1.0 v0.2.0 ...`
    fn parse_latest_version_from_list_output(output: &str) -> Option<String> {
        let tokens: Vec<&str> = output.split_whitespace().collect();
        if tokens.len() <= 1 {
            return None;
        }

        tokens
            .iter()
            .skip(1)
            .rev()
            .find(|token| token.starts_with('v'))
            .map(|token| (*token).to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_module_path() {
        let info = r#"/path/to/binary: go1.21.0
	path	github.com/user/myapp
	mod	github.com/user/myapp	v1.2.3
"#;
        let module = GoManager::extract_module_path(info);
        assert_eq!(module, Some("github.com/user/myapp".to_string()));
    }

    #[test]
    fn test_extract_version() {
        let info = r#"/path/to/binary: go1.21.0
	path	github.com/user/myapp
	mod	github.com/user/myapp	v1.2.3
"#;
        let version = GoManager::extract_version(info);
        assert_eq!(version, Some("v1.2.3".to_string()));
    }

    #[test]
    fn test_extract_version_with_suffix() {
        let info = r#"/path/to/binary: go1.21.0
	path	github.com/cli/cli/v2
	mod	github.com/cli/cli/v2	v2.40.1-pre.0
"#;
        let version = GoManager::extract_version(info);
        assert_eq!(version, Some("v2.40.1-pre.0".to_string()));
    }

    #[test]
    fn test_extract_module_path_none() {
        let info = "some random text without module path";
        let module = GoManager::extract_module_path(info);
        assert_eq!(module, None);
    }

    #[test]
    fn test_extract_version_none() {
        let info = "some random text without version";
        let version = GoManager::extract_version(info);
        assert_eq!(version, None);
    }

    #[test]
    fn test_parse_latest_version_from_list_output() {
        let output = "github.com/user/tool v0.1.0 v0.2.0 v0.2.1\n";
        let version = GoManager::parse_latest_version_from_list_output(output);
        assert_eq!(version, Some("v0.2.1".to_string()));
    }

    #[test]
    fn test_parse_latest_version_from_list_output_without_versions() {
        let output = "github.com/user/tool\n";
        let version = GoManager::parse_latest_version_from_list_output(output);
        assert_eq!(version, None);
    }

    #[test]
    fn test_build_install_path_appends_latest_for_module_path() {
        let install_path = GoManager::build_install_path("github.com/nao1215/gup");
        assert_eq!(install_path, "github.com/nao1215/gup@latest");
    }

    #[test]
    fn test_build_install_path_preserves_explicit_version() {
        let install_path = GoManager::build_install_path("github.com/nao1215/gup@v1.1.3");
        assert_eq!(install_path, "github.com/nao1215/gup@v1.1.3");
    }

    #[test]
    fn test_looks_like_module_path_for_domain_based_package() {
        assert!(GoManager::looks_like_module_path("github.com/nao1215/gup"));
    }

    #[test]
    fn test_looks_like_module_path_for_binary_name() {
        assert!(!GoManager::looks_like_module_path("gup"));
    }
}
