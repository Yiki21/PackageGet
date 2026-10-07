// Updates view with filtering, sorting, and search capabilities.

use std::collections::{HashMap, HashSet};

use iced::Task;
use updater_core::{CancellationToken, OperationOutcome, OperationProgress};
use updater_manager_api::{
    ManagerCapability, ManagerId, PackageAction, PackageInfo, PackageTarget, PackageUpdate,
};

use crate::{
    activity,
    content::InstalledInfo,
    content::errors::{ManagerErrors, apply_manager_counted_items_result},
    content::shared::{self, ManagerSectionStyle, PackageSelectionKey},
    content::workflows::{
        PackageActionPlan, collect_selected_package_groups, push_command_log,
        run_grouped_package_action,
    },
    manager_catalog::ManagerCatalog,
    theme,
};

/// Pacman applies its updates as one `pacman -Syu` system upgrade.
const FULL_SYSTEM_UPGRADE_MANAGER: &str = "builtin:pacman";

#[derive(Debug, Clone, Default)]
pub struct Updates {
    /// Search text for filtering updates in UI.
    search_query: String,
    /// Whether the package-manager source picker is expanded.
    sources_expanded: bool,
    /// Search text inside the package-manager source picker.
    source_query: String,
    /// Update currently shown in the details inspector.
    inspected_package: Option<PackageSelectionKey>,
    /// On-demand metadata request and result for the current inspector package.
    package_detail: shared::PackageDetailState,
    /// Last inspector action error shown in UI.
    inspector_error: Option<String>,
    /// Sources still refreshing for an Update All preflight.
    update_all_refreshing: HashSet<ManagerId>,
    /// Full source scope used to build the current preflight plan.
    update_all_scope: HashSet<ManagerId>,
    /// Frozen selected or Update All plan waiting for confirmation.
    pending_update: Option<UpdatePlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdatePlanScope {
    Selected,
    All,
}

#[derive(Debug, Clone)]
struct UpdatePlan {
    scope: UpdatePlanScope,
    packages: PackageActionPlan,
    failed_sources: Vec<ManagerId>,
}

impl UpdatePlan {
    fn package_count(&self) -> usize {
        self.packages.package_count()
    }

    fn manager_count(&self) -> usize {
        self.packages.manager_groups.len()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Expand or collapse the package-manager source picker.
    ToggleSourcePicker,
    /// Filter package managers inside the source picker.
    SourceQueryChanged(String),
    /// Select or clear the visible package-manager sources.
    SetSourceSelection(Vec<ManagerId>, bool),
    /// Package-manager selection message.
    SelectPackageManager(ManagerId, bool),
    /// Updates-load result message.
    LoadUpdatesResult {
        /// Request generation assigned when this manager load started.
        request_id: u64,
        /// Source manager.
        manager: ManagerId,
        /// Available updates or failure detail.
        result: Result<Vec<PackageUpdate>, String>,
    },
    /// Search-query change message.
    SearchQueryChanged(String),
    /// Sort-option change message.
    SortOptionChanged(SortOption),
    /// Package-selection toggle message.
    TogglePackageSelection(ManagerId, String, bool),
    /// Select-all toggle message.
    ToggleSelectAll(bool),
    /// Freeze the selected packages into an update plan.
    PrepareSelectedUpdate,
    /// Update progress message.
    UpdateProgress {
        /// Number of finished packages.
        completed: usize,
        /// Total packages to update.
        total: usize,
        /// Manager currently executing command.
        manager: ManagerId,
        /// Current package being processed.
        current_package: String,
        /// Optional command output/status line.
        command_message: Option<String>,
    },
    /// Update result message.
    UpdatePackagesResult(OperationOutcome),
    /// Selected-managers refresh message.
    RefreshSelected,
    /// Full refresh message.
    RefreshAll,
    /// Retry loading one package manager.
    RetryLoad(ManagerId),
    /// Abandon pending per-source loads and keep already loaded updates.
    StopWaiting,
    /// Show an update in the package inspector.
    InspectPackage(ManagerId, String),
    /// Retry on-demand package metadata loading.
    RetryPackageInfo(ManagerId, String),
    /// On-demand package metadata result.
    PackageInfoLoaded {
        generation: u64,
        manager: ManagerId,
        package_name: String,
        result: Box<Result<Option<PackageInfo>, String>>,
    },
    /// Copy text from the inspector.
    CopyInspectorText(String),
    /// Refresh all sources and prepare an Update All plan.
    PrepareUpdateAll,
    /// Execute the frozen update plan.
    ConfirmUpdate,
    /// Dismiss the frozen update plan.
    CancelUpdate,
    /// Re-scan the failed update source before retrying.
    PrepareFailedUpdateRetry,
    /// Dismiss the last package-operation notice.
    DismissOperationNotice,
    /// Open the Package Managers page, where configuration is fixed.
    OpenManagers,
}

/// Whether an updates load may run a privileged metadata sync.
///
/// System managers answer a privileged refresh with `apt-get update`,
/// `dnf check-upgrade --refresh` and friends, which raise their own
/// authorization prompt. Only explicit user refreshes and retries ask for
/// that: after a package operation the local listing (`apt list
/// --upgradable`, `dnf check-upgrade`, `pacman -Qu`) already reflects the
/// write, so a post-operation reload must stay [`RefreshMode::Local`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshMode {
    /// Re-read local package metadata only.
    Local,
    /// Refresh remote metadata before listing, which may prompt for authorization.
    Privileged,
}

impl RefreshMode {
    /// Whether this mode forces the manager to refresh remote metadata.
    fn forces_metadata_sync(self) -> bool {
        matches!(self, Self::Privileged)
    }
}

#[derive(Debug, Clone, Default)]
pub struct UpdatesInfo {
    /// Updates cache by manager `(count, updates)`.
    pub updates_by_manager: HashMap<ManagerId, (usize, Vec<PackageUpdate>)>,
    /// Initial update-loading failures grouped by manager.
    pub init_errors: ManagerErrors,
    /// Full update-list loading failures grouped by manager.
    pub load_errors: ManagerErrors,
    /// Managers selected in the filter panel.
    pub selected_managers: HashSet<ManagerId>,
    /// Managers currently loading update list.
    pub loading_updates: HashMap<ManagerId, u64>,
    /// [`RefreshMode`] of each in-flight updates request, so a privileged
    /// metadata sync stays traceable to the action that asked for it.
    pub refresh_modes: HashMap<ManagerId, RefreshMode>,
    /// Time of the last successful check per manager, in RFC 3339.
    pub checked_at: HashMap<ManagerId, String>,
    /// Last allocated updates-load request generation.
    pub request_generation: u64,
    /// Whether initial per-manager counts are loading.
    pub is_loading_count: bool,
    /// Whether counts have ever been loaded.
    pub has_loading_count: bool,
    /// Initialization progress `(completed, total)`.
    pub init_progress: Option<(usize, usize)>,
    /// Initialization command logs.
    pub init_logs: Vec<String>,
    /// Current sort option.
    pub sort_by: SortOption,
    /// Selected package keys for batch operations.
    pub selected_packages: HashSet<PackageSelectionKey>,
    /// Whether update operation is in progress.
    pub is_updating: bool,
    /// Update progress `(completed, total, manager, package)`.
    pub update_progress: Option<(usize, usize, ManagerId, String)>,
    /// Update command logs.
    pub update_logs: Vec<String>,
    /// Last update operation notice shown in UI.
    pub last_operation_notice: Option<shared::OperationNotice>,
    /// Source that failed during the most recent update operation.
    pub failed_update_manager: Option<ManagerId>,
}

impl UpdatesInfo {
    fn selected_loading_sources(&self) -> usize {
        self.selected_managers
            .iter()
            .filter(|manager| {
                self.loading_updates.contains_key(*manager)
                    || (self.is_loading_count
                        && !self.updates_by_manager.contains_key(*manager)
                        && !self.init_errors.contains_key(*manager))
            })
            .count()
    }

    fn selected_sources_have_errors(&self) -> bool {
        self.selected_managers.iter().any(|manager| {
            self.init_errors.contains_key(manager) || self.load_errors.contains_key(manager)
        })
    }

    /// Whether the most recent check of `manager` failed.
    fn has_error(&self, manager: &ManagerId) -> bool {
        self.load_errors.contains_key(manager) || self.init_errors.contains_key(manager)
    }

    /// Updates count that still reflects a successful check.
    ///
    /// A failed source keeps its previous count in [`Self::updates_by_manager`]
    /// so it can be shown as last known, but it must not be presented as a
    /// current total.
    pub fn current_update_count(&self) -> usize {
        self.updates_by_manager
            .iter()
            .filter(|(manager, _)| !self.has_error(manager))
            .map(|(_, (count, _))| *count)
            .sum()
    }

    /// Stamps `manager` as successfully checked at `checked_at`.
    pub fn mark_checked(&mut self, manager: ManagerId, checked_at: String) {
        self.checked_at.insert(manager, checked_at);
    }

    /// Oldest successful check among `managers`, as its RFC 3339 timestamp.
    ///
    /// [`crate::activity::now_timestamp`] emits fixed-width UTC timestamps, so
    /// comparing the strings orders them chronologically.
    fn oldest_checked_at<'a>(
        &'a self,
        managers: impl Iterator<Item = &'a ManagerId>,
    ) -> Option<&'a str> {
        managers
            .filter_map(|manager| self.checked_at.get(manager).map(String::as_str))
            .min()
    }

    /// Formats an RFC 3339 check time as a local `HH:MM` label.
    fn checked_at_label(checked_at: &str) -> Option<String> {
        chrono::DateTime::parse_from_rfc3339(checked_at)
            .ok()
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%H:%M")
                    .to_string()
            })
    }
}

pub enum Action {
    /// No-op action.
    None,
    /// Asynchronous task action.
    Run(iced::Task<Message>),
    /// Cooperative package operation task.
    CancellableRun(iced::Task<Message>, CancellationToken),
    /// Complete a package operation and refresh managers that succeeded.
    PackageOperationFinished { outcome: OperationOutcome },
    /// Switch the visible page.
    Navigate(crate::content::ActiveContentPage),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortOption {
    #[default]
    Name,
    CurrentVersion,
    NewVersion,
}

impl SortOption {
    pub fn name(&self) -> &'static str {
        match self {
            SortOption::Name => "Name",
            SortOption::CurrentVersion => "Current Version",
            SortOption::NewVersion => "New Version",
        }
    }

    pub const ALL: [SortOption; 3] = [
        SortOption::Name,
        SortOption::CurrentVersion,
        SortOption::NewVersion,
    ];
}

impl Updates {
    pub fn update(
        &mut self,
        message: Message,
        pm_config: &updater_core::Config,
        info: &mut UpdatesInfo,
        catalog: &ManagerCatalog,
    ) -> Action {
        match message {
            Message::ToggleSourcePicker => {
                self.sources_expanded = !self.sources_expanded;
                if !self.sources_expanded {
                    self.source_query.clear();
                }
                Action::None
            }
            Message::SourceQueryChanged(query) => {
                self.source_query = query;
                Action::None
            }
            Message::SetSourceSelection(managers, selected) => {
                if self.pending_update.is_some() {
                    return Action::None;
                }
                if !selected {
                    for manager in managers {
                        Self::set_source_selection(false, manager, pm_config, info, catalog);
                    }
                    return Action::None;
                }

                let tasks = managers
                    .into_iter()
                    .filter_map(|manager| {
                        match Self::set_source_selection(true, manager, pm_config, info, catalog) {
                            Action::Run(task) => Some(task),
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>();
                if tasks.is_empty() {
                    Action::None
                } else {
                    Action::Run(Task::batch(tasks))
                }
            }
            Message::SelectPackageManager(pm_type, selected) => {
                if self.pending_update.is_some() {
                    return Action::None;
                }
                Self::set_source_selection(selected, pm_type, pm_config, info, catalog)
            }
            Message::LoadUpdatesResult {
                request_id,
                manager: pm_type,
                result,
            } => {
                if info.loading_updates.get(&pm_type) != Some(&request_id) {
                    return Action::None;
                }
                info.loading_updates.remove(&pm_type);
                info.refresh_modes.remove(&pm_type);
                if result.is_ok() {
                    info.mark_checked(pm_type.clone(), activity::now_timestamp());
                }
                apply_manager_counted_items_result(
                    &mut info.updates_by_manager,
                    &mut info.load_errors,
                    pm_type.clone(),
                    result,
                );

                if self.update_all_refreshing.remove(&pm_type)
                    && self.update_all_refreshing.is_empty()
                {
                    self.pending_update = Some(Self::build_update_plan(
                        info,
                        &self.update_all_scope,
                        catalog,
                        UpdatePlanScope::All,
                    ));
                }
                Action::None
            }
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
                Action::None
            }
            Message::InspectPackage(pm_type, package_name)
            | Message::RetryPackageInfo(pm_type, package_name) => {
                let key = shared::selection_key(&pm_type, &package_name);
                self.inspected_package = Some(key.clone());
                self.inspector_error = None;
                let Some(target) = info
                    .updates_by_manager
                    .get(&pm_type)
                    .and_then(|(_, packages)| {
                        packages
                            .iter()
                            .find(|package| package.target.name == package_name)
                    })
                    .map(|package| package.target.clone())
                else {
                    return Action::None;
                };
                let Some(config) = pm_config.manager(&pm_type).cloned() else {
                    self.inspector_error = Some(format!("manager is not configured: {pm_type}"));
                    return Action::None;
                };
                let generation = self.package_detail.begin(key);
                let registry = catalog.registry();
                Action::Run(
                    Task::future(shared::load_package_info(registry, config, target)).then(
                        move |result| {
                            Task::done(Message::PackageInfoLoaded {
                                generation,
                                manager: pm_type.clone(),
                                package_name: package_name.clone(),
                                result: Box::new(result),
                            })
                        },
                    ),
                )
            }
            Message::PackageInfoLoaded {
                generation,
                manager,
                package_name,
                result,
            } => {
                self.package_detail.finish(
                    generation,
                    shared::selection_key(&manager, &package_name),
                    *result,
                );
                Action::None
            }
            Message::CopyInspectorText(value) => {
                self.inspector_error = None;
                Action::Run(iced::clipboard::write(value))
            }
            Message::SortOptionChanged(sort_option) => {
                info.sort_by = sort_option;
                Action::None
            }
            Message::TogglePackageSelection(pm_type, package_name, selected) => {
                if info.is_updating || self.pending_update.is_some() {
                    return Action::None;
                }
                let key = shared::selection_key(&pm_type, &package_name);
                if selected {
                    info.selected_packages.insert(key);
                } else {
                    info.selected_packages.remove(&key);
                }
                Action::None
            }
            Message::ToggleSelectAll(select_all) => {
                if info.is_updating || self.pending_update.is_some() {
                    return Action::None;
                }

                let query = self.search_query.trim().to_lowercase();
                let visible = info
                    .selected_managers
                    .iter()
                    .filter_map(|manager| {
                        info.updates_by_manager
                            .get(manager)
                            .map(|(_, packages)| (manager, packages))
                    })
                    .flat_map(|(manager, packages)| {
                        packages
                            .iter()
                            .filter(|package| {
                                query.is_empty()
                                    || package.target.name.to_lowercase().contains(query.as_str())
                            })
                            .map(move |package| {
                                shared::selection_key(manager, &package.target.name)
                            })
                    });
                if select_all {
                    info.selected_packages.extend(visible);
                } else {
                    visible.for_each(|key| {
                        info.selected_packages.remove(&key);
                    });
                }
                Action::None
            }
            Message::PrepareSelectedUpdate => {
                if info.selected_packages.is_empty()
                    || info.is_updating
                    || self.pending_update.is_some()
                    || !self.update_all_refreshing.is_empty()
                {
                    return Action::None;
                }
                info.last_operation_notice = None;
                info.failed_update_manager = None;
                let mut manager_groups = collect_selected_package_groups(
                    info.selected_managers.iter().filter_map(|manager| {
                        info.updates_by_manager
                            .get(manager)
                            .map(|(_, packages)| (manager.clone(), packages.as_slice()))
                    }),
                    &info.selected_packages,
                    catalog,
                    |package| package.target.clone(),
                );
                // Selecting any Pacman update plans all of them, because
                // pacman upgrades the whole system in one transaction.
                for (manager, targets) in &mut manager_groups {
                    if manager.as_str() == FULL_SYSTEM_UPGRADE_MANAGER
                        && let Some((_, packages)) = info.updates_by_manager.get(manager)
                    {
                        *targets = packages
                            .iter()
                            .map(|package| package.target.clone())
                            .collect();
                    }
                }
                if manager_groups.is_empty() {
                    info.last_operation_notice = Some(shared::OperationNotice::failed(
                        PackageAction::Update,
                        "Selected packages are no longer available to update".to_owned(),
                    ));
                    return Action::None;
                }
                self.pending_update = Some(UpdatePlan {
                    scope: UpdatePlanScope::Selected,
                    packages: PackageActionPlan { manager_groups },
                    failed_sources: Vec::new(),
                });
                Action::None
            }
            Message::UpdateProgress {
                completed,
                total,
                manager,
                current_package,
                command_message,
            } => {
                info.update_progress = Some((completed, total, manager.clone(), current_package));
                if let Some(command_message) = command_message {
                    push_command_log(
                        &mut info.update_logs,
                        PackageAction::Update,
                        &manager,
                        catalog,
                        info.update_progress
                            .as_ref()
                            .map_or("", |(_, _, _, package)| package.as_str()),
                        command_message,
                    );
                }
                Action::None
            }
            Message::UpdatePackagesResult(outcome) => {
                self.reset_pending_updates();
                info.is_updating = false;
                info.update_progress = None;
                info.last_operation_notice = shared::OperationNotice::from_outcome(&outcome);
                if let Some(notice) = &info.last_operation_notice {
                    log::error!(
                        "Update packages {}: {}",
                        if notice.is_stopped() {
                            "stopped"
                        } else {
                            "failed"
                        },
                        shared::operation_notice_message(notice).unwrap_or_default()
                    );
                }
                if outcome.is_success() {
                    info.selected_packages.clear();
                    info.failed_update_manager = None;
                } else {
                    info.failed_update_manager = outcome.failed_manager.clone();
                }
                Action::PackageOperationFinished { outcome }
            }
            Message::DismissOperationNotice => {
                info.last_operation_notice = None;
                Action::None
            }
            Message::OpenManagers => Action::Navigate(crate::content::ActiveContentPage::Health),
            Message::RefreshSelected => {
                let selected: Vec<ManagerId> = info.selected_managers.iter().cloned().collect();
                if info.is_updating
                    || self.pending_update.is_some()
                    || !self.update_all_refreshing.is_empty()
                    || Self::any_loading_updates(info, selected.into_iter())
                {
                    return Action::None;
                }
                let managers: Vec<ManagerId> = info.selected_managers.iter().cloned().collect();

                if managers.is_empty() {
                    return Action::None;
                }

                let tasks: Vec<Task<Message>> = managers
                    .into_iter()
                    .map(|manager| {
                        Self::start_load(pm_config, info, manager, catalog, RefreshMode::Privileged)
                    })
                    .collect();

                Action::Run(Task::batch(tasks))
            }
            Message::RefreshAll => {
                let pm_types = shared::configured_managers_with_capability(
                    pm_config,
                    catalog,
                    ManagerCapability::Updates,
                );
                if info.is_updating
                    || self.pending_update.is_some()
                    || !self.update_all_refreshing.is_empty()
                    || Self::any_loading_updates(info, pm_types.iter().cloned())
                {
                    return Action::None;
                }

                if pm_types.is_empty() {
                    return Action::None;
                }

                let tasks: Vec<Task<Message>> = pm_types
                    .into_iter()
                    .map(|manager| {
                        Self::start_load(pm_config, info, manager, catalog, RefreshMode::Privileged)
                    })
                    .collect();

                Action::Run(Task::batch(tasks))
            }
            Message::RetryLoad(pm_type) => {
                if info.is_updating
                    || self.pending_update.is_some()
                    || !self.update_all_refreshing.is_empty()
                    || info.loading_updates.contains_key(&pm_type)
                {
                    return Action::None;
                }
                info.init_errors.remove(&pm_type);
                info.load_errors.remove(&pm_type);
                Action::Run(Self::start_load(
                    pm_config,
                    info,
                    pm_type,
                    catalog,
                    RefreshMode::Privileged,
                ))
            }
            Message::StopWaiting => {
                let waiting = self.stoppable_sources(info);
                if waiting.is_empty() {
                    return Action::None;
                }
                let mut finished_preflight = false;
                for manager in waiting {
                    info.loading_updates.remove(&manager);
                    info.refresh_modes.remove(&manager);
                    apply_manager_counted_items_result(
                        &mut info.updates_by_manager,
                        &mut info.load_errors,
                        manager.clone(),
                        Err(shared::stopped_waiting_error(
                            catalog.display_name(&manager),
                        )),
                    );
                    if self.update_all_refreshing.remove(&manager)
                        && self.update_all_refreshing.is_empty()
                    {
                        finished_preflight = true;
                    }
                }
                if finished_preflight {
                    let scope = self.update_all_scope.clone();
                    self.pending_update = Some(Self::build_update_plan(
                        info,
                        &scope,
                        catalog,
                        UpdatePlanScope::All,
                    ));
                }
                Action::None
            }
            Message::PrepareUpdateAll => {
                let managers = shared::configured_managers_with_capability(
                    pm_config,
                    catalog,
                    ManagerCapability::Updates,
                );
                if info.is_updating
                    || self.pending_update.is_some()
                    || !self.update_all_refreshing.is_empty()
                    || Self::any_loading_updates(info, managers.iter().cloned())
                {
                    return Action::None;
                }
                if managers.is_empty() {
                    return Action::None;
                }

                self.pending_update = None;
                self.update_all_scope = managers.iter().cloned().collect();
                self.update_all_refreshing = self.update_all_scope.clone();
                for manager in &managers {
                    info.init_errors.remove(manager);
                    info.load_errors.remove(manager);
                }

                Action::Run(Task::batch(managers.into_iter().map(|manager| {
                    Self::start_load(pm_config, info, manager, catalog, RefreshMode::Privileged)
                })))
            }
            Message::ConfirmUpdate => {
                let Some(plan) = self.pending_update.take() else {
                    return Action::None;
                };
                if info.is_updating {
                    return Action::None;
                }
                let Some((initial_manager, _)) = plan.packages.manager_groups.first() else {
                    info.last_operation_notice = Some(shared::OperationNotice::failed(
                        PackageAction::Update,
                        "The update plan does not contain any packages".to_owned(),
                    ));
                    return Action::None;
                };

                let total = plan.package_count();
                info.is_updating = true;
                info.last_operation_notice = None;
                info.failed_update_manager = None;
                info.update_logs.clear();
                info.update_progress = Some((0, total, initial_manager.clone(), String::new()));
                Self::update_plan_action(pm_config, plan.packages.manager_groups, catalog)
            }
            Message::CancelUpdate => {
                self.pending_update = None;
                self.update_all_scope.clear();
                Action::None
            }
            Message::PrepareFailedUpdateRetry => {
                if info.is_updating
                    || self.pending_update.is_some()
                    || !self.update_all_refreshing.is_empty()
                    || !info.loading_updates.is_empty()
                {
                    return Action::None;
                }
                let Some(manager) = info.failed_update_manager.take() else {
                    return Action::None;
                };

                self.pending_update = None;
                self.update_all_scope = HashSet::from([manager.clone()]);
                self.update_all_refreshing = self.update_all_scope.clone();
                info.init_errors.remove(&manager);
                info.load_errors.remove(&manager);
                Action::Run(Self::start_load(
                    pm_config,
                    info,
                    manager,
                    catalog,
                    RefreshMode::Privileged,
                ))
            }
        }
    }

    /// Selected or Update All sources whose in-flight update load can be
    /// abandoned; the request-id guard discards their late results.
    fn stoppable_sources(&self, info: &UpdatesInfo) -> HashSet<ManagerId> {
        self.update_all_scope
            .iter()
            .chain(info.selected_managers.iter())
            .filter(|manager| info.loading_updates.contains_key(*manager))
            .cloned()
            .collect()
    }

    /// Whether any of `managers` is still loading its update list.
    fn any_loading_updates(info: &UpdatesInfo, managers: impl Iterator<Item = ManagerId>) -> bool {
        managers.into_iter().any(|manager| {
            info.loading_updates.contains_key(&manager)
                || (info.is_loading_count && !info.updates_by_manager.contains_key(&manager))
        })
    }

    fn set_source_selection(
        selected: bool,
        manager: ManagerId,
        pm_config: &updater_core::Config,
        info: &mut UpdatesInfo,
        catalog: &ManagerCatalog,
    ) -> Action {
        if !selected {
            info.selected_managers.remove(&manager);
            info.selected_packages
                .retain(|(selected_manager, _)| selected_manager != &manager);
            return Action::None;
        }

        if !info.has_loading_count
            || (info.is_loading_count && !info.updates_by_manager.contains_key(&manager))
        {
            return Action::None;
        }
        info.selected_managers.insert(manager.clone());

        if info.init_errors.contains_key(&manager) || info.load_errors.contains_key(&manager) {
            info.init_errors.remove(&manager);
            info.load_errors.remove(&manager);
            Action::Run(Self::start_load(
                pm_config,
                info,
                manager,
                catalog,
                RefreshMode::Privileged,
            ))
        } else if info.loading_updates.contains_key(&manager) {
            Action::None
        } else if let Some((count, packages)) = info.updates_by_manager.get(&manager) {
            if *count == packages.len() {
                Action::None
            } else {
                Action::Run(Self::start_load(
                    pm_config,
                    info,
                    manager,
                    catalog,
                    RefreshMode::Local,
                ))
            }
        } else {
            Action::Run(Self::start_load(
                pm_config,
                info,
                manager,
                catalog,
                RefreshMode::Local,
            ))
        }
    }

    pub(crate) fn reset_pending_updates(&mut self) {
        self.pending_update = None;
        self.update_all_scope.clear();
        self.update_all_refreshing.clear();
    }

    pub fn has_inspector_selection(&self) -> bool {
        self.inspected_package.is_some()
    }

    pub fn dismiss_transient(&mut self) -> bool {
        if self.pending_update.take().is_some() {
            self.update_all_scope.clear();
            true
        } else if self.inspected_package.take().is_some() {
            self.inspector_error = None;
            true
        } else {
            false
        }
    }

    pub fn primary_action(&self, info: &UpdatesInfo) -> Option<Message> {
        if self.pending_update.is_some() {
            return self
                .pending_update
                .as_ref()
                .is_some_and(|plan| plan.package_count() > 0 && !info.is_updating)
                .then_some(Message::ConfirmUpdate);
        }
        (!info.is_updating
            && self.update_all_refreshing.is_empty()
            && !info.selected_packages.is_empty())
        .then_some(Message::PrepareSelectedUpdate)
    }

    pub fn can_select_packages(&self) -> bool {
        self.pending_update.is_none() && self.update_all_refreshing.is_empty()
    }

    pub fn move_keyboard_selection(
        &self,
        info: &UpdatesInfo,
        catalog: &ManagerCatalog,
        direction: crate::shortcut::SelectionDirection,
    ) -> Option<Message> {
        crate::content::shared::next_keyboard_package(
            &self.keyboard_packages(info, catalog),
            self.inspected_package.as_ref(),
            direction,
        )
        .map(|(manager, name)| Message::InspectPackage(manager, name))
    }

    pub fn toggle_keyboard_selection(&self, info: &UpdatesInfo) -> Option<Message> {
        let (manager, name) = self.inspected_package.as_ref()?;
        if info.is_updating
            || self.pending_update.is_some()
            || !self.update_all_refreshing.is_empty()
        {
            return None;
        }
        let selected = !info
            .selected_packages
            .contains(&shared::selection_key(manager, name));
        Some(Message::TogglePackageSelection(
            manager.clone(),
            name.clone(),
            selected,
        ))
    }

    fn keyboard_packages(
        &self,
        info: &UpdatesInfo,
        catalog: &ManagerCatalog,
    ) -> Vec<PackageSelectionKey> {
        let query = self.search_query.trim().to_lowercase();
        let mut managers: Vec<_> = info.selected_managers.iter().cloned().collect();
        managers.sort_by(|left, right| {
            catalog
                .display_name(left)
                .cmp(catalog.display_name(right))
                .then_with(|| left.cmp(right))
        });
        managers
            .into_iter()
            .flat_map(|manager| {
                let query = query.clone();
                let packages = info
                    .updates_by_manager
                    .get(&manager)
                    .map_or(&[][..], |(_, packages)| packages.as_slice());
                self.filter_and_sort_updates(packages, info.sort_by)
                    .into_iter()
                    .filter(move |package| {
                        query.is_empty() || package.target.name.to_lowercase().contains(&query)
                    })
                    .map(move |package| (manager.clone(), package.target.name.clone()))
            })
            .collect()
    }

    pub fn view<'a>(
        &'a self,
        info: &'a UpdatesInfo,
        installed_info: &'a InstalledInfo,
        pm_config: &updater_core::Config,
        catalog: &'a ManagerCatalog,
        show_inspector: bool,
        inspector_drawer: bool,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, container, row};

        let update_count = info.current_update_count();
        let configured_managers = shared::configured_managers_with_capability(
            pm_config,
            catalog,
            ManagerCapability::Updates,
        )
        .len();
        let selected_loading_sources = info.selected_loading_sources();
        let base_can_refresh = !info.is_updating
            && self.pending_update.is_none()
            && self.update_all_refreshing.is_empty();
        let can_refresh_selected = base_can_refresh
            && !Self::any_loading_updates(info, info.selected_managers.iter().cloned());
        let configured_updates_managers = shared::configured_managers_with_capability(
            pm_config,
            catalog,
            ManagerCapability::Updates,
        );
        let can_refresh_all = base_can_refresh
            && !Self::any_loading_updates(info, configured_updates_managers.iter().cloned());

        let toolbar = shared::toolbar(
            column![
                row![
                    container(self.search_input_view()).width(iced::Length::FillPortion(2)),
                    column![
                        shared::section_title("Actions"),
                        row![
                            shared::refresh_button_with_label(
                                "Refresh Selected",
                                can_refresh_selected,
                                Message::RefreshSelected
                            ),
                            shared::refresh_button_with_label(
                                "Refresh All",
                                can_refresh_all,
                                Message::RefreshAll
                            ),
                        ]
                        .spacing(8)
                        .wrap()
                    ]
                    .spacing(theme::spacing::SM)
                    .width(iced::Length::FillPortion(1)),
                ]
                .spacing(theme::spacing::MD)
                .align_y(iced::Alignment::Start),
                row![
                    container(self.manager_filter_view(info, pm_config, catalog))
                        .width(iced::Length::FillPortion(2)),
                    container(self.sort_order_view(info)).width(iced::Length::FillPortion(1)),
                ]
                .spacing(theme::spacing::LG)
                .align_y(iced::Alignment::Start),
            ]
            .spacing(theme::spacing::MD),
        );

        let failed_sources = info.init_errors.len() + info.load_errors.len();
        let source_scope = if info.selected_managers.is_empty() {
            "No sources selected".to_owned()
        } else {
            format!("{} sources selected", info.selected_managers.len())
        };
        let mut summary_items = vec![
            (format!("{update_count} updates"), theme::colors::UPDATES),
            (
                info.oldest_checked_at(configured_updates_managers.iter())
                    .map_or_else(
                        || "Not checked yet".to_owned(),
                        |checked_at| {
                            format!(
                                "Last checked {}",
                                UpdatesInfo::checked_at_label(checked_at)
                                    .unwrap_or_else(|| checked_at.to_owned())
                            )
                        },
                    ),
                theme::colors::ON_SURFACE_MUTED,
            ),
            (source_scope, theme::colors::ON_SURFACE_MUTED),
            (
                format!("{} packages selected", info.selected_packages.len()),
                theme::colors::INSTALLED,
            ),
        ];
        if failed_sources > 0 {
            summary_items.push((
                format!("{failed_sources} sources failed"),
                theme::colors::ERROR,
            ));
        }
        if selected_loading_sources > 0 {
            summary_items.push((
                format!("{selected_loading_sources} sources loading"),
                theme::colors::UPDATES,
            ));
        }

        // The empty state reports configuration, not capability coverage, so
        // "no managers are configured" only appears when that is literally true.
        let updates_list: iced::Element<'_, Message> = if let Some(empty) = shared::empty_state(
            shared::configured_managers(pm_config).len(),
            info.selected_managers.len(),
        ) {
            shared::empty_state_view(empty, Message::OpenManagers)
                .unwrap_or_else(|| shared::centered_message(shared::NO_SOURCE_SELECTED_HINT))
        } else {
            self.updates_list_view(
                info,
                installed_info,
                catalog,
                show_inspector,
                inspector_drawer,
                selected_loading_sources,
            )
        };

        column![
            shared::page_header(
                "Updates",
                format!(
                    "{update_count} available updates across {configured_managers} configured managers"
                ),
                theme::colors::UPDATES,
            ),
            shared::summary_row(summary_items),
            toolbar,
            self.batch_actions_view(info, pm_config, catalog),
            self.update_confirmation_view(catalog),
            updates_list,
        ]
        .spacing(theme::spacing::LG)
        .height(iced::Length::Fill)
        .into()
    }

    // View components.

    fn manager_filter_view<'a>(
        &'a self,
        info: &'a UpdatesInfo,
        pm_config: &updater_core::Config,
        catalog: &'a ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        let managers = shared::configured_managers_with_capability(
            pm_config,
            catalog,
            ManagerCapability::Updates,
        );
        if managers.is_empty() {
            return iced::widget::column![
                shared::section_title("Sources"),
                shared::empty_filter_view("No package managers detected")
            ]
            .spacing(theme::spacing::SM)
            .into();
        }
        let entries = managers
            .into_iter()
            .map(|manager| shared::ManagerSourceEntry {
                count: info
                    .updates_by_manager
                    .get(&manager)
                    .map(|(count, _)| *count),
                status: if info.loading_updates.contains_key(&manager) {
                    shared::ManagerSourceStatus::Loading
                } else if info.init_errors.contains_key(&manager)
                    || info.load_errors.contains_key(&manager)
                {
                    shared::ManagerSourceStatus::Failed
                } else if !info.has_loading_count
                    || (info.is_loading_count && !info.updates_by_manager.contains_key(&manager))
                {
                    shared::ManagerSourceStatus::Initializing
                } else {
                    shared::ManagerSourceStatus::Ready
                },
                manager,
            })
            .collect();
        let filters_content = shared::manager_source_picker(
            entries,
            catalog,
            shared::ManagerSourcePickerState {
                selected_managers: &info.selected_managers,
                expanded: self.sources_expanded,
                query: &self.source_query,
                count_label: "updates",
                disabled: self.pending_update.is_some() || !info.has_loading_count,
                label_exact_lookup: false,
            },
            shared::ManagerSourcePickerMessages {
                toggle_picker: Message::ToggleSourcePicker,
                query_changed: Message::SourceQueryChanged,
                set_visible_selection: Message::SetSourceSelection,
                toggle_manager: Message::SelectPackageManager,
            },
        );

        let mut content = iced::widget::column![shared::section_title("Sources")];
        if !info.init_errors.is_empty() {
            content = content.push(
                iced::widget::text("Some package managers failed to initialize")
                    .size(12)
                    .style(theme::text_error),
            );
        }

        content
            .push(filters_content)
            .spacing(theme::spacing::SM)
            .into()
    }

    fn sort_order_view<'a>(&self, info: &'a UpdatesInfo) -> iced::Element<'a, Message> {
        use iced::widget::{column, row};

        let sort_options = row(SortOption::ALL.iter().map(|option| {
            let option = *option;
            shared::segmented_button(
                option.name(),
                option == info.sort_by,
                Message::SortOptionChanged(option),
            )
            .into()
        }))
        .spacing(2)
        .width(iced::Length::Fill);

        column![
            shared::section_title("Sort"),
            shared::segmented_group(sort_options)
        ]
        .spacing(theme::spacing::SM)
        .into()
    }

    fn search_input_view<'a>(&self) -> iced::Element<'a, Message> {
        shared::search_input_view(
            crate::content::shared::search_input_id(crate::content::ActiveContentPage::Updates),
            "Search",
            "Search updates...",
            &self.search_query,
            Message::SearchQueryChanged,
        )
    }

    fn updates_list_view<'a>(
        &'a self,
        info: &'a UpdatesInfo,
        installed_info: &'a InstalledInfo,
        catalog: &'a ManagerCatalog,
        show_inspector: bool,
        inspector_drawer: bool,
        selected_loading_sources: usize,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, container, row, scrollable};

        if !info.has_loading_count {
            return shared::centered_message(if info.is_loading_count {
                "Loading update information..."
            } else {
                "Waiting to load update information"
            });
        }

        if info.selected_managers.is_empty() {
            return shared::centered_message(shared::NO_SOURCE_SELECTED_HINT);
        }

        let filtered_managers: Vec<_> = info
            .selected_managers
            .iter()
            .filter_map(|manager| {
                info.updates_by_manager
                    .get(manager)
                    .map(|entry| (manager.clone(), entry))
            })
            .collect();

        if filtered_managers.is_empty()
            && selected_loading_sources > 0
            && self.stoppable_sources(info).is_empty()
        {
            return shared::centered_message("Loading selected package manager updates...");
        }

        let total_updates: usize = filtered_managers.iter().map(|(_, (count, _))| *count).sum();
        let has_visible_errors = info.selected_sources_have_errors();

        if total_updates == 0 && !has_visible_errors && selected_loading_sources == 0 {
            return shared::centered_message("No updates available");
        }

        let search_query = self.search_query.trim().to_lowercase();
        if !search_query.is_empty() {
            let has_any_match = filtered_managers.iter().any(|(_, (_, packages))| {
                packages
                    .iter()
                    .any(|pkg| pkg.target.name.to_lowercase().contains(&search_query))
            });

            if !has_any_match && !has_visible_errors && selected_loading_sources == 0 {
                return shared::centered_message("No updates match your search");
            }
        }

        let mut updates_sections =
            Vec::with_capacity(filtered_managers.len() + usize::from(selected_loading_sources > 0));
        if selected_loading_sources > 0 {
            updates_sections.push(shared::pending_sources_notice(
                format!(
                    "Loading {selected_loading_sources} remaining selected source{}...",
                    if selected_loading_sources == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
                (!self.stoppable_sources(info).is_empty()).then_some(Message::StopWaiting),
            ));
        }
        updates_sections.extend(filtered_managers.into_iter().map(
            |(manager, (count, packages))| {
                self.package_manager_section(manager, *count, packages, info, catalog)
            },
        ));

        let update_list = scrollable(column(updates_sections).spacing(20))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill);
        let inspected = self.inspected_package.as_ref().and_then(|(manager, name)| {
            info.updates_by_manager
                .get(manager)
                .and_then(|(_, packages)| {
                    packages.iter().find(|package| package.target.name == *name)
                })
                .map(|package| {
                    let key = shared::selection_key(manager, &package.target.name);
                    let installed = self.package_detail.package(&key).or_else(|| {
                        installed_info
                            .installed_packages
                            .get(manager)
                            .and_then(|(_, packages)| {
                                packages
                                    .iter()
                                    .find(|installed| installed.name == package.target.name)
                            })
                    });
                    crate::content::shared::PackageInspector {
                        manager: manager.clone(),
                        name: &package.target.name,
                        version: &package.current_version,
                        available_version: Some(&package.available_version),
                        description: installed.and_then(|package| package.description.as_deref()),
                        size: installed.and_then(|package| package.size),
                        install_date: installed.and_then(|package| package.install_date.as_deref()),
                        homepage: installed.and_then(|package| package.homepage.as_deref()),
                        scope: installed.map_or(package.target.scope, |package| package.scope),
                        origin: installed
                            .and_then(|package| package.origin.as_ref())
                            .or(package.target.origin.as_ref()),
                        is_loading: self.package_detail.is_loading(&key),
                        detail_error: self
                            .package_detail
                            .error(&key)
                            .or(self.inspector_error.as_deref()),
                    }
                })
        });
        let retry_info = self.inspected_package.as_ref().and_then(|(manager, name)| {
            self.package_detail
                .error(&shared::selection_key(manager, name))
                .map(|_| Message::RetryPackageInfo(manager.clone(), name.clone()))
        });
        let inspector = shared::package_inspector(
            inspected,
            catalog,
            Message::CopyInspectorText,
            Message::CopyInspectorText,
            Message::CopyInspectorText,
            retry_info,
        );

        if !show_inspector {
            return container(update_list)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into();
        }

        let inspector = container(inspector)
            .padding(theme::spacing::LG)
            .width(if inspector_drawer {
                iced::Length::Fill
            } else {
                iced::Length::Fixed(268.0)
            })
            .height(iced::Length::Fill)
            .style(theme::surface_container);
        if inspector_drawer {
            return column![container(update_list).width(iced::Length::Fill), inspector]
                .spacing(theme::spacing::LG)
                .into();
        }

        row![
            container(update_list)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill),
            inspector,
        ]
        .spacing(theme::spacing::LG)
        .height(iced::Length::Fill)
        .into()
    }

    fn package_manager_section<'a>(
        &self,
        manager: ManagerId,
        count: usize,
        packages: &'a [PackageUpdate],
        info: &'a UpdatesInfo,
        catalog: &'a ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        let is_loading = info.loading_updates.contains_key(&manager);
        let filtered_packages = self.filter_and_sort_updates(packages, info.sort_by);
        let subtitle = self.source_subtitle(info, &manager, count, is_loading);

        let body = (!filtered_packages.is_empty()).then(|| {
            iced::widget::column(
                filtered_packages
                    .into_iter()
                    .map(|pkg| self.package_item_view(manager.clone(), pkg, info)),
            )
            .spacing(8)
            .into()
        });

        shared::manager_section(
            manager.clone(),
            catalog,
            subtitle,
            ManagerSectionStyle {
                accent: theme::colors::UPDATES,
                error_prefix: "Failed to load updates",
            },
            info.load_errors
                .get(&manager)
                .or_else(|| info.init_errors.get(&manager))
                .map(String::as_str),
            || Message::RetryLoad(manager),
            Message::CopyInspectorText,
            body,
        )
    }

    /// Subtitle for one updates source header.
    ///
    /// A source whose last check failed keeps its previous count for context,
    /// but labels it as last known so it cannot read as current, and reports
    /// the time of its last successful check when there is one.
    fn source_subtitle(
        &self,
        info: &UpdatesInfo,
        manager: &ManagerId,
        count: usize,
        is_loading: bool,
    ) -> String {
        if is_loading {
            return "(Loading...)".to_owned();
        }
        if info.has_error(manager) {
            return if count == 0 {
                "(last known: none)".to_owned()
            } else {
                format!("(last known: {count})")
            };
        }
        let checked = info
            .checked_at
            .get(manager)
            .and_then(|checked_at| UpdatesInfo::checked_at_label(checked_at));
        match checked {
            Some(checked) => format!("({count} updates · checked {checked})"),
            None => format!("({count} updates)"),
        }
    }

    fn filter_and_sort_updates<'a>(
        &self,
        packages: &'a [PackageUpdate],
        sort_by: SortOption,
    ) -> Vec<&'a PackageUpdate> {
        let query = self.search_query.trim().to_lowercase();
        let mut filtered: Vec<_> = packages
            .iter()
            .filter(|pkg| {
                if query.is_empty() {
                    true
                } else {
                    pkg.target.name.to_lowercase().contains(&query)
                }
            })
            .collect();

        match sort_by {
            SortOption::Name => {
                filtered.sort_by(|a, b| a.target.name.cmp(&b.target.name));
            }
            SortOption::CurrentVersion => {
                filtered.sort_by(|a, b| a.current_version.cmp(&b.current_version));
            }
            SortOption::NewVersion => {
                filtered.sort_by(|a, b| a.available_version.cmp(&b.available_version));
            }
        }

        filtered
    }

    fn package_item_view<'a>(
        &self,
        manager: ManagerId,
        package: &'a PackageUpdate,
        info: &'a UpdatesInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, checkbox, column, row, text};

        let package_name = package.target.name.clone();
        let is_selected = info
            .selected_packages
            .contains(&shared::selection_key(&manager, &package.target.name));

        let package_checkbox = checkbox(is_selected)
            .on_toggle_maybe(
                (!info.is_updating && self.pending_update.is_none()).then_some({
                    let package_name = package_name.clone();
                    let manager = manager.clone();
                    move |selected| {
                        Message::TogglePackageSelection(
                            manager.clone(),
                            package_name.clone(),
                            selected,
                        )
                    }
                }),
            )
            .size(18)
            .spacing(8)
            .style(shared::checkbox_style(false));

        let versions = row![
            column![
                text("Current").size(11).style(theme::text_on_surface_muted),
                text(&package.current_version)
                    .size(13)
                    .font(theme::FONT_MONO)
                    .style(theme::text_on_surface_alt)
                    .width(iced::Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
            ]
            .spacing(2)
            .width(iced::Length::FillPortion(1)),
            text("->").size(13).style(theme::text_on_surface_muted),
            column![
                text("Available")
                    .size(11)
                    .style(theme::text_on_surface_muted),
                text(&package.available_version)
                    .size(13)
                    .font(theme::FONT_MONO)
                    .style(theme::text_on_surface)
                    .width(iced::Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
            ]
            .spacing(2)
            .width(iced::Length::FillPortion(1)),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center)
        .width(iced::Length::Fill);

        let is_inspected =
            self.inspected_package
                .as_ref()
                .is_some_and(|(selected_manager, name)| {
                    selected_manager == &manager && name == &package.target.name
                });
        let details = button(
            column![
                text(&package.target.name)
                    .size(15)
                    .font(theme::FONT_SEMIBOLD)
                    .style(theme::text_on_surface),
                versions,
            ]
            .spacing(6)
            .width(iced::Length::Fill),
        )
        .padding([8, 10])
        .width(iced::Length::Fill)
        .style(theme::list_row(is_inspected))
        .on_press(Message::InspectPackage(manager, package_name));

        row![package_checkbox, details]
            .spacing(theme::spacing::SM)
            .align_y(iced::Alignment::Center)
            .into()
    }

    fn update_confirmation_view<'a>(
        &'a self,
        catalog: &'a ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, column, container, row, text};

        let Some(plan) = &self.pending_update else {
            return container("").height(iced::Length::Shrink).into();
        };

        let package_count = plan.package_count();
        let manager_count = plan.manager_count();
        let failed_count = plan.failed_sources.len();
        let failed_names = plan
            .failed_sources
            .iter()
            .map(|manager| catalog.display_name(manager))
            .collect::<Vec<_>>()
            .join(", ");
        let detail = if package_count == 0 {
            "No updates were found after refreshing the requested sources.".to_owned()
        } else {
            format!("{package_count} package(s) from {manager_count} source(s)")
        };
        let failed_detail = (failed_count > 0).then(|| {
            format!("Excluded failed source(s): {failed_names}. Re-scan them before retrying.")
        });
        let title = if package_count == 0 {
            "No updates found"
        } else if plan.scope == UpdatePlanScope::Selected {
            "Selected update plan ready"
        } else {
            "Update All plan ready"
        };

        let confirm = button(
            text(format!("Update {package_count} Packages"))
                .size(13)
                .font(theme::FONT_SEMIBOLD)
                .style(theme::text_on_primary),
        )
        .padding([8, 14])
        .style(theme::action_button(
            package_count > 0,
            theme::colors::UPDATE_ACTION,
            theme::colors::UPDATE_ACTION_HOVER,
            theme::colors::UPDATE_ACTION_ACTIVE,
        ));
        let confirm = if package_count > 0 {
            confirm.on_press(Message::ConfirmUpdate)
        } else {
            confirm
        };

        let mut content = column![
            row![
                column![
                    text(title)
                        .size(14)
                        .font(theme::FONT_SEMIBOLD)
                        .style(theme::text_on_surface),
                    text(detail).size(13).style(theme::text_on_surface_muted),
                ]
                .spacing(theme::spacing::XS)
                .width(iced::Length::Fill),
                button(text("Cancel").size(13))
                    .padding([8, 12])
                    .style(theme::secondary_button(true))
                    .on_press(Message::CancelUpdate),
                confirm,
            ]
            .spacing(theme::spacing::MD)
            .align_y(iced::Alignment::Center)
            .wrap(),
        ]
        .spacing(theme::spacing::MD);
        if !plan.packages.manager_groups.is_empty() {
            content = content.push(shared::package_action_plan_view(
                &plan.packages.manager_groups,
                catalog,
            ));
        }
        if plan
            .packages
            .manager_groups
            .iter()
            .any(|(manager, _)| manager.as_str() == FULL_SYSTEM_UPGRADE_MANAGER)
        {
            content = content.push(
                text(
                    "Pacman updates run as one full system upgrade (pacman -Syu), which also \
                     applies any newer updates found when it syncs.",
                )
                .size(12)
                .style(theme::text_on_surface_muted)
                .width(iced::Length::Fill)
                .wrapping(text::Wrapping::WordOrGlyph),
            );
        }
        if let Some(failed_detail) = failed_detail {
            content = content.push(
                text(failed_detail)
                    .size(12)
                    .style(theme::text_error)
                    .width(iced::Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
            );
        }

        container(content)
            .padding(theme::spacing::MD)
            .width(iced::Length::Fill)
            .style(theme::surface_container)
            .into()
    }

    fn batch_actions_view<'a>(
        &self,
        info: &'a UpdatesInfo,
        pm_config: &updater_core::Config,
        catalog: &'a ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, checkbox, column, row, text};

        let selected_count = info.selected_packages.len();
        let is_preparing_all = !self.update_all_refreshing.is_empty();
        let is_enabled = selected_count > 0
            && !info.is_updating
            && self.pending_update.is_none()
            && !is_preparing_all;

        let query = self.search_query.trim().to_lowercase();
        let mut total_visible = 0;
        let mut selected_visible = 0;
        for pm_type in &info.selected_managers {
            if let Some((_, packages)) = info.updates_by_manager.get(pm_type) {
                for package in packages {
                    if !query.is_empty() && !package.target.name.to_lowercase().contains(&query) {
                        continue;
                    }
                    total_visible += 1;
                    if info
                        .selected_packages
                        .contains(&shared::selection_key(pm_type, &package.target.name))
                    {
                        selected_visible += 1;
                    }
                }
            }
        }

        let all_selected = total_visible > 0 && selected_visible == total_visible;

        let button_text = if info.is_updating {
            if let Some((completed, total, manager, package)) = &info.update_progress {
                if package.is_empty() {
                    format!("Updating {}/{}...", completed, total)
                } else {
                    format!(
                        "Updating {}/{}: {} ({})",
                        completed,
                        total,
                        package,
                        catalog.display_name(manager)
                    )
                }
            } else {
                "Updating...".to_string()
            }
        } else if selected_count > 0 {
            format!("Update {} package(s)", selected_count)
        } else {
            "Update Selected".to_string()
        };

        let select_all_checkbox = checkbox(all_selected)
            .label("Select All")
            .on_toggle_maybe(
                (!info.is_updating && self.pending_update.is_none())
                    .then_some(Message::ToggleSelectAll),
            )
            .size(18)
            .spacing(8)
            .text_size(14)
            .style(shared::checkbox_style(false));

        let update_button = button(text(button_text).size(14).font(theme::FONT_SEMIBOLD).style(
            if is_enabled {
                theme::text_on_primary
            } else {
                theme::text_on_surface_muted
            },
        ))
        .padding([8, 16])
        .style(theme::action_button(
            is_enabled,
            theme::colors::UPDATE_ACTION,
            theme::colors::UPDATE_ACTION_HOVER,
            theme::colors::UPDATE_ACTION_ACTIVE,
        ));

        let update_button = if is_enabled {
            update_button.on_press(Message::PrepareSelectedUpdate)
        } else {
            update_button
        };

        let update_all_enabled = !info.is_updating
            && self.pending_update.is_none()
            && !is_preparing_all
            && !Self::any_loading_updates(
                info,
                shared::configured_managers_with_capability(
                    pm_config,
                    catalog,
                    ManagerCapability::Updates,
                )
                .into_iter(),
            );
        let update_all = button(
            text(if is_preparing_all {
                "Preparing Update All..."
            } else {
                "Update All Available"
            })
            .size(13)
            .font(theme::FONT_SEMIBOLD)
            .style(if update_all_enabled {
                theme::text_on_surface
            } else {
                theme::text_on_surface_muted
            }),
        )
        .padding([8, 14])
        .style(theme::secondary_button(update_all_enabled));
        let update_all = if update_all_enabled {
            update_all.on_press(Message::PrepareUpdateAll)
        } else {
            update_all
        };

        let actions_row = row![select_all_checkbox, update_button, update_all]
            .spacing(12)
            .align_y(iced::Alignment::Center);

        if let Some(notice) = &info.last_operation_notice {
            let retry = if notice.is_stopped() {
                None
            } else {
                info.failed_update_manager.as_ref().map(|_| {
                    button(
                        text("Re-scan Failed Source")
                            .size(13)
                            .font(theme::FONT_SEMIBOLD),
                    )
                    .padding([7, 12])
                    .style(theme::secondary_button(true))
                    .on_press(Message::PrepareFailedUpdateRetry)
                })
            };
            let mut notice_row = row![shared::operation_notice_card(
                notice,
                catalog,
                Message::CopyInspectorText,
                Message::DismissOperationNotice,
            )]
            .width(iced::Length::Fill)
            .align_y(iced::Alignment::Center);
            if let Some(retry) = retry {
                notice_row = notice_row.push(retry);
            }

            column![actions_row, notice_row].spacing(8).into()
        } else {
            actions_row.into()
        }
    }

    pub(crate) fn start_load(
        pm_config: &updater_core::Config,
        info: &mut UpdatesInfo,
        manager: ManagerId,
        catalog: &ManagerCatalog,
        mode: RefreshMode,
    ) -> Task<Message> {
        info.request_generation = info.request_generation.wrapping_add(1);
        let request_id = info.request_generation;
        info.loading_updates.insert(manager.clone(), request_id);
        info.refresh_modes.insert(manager.clone(), mode);

        let pm_config = pm_config.clone();
        let registry = catalog.registry();
        let result_manager = manager.clone();

        Task::future(async move {
            let runtime = registry
                .manager_for(&manager, ManagerCapability::Updates)
                .map_err(|error| error.to_string())?;
            let manager_config = pm_config
                .manager(&manager)
                .ok_or_else(|| format!("Manager is not configured: {manager}"))?;
            runtime
                .updates(manager_config, mode.forces_metadata_sync())
                .await
                .map_err(|error| Self::describe_load_error(&error))
        })
        .then(move |result| {
            Task::done(Message::LoadUpdatesResult {
                request_id,
                manager: result_manager.clone(),
                result,
            })
        })
    }

    /// Renders an Updates load failure into the error string kept in state.
    fn describe_load_error(error: &updater_manager_api::ManagerError) -> String {
        shared::describe_manager_error(error)
    }

    fn build_update_plan(
        info: &UpdatesInfo,
        scope: &HashSet<ManagerId>,
        catalog: &ManagerCatalog,
        plan_scope: UpdatePlanScope,
    ) -> UpdatePlan {
        let mut manager_groups: Vec<_> = info
            .updates_by_manager
            .iter()
            .filter(|(manager, _)| scope.contains(manager))
            .filter(|(manager, _)| {
                !info.init_errors.contains_key(manager) && !info.load_errors.contains_key(manager)
            })
            .filter(|(_, (_, packages))| !packages.is_empty())
            .map(|(manager, (_, packages))| {
                let mut targets: Vec<_> = packages
                    .iter()
                    .map(|package| package.target.clone())
                    .collect();
                targets.sort_by(|left, right| left.name.cmp(&right.name));
                (manager.clone(), targets)
            })
            .collect();
        manager_groups.sort_by(|(left, _), (right, _)| {
            catalog
                .display_name(left)
                .cmp(catalog.display_name(right))
                .then_with(|| left.cmp(right))
        });

        let mut failed_sources: Vec<_> = info
            .init_errors
            .keys()
            .chain(info.load_errors.keys())
            .filter(|manager| scope.contains(manager))
            .cloned()
            .collect();
        failed_sources.sort_by(|left, right| {
            catalog
                .display_name(left)
                .cmp(catalog.display_name(right))
                .then_with(|| left.cmp(right))
        });
        failed_sources.dedup();

        UpdatePlan {
            scope: plan_scope,
            packages: PackageActionPlan { manager_groups },
            failed_sources,
        }
    }

    fn update_plan_action(
        pm_config: &updater_core::Config,
        manager_groups: Vec<(ManagerId, Vec<PackageTarget>)>,
        catalog: &ManagerCatalog,
    ) -> Action {
        let cancellation = CancellationToken::default();
        let task = run_grouped_package_action(
            catalog.registry(),
            pm_config,
            PackageAction::Update,
            manager_groups,
            cancellation.clone(),
            |OperationProgress {
                 completed,
                 total,
                 manager,
                 current_package,
                 command_message,
             }| Message::UpdateProgress {
                completed,
                total,
                manager,
                current_package,
                command_message,
            },
            Message::UpdatePackagesResult,
        );
        Action::CancellableRun(task, cancellation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use updater_manager_api::PackageTarget;

    fn manager_id(value: &str) -> ManagerId {
        ManagerId::parse(value).unwrap()
    }

    fn update(manager: &ManagerId, name: &str) -> PackageUpdate {
        PackageUpdate::new(PackageTarget::new(manager.clone(), name), "1.0", "2.0")
    }

    #[test]
    fn selected_loading_sources_counts_initialization_without_cached_results() {
        let mut info = UpdatesInfo::default();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        info.is_loading_count = true;
        info.selected_managers = HashSet::from([cargo.clone(), npm]);
        info.updates_by_manager.insert(cargo, (0, Vec::new()));

        assert_eq!(info.selected_loading_sources(), 1);
    }

    #[test]
    fn selected_loading_sources_counts_refreshes_with_cached_results() {
        let mut info = UpdatesInfo::default();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        info.selected_managers.insert(cargo.clone());
        info.updates_by_manager
            .insert(cargo.clone(), (1, vec![update(&cargo, "cargo-edit")]));
        info.loading_updates.insert(cargo, 1);
        info.loading_updates.insert(npm, 2);

        assert_eq!(info.selected_loading_sources(), 1);
    }

    #[test]
    fn selected_sources_have_errors_includes_initialization_failures() {
        let mut info = UpdatesInfo::default();
        let cargo = manager_id("builtin:cargo");
        info.selected_managers.insert(cargo.clone());
        info.updates_by_manager
            .insert(cargo.clone(), (0, Vec::new()));
        info.init_errors
            .insert(cargo, "failed to initialize".to_owned());

        assert!(info.selected_sources_have_errors());
    }

    #[test]
    fn refresh_all_is_ignored_while_initialization_is_running() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo {
            is_loading_count: true,
            ..UpdatesInfo::default()
        };

        let action = updates.update(
            Message::RefreshAll,
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        assert!(matches!(action, Action::None));
    }

    #[test]
    fn clearing_visible_sources_preserves_hidden_selection() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo::default();
        let cargo = manager_id("builtin:cargo");
        let flatpak = manager_id("builtin:flatpak");
        info.selected_managers = HashSet::from([cargo.clone(), flatpak.clone()]);
        info.selected_packages
            .insert(shared::selection_key(&cargo, "cargo-edit"));
        info.selected_packages
            .insert(shared::selection_key(&flatpak, "org.example.App"));

        let action = updates.update(
            Message::SetSourceSelection(vec![cargo.clone()], false),
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        assert!(matches!(action, Action::None));
        assert_eq!(info.selected_managers, HashSet::from([flatpak.clone()]));
        assert!(
            !info
                .selected_packages
                .contains(&shared::selection_key(&cargo, "cargo-edit"))
        );
        assert!(
            info.selected_packages
                .contains(&shared::selection_key(&flatpak, "org.example.App"))
        );
    }

    #[test]
    fn selected_update_confirmation_executes_the_frozen_plan() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo::default();
        let manager = manager_id("builtin:cargo");
        info.selected_managers.insert(manager.clone());
        info.updates_by_manager.insert(
            manager.clone(),
            (2, vec![update(&manager, "alpha"), update(&manager, "beta")]),
        );
        info.selected_packages
            .insert(shared::selection_key(&manager, "alpha"));
        let config = updater_core::Config::default();
        let catalog = ManagerCatalog::builtin();

        let action = updates.update(Message::PrepareSelectedUpdate, &config, &mut info, &catalog);

        assert!(matches!(action, Action::None));
        assert!(!info.is_updating);
        let plan = updates.pending_update.as_ref().unwrap();
        assert_eq!(plan.scope, UpdatePlanScope::Selected);
        assert_eq!(
            plan.packages.manager_groups,
            vec![(
                manager.clone(),
                vec![PackageTarget::new(manager.clone(), "alpha")],
            )]
        );

        info.selected_packages.clear();
        info.selected_packages
            .insert(shared::selection_key(&manager, "beta"));
        info.updates_by_manager.clear();

        let action = updates.update(Message::ConfirmUpdate, &config, &mut info, &catalog);

        assert!(matches!(action, Action::CancellableRun(_, _)));
        assert!(info.is_updating);
        assert_eq!(info.update_progress, Some((0, 1, manager, String::new())));
        assert!(updates.pending_update.is_none());
    }

    #[test]
    fn updates_load_failure_keeps_manager_error_detail() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo::default();
        let manager = manager_id("builtin:apt");
        info.loading_updates.insert(manager.clone(), 1);

        let error = updater_manager_api::ManagerError::new(
            updater_manager_api::ManagerErrorKind::Busy,
            "package manager command failed",
        )
        .with_detail("apt-get update failed:\nE: Could not get lock /var/lib/dpkg/lock");
        let result = Err(Updates::describe_load_error(&error));

        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: 1,
                manager: manager.clone(),
                result,
            },
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        let stored = info
            .load_errors
            .get(&manager)
            .expect("the read path records the failure");
        assert!(stored.contains("E: Could not get lock /var/lib/dpkg/lock"));
        assert!(stored.contains("Another program holds the package database lock"));
    }

    #[test]
    fn selecting_one_pacman_update_plans_the_full_system_upgrade() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo::default();
        let pacman = manager_id(FULL_SYSTEM_UPGRADE_MANAGER);
        let cargo = manager_id("builtin:cargo");
        info.selected_managers = HashSet::from([pacman.clone(), cargo.clone()]);
        info.updates_by_manager.insert(
            pacman.clone(),
            (2, vec![update(&pacman, "glibc"), update(&pacman, "python")]),
        );
        info.updates_by_manager.insert(
            cargo.clone(),
            (2, vec![update(&cargo, "alpha"), update(&cargo, "beta")]),
        );
        info.selected_packages
            .insert(shared::selection_key(&pacman, "python"));
        info.selected_packages
            .insert(shared::selection_key(&cargo, "alpha"));

        updates.update(
            Message::PrepareSelectedUpdate,
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        let mut groups = updates.pending_update.unwrap().packages.manager_groups;
        groups.sort_by(|(left, _), (right, _)| left.cmp(right));
        assert_eq!(
            groups,
            vec![
                (
                    cargo.clone(),
                    vec![PackageTarget::new(cargo.clone(), "alpha")],
                ),
                (
                    pacman.clone(),
                    vec![
                        PackageTarget::new(pacman.clone(), "glibc"),
                        PackageTarget::new(pacman.clone(), "python"),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn stale_update_selection_does_not_open_confirmation() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo::default();
        let manager = manager_id("builtin:cargo");
        info.selected_packages
            .insert(shared::selection_key(&manager, "missing"));

        let action = updates.update(
            Message::PrepareSelectedUpdate,
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        assert!(matches!(action, Action::None));
        assert!(updates.pending_update.is_none());
        assert!(!info.is_updating);
        assert_eq!(
            info.last_operation_notice,
            Some(shared::OperationNotice::failed(
                PackageAction::Update,
                "Selected packages are no longer available to update".to_owned(),
            ))
        );
    }

    #[test]
    fn escape_dismisses_update_confirmation_before_the_inspector() {
        let manager = manager_id("builtin:cargo");
        let mut updates = Updates {
            inspected_package: Some(shared::selection_key(&manager, "alpha")),
            pending_update: Some(UpdatePlan {
                scope: UpdatePlanScope::Selected,
                packages: PackageActionPlan {
                    manager_groups: vec![(
                        manager.clone(),
                        vec![PackageTarget::new(manager, "alpha")],
                    )],
                },
                failed_sources: Vec::new(),
            }),
            ..Updates::default()
        };

        assert!(updates.dismiss_transient());
        assert!(updates.pending_update.is_none());
        assert!(updates.inspected_package.is_some());
        assert!(updates.dismiss_transient());
        assert!(updates.inspected_package.is_none());
    }

    #[test]
    fn update_all_plan_excludes_failed_sources() {
        let mut info = UpdatesInfo::default();
        let dnf = manager_id("builtin:dnf");
        let flatpak = manager_id("builtin:flatpak");
        info.updates_by_manager.insert(
            dnf.clone(),
            (2, vec![update(&dnf, "alpha"), update(&dnf, "beta")]),
        );
        info.updates_by_manager
            .insert(flatpak.clone(), (1, vec![update(&flatpak, "gamma")]));
        info.load_errors
            .insert(flatpak.clone(), "network error".to_owned());
        let scope = HashSet::from([dnf.clone(), flatpak.clone()]);

        let plan = Updates::build_update_plan(
            &info,
            &scope,
            &ManagerCatalog::builtin(),
            UpdatePlanScope::All,
        );

        assert_eq!(plan.package_count(), 2);
        assert_eq!(plan.manager_count(), 1);
        assert_eq!(plan.scope, UpdatePlanScope::All);
        assert_eq!(plan.packages.manager_groups[0].0, dnf);
        assert_eq!(plan.failed_sources, vec![flatpak]);
    }

    #[test]
    fn failed_source_retry_plan_does_not_repeat_successful_sources() {
        let mut info = UpdatesInfo::default();
        let dnf = manager_id("builtin:dnf");
        let flatpak = manager_id("builtin:flatpak");
        info.updates_by_manager
            .insert(dnf.clone(), (1, vec![update(&dnf, "already-done")]));
        info.updates_by_manager
            .insert(flatpak.clone(), (1, vec![update(&flatpak, "retry-me")]));
        let scope = HashSet::from([flatpak.clone()]);

        let plan = Updates::build_update_plan(
            &info,
            &scope,
            &ManagerCatalog::builtin(),
            UpdatePlanScope::All,
        );

        assert_eq!(plan.package_count(), 1);
        assert_eq!(plan.manager_count(), 1);
        assert_eq!(plan.packages.manager_groups[0].0, flatpak);
        assert_eq!(
            plan.packages.manager_groups[0].1,
            vec![PackageTarget::new(flatpak, "retry-me")]
        );
    }

    #[test]
    fn update_result_only_advances_the_active_preflight_request() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo::default();
        let manager = manager_id("builtin:cargo");
        updates.update_all_scope.insert(manager.clone());
        updates.update_all_refreshing.insert(manager.clone());
        info.loading_updates.insert(manager.clone(), 2);

        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: 1,
                manager: manager.clone(),
                result: Err("stale result".to_owned()),
            },
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        assert_eq!(info.loading_updates.get(&manager), Some(&2));
        assert!(info.load_errors.is_empty());
        assert!(updates.update_all_refreshing.contains(&manager));
        assert!(updates.pending_update.is_none());

        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: 2,
                manager: manager.clone(),
                result: Err("current result".to_owned()),
            },
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        assert!(!info.loading_updates.contains_key(&manager));
        assert_eq!(
            info.load_errors.get(&manager).map(String::as_str),
            Some("current result")
        );
        assert!(updates.update_all_refreshing.is_empty());
        assert!(updates.pending_update.is_some());
    }

    fn configured(managers: &[&ManagerId]) -> updater_core::Config {
        updater_core::Config {
            managers: managers
                .iter()
                .map(|manager| updater_core::ManagerConfig::new((*manager).clone()))
                .collect(),
            ..updater_core::Config::default()
        }
    }

    #[test]
    fn stop_waiting_releases_refresh_and_ignores_late_update_result() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo {
            has_loading_count: true,
            ..UpdatesInfo::default()
        };
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        let config = configured(&[&cargo, &npm]);
        let catalog = ManagerCatalog::builtin();
        info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        info.updates_by_manager
            .insert(npm.clone(), (1, vec![update(&npm, "typescript")]));
        info.updates_by_manager
            .insert(cargo.clone(), (2, Vec::new()));
        info.loading_updates.insert(cargo.clone(), 3);

        assert!(matches!(
            updates.update(Message::RefreshSelected, &config, &mut info, &catalog),
            Action::None
        ));
        assert!(updates.stoppable_sources(&info).contains(&cargo));

        let action = updates.update(Message::StopWaiting, &config, &mut info, &catalog);

        assert!(matches!(action, Action::None));
        assert!(info.loading_updates.is_empty());
        assert_eq!(
            info.load_errors.get(&cargo).map(String::as_str),
            Some("Stopped waiting for Cargo; a late response will be ignored.")
        );
        assert_eq!(info.selected_loading_sources(), 0);

        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: 3,
                manager: cargo.clone(),
                result: Ok(vec![update(&cargo, "late-crate")]),
            },
            &config,
            &mut info,
            &catalog,
        );
        assert!(
            info.updates_by_manager
                .get(&cargo)
                .is_some_and(|(_, packages)| packages.is_empty())
        );
        assert!(info.load_errors.contains_key(&cargo));
        assert_eq!(
            info.updates_by_manager.get(&npm).map(|(count, _)| *count),
            Some(1)
        );

        assert!(matches!(
            updates.update(Message::RefreshSelected, &config, &mut info, &catalog),
            Action::Run(_)
        ));
        assert!(info.loading_updates.contains_key(&cargo));
        assert!(
            info.loading_updates.contains_key(&npm),
            "Refresh Selected reloads every selected source"
        );
        assert!(matches!(
            updates.update(
                Message::RetryLoad(npm.clone()),
                &config,
                &mut info,
                &catalog
            ),
            Action::None
        ));
    }

    #[test]
    fn refresh_selected_ignores_loads_of_unselected_sources() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo {
            has_loading_count: true,
            ..UpdatesInfo::default()
        };
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        let config = configured(&[&cargo, &npm]);
        info.selected_managers.insert(cargo.clone());
        info.updates_by_manager
            .insert(cargo.clone(), (0, Vec::new()));
        info.loading_updates.insert(npm.clone(), 9);

        assert!(matches!(
            updates.update(
                Message::RefreshSelected,
                &config,
                &mut info,
                &ManagerCatalog::builtin()
            ),
            Action::Run(_)
        ));
        assert!(matches!(
            updates.update(
                Message::PrepareUpdateAll,
                &config,
                &mut info,
                &ManagerCatalog::builtin()
            ),
            Action::None
        ));
        assert!(updates.update_all_refreshing.is_empty());
    }

    #[test]
    fn stop_waiting_completes_update_all_preflight_without_the_stopped_source() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo {
            has_loading_count: true,
            ..UpdatesInfo::default()
        };
        let dnf = manager_id("builtin:dnf");
        let flatpak = manager_id("builtin:flatpak");
        let config = configured(&[&dnf, &flatpak]);
        let catalog = ManagerCatalog::builtin();
        info.updates_by_manager.insert(dnf.clone(), (0, Vec::new()));
        info.updates_by_manager
            .insert(flatpak.clone(), (0, Vec::new()));

        assert!(matches!(
            updates.update(Message::PrepareUpdateAll, &config, &mut info, &catalog),
            Action::Run(_)
        ));
        let flatpak_request = info.loading_updates[&flatpak];
        let dnf_request = info.loading_updates[&dnf];
        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: flatpak_request,
                manager: flatpak.clone(),
                result: Ok(vec![update(&flatpak, "org.example.App")]),
            },
            &config,
            &mut info,
            &catalog,
        );
        assert!(updates.pending_update.is_none());

        let _ = updates.update(Message::StopWaiting, &config, &mut info, &catalog);

        let plan = updates
            .pending_update
            .as_ref()
            .expect("stopping the last source completes the preflight");
        assert_eq!(plan.packages.manager_groups.len(), 1);
        assert_eq!(plan.packages.manager_groups[0].0, flatpak);
        assert_eq!(plan.failed_sources, vec![dnf.clone()]);

        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: dnf_request,
                manager: dnf.clone(),
                result: Ok(vec![update(&dnf, "late-package")]),
            },
            &config,
            &mut info,
            &catalog,
        );
        assert_eq!(
            updates
                .pending_update
                .as_ref()
                .map(UpdatePlan::package_count),
            Some(1)
        );
    }

    #[test]
    fn successful_load_records_checked_at() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo::default();
        let cargo = manager_id("builtin:cargo");
        let config = configured(&[&cargo]);
        let catalog = ManagerCatalog::builtin();
        info.loading_updates.insert(cargo.clone(), 1);

        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: 1,
                manager: cargo.clone(),
                result: Ok(vec![update(&cargo, "cargo-edit")]),
            },
            &config,
            &mut info,
            &catalog,
        );

        let checked_at = info
            .checked_at
            .get(&cargo)
            .expect("a successful load stamps the source as checked");
        assert!(chrono::DateTime::parse_from_rfc3339(checked_at).is_ok());
        assert_eq!(info.current_update_count(), 1);
        assert_eq!(
            updates.source_subtitle(&info, &cargo, 1, false),
            format!(
                "(1 updates · checked {})",
                UpdatesInfo::checked_at_label(checked_at).unwrap()
            )
        );
    }

    #[test]
    fn failed_refresh_marks_count_as_last_known_and_drops_out_of_the_total() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo::default();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        let config = configured(&[&cargo, &npm]);
        let catalog = ManagerCatalog::builtin();
        info.loading_updates.insert(cargo.clone(), 1);
        info.loading_updates.insert(npm.clone(), 2);

        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: 1,
                manager: cargo.clone(),
                result: Ok(vec![
                    update(&cargo, "cargo-edit"),
                    update(&cargo, "cargo-nextest"),
                    update(&cargo, "cargo-audit"),
                ]),
            },
            &config,
            &mut info,
            &catalog,
        );
        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: 2,
                manager: npm.clone(),
                result: Ok(vec![update(&npm, "typescript")]),
            },
            &config,
            &mut info,
            &catalog,
        );
        assert_eq!(info.current_update_count(), 4);

        info.loading_updates.insert(cargo.clone(), 3);
        let _ = updates.update(
            Message::LoadUpdatesResult {
                request_id: 3,
                manager: cargo.clone(),
                result: Err("Failed to load updates".to_owned()),
            },
            &config,
            &mut info,
            &catalog,
        );

        assert_eq!(
            info.updates_by_manager.get(&cargo).map(|(count, _)| *count),
            Some(3),
            "the previous count is kept for context"
        );
        assert_eq!(
            info.current_update_count(),
            1,
            "a stale count must not be presented as current"
        );
        assert_eq!(
            updates.source_subtitle(&info, &cargo, 3, false),
            "(last known: 3)"
        );
        assert_eq!(
            updates.source_subtitle(&info, &npm, 1, false),
            format!(
                "(1 updates · checked {})",
                UpdatesInfo::checked_at_label(info.checked_at.get(&npm).unwrap()).unwrap()
            )
        );
    }

    #[test]
    fn loading_source_subtitle_hides_the_last_known_count() {
        let updates = Updates::default();
        let info = UpdatesInfo::default();
        let cargo = manager_id("builtin:cargo");

        assert_eq!(
            updates.source_subtitle(&info, &cargo, 3, true),
            "(Loading...)"
        );
    }

    #[test]
    fn explicit_refresh_forces_a_privileged_metadata_sync() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo {
            has_loading_count: true,
            ..UpdatesInfo::default()
        };
        let cargo = manager_id("builtin:cargo");
        let config = configured(&[&cargo]);
        let catalog = ManagerCatalog::builtin();
        info.selected_managers.insert(cargo.clone());
        info.updates_by_manager
            .insert(cargo.clone(), (0, Vec::new()));

        let _ = updates.update(Message::RefreshSelected, &config, &mut info, &catalog);

        assert_eq!(
            info.refresh_modes.get(&cargo),
            Some(&RefreshMode::Privileged)
        );
    }

    #[test]
    fn reloading_a_cached_listing_stays_local() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo {
            has_loading_count: true,
            ..UpdatesInfo::default()
        };
        let cargo = manager_id("builtin:cargo");
        let config = configured(&[&cargo]);
        let catalog = ManagerCatalog::builtin();
        info.selected_managers.insert(cargo.clone());
        info.updates_by_manager
            .insert(cargo.clone(), (1, Vec::new()));

        let _ = updates.update(
            Message::SelectPackageManager(cargo.clone(), true),
            &config,
            &mut info,
            &catalog,
        );

        assert_eq!(info.refresh_modes.get(&cargo), Some(&RefreshMode::Local));
    }

    #[test]
    fn only_a_privileged_refresh_forces_a_metadata_sync() {
        assert!(!RefreshMode::Local.forces_metadata_sync());
        assert!(RefreshMode::Privileged.forces_metadata_sync());
    }

    fn update_outcome(cancelled: bool, error: Option<&str>) -> OperationOutcome {
        OperationOutcome {
            action: PackageAction::Update,
            completed_packages: if cancelled { 1 } else { 0 },
            total_packages: 2,
            completed_managers: 0,
            total_managers: 1,
            failed_manager: (!cancelled).then(|| manager_id("builtin:apt")),
            error: error.map(str::to_owned),
            cancelled,
            manager_outcomes: Vec::new(),
            scope: updater_manager_api::PackageScope::System,
        }
    }

    #[test]
    fn cancelled_update_outcome_is_not_reported_as_a_failure() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo {
            is_updating: true,
            ..UpdatesInfo::default()
        };

        let _ = updates.update(
            Message::UpdatePackagesResult(update_outcome(
                true,
                Some("pkexec authentication was dismissed"),
            )),
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        let notice = info.last_operation_notice.as_ref().expect("a notice");
        assert!(notice.is_stopped());
        assert!(info.failed_update_manager.is_none());
        let headline = shared::operation_notice_headline(notice, &ManagerCatalog::builtin());
        assert_eq!(headline, "Update stopped after 1 of 2 packages");
        assert!(!headline.to_lowercase().contains("fail"));
    }

    #[test]
    fn failed_update_outcome_names_the_manager_and_offers_retry() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo {
            is_updating: true,
            ..UpdatesInfo::default()
        };

        let _ = updates.update(
            Message::UpdatePackagesResult(update_outcome(
                false,
                Some("Failed to update packages from builtin:apt: package manager command failed"),
            )),
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        let notice = info.last_operation_notice.as_ref().expect("a notice");
        assert!(!notice.is_stopped());
        assert_eq!(
            shared::operation_notice_headline(notice, &ManagerCatalog::builtin()),
            "APT: update failed"
        );
        assert!(info.failed_update_manager.is_some());
    }

    #[test]
    fn open_managers_navigates_to_the_managers_page() {
        let mut updates = Updates::default();
        let mut info = UpdatesInfo::default();

        let action = updates.update(
            Message::OpenManagers,
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        assert!(matches!(
            action,
            Action::Navigate(crate::content::ActiveContentPage::Health)
        ));
    }
}
