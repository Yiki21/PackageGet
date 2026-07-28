use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use futures::channel::mpsc;
use iced::Task;
use updater_core::{Config, InstallProgress, PackageManagerType};

use crate::content::shared::PackageSelectionKey;

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageBatchAction {
    Install,
    Remove,
    Update,
}

impl PackageBatchAction {
    pub fn completed_verb(self) -> &'static str {
        match self {
            Self::Install => "installed",
            Self::Remove => "removed",
            Self::Update => "updated",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Install => "Install",
            Self::Remove => "Remove",
            Self::Update => "Update",
        }
    }

    fn log_label(self) -> &'static str {
        self.label()
    }

    fn error_verb(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Remove => "remove",
            Self::Update => "update",
        }
    }

    pub async fn run_with_progress<F>(
        self,
        pm_type: PackageManagerType,
        pm_config: &Config,
        package_names: &[String],
        on_progress: F,
    ) -> Result<(), String>
    where
        F: FnMut(InstallProgress),
    {
        let result = match self {
            Self::Install => {
                pm_type
                    .install_packages_with_progress(pm_config, package_names, on_progress)
                    .await
            }
            Self::Remove => {
                pm_type
                    .uninstall_packages_with_progress(pm_config, package_names, on_progress)
                    .await
            }
            Self::Update => {
                pm_type
                    .update_packages_with_progress(pm_config, package_names, on_progress)
                    .await
            }
        };

        result.map_err(|e| {
            format!(
                "Failed to {} packages from {}: {}",
                self.error_verb(),
                pm_type.name(),
                e
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct BatchProgress {
    pub completed: usize,
    pub total: usize,
    pub manager: PackageManagerType,
    pub current_package: String,
    pub command_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationOutcome {
    pub action: PackageBatchAction,
    pub completed_packages: usize,
    pub total_packages: usize,
    pub completed_managers: usize,
    pub total_managers: usize,
    pub failed_manager: Option<PackageManagerType>,
    pub error: Option<String>,
}

impl OperationOutcome {
    pub fn is_success(&self) -> bool {
        self.error.is_none() && self.completed_packages == self.total_packages
    }

    pub fn summary(&self) -> String {
        if self.is_success() {
            return format!(
                "{} package{} {} across {} source{}",
                self.total_packages,
                if self.total_packages == 1 { "" } else { "s" },
                self.action.completed_verb(),
                self.total_managers,
                if self.total_managers == 1 { "" } else { "s" },
            );
        }

        format!(
            "{} of {} packages {} before the operation stopped",
            self.completed_packages,
            self.total_packages,
            self.action.completed_verb(),
        )
    }
}

#[derive(Debug, Clone)]
enum BatchActionEvent {
    Progress(BatchProgress),
    Done(OperationOutcome),
}

pub fn collect_selected_package_groups<'a, T: 'a, I, N>(
    package_sets: I,
    selected_packages: &HashSet<PackageSelectionKey>,
    package_name: N,
) -> Vec<(PackageManagerType, Vec<String>)>
where
    I: IntoIterator<Item = (PackageManagerType, &'a [T])>,
    N: Fn(&T) -> &str,
{
    let mut packages_by_manager: HashMap<PackageManagerType, Vec<String>> = HashMap::new();

    for (pm_type, packages) in package_sets {
        for package in packages {
            let name = package_name(package);
            if selected_packages.contains(&(pm_type, name.to_owned())) {
                packages_by_manager
                    .entry(pm_type)
                    .or_default()
                    .push(name.to_owned());
            }
        }
    }

    let mut manager_groups: Vec<_> = packages_by_manager.into_iter().collect();
    manager_groups.sort_by_key(|(pm_type, _)| pm_type.name());
    for (_, package_names) in &mut manager_groups {
        package_names.sort();
    }

    manager_groups
}

pub fn run_grouped_package_action<Message, ProgressMessage, DoneMessage>(
    pm_config: &Config,
    action: PackageBatchAction,
    manager_groups: Vec<(PackageManagerType, Vec<String>)>,
    cancellation: CancellationToken,
    progress_message: ProgressMessage,
    done_message: DoneMessage,
) -> Task<Message>
where
    Message: Send + 'static,
    ProgressMessage: Fn(BatchProgress) -> Message + Copy + Send + 'static,
    DoneMessage: Fn(OperationOutcome) -> Message + Copy + Send + 'static,
{
    let total_packages: usize = manager_groups
        .iter()
        .map(|(_, packages)| packages.len())
        .sum();
    let total_managers = manager_groups.len();

    if total_packages == 0 {
        return Task::done(done_message(OperationOutcome {
            action,
            completed_packages: 0,
            total_packages: 0,
            completed_managers: 0,
            total_managers,
            failed_manager: None,
            error: None,
        }));
    }

    let (sender, receiver) = mpsc::unbounded::<BatchActionEvent>();
    let runner_sender = sender.clone();
    let pm_config = pm_config.clone();

    let runner_task = Task::future(async move {
        let mut global_offset = 0usize;
        let mut completed_managers = 0usize;

        for (pm_type, package_names) in manager_groups {
            if cancellation.is_cancelled() {
                let _ = runner_sender.unbounded_send(BatchActionEvent::Done(OperationOutcome {
                    action,
                    completed_packages: global_offset,
                    total_packages,
                    completed_managers,
                    total_managers,
                    failed_manager: None,
                    error: Some("Cancelled by user".to_owned()),
                }));
                return;
            }
            let offset = global_offset;
            let progress_sender = runner_sender.clone();
            let mut manager_completed = 0usize;

            let result = action
                .run_with_progress(pm_type, &pm_config, &package_names, |progress| {
                    manager_completed = manager_completed.max(progress.completed);
                    let _ =
                        progress_sender.unbounded_send(BatchActionEvent::Progress(BatchProgress {
                            completed: offset + progress.completed,
                            total: total_packages,
                            manager: progress.manager,
                            current_package: progress.current_package,
                            command_message: progress.command_message,
                        }));
                })
                .await;

            match result {
                Ok(()) => {
                    global_offset += package_names.len();
                    completed_managers += 1;
                }
                Err(error) => {
                    let _ =
                        runner_sender.unbounded_send(BatchActionEvent::Done(OperationOutcome {
                            action,
                            completed_packages: global_offset
                                + manager_completed.min(package_names.len()),
                            total_packages,
                            completed_managers,
                            total_managers,
                            failed_manager: Some(pm_type),
                            error: Some(error),
                        }));
                    return;
                }
            }
        }

        let _ = runner_sender.unbounded_send(BatchActionEvent::Done(OperationOutcome {
            action,
            completed_packages: total_packages,
            total_packages,
            completed_managers,
            total_managers,
            failed_manager: None,
            error: None,
        }));
    })
    .discard();

    let progress_task = Task::run(receiver, move |event| match event {
        BatchActionEvent::Progress(progress) => progress_message(progress),
        BatchActionEvent::Done(result) => done_message(result),
    });

    Task::batch(vec![runner_task, progress_task])
}

pub fn push_command_log(
    logs: &mut Vec<String>,
    action: PackageBatchAction,
    manager: PackageManagerType,
    package_name: &str,
    command_message: String,
) {
    let command_message = command_message.trim();
    if command_message.is_empty() {
        return;
    }

    let package_name = if package_name.is_empty() {
        "batch"
    } else {
        package_name
    };

    logs.push(format!(
        "[{}][{}][{}] {}",
        action.log_label(),
        manager.name(),
        package_name,
        command_message
    ));

    const MAX_COMMAND_LOGS: usize = 120;
    if logs.len() > MAX_COMMAND_LOGS {
        let overflow = logs.len() - MAX_COMMAND_LOGS;
        logs.drain(0..overflow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_outcome_has_natural_summary() {
        let outcome = OperationOutcome {
            action: PackageBatchAction::Update,
            completed_packages: 12,
            total_packages: 12,
            completed_managers: 3,
            total_managers: 3,
            failed_manager: None,
            error: None,
        };

        assert!(outcome.is_success());
        assert_eq!(outcome.summary(), "12 packages updated across 3 sources");
    }

    #[test]
    fn failed_outcome_reports_partial_progress() {
        let outcome = OperationOutcome {
            action: PackageBatchAction::Install,
            completed_packages: 2,
            total_packages: 5,
            completed_managers: 1,
            total_managers: 2,
            failed_manager: Some(PackageManagerType::Flatpak),
            error: Some("network error".to_owned()),
        };

        assert!(!outcome.is_success());
        assert_eq!(
            outcome.summary(),
            "2 of 5 packages installed before the operation stopped"
        );
    }
}
