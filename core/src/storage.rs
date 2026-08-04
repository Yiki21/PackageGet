use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use directories_next::ProjectDirs;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use updater_manager_api::{ManagerCategory, ManagerConfig, ManagerId};

use crate::{CoreResult, ManagerRegistry, error::CoreError};

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Persisted application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Enabled package managers and their manager-owned settings.
    pub managers: Vec<ManagerConfig>,
    /// Preferred desktop appearance: system, light, dark, or high_contrast.
    #[serde(default = "default_appearance")]
    pub appearance: String,
    /// Whether native completion and failure notifications are enabled.
    #[serde(default)]
    pub notifications_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            managers: Vec::new(),
            appearance: default_appearance(),
            notifications_enabled: false,
        }
    }
}

fn default_appearance() -> String {
    "system".to_owned()
}

impl Config {
    /// Returns the current user's configuration file path.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::ConfigError`] when the platform configuration
    /// directory cannot be determined.
    pub fn file_path() -> CoreResult<PathBuf> {
        let config_dir = ProjectDirs::from("com", "ayi", "updater")
            .ok_or_else(|| CoreError::ConfigError("could not determine config directory".into()))?;
        Ok(config_dir.config_dir().join("config.json"))
    }

    /// Loads the configuration, or detects managers and creates it when absent.
    pub async fn load(registry: &ManagerRegistry) -> CoreResult<Self> {
        Self::load_from_path(Self::file_path()?, registry).await
    }

    /// Loads a configuration from `path`, creating a detected default when absent.
    pub async fn load_from_path(
        path: impl AsRef<Path>,
        registry: &ManagerRegistry,
    ) -> CoreResult<Self> {
        let path = path.as_ref();
        if path.exists() {
            let config = Self::read_from_path(path).await?;
            config.validate_registered_managers(registry)?;
            return Ok(config);
        }

        let config = Self::detect_package_managers(registry).await;
        config.save_to_path(path).await?;
        Ok(config)
    }

    /// Reads and validates an existing configuration document.
    pub async fn read_from_path(path: impl AsRef<Path>) -> CoreResult<Self> {
        let json = tokio::fs::read_to_string(path.as_ref())
            .await
            .map_err(|error| config_io_error("read", path.as_ref(), error))?;
        let config: Self = serde_json::from_str(&json).map_err(|error| {
            CoreError::ConfigError(format!(
                "invalid configuration document at {}: {error}",
                path.as_ref().display()
            ))
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Detects available built-in package managers and creates a configuration.
    pub async fn detect_package_managers(registry: &ManagerRegistry) -> Self {
        let mut system_manager = None;
        let mut other_managers = Vec::new();

        for manager in registry.managers() {
            let manager_config = ManagerConfig::new(manager.descriptor().id().clone());
            let is_available = manager
                .availability(&manager_config)
                .await
                .is_ok_and(|availability| availability.is_available());
            if !is_available {
                continue;
            }

            if manager.descriptor().category() == ManagerCategory::System {
                if system_manager.is_none() {
                    system_manager = Some(manager_config);
                }
            } else {
                other_managers.push(manager_config);
            }
        }

        Self {
            managers: system_manager.into_iter().chain(other_managers).collect(),
            ..Self::default()
        }
    }

    /// Reloads the configuration when its persistent file exists.
    pub async fn reload(&mut self) -> CoreResult<()> {
        let path = Self::file_path()?;
        if path.exists() {
            *self = Self::read_from_path(path).await?;
        }
        Ok(())
    }

    /// Saves the configuration using an atomic replacement in its config directory.
    pub async fn save(&self) -> CoreResult<()> {
        self.save_to_path(Self::file_path()?).await
    }

    /// Validates and atomically saves the configuration to `path`.
    pub async fn save_to_path(&self, path: impl AsRef<Path>) -> CoreResult<()> {
        self.validate()?;

        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| config_io_error("create directory for", path, error))?;

        let mut json = serde_json::to_vec_pretty(self).map_err(|error| {
            CoreError::ConfigError(format!("could not serialize configuration: {error}"))
        })?;
        json.push(b'\n');

        let temporary_path = temporary_path(path);
        let write_result = async {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temporary_path)
                .await?;
            file.write_all(&json).await?;
            file.flush().await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&temporary_path, path).await
        }
        .await;

        if let Err(error) = write_result {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(config_io_error("atomically write", path, error));
        }

        Ok(())
    }

    /// Validates invariants required by the storage and runtime contract.
    pub fn validate(&self) -> CoreResult<()> {
        let mut ids = HashSet::with_capacity(self.managers.len());
        for manager in &self.managers {
            if !ids.insert(&manager.id) {
                return Err(CoreError::ConfigError(format!(
                    "duplicate manager ID in configuration: {}",
                    manager.id
                )));
            }
            if !manager.settings.is_object() {
                return Err(CoreError::ConfigError(format!(
                    "settings for manager {} must be a JSON object",
                    manager.id
                )));
            }
        }

        Ok(())
    }

    /// Validates settings owned by manager implementations registered in this build.
    ///
    /// Unknown manager configurations remain opaque so another build can preserve
    /// and later restore them.
    pub fn validate_registered_managers(&self, registry: &ManagerRegistry) -> CoreResult<()> {
        for config in &self.managers {
            let Some(manager) = registry.get(&config.id) else {
                continue;
            };
            manager.validate_config(config).map_err(|error| {
                CoreError::ConfigError(format!("invalid settings for {}: {error}", config.id))
            })?;
        }
        Ok(())
    }

    /// Returns a configured manager by stable ID.
    #[must_use]
    pub fn manager(&self, id: &ManagerId) -> Option<&ManagerConfig> {
        self.managers.iter().find(|manager| &manager.id == id)
    }

    /// Returns a mutable configured manager by stable ID.
    pub fn manager_mut(&mut self, id: &ManagerId) -> Option<&mut ManagerConfig> {
        self.managers.iter_mut().find(|manager| &manager.id == id)
    }

    /// Returns the configured executable path for a manager.
    #[must_use]
    pub fn manager_executable(&self, id: &ManagerId) -> Option<PathBuf> {
        self.manager(id)
            .and_then(|manager| manager.executable().map(Path::to_path_buf))
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "config.json".into());
    path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ))
}

fn config_io_error(action: &str, path: &Path, error: std::io::Error) -> CoreError {
    CoreError::ConfigError(format!(
        "could not {action} configuration at {}: {error}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value};
    use tempfile::tempdir;
    use updater_managers::{
        configured_go_bin_dir, configured_nix_profile, set_configured_go_bin_dir,
        set_configured_nix_profile,
    };

    use super::*;

    fn manager(id: &str) -> ManagerConfig {
        ManagerConfig::new(ManagerId::parse(id).expect("valid test manager ID"))
    }

    #[test]
    fn current_config_path_uses_config_json() {
        let path = Config::file_path().expect("resolve current config path");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("config.json")
        );
    }

    #[tokio::test]
    async fn round_trip_preserves_unknown_manager_and_settings() {
        let directory = tempdir().expect("create temporary directory");
        let path = directory.path().join("config.json");
        let mut unknown = manager("org.example:tool").with_executable("/opt/tool/bin/tool");
        unknown.settings = serde_json::json!({"channel": "edge", "nested": {"enabled": true}});
        let config = Config {
            managers: vec![unknown],
            appearance: "dark".to_owned(),
            notifications_enabled: true,
        };

        config.save_to_path(&path).await.expect("save config");
        let loaded = Config::read_from_path(&path).await.expect("load config");

        assert_eq!(loaded, config);
    }

    #[tokio::test]
    async fn rejects_missing_managers() {
        let directory = tempdir().expect("create temporary directory");
        let path = directory.path().join("config.json");
        tokio::fs::write(&path, r#"{"appearance":"system"}"#)
            .await
            .expect("write fixture");

        let error = Config::read_from_path(&path)
            .await
            .expect_err("missing managers must fail");

        assert!(error.to_string().contains("managers"));
    }

    #[tokio::test]
    async fn rejects_unknown_top_level_fields_without_rewriting_file() {
        let directory = tempdir().expect("create temporary directory");
        let path = directory.path().join("config.json");
        let original = r#"{"managers":[],"obsolete":true}"#;
        tokio::fs::write(&path, original)
            .await
            .expect("write fixture");

        let error = Config::read_from_path(&path)
            .await
            .expect_err("unknown top-level field must fail");

        assert!(error.to_string().contains("unknown field `obsolete`"));
        assert_eq!(
            tokio::fs::read_to_string(path)
                .await
                .expect("read unchanged fixture"),
            original
        );
    }

    #[tokio::test]
    async fn rejects_malformed_document_without_rewriting_file() {
        let directory = tempdir().expect("create temporary directory");
        let path = directory.path().join("config.json");
        let original = r#"{"managers":["#;
        tokio::fs::write(&path, original)
            .await
            .expect("write malformed fixture");

        let error = Config::read_from_path(&path)
            .await
            .expect_err("malformed document must fail");

        assert!(error.to_string().contains("invalid configuration document"));
        assert_eq!(
            tokio::fs::read_to_string(path)
                .await
                .expect("read unchanged fixture"),
            original
        );
    }

    #[test]
    fn rejects_duplicate_manager_ids() {
        let config = Config {
            managers: vec![manager("builtin:cargo"), manager("builtin:cargo")],
            ..Config::default()
        };

        let error = config.validate().expect_err("duplicate IDs must fail");

        assert!(error.to_string().contains("duplicate manager ID"));
    }

    #[test]
    fn rejects_non_object_manager_settings() {
        let mut cargo = manager("builtin:cargo");
        cargo.settings = Value::Null;
        let config = Config {
            managers: vec![cargo],
            ..Config::default()
        };

        let error = config
            .validate()
            .expect_err("non-object settings must fail");

        assert!(error.to_string().contains("must be a JSON object"));
    }

    #[test]
    fn go_bin_dir_is_owned_by_go_manager_settings() {
        let mut config = Config {
            managers: vec![manager("builtin:go")],
            ..Config::default()
        };

        set_configured_go_bin_dir(&mut config.managers[0], Some("/custom/go/bin".into()))
            .expect("set Go bin directory");
        assert_eq!(
            configured_go_bin_dir(&config.managers[0])
                .expect("read Go bin directory")
                .as_deref()
                .and_then(|path| path.to_str()),
            Some("/custom/go/bin")
        );

        set_configured_go_bin_dir(&mut config.managers[0], None).expect("clear Go bin directory");
        assert!(
            configured_go_bin_dir(&config.managers[0])
                .expect("read cleared Go bin directory")
                .is_none()
        );
        assert_eq!(config.managers[0].settings, Value::Object(Map::new()));
    }

    #[test]
    fn nix_profile_is_explicit_and_owned_by_manager_settings() {
        let mut config = Config {
            managers: vec![manager("builtin:nix-profile")],
            ..Config::default()
        };

        assert!(configured_nix_profile(&config.managers[0]).is_err());
        set_configured_nix_profile(
            &mut config.managers[0],
            "/home/test/.local/state/nix/profiles/profile".into(),
        )
        .expect("set explicit Nix profile");
        assert_eq!(
            configured_nix_profile(&config.managers[0])
                .expect("read explicit Nix profile")
                .to_str(),
            Some("/home/test/.local/state/nix/profiles/profile")
        );
    }

    #[test]
    fn nix_profile_rejects_relative_and_system_paths() {
        let mut config = Config {
            managers: vec![manager("builtin:nix-profile")],
            ..Config::default()
        };

        for profile in [
            "profiles/current",
            "/nix/var/nix/profiles/system",
            "/nix/var/nix/profiles/system-42-link",
            "/nix/var/nix/profiles/default",
        ] {
            assert!(set_configured_nix_profile(&mut config.managers[0], profile.into()).is_err());
        }
    }

    #[tokio::test]
    async fn atomic_save_replaces_existing_document_and_removes_temporary_file() {
        let directory = tempdir().expect("create temporary directory");
        let path = directory.path().join("config.json");
        tokio::fs::write(&path, "old document")
            .await
            .expect("write old document");
        let config = Config {
            managers: vec![manager("builtin:cargo")],
            ..Config::default()
        };

        config.save_to_path(&path).await.expect("replace config");

        assert_eq!(
            Config::read_from_path(&path)
                .await
                .expect("read replacement"),
            config
        );
        let entries = std::fs::read_dir(directory.path())
            .expect("list config directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect directory entries");
        assert_eq!(entries.len(), 1);
    }

    #[tokio::test]
    async fn validation_failure_does_not_replace_existing_document() {
        let directory = tempdir().expect("create temporary directory");
        let path = directory.path().join("config.json");
        let original = "existing configuration";
        tokio::fs::write(&path, original)
            .await
            .expect("write existing document");
        let invalid = Config {
            managers: vec![manager("builtin:cargo"), manager("builtin:cargo")],
            ..Config::default()
        };

        invalid
            .save_to_path(&path)
            .await
            .expect_err("invalid config must not be saved");

        assert_eq!(
            tokio::fs::read_to_string(path)
                .await
                .expect("read unchanged document"),
            original
        );
    }
}
