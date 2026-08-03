use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use serde::{Deserialize, Serialize};
use updater_manager_api::{
    ManagerErrorKind, ManagerId, PackageAction, PackageScope, PackageTarget, ProgressEvent,
    ProgressSink,
};

use crate::{Config, ManagerRegistry};

/// Cooperative cancellation shared with the active manager command.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Requests cancellation of the active manager command.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Whether execution stopped in response to a confirmed cancellation.
    pub cancelled: bool,
    /// Per-manager results in execution order.
    pub manager_outcomes: Vec<ManagerOperationOutcome>,
    /// Aggregate scope when all targets share one scope, otherwise unknown.
    pub scope: PackageScope,
}

/// Stable result for one manager group within an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerOperationOutcome {
    /// Manager that received the group.
    pub manager_id: ManagerId,
    /// Scope shared by this group, or unknown when mixed.
    pub scope: PackageScope,
    /// Number of targets requested for this manager.
    pub requested_packages: usize,
    /// Number of targets reported complete before the manager stopped.
    pub completed_packages: usize,
    /// Final manager status.
    pub status: ManagerOperationStatus,
    /// Manager-specific failure or cancellation detail.
    pub error: Option<String>,
}

/// Final status of one manager group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagerOperationStatus {
    /// The manager group completed successfully.
    Succeeded,
    /// The manager group returned a non-cancellation error.
    Failed,
    /// The manager command exited after cancellation was requested.
    Cancelled,
    /// The manager group was not started because an earlier group stopped execution.
    NotStarted,
}

impl OperationOutcome {
    /// Returns whether every requested package completed successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        !self.cancelled && self.error.is_none() && self.completed_packages == self.total_packages
    }

    /// Returns whether the active command exited after cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
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

        if self.is_cancelled() {
            return format!(
                "Operation cancelled after {} of {} packages",
                self.completed_packages, self.total_packages,
            );
        }

        format!(
            "{} of {} packages {} before the operation stopped",
            self.completed_packages, self.total_packages, completed_verb,
        )
    }
}

struct CancellableProgressSink<'a> {
    cancellation: &'a CancellationToken,
    emit: &'a (dyn Fn(ProgressEvent) + Send + Sync),
}

impl ProgressSink for CancellableProgressSink<'_> {
    fn emit(&self, event: ProgressEvent) {
        (self.emit)(event);
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

/// Executes package groups sequentially in their supplied manager order.
///
/// Cancellation is observed before each manager group and exposed to the active
/// manager through its progress sink. Core only reports cancellation after the
/// manager confirms that its command has exited.
pub async fn execute_package_groups(
    registry: &ManagerRegistry,
    config: &Config,
    action: PackageAction,
    manager_groups: &[(ManagerId, Vec<PackageTarget>)],
    cancellation: &CancellationToken,
    on_progress: &(dyn Fn(OperationProgress) + Send + Sync),
) -> OperationOutcome {
    let total_packages = manager_groups
        .iter()
        .map(|(_, packages)| packages.len())
        .sum();
    let groups: Vec<_> = manager_groups
        .iter()
        .filter(|(_, packages)| !packages.is_empty())
        .collect();
    let total_managers = groups.len();
    let scope = aggregate_scope(manager_groups.iter().flat_map(|(_, targets)| targets));
    let mut completed_packages = 0;
    let mut completed_managers = 0;
    let mut manager_outcomes = Vec::new();

    for (group_index, group) in groups.iter().enumerate() {
        let (manager_id, targets) = *group;
        if cancellation.is_cancelled() {
            manager_outcomes.push(ManagerOperationOutcome {
                manager_id: manager_id.clone(),
                scope: aggregate_scope(targets.iter()),
                requested_packages: targets.len(),
                completed_packages: 0,
                status: ManagerOperationStatus::NotStarted,
                error: Some("Stopped before starting this manager".to_owned()),
            });
            append_not_started_outcomes(
                &mut manager_outcomes,
                groups.iter().copied().skip(group_index + 1),
            );
            return OperationOutcome {
                action,
                completed_packages,
                total_packages,
                completed_managers,
                total_managers,
                failed_manager: None,
                error: Some("Stopped before starting another manager".to_owned()),
                cancelled: true,
                manager_outcomes,
                scope,
            };
        }

        let manager = match registry.manager_for(manager_id, action.capability()) {
            Ok(manager) => manager,
            Err(error) => {
                manager_outcomes.push(ManagerOperationOutcome {
                    manager_id: manager_id.clone(),
                    scope: aggregate_scope(targets.iter()),
                    requested_packages: targets.len(),
                    completed_packages: 0,
                    status: ManagerOperationStatus::Failed,
                    error: Some(error.to_string()),
                });
                append_not_started_outcomes(
                    &mut manager_outcomes,
                    groups.iter().copied().skip(group_index + 1),
                );
                return OperationOutcome {
                    action,
                    completed_packages,
                    total_packages,
                    completed_managers,
                    total_managers,
                    failed_manager: Some(manager_id.clone()),
                    error: Some(error.to_string()),
                    cancelled: false,
                    manager_outcomes,
                    scope,
                };
            }
        };
        let Some(manager_config) = config.manager(manager_id) else {
            manager_outcomes.push(ManagerOperationOutcome {
                manager_id: manager_id.clone(),
                scope: aggregate_scope(targets.iter()),
                requested_packages: targets.len(),
                completed_packages: 0,
                status: ManagerOperationStatus::Failed,
                error: Some(format!("manager is not configured: {manager_id}")),
            });
            append_not_started_outcomes(
                &mut manager_outcomes,
                groups.iter().copied().skip(group_index + 1),
            );
            return OperationOutcome {
                action,
                completed_packages,
                total_packages,
                completed_managers,
                total_managers,
                failed_manager: Some(manager_id.clone()),
                error: Some(format!("manager is not configured: {manager_id}")),
                cancelled: false,
                manager_outcomes,
                scope,
            };
        };
        if targets
            .iter()
            .any(|target| &target.manager_id != manager_id)
        {
            let error =
                format!("package target belongs to a different manager group: {manager_id}");
            manager_outcomes.push(ManagerOperationOutcome {
                manager_id: manager_id.clone(),
                scope: aggregate_scope(targets.iter()),
                requested_packages: targets.len(),
                completed_packages: 0,
                status: ManagerOperationStatus::Failed,
                error: Some(error.clone()),
            });
            append_not_started_outcomes(
                &mut manager_outcomes,
                groups.iter().copied().skip(group_index + 1),
            );
            return OperationOutcome {
                action,
                completed_packages,
                total_packages,
                completed_managers,
                total_managers,
                failed_manager: Some(manager_id.clone()),
                error: Some(error),
                cancelled: false,
                manager_outcomes,
                scope,
            };
        }
        let manager_completed = AtomicUsize::new(0);
        let progress_events = |event| match event {
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
        let progress = CancellableProgressSink {
            cancellation,
            emit: &progress_events,
        };

        if let Err(error) = manager
            .execute(manager_config, action, targets, &progress)
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
            let cancelled = error.kind() == ManagerErrorKind::Cancelled;
            let completed =
                completed_packages + manager_completed.load(Ordering::Relaxed).min(targets.len());
            manager_outcomes.push(ManagerOperationOutcome {
                manager_id: manager_id.clone(),
                scope: aggregate_scope(targets.iter()),
                requested_packages: targets.len(),
                completed_packages: completed.saturating_sub(completed_packages),
                status: if cancelled {
                    ManagerOperationStatus::Cancelled
                } else {
                    ManagerOperationStatus::Failed
                },
                error: Some(detail.clone()),
            });
            append_not_started_outcomes(
                &mut manager_outcomes,
                groups.iter().copied().skip(group_index + 1),
            );
            return OperationOutcome {
                action,
                completed_packages: completed,
                total_packages,
                completed_managers,
                total_managers,
                failed_manager: (!cancelled).then(|| manager_id.clone()),
                error: Some(if cancelled {
                    detail
                } else {
                    format!("Failed to {action_name} packages from {manager_id}: {detail}")
                }),
                cancelled,
                manager_outcomes,
                scope,
            };
        }

        manager_outcomes.push(ManagerOperationOutcome {
            manager_id: manager_id.clone(),
            scope: aggregate_scope(targets.iter()),
            requested_packages: targets.len(),
            completed_packages: targets.len(),
            status: ManagerOperationStatus::Succeeded,
            error: None,
        });
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
        cancelled: false,
        manager_outcomes,
        scope,
    }
}

fn append_not_started_outcomes<'a>(
    manager_outcomes: &mut Vec<ManagerOperationOutcome>,
    groups: impl IntoIterator<Item = &'a (ManagerId, Vec<PackageTarget>)>,
) {
    manager_outcomes.extend(groups.into_iter().map(|(manager_id, targets)| {
        ManagerOperationOutcome {
            manager_id: manager_id.clone(),
            scope: aggregate_scope(targets.iter()),
            requested_packages: targets.len(),
            completed_packages: 0,
            status: ManagerOperationStatus::NotStarted,
            error: Some("Not started because an earlier manager stopped execution".to_owned()),
        }
    }));
}

fn aggregate_scope<'a>(targets: impl IntoIterator<Item = &'a PackageTarget>) -> PackageScope {
    let mut scopes = targets.into_iter().map(|target| target.scope);
    let Some(first) = scopes.next() else {
        return PackageScope::Unknown;
    };
    if scopes.all(|scope| scope == first) {
        first
    } else {
        PackageScope::Unknown
    }
}
