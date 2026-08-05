// Updates view with filtering, sorting, and search capabilities.

use std::collections::{HashMap, HashSet};

use iced::Task;
use updater_core::{CancellationToken, OperationOutcome, OperationProgress};
use updater_manager_api::{
    ManagerCapability, ManagerId, PackageAction, PackageInfo, PackageTarget, PackageUpdate,
};

use crate::{
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
    /// Last update error shown in UI.
    pub last_update_error: Option<String>,
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
                info.last_update_error = None;
                info.failed_update_manager = None;
                let manager_groups = collect_selected_package_groups(
                    info.selected_managers.iter().filter_map(|manager| {
                        info.updates_by_manager
                            .get(manager)
                            .map(|(_, packages)| (manager.clone(), packages.as_slice()))
                    }),
                    &info.selected_packages,
                    catalog,
                    |package| package.target.clone(),
                );
                if manager_groups.is_empty() {
                    info.last_update_error =
                        Some("Selected packages are no longer available to update".to_owned());
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
                if outcome.is_success() {
                    info.selected_packages.clear();
                    info.last_update_error = None;
                    info.failed_update_manager = None;
                    Action::PackageOperationFinished { outcome }
                } else {
                    info.failed_update_manager = outcome.failed_manager.clone();
                    let error = outcome.error.clone().unwrap_or_else(|| outcome.summary());
                    log::error!("Failed to update packages: {}", error);
                    info.last_update_error = Some(error);
                    Action::PackageOperationFinished { outcome }
                }
            }
            Message::RefreshSelected => {
                if info.is_updating
                    || self.pending_update.is_some()
                    || !self.update_all_refreshing.is_empty()
                    || info.is_loading_count
                    || !info.loading_updates.is_empty()
                {
                    return Action::None;
                }
                let managers: Vec<ManagerId> = info.selected_managers.iter().cloned().collect();

                if managers.is_empty() {
                    return Action::None;
                }

                let tasks: Vec<Task<Message>> = managers
                    .into_iter()
                    .map(|manager| Self::start_load(pm_config, info, manager, catalog, true))
                    .collect();

                Action::Run(Task::batch(tasks))
            }
            Message::RefreshAll => {
                if info.is_updating
                    || self.pending_update.is_some()
                    || !self.update_all_refreshing.is_empty()
                    || info.is_loading_count
                    || !info.loading_updates.is_empty()
                {
                    return Action::None;
                }
                let pm_types = shared::configured_managers_with_capability(
                    pm_config,
                    catalog,
                    ManagerCapability::Updates,
                );

                if pm_types.is_empty() {
                    return Action::None;
                }

                let tasks: Vec<Task<Message>> = pm_types
                    .into_iter()
                    .map(|manager| Self::start_load(pm_config, info, manager, catalog, true))
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
                Action::Run(Self::start_load(pm_config, info, pm_type, catalog, true))
            }
            Message::PrepareUpdateAll => {
                if info.is_updating
                    || self.pending_update.is_some()
                    || !self.update_all_refreshing.is_empty()
                    || info.is_loading_count
                    || !info.loading_updates.is_empty()
                {
                    return Action::None;
                }
                let managers = shared::configured_managers_with_capability(
                    pm_config,
                    catalog,
                    ManagerCapability::Updates,
                );
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
                    Self::start_load(pm_config, info, manager, catalog, true)
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
                    info.last_update_error =
                        Some("The update plan does not contain any packages".to_owned());
                    return Action::None;
                };

                let total = plan.package_count();
                info.is_updating = true;
                info.last_update_error = None;
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
                Action::Run(Self::start_load(pm_config, info, manager, catalog, true))
            }
        }
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
            Action::Run(Self::start_load(pm_config, info, manager, catalog, true))
        } else if info.loading_updates.contains_key(&manager) {
            Action::None
        } else if let Some((count, packages)) = info.updates_by_manager.get(&manager) {
            if *count == packages.len() {
                Action::None
            } else {
                Action::Run(Self::start_load(pm_config, info, manager, catalog, false))
            }
        } else {
            Action::Run(Self::start_load(pm_config, info, manager, catalog, false))
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

        let update_count: usize = info
            .updates_by_manager
            .values()
            .map(|(count, _)| *count)
            .sum();
        let configured_managers = shared::configured_managers_with_capability(
            pm_config,
            catalog,
            ManagerCapability::Updates,
        )
        .len();
        let selected_loading_sources = info.selected_loading_sources();
        let can_refresh = !info.is_updating
            && self.pending_update.is_none()
            && self.update_all_refreshing.is_empty()
            && !info.is_loading_count
            && info.loading_updates.is_empty();

        let toolbar = shared::toolbar(
            column![
                row![
                    container(self.search_input_view()).width(iced::Length::FillPortion(2)),
                    column![
                        shared::section_title("Actions"),
                        row![
                            shared::refresh_button_with_label(
                                "Refresh Selected",
                                can_refresh,
                                Message::RefreshSelected
                            ),
                            shared::refresh_button_with_label(
                                "Refresh All",
                                can_refresh,
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
            self.batch_actions_view(info, catalog),
            self.update_confirmation_view(catalog),
            self.updates_list_view(
                info,
                installed_info,
                catalog,
                show_inspector,
                inspector_drawer,
                selected_loading_sources,
            ),
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
            return shared::centered_message("Please select a package manager to view");
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

        if filtered_managers.is_empty() && selected_loading_sources > 0 {
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
            updates_sections.push(
                iced::widget::text(format!(
                    "Loading {selected_loading_sources} remaining selected source{}...",
                    if selected_loading_sources == 1 {
                        ""
                    } else {
                        "s"
                    }
                ))
                .size(13)
                .style(theme::text_accent)
                .into(),
            );
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
        let subtitle = if is_loading {
            "(Loading...)".to_owned()
        } else {
            format!("({} updates)", count)
        };

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
            body,
        )
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
            && !info.is_loading_count
            && info.loading_updates.is_empty();
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

        if let Some(error) = &info.last_update_error {
            let retry = info.failed_update_manager.as_ref().map(|_| {
                button(
                    text("Re-scan Failed Source")
                        .size(13)
                        .font(theme::FONT_SEMIBOLD),
                )
                .padding([7, 12])
                .style(theme::secondary_button(true))
                .on_press(Message::PrepareFailedUpdateRetry)
            });
            let mut error_row = row![
                text(format!("Update failed: {}", error))
                    .size(13)
                    .style(theme::text_error)
                    .width(iced::Length::Fill)
            ]
            .align_y(iced::Alignment::Center);
            if let Some(retry) = retry {
                error_row = error_row.push(retry);
            }

            column![actions_row, error_row].spacing(8).into()
        } else {
            actions_row.into()
        }
    }

    pub(crate) fn start_load(
        pm_config: &updater_core::Config,
        info: &mut UpdatesInfo,
        manager: ManagerId,
        catalog: &ManagerCatalog,
        force_refresh: bool,
    ) -> Task<Message> {
        info.request_generation = info.request_generation.wrapping_add(1);
        let request_id = info.request_generation;
        info.loading_updates.insert(manager.clone(), request_id);

        let pm_config = pm_config.clone();
        let manager_name = catalog.display_name(&manager).to_owned();
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
                .updates(manager_config, force_refresh)
                .await
                .map_err(|e| format!("Failed to load updates for {manager_name}: {e}"))
        })
        .then(move |result| {
            Task::done(Message::LoadUpdatesResult {
                request_id,
                manager: result_manager.clone(),
                result,
            })
        })
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
            info.last_update_error.as_deref(),
            Some("Selected packages are no longer available to update")
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
}
