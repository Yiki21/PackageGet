use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use futures::channel::mpsc;
use iced::Task;
use updater_core::{
    CancellationToken, Config, ManagerRegistry, OperationOutcome, OperationProgress,
    execute_package_groups,
};
use updater_manager_api::{ManagerId, PackageAction};

use crate::{content::shared::PackageSelectionKey, manager_catalog::ManagerCatalog};

#[derive(Debug, Clone)]
enum BatchActionEvent {
    Progress(OperationProgress),
    Done(OperationOutcome),
}

/// Manager/package scope frozen before a package write is confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageActionPlan {
    pub manager_groups: Vec<(ManagerId, Vec<String>)>,
}

impl PackageActionPlan {
    pub fn package_count(&self) -> usize {
        self.manager_groups
            .iter()
            .map(|(_, packages)| packages.len())
            .sum()
    }
}

pub fn collect_selected_package_groups<'a, T: 'a, I, N>(
    package_sets: I,
    selected_packages: &HashSet<PackageSelectionKey>,
    catalog: &ManagerCatalog,
    package_name: N,
) -> Vec<(ManagerId, Vec<String>)>
where
    I: IntoIterator<Item = (ManagerId, &'a [T])>,
    N: Fn(&T) -> &str,
{
    let mut packages_by_manager: BTreeMap<ManagerId, Vec<String>> = BTreeMap::new();

    for (manager, packages) in package_sets {
        for package in packages {
            let name = package_name(package);
            if selected_packages.contains(&(manager.clone(), name.to_owned())) {
                packages_by_manager
                    .entry(manager.clone())
                    .or_default()
                    .push(name.to_owned());
            }
        }
    }

    let mut manager_groups: Vec<_> = packages_by_manager.into_iter().collect();
    manager_groups.sort_by(|(left, _), (right, _)| {
        catalog
            .display_name(left)
            .cmp(catalog.display_name(right))
            .then_with(|| left.cmp(right))
    });
    manager_groups
        .iter_mut()
        .for_each(|(_, package_names)| package_names.sort());
    manager_groups
}

pub fn run_grouped_package_action<Message, ProgressMessage, DoneMessage>(
    registry: Arc<ManagerRegistry>,
    config: &Config,
    action: PackageAction,
    manager_groups: Vec<(ManagerId, Vec<String>)>,
    cancellation: CancellationToken,
    progress_message: ProgressMessage,
    done_message: DoneMessage,
) -> Task<Message>
where
    Message: Send + 'static,
    ProgressMessage: Fn(OperationProgress) -> Message + Copy + Send + 'static,
    DoneMessage: Fn(OperationOutcome) -> Message + Copy + Send + 'static,
{
    if manager_groups
        .iter()
        .all(|(_, package_names)| package_names.is_empty())
    {
        return Task::done(done_message(OperationOutcome {
            action,
            completed_packages: 0,
            total_packages: 0,
            completed_managers: 0,
            total_managers: 0,
            failed_manager: None,
            error: None,
        }));
    }

    let (sender, receiver) = mpsc::unbounded::<BatchActionEvent>();
    let config = config.clone();
    let runner_sender = sender.clone();
    let runner_task = Task::future(async move {
        let progress_sender = runner_sender.clone();
        let outcome = execute_package_groups(
            &registry,
            &config,
            action,
            &manager_groups,
            &cancellation,
            &move |progress| {
                let _ = progress_sender.unbounded_send(BatchActionEvent::Progress(progress));
            },
        )
        .await;
        let _ = runner_sender.unbounded_send(BatchActionEvent::Done(outcome));
    })
    .discard();
    let progress_task = Task::run(receiver, move |event| match event {
        BatchActionEvent::Progress(progress) => progress_message(progress),
        BatchActionEvent::Done(outcome) => done_message(outcome),
    });

    Task::batch(vec![runner_task, progress_task])
}

pub fn push_command_log(
    logs: &mut Vec<String>,
    action: PackageAction,
    manager: &ManagerId,
    catalog: &ManagerCatalog,
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
    let action = match action {
        PackageAction::Install => "Install",
        PackageAction::Update => "Update",
        PackageAction::Uninstall => "Remove",
        _ => "Package action",
    };

    logs.push(format!(
        "[{action}][{}][{package_name}] {command_message}",
        catalog.display_name(manager),
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

    fn manager_id(value: &str) -> ManagerId {
        ManagerId::parse(value).unwrap()
    }

    #[test]
    fn successful_outcome_has_natural_summary() {
        let outcome = OperationOutcome {
            action: PackageAction::Update,
            completed_packages: 12,
            total_packages: 12,
            completed_managers: 3,
            total_managers: 3,
            failed_manager: None,
            error: None,
        };

        assert_eq!(outcome.summary(), "12 packages updated across 3 sources");
    }

    #[test]
    fn partial_outcome_reports_completed_packages() {
        let outcome = OperationOutcome {
            action: PackageAction::Install,
            completed_packages: 2,
            total_packages: 5,
            completed_managers: 1,
            total_managers: 3,
            failed_manager: Some(manager_id("builtin:cargo")),
            error: Some("failed".to_owned()),
        };

        assert_eq!(
            outcome.summary(),
            "2 of 5 packages installed before the operation stopped"
        );
    }
}
