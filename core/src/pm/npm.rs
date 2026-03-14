use std::collections::HashMap;

use async_trait::async_trait;
use tokio::process::Command;

use crate::{
    Config, CoreResult, PackageInfo, PackageManager, PackageManagerType, PackageUpdate,
    pm::{
        common::manager_command_path,
        progress::{CommandProgressEvent, run_command_with_progress},
    },
};

#[derive(Debug, Clone, Copy)]
pub struct NpmManager;

#[derive(Debug, Clone, Copy)]
pub struct PnpmManager;

#[derive(Debug, Clone, Copy)]
enum GlobalPackageAction {
    Install,
    Update,
    Uninstall,
}

impl GlobalPackageAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::Uninstall => "uninstall",
        }
    }
}

fn command_path(config: &Config, manager_type: PackageManagerType) -> String {
    manager_command_path(config, manager_type)
}

async fn run_packages_silent(
    config: &Config,
    manager_type: PackageManagerType,
    action: GlobalPackageAction,
    package_names: &[String],
) -> CoreResult<()> {
    for package_name in package_names {
        run_global_package_command_with_progress(
            config,
            manager_type,
            action.as_str(),
            package_name,
            |_| {},
        )
        .await?;
    }

    Ok(())
}

macro_rules! impl_global_js_manager {
    ($manager:ty, $manager_type:expr) => {
        #[async_trait]
        impl PackageManager for $manager {
            async fn list_updates(config: &Config) -> CoreResult<Vec<PackageUpdate>> {
                list_updates_by_manager(config, $manager_type).await
            }

            async fn get_current_version(
                config: &Config,
                package_name: &str,
            ) -> CoreResult<String> {
                get_current_version_by_manager(config, $manager_type, package_name).await
            }

            async fn list_installed(config: &Config) -> CoreResult<Vec<PackageInfo>> {
                list_installed_by_manager(config, $manager_type).await
            }

            async fn count_installed(config: &Config) -> CoreResult<usize> {
                count_installed_by_manager(config, $manager_type).await
            }

            async fn search_package(
                config: &Config,
                package_name: &str,
            ) -> CoreResult<Vec<PackageInfo>> {
                search_package_by_manager(config, $manager_type, package_name).await
            }

            async fn uninstall_packages(
                &self,
                config: &Config,
                package_names: &[String],
            ) -> CoreResult<()> {
                run_packages_silent(
                    config,
                    $manager_type,
                    GlobalPackageAction::Uninstall,
                    package_names,
                )
                .await
            }

            async fn update_packages(
                &self,
                config: &Config,
                package_names: &[String],
            ) -> CoreResult<()> {
                run_packages_silent(
                    config,
                    $manager_type,
                    GlobalPackageAction::Update,
                    package_names,
                )
                .await
            }

            async fn install_packages(
                &self,
                config: &Config,
                package_names: &[String],
            ) -> CoreResult<()> {
                run_packages_silent(
                    config,
                    $manager_type,
                    GlobalPackageAction::Install,
                    package_names,
                )
                .await
            }
        }
    };
}

impl_global_js_manager!(NpmManager, PackageManagerType::Npm);
impl_global_js_manager!(PnpmManager, PackageManagerType::Pnpm);

impl NpmManager {
    pub async fn uninstall_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_global_package_command_with_progress(
            config,
            PackageManagerType::Npm,
            "uninstall",
            package_name,
            on_progress,
        )
        .await
    }

    pub async fn update_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_global_package_command_with_progress(
            config,
            PackageManagerType::Npm,
            "update",
            package_name,
            on_progress,
        )
        .await
    }

    pub async fn install_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_global_package_command_with_progress(
            config,
            PackageManagerType::Npm,
            "install",
            package_name,
            on_progress,
        )
        .await
    }
}

impl PnpmManager {
    pub async fn uninstall_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_global_package_command_with_progress(
            config,
            PackageManagerType::Pnpm,
            "uninstall",
            package_name,
            on_progress,
        )
        .await
    }

    pub async fn update_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_global_package_command_with_progress(
            config,
            PackageManagerType::Pnpm,
            "update",
            package_name,
            on_progress,
        )
        .await
    }

    pub async fn install_package_with_progress(
        config: &Config,
        package_name: &str,
        on_progress: impl FnMut(CommandProgressEvent),
    ) -> CoreResult<()> {
        run_global_package_command_with_progress(
            config,
            PackageManagerType::Pnpm,
            "install",
            package_name,
            on_progress,
        )
        .await
    }
}

fn parse_global_dependencies(value: &serde_json::Value) -> Vec<(String, String)> {
    let mut items = Vec::new();

    if let Some(dependencies) = value.get("dependencies") {
        if let Some(map) = dependencies.as_object() {
            for (name, detail) in map {
                let version = detail
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned();
                items.push((name.clone(), version));
            }
        } else if let Some(array) = dependencies.as_array() {
            for detail in array {
                let Some(name) = detail.get("name").and_then(|v| v.as_str()) else {
                    continue;
                };

                let version = detail
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_owned();
                items.push((name.to_owned(), version));
            }
        }
    }

    items
}

fn parse_installed_from_json(
    manager_type: PackageManagerType,
    stdout: &str,
) -> CoreResult<Vec<PackageInfo>> {
    let json: serde_json::Value = serde_json::from_str(stdout)?;
    let mut installed = Vec::new();

    if let Some(nodes) = json.as_array() {
        for node in nodes {
            for (name, version) in parse_global_dependencies(node) {
                installed.push(PackageInfo {
                    name,
                    version,
                    source: manager_type,
                    description: None,
                    size: None,
                    install_date: None,
                    homepage: None,
                });
            }
        }
    } else {
        for (name, version) in parse_global_dependencies(&json) {
            installed.push(PackageInfo {
                name,
                version,
                source: manager_type,
                description: None,
                size: None,
                install_date: None,
                homepage: None,
            });
        }
    }

    installed.sort_by(|a, b| a.name.cmp(&b.name));
    installed.dedup_by(|a, b| a.name == b.name);

    Ok(installed)
}

async fn list_installed_by_manager(
    config: &Config,
    manager_type: PackageManagerType,
) -> CoreResult<Vec<PackageInfo>> {
    let path = command_path(config, manager_type);

    let output = Command::new(&path)
        .arg("ls")
        .arg("-g")
        .arg("--depth=0")
        .arg("--json")
        .output()
        .await?;

    if !output.status.success() {
        return Err(crate::error::CoreError::UnknownError(format!(
            "{} ls -g --depth=0 --json failed",
            manager_type.name()
        )));
    }

    let stdout = String::from_utf8(output.stdout)?;
    parse_installed_from_json(manager_type, &stdout)
}

async fn count_installed_by_manager(
    config: &Config,
    manager_type: PackageManagerType,
) -> CoreResult<usize> {
    Ok(list_installed_by_manager(config, manager_type).await?.len())
}

async fn get_current_version_by_manager(
    config: &Config,
    manager_type: PackageManagerType,
    package_name: &str,
) -> CoreResult<String> {
    let installed = list_installed_by_manager(config, manager_type).await?;

    installed
        .into_iter()
        .find(|pkg| pkg.name == package_name)
        .map(|pkg| pkg.version)
        .ok_or_else(|| {
            crate::error::CoreError::UnknownError(format!("Package {} not installed", package_name))
        })
}

fn parse_updates_from_json(stdout: &str) -> CoreResult<Vec<PackageUpdate>> {
    let json: serde_json::Value = serde_json::from_str(stdout)?;
    let mut updates = Vec::new();

    if let Some(obj) = json.as_object() {
        for (name, detail) in obj {
            let current = detail
                .get("current")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_owned();

            let new_version = detail
                .get("latest")
                .and_then(|v| v.as_str())
                .or_else(|| detail.get("wanted").and_then(|v| v.as_str()))
                .unwrap_or("unknown")
                .to_owned();

            if current != new_version {
                updates.push(PackageUpdate {
                    name: name.clone(),
                    current_version: current,
                    new_version,
                });
            }
        }
    } else if let Some(arr) = json.as_array() {
        for item in arr {
            let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
                continue;
            };

            let current = item
                .get("current")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_owned();

            let new_version = item
                .get("latest")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("wanted").and_then(|v| v.as_str()))
                .unwrap_or("unknown")
                .to_owned();

            if current != new_version {
                updates.push(PackageUpdate {
                    name: name.to_owned(),
                    current_version: current,
                    new_version,
                });
            }
        }
    }

    Ok(updates)
}

async fn list_updates_by_manager(
    config: &Config,
    manager_type: PackageManagerType,
) -> CoreResult<Vec<PackageUpdate>> {
    let path = command_path(config, manager_type);

    let mut command = Command::new(&path);
    match manager_type {
        PackageManagerType::Npm => {
            command.arg("outdated").arg("-g").arg("--json");
        }
        PackageManagerType::Pnpm => {
            command
                .arg("outdated")
                .arg("-g")
                .arg("--format")
                .arg("json");
        }
        _ => {}
    }

    let output = command.output().await?;
    let stdout = String::from_utf8(output.stdout)?;

    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed = parse_updates_from_json(&stdout);
    if parsed.is_ok() {
        return parsed;
    }

    if !output.status.success() {
        return Err(crate::error::CoreError::UnknownError(format!(
            "{} outdated failed",
            manager_type.name()
        )));
    }

    parsed
}

async fn search_package_by_manager(
    config: &Config,
    manager_type: PackageManagerType,
    package_name: &str,
) -> CoreResult<Vec<PackageInfo>> {
    let path = command_path(config, manager_type);
    let output = Command::new(&path)
        .arg("search")
        .arg(package_name)
        .arg("--json")
        .output()
        .await?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8(output.stdout)?;
    let json: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(value) => value,
        Err(_) => return Ok(Vec::new()),
    };

    let installed_versions: HashMap<String, String> =
        list_installed_by_manager(config, manager_type)
            .await?
            .into_iter()
            .map(|pkg| (pkg.name, pkg.version))
            .collect();

    let mut packages = Vec::new();

    if let Some(results) = json.as_array() {
        for item in results {
            let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
                continue;
            };

            let description = item
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());

            let homepage = item
                .get("links")
                .and_then(|links| links.get("homepage"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned())
                .or_else(|| {
                    item.get("links")
                        .and_then(|links| links.get("npm"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned())
                });

            packages.push(PackageInfo {
                name: name.to_owned(),
                version: installed_versions
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| "Not Installed".to_owned()),
                source: manager_type,
                description,
                size: None,
                install_date: None,
                homepage,
            });
        }
    }

    Ok(packages)
}

async fn run_global_package_command_with_progress(
    config: &Config,
    manager_type: PackageManagerType,
    action: &str,
    package_name: &str,
    on_progress: impl FnMut(CommandProgressEvent),
) -> CoreResult<()> {
    let path = command_path(config, manager_type);
    let args = global_package_command_args(manager_type, action, package_name)?;

    run_command_with_progress(&path, &args, on_progress).await
}

fn global_package_command_args(
    manager_type: PackageManagerType,
    action: &str,
    package_name: &str,
) -> CoreResult<Vec<String>> {
    match manager_type {
        PackageManagerType::Npm => match action {
            "install" => Ok(vec![
                "install".to_owned(),
                "-g".to_owned(),
                package_name.to_owned(),
            ]),
            "update" => Ok(vec![
                "install".to_owned(),
                "-g".to_owned(),
                format!("{}@latest", package_name),
            ]),
            "uninstall" => Ok(vec![
                "uninstall".to_owned(),
                "-g".to_owned(),
                package_name.to_owned(),
            ]),
            _ => Err(crate::error::CoreError::UnknownError(format!(
                "Unsupported npm action: {}",
                action
            ))),
        },
        PackageManagerType::Pnpm => match action {
            "install" => Ok(vec![
                "add".to_owned(),
                "-g".to_owned(),
                package_name.to_owned(),
            ]),
            "update" => Ok(vec![
                "add".to_owned(),
                "-g".to_owned(),
                format!("{}@latest", package_name),
            ]),
            "uninstall" => Ok(vec![
                "remove".to_owned(),
                "-g".to_owned(),
                package_name.to_owned(),
            ]),
            _ => Err(crate::error::CoreError::UnknownError(format!(
                "Unsupported pnpm action: {}",
                action
            ))),
        },
        _ => Err(crate::error::CoreError::UnknownError(format!(
            "Unsupported manager for global command: {:?}",
            manager_type
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_installed_from_json_supports_object_and_array() {
        let stdout = r#"[
          {
            "dependencies": {
              "eslint": { "version": "8.57.0" },
              "typescript": { "version": "5.8.2" }
            }
          },
          {
            "dependencies": [
              { "name": "pnpm", "version": "10.6.5" },
              { "name": "eslint", "version": "8.57.0" }
            ]
          }
        ]"#;

        let installed = parse_installed_from_json(PackageManagerType::Npm, stdout).unwrap();

        assert_eq!(installed.len(), 3);
        assert_eq!(installed[0].name, "eslint");
        assert_eq!(installed[0].version, "8.57.0");
        assert_eq!(installed[1].name, "pnpm");
        assert_eq!(installed[2].name, "typescript");
    }

    #[test]
    fn test_parse_installed_from_json_missing_version_is_unknown() {
        let stdout = r#"{
          "dependencies": {
            "corepack": {}
          }
        }"#;

        let installed = parse_installed_from_json(PackageManagerType::Npm, stdout).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "corepack");
        assert_eq!(installed[0].version, "unknown");
    }

    #[test]
    fn test_parse_updates_from_json_supports_object_and_array() {
        let object_stdout = r#"{
          "eslint": { "current": "8.57.0", "latest": "9.0.0" },
          "typescript": { "current": "5.8.2", "wanted": "5.8.2" }
        }"#;

        let object_updates = parse_updates_from_json(object_stdout).unwrap();
        assert_eq!(object_updates.len(), 1);
        assert_eq!(object_updates[0].name, "eslint");
        assert_eq!(object_updates[0].current_version, "8.57.0");
        assert_eq!(object_updates[0].new_version, "9.0.0");

        let array_stdout = r#"[
          { "name": "pnpm", "current": "10.6.5", "latest": "10.7.0" },
          { "name": "npm", "current": "10.9.2", "wanted": "10.9.2" }
        ]"#;

        let array_updates = parse_updates_from_json(array_stdout).unwrap();
        assert_eq!(array_updates.len(), 1);
        assert_eq!(array_updates[0].name, "pnpm");
    }

    #[test]
    fn test_global_package_command_args_for_npm_and_pnpm() {
        assert_eq!(
            global_package_command_args(PackageManagerType::Npm, "install", "eslint").unwrap(),
            vec!["install".to_owned(), "-g".to_owned(), "eslint".to_owned()]
        );

        assert_eq!(
            global_package_command_args(PackageManagerType::Pnpm, "install", "eslint").unwrap(),
            vec!["add".to_owned(), "-g".to_owned(), "eslint".to_owned()]
        );
    }

    #[test]
    fn test_global_package_update_args_force_latest() {
        assert_eq!(
            global_package_command_args(PackageManagerType::Npm, "update", "@google/gemini-cli")
                .unwrap(),
            vec![
                "install".to_owned(),
                "-g".to_owned(),
                "@google/gemini-cli@latest".to_owned()
            ]
        );

        assert_eq!(
            global_package_command_args(PackageManagerType::Pnpm, "update", "@google/gemini-cli")
                .unwrap(),
            vec![
                "add".to_owned(),
                "-g".to_owned(),
                "@google/gemini-cli@latest".to_owned()
            ]
        );
    }

    #[test]
    fn test_global_package_command_args_rejects_unsupported_action() {
        let result = global_package_command_args(PackageManagerType::Npm, "remove", "eslint");
        assert!(result.is_err());
    }
}
