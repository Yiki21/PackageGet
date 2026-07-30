use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use updater_manager_api::{ManagerId, PackageAction, PackageTarget, ProgressEvent, ProgressSink};

use crate::{Config, ManagerRegistry};

/// Cooperative cancellation checked between manager groups.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Requests cancellation before the next manager group starts.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Cross-manager progress reported by a package operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationProgress {
    /// Number of completed packages across all manager groups.
    pub completed: usize,
    /// Total number of requested packages.
    pub total: usize,
    /// Manager currently executing.
    pub manager: ManagerId,
    /// Current package when the manager reports one.
    pub current_package: String,
    /// Optional manager diagnostic output.
    pub command_message: Option<String>,
}

/// Final result of a cross-manager package operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationOutcome {
    /// Requested action.
    pub action: PackageAction,
    /// Number of packages completed before the operation stopped.
    pub completed_packages: usize,
    /// Total number of requested packages.
    pub total_packages: usize,
    /// Number of manager groups completed successfully.
    pub completed_managers: usize,
    /// Total number of non-empty manager groups.
    pub total_managers: usize,
    /// Manager where execution failed, when applicable.
    pub failed_manager: Option<ManagerId>,
    /// Failure or cancellation detail.
    pub error: Option<String>,
}

impl OperationOutcome {
    /// Returns whether every requested package completed successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.error.is_none() && self.completed_packages == self.total_packages
    }

    /// Returns a compact user-facing operation summary.
    #[must_use]
    pub fn summary(&self) -> String {
        let completed_verb = match self.action {
            PackageAction::Install => "installed",
            PackageAction::Update => "updated",
            PackageAction::Uninstall => "removed",
            _ => "processed",
        };

        if self.is_success() {
            return format!(
                "{} package{} {} across {} source{}",
                self.total_packages,
                if self.total_packages == 1 { "" } else { "s" },
                completed_verb,
                self.total_managers,
                if self.total_managers == 1 { "" } else { "s" },
            );
        }

        format!(
            "{} of {} packages {} before the operation stopped",
            self.completed_packages, self.total_packages, completed_verb,
        )
    }
}

/// Executes package groups sequentially in their supplied manager order.
///
/// Cancellation is observed before each manager group. A manager command that
/// has already started is allowed to finish so core never claims to interrupt
/// an external process that may still be changing the system.
pub async fn execute_package_groups(
    registry: &ManagerRegistry,
    config: &Config,
    action: PackageAction,
    manager_groups: &[(ManagerId, Vec<String>)],
    cancellation: &CancellationToken,
    on_progress: &(dyn Fn(OperationProgress) + Send + Sync),
) -> OperationOutcome {
    let total_packages = manager_groups
        .iter()
        .map(|(_, packages)| packages.len())
        .sum();
    let total_managers = manager_groups
        .iter()
        .filter(|(_, packages)| !packages.is_empty())
        .count();
    let mut completed_packages = 0;
    let mut completed_managers = 0;

    for (manager_id, package_names) in manager_groups
        .iter()
        .filter(|(_, packages)| !packages.is_empty())
    {
        if cancellation.is_cancelled() {
            return OperationOutcome {
                action,
                completed_packages,
                total_packages,
                completed_managers,
                total_managers,
                failed_manager: None,
                error: Some("Stopped before starting another manager".to_owned()),
            };
        }

        let manager = match registry.manager_for(manager_id, action.capability()) {
            Ok(manager) => manager,
            Err(error) => {
                return OperationOutcome {
                    action,
                    completed_packages,
                    total_packages,
                    completed_managers,
                    total_managers,
                    failed_manager: Some(manager_id.clone()),
                    error: Some(error.to_string()),
                };
            }
        };
        let Some(manager_config) = config.manager(manager_id) else {
            return OperationOutcome {
                action,
                completed_packages,
                total_packages,
                completed_managers,
                total_managers,
                failed_manager: Some(manager_id.clone()),
                error: Some(format!("manager is not configured: {manager_id}")),
            };
        };
        let targets = package_names
            .iter()
            .map(|name| PackageTarget::new(manager_id.clone(), name))
            .collect::<Vec<_>>();
        let manager_completed = AtomicUsize::new(0);
        let progress = |event| match event {
            ProgressEvent::Advanced {
                completed,
                current_package,
                ..
            } => {
                let completed = completed.min(targets.len());
                manager_completed.fetch_max(completed, Ordering::Relaxed);
                on_progress(OperationProgress {
                    completed: completed_packages + completed,
                    total: total_packages,
                    manager: manager_id.clone(),
                    current_package: current_package.unwrap_or_default(),
                    command_message: None,
                });
            }
            ProgressEvent::Message { message } => on_progress(OperationProgress {
                completed: completed_packages + manager_completed.load(Ordering::Relaxed),
                total: total_packages,
                manager: manager_id.clone(),
                current_package: String::new(),
                command_message: Some(message),
            }),
            ProgressEvent::Finished { completed, .. } => {
                let completed = completed.min(targets.len());
                manager_completed.fetch_max(completed, Ordering::Relaxed);
                on_progress(OperationProgress {
                    completed: completed_packages + completed,
                    total: total_packages,
                    manager: manager_id.clone(),
                    current_package: String::new(),
                    command_message: None,
                });
            }
            ProgressEvent::Started { .. } => {}
            _ => {}
        };

        if let Err(error) = manager
            .execute(
                manager_config,
                action,
                &targets,
                &progress as &dyn ProgressSink,
            )
            .await
        {
            let detail = error.detail().map_or_else(
                || error.message().to_owned(),
                |detail| format!("{}: {detail}", error.message()),
            );
            let action_name = match action {
                PackageAction::Install => "install",
                PackageAction::Update => "update",
                PackageAction::Uninstall => "remove",
                _ => "process",
            };
            return OperationOutcome {
                action,
                completed_packages: completed_packages
                    + manager_completed.load(Ordering::Relaxed).min(targets.len()),
                total_packages,
                completed_managers,
                total_managers,
                failed_manager: Some(manager_id.clone()),
                error: Some(format!(
                    "Failed to {action_name} packages from {manager_id}: {detail}"
                )),
            };
        }

        completed_packages += targets.len();
        completed_managers += 1;
    }

    OperationOutcome {
        action,
        completed_packages,
        total_packages,
        completed_managers,
        total_managers,
        failed_manager: None,
        error: None,
    }
}
