//! Structured, bounded operation activity history.

use std::collections::VecDeque;
use std::path::PathBuf;

use directories_next::ProjectDirs;
use serde::{Deserialize, Serialize};
use updater_manager_api::{ManagerId, PackageAction};

use crate::{content::OperationOutcome, manager_catalog::ManagerCatalog};

const MAX_ACTIVITY_RECORDS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityRecord {
    pub id: u64,
    pub action: String,
    pub completed_packages: usize,
    pub total_packages: usize,
    pub completed_managers: usize,
    pub total_managers: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_manager: Option<ManagerId>,
    pub error: Option<String>,
}

impl ActivityRecord {
    pub fn from_outcome(id: u64, outcome: &OperationOutcome) -> Self {
        Self {
            id,
            action: match outcome.action {
                PackageAction::Install => "Install",
                PackageAction::Update => "Update",
                PackageAction::Uninstall => "Remove",
                _ => "Package action",
            }
            .to_owned(),
            completed_packages: outcome.completed_packages,
            total_packages: outcome.total_packages,
            completed_managers: outcome.completed_managers,
            total_managers: outcome.total_managers,
            failed_manager: outcome.failed_manager.clone(),
            error: outcome.error.as_deref().map(redact_detail),
        }
    }

    pub fn title(&self) -> String {
        format!("Operation #{} · {}", self.id, self.action)
    }

    pub fn summary(&self, catalog: &ManagerCatalog) -> String {
        let manager_name = self
            .failed_manager
            .as_ref()
            .map(|manager| catalog.display_name(manager));
        let manager_suffix = manager_name
            .map(|manager| format!(" · failed at {manager}"))
            .unwrap_or_default();
        let error_suffix = self
            .error
            .as_deref()
            .map(|error| format!(" · {error}"))
            .unwrap_or_default();
        format!(
            "{}/{} packages · {}/{} sources{}{}",
            self.completed_packages,
            self.total_packages,
            self.completed_managers,
            self.total_managers,
            manager_suffix,
            error_suffix,
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActivityHistory {
    records: VecDeque<ActivityRecord>,
}

impl ActivityHistory {
    pub async fn load() -> Self {
        let Some(path) = history_path() else {
            return Self::default();
        };
        let Ok(json) = tokio::fs::read_to_string(path).await else {
            return Self::default();
        };
        let Ok(records) = serde_json::from_str::<VecDeque<ActivityRecord>>(&json) else {
            return Self::default();
        };
        Self {
            records: records.into_iter().take(MAX_ACTIVITY_RECORDS).collect(),
        }
    }

    pub async fn save(&self) -> Result<(), String> {
        let path = history_path().ok_or_else(|| "No application data directory".to_owned())?;
        let parent = path
            .parent()
            .ok_or_else(|| "Invalid activity history path".to_owned())?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
        let json =
            serde_json::to_string_pretty(&self.records).map_err(|error| error.to_string())?;
        tokio::fs::write(path, json)
            .await
            .map_err(|error| error.to_string())
    }

    pub fn push(&mut self, record: ActivityRecord) {
        self.records.push_front(record);
        self.records.truncate(MAX_ACTIVITY_RECORDS);
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ActivityRecord> {
        self.records.iter()
    }
}

fn history_path() -> Option<PathBuf> {
    ProjectDirs::from("com", "ayi", "updater")
        .map(|directories| directories.data_local_dir().join("activity.json"))
}

fn redact_detail(detail: &str) -> String {
    let redacted = detail
        .split_whitespace()
        .map(|part| {
            if part.starts_with('/') || part.contains("token=") || part.contains("password=") {
                "<redacted>"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    const MAX_DETAIL_CHARS: usize = 240;
    if redacted.chars().count() > MAX_DETAIL_CHARS {
        format!(
            "{}…",
            redacted.chars().take(MAX_DETAIL_CHARS).collect::<String>()
        )
    } else {
        redacted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_newest_first() {
        let mut history = ActivityHistory::default();
        for id in 1..=60 {
            history.push(ActivityRecord {
                id,
                action: "Update".to_owned(),
                completed_packages: 1,
                total_packages: 1,
                completed_managers: 1,
                total_managers: 1,
                failed_manager: None,
                error: None,
            });
        }

        assert_eq!(history.iter().count(), MAX_ACTIVITY_RECORDS);
        assert_eq!(history.iter().next().map(|record| record.id), Some(60));
    }

    #[test]
    fn activity_details_redact_paths_and_credentials() {
        assert_eq!(
            redact_detail("failed /home/user/private token=secret safe"),
            "failed <redacted> <redacted> safe"
        );
    }

    #[test]
    fn current_schema_round_trip_preserves_manager_id() {
        let record = ActivityRecord {
            id: 4,
            action: "Update".to_owned(),
            completed_packages: 0,
            total_packages: 1,
            completed_managers: 0,
            total_managers: 1,
            failed_manager: Some(ManagerId::parse("builtin:dnf").unwrap()),
            error: Some("failed".to_owned()),
        };

        let json = serde_json::to_string(&record).unwrap();
        let decoded = serde_json::from_str::<ActivityRecord>(&json).unwrap();

        assert_eq!(decoded, record);
    }

    #[test]
    fn versioned_activity_schema_is_rejected() {
        let result = serde_json::from_str::<ActivityRecord>(
            r#"{"version":2,"id":4,"action":"Update","completed_packages":0,"total_packages":1,"completed_managers":0,"total_managers":1,"failed_manager":"builtin:dnf","error":"failed"}"#,
        );

        assert!(result.is_err());
    }
}
