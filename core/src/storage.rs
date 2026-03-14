use directories_next::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::{
    ALL_APP_PACKAGE_MANAGERS, ALL_SYSTEM_PACKAGE_MANAGERS, CoreResult, PackageManagerType,
    error::CoreError,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub system_manager: Option<PackageManagerConfig>,
    pub app_managers: Vec<PackageManagerConfig>,
    /// 自定义 Go bin 目录，如果为 None 则使用默认规则（GOBIN > GOPATH/bin > ~/go/bin）
    pub go_bin_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManagerConfig {
    pub manager_type: PackageManagerType,
    pub custom_path: Option<String>,
}

impl Config {
    /// Load configuration from file, or detect and initialize if not exists
    pub async fn load() -> CoreResult<Self> {
        let config_dir = ProjectDirs::from("com", "ayi", "updater").ok_or_else(|| {
            CoreError::UnknownError("Could not determine config directory".into())
        })?;

        let path = config_dir.config_dir().join("config.json");

        // If config file exists, load it
        if path.exists() {
            let json = tokio::fs::read_to_string(&path).await?;
            let config = serde_json::from_str::<Config>(&json)?;
            return Ok(config);
        }

        // Otherwise, detect available package managers and create config
        let config = Self::detect_package_managers().await;

        // Save the detected configuration
        config.save().await?;

        Ok(config)
    }

    /// Detect Package Managers in $PATH and initialize config
    pub async fn detect_package_managers() -> Self {
        let mut system_manager: Option<PackageManagerConfig> = None;
        if let Some(system_type) = Self::detect_system_manager_type().await {
            system_manager = Some(PackageManagerConfig {
                manager_type: system_type,
                custom_path: None,
            });
        }

        let app_managers: Vec<PackageManagerConfig> = Self::detect_available_app_managers()
            .await
            .into_iter()
            .map(|manager_type| PackageManagerConfig {
                manager_type,
                custom_path: None,
            })
            .collect();

        Config {
            system_manager,
            app_managers,
            go_bin_dir: None,
        }
    }

    /// Detect available system manager in $PATH (first match by priority)
    pub async fn detect_system_manager_type() -> Option<PackageManagerType> {
        for manager_type in &ALL_SYSTEM_PACKAGE_MANAGERS {
            if manager_type.is_available().await {
                return Some(*manager_type);
            }
        }
        None
    }

    /// Detect available app package managers in $PATH
    pub async fn detect_available_app_managers() -> Vec<PackageManagerType> {
        let mut managers = Vec::new();

        for manager_type in &ALL_APP_PACKAGE_MANAGERS {
            if manager_type.is_available().await {
                managers.push(*manager_type);
            }
        }

        managers
    }

    /// Reload configuration from file
    pub async fn reload(&mut self) -> CoreResult<()> {
        let config_dir = ProjectDirs::from("com", "ayi", "updater").ok_or_else(|| {
            CoreError::UnknownError("Could not determine config directory".into())
        })?;

        let path = config_dir.config_dir().join("config.json");

        if path.exists() {
            let json = tokio::fs::read_to_string(&path).await?;
            let loaded = serde_json::from_str::<Config>(&json)?;
            *self = loaded;
        }

        Ok(())
    }

    /// Save configuration to file
    pub async fn save(&self) -> CoreResult<()> {
        let config_dir = ProjectDirs::from("com", "ayi", "updater").ok_or_else(|| {
            CoreError::UnknownError("Could not determine config directory".into())
        })?;

        let dir_path = config_dir.config_dir();
        let file_path = dir_path.join("config.json");

        // Create config directory if it doesn't exist
        tokio::fs::create_dir_all(dir_path).await?;

        let json = serde_json::to_string_pretty(self)?;
        tokio::fs::write(&file_path, json).await?;

        Ok(())
    }

    pub fn get_package_path(&self, manager_type: PackageManagerType) -> Option<String> {
        if let Some(ref sys_mgr) = self.system_manager
            && sys_mgr.manager_type == manager_type
        {
            return sys_mgr.custom_path.clone();
        }

        for app_mgr in &self.app_managers {
            if app_mgr.manager_type == manager_type {
                return app_mgr.custom_path.clone();
            }
        }

        None
    }

    pub fn get_go_bin_dir(&self) -> String {
        use directories_next::UserDirs;
        use std::env;

        if let Some(ref dir) = self.go_bin_dir {
            return dir.clone();
        }

        // 按优先级检查环境变量
        if let Ok(gobin) = env::var("GOBIN") {
            return gobin;
        }

        if let Ok(gopath) = env::var("GOPATH") {
            return format!("{}/bin", gopath);
        }

        // 默认使用 ~/go/bin
        UserDirs::new()
            .and_then(|dirs| {
                dirs.home_dir()
                    .join("go/bin")
                    .to_str()
                    .map(|s| s.to_owned())
            })
            .unwrap_or_else(|| "go/bin".to_owned())
    }
}
