// Updates view with filtering, sorting, and search capabilities.

use std::collections::{HashMap, HashSet};

use iced::Task;
use updater_core::{PackageManagerType, PackageUpdate};

use crate::{
    content::errors::{ManagerErrors, apply_manager_counted_items_result},
    content::shared::{PackageSelectionKey, SharedUi},
    content::workflows::{
        BatchProgress, CancellationToken, OperationOutcome, PackageBatchAction,
        collect_selected_package_groups, push_command_log, run_grouped_package_action,
    },
    theme,
};

#[derive(Debug, Clone, Default)]
pub struct Updates {
    /// Search text for filtering updates in UI.
    search_query: String,
    /// Update currently shown in the details inspector.
    inspected_package: Option<PackageSelectionKey>,
    /// Last inspector action error shown in UI.
    inspector_error: Option<String>,
    /// Sources still refreshing for an Update All preflight.
    update_all_refreshing: HashSet<PackageManagerType>,
    /// Full source scope used to build the current preflight plan.
    update_all_scope: HashSet<PackageManagerType>,
    /// Frozen Update All plan waiting for confirmation.
    pending_update_all: Option<UpdatePlan>,
}

#[derive(Debug, Clone)]
struct UpdatePlan {
    manager_groups: Vec<(PackageManagerType, Vec<String>)>,
    failed_sources: Vec<PackageManagerType>,
}

impl UpdatePlan {
    fn package_count(&self) -> usize {
        self.manager_groups
            .iter()
            .map(|(_, packages)| packages.len())
            .sum()
    }

    fn manager_count(&self) -> usize {
        self.manager_groups.len()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Package-manager selection message.
    SelectPackageManager(PackageManagerType, bool),
    /// Updates-load result message.
    LoadUpdatesResult(PackageManagerType, Result<Vec<PackageUpdate>, String>),
    /// Search-query change message.
    SearchQueryChanged(String),
    /// Sort-option change message.
    SortOptionChanged(SortOption),
    /// Package-selection toggle message.
    TogglePackageSelection(PackageManagerType, String, bool),
    /// Select-all toggle message.
    ToggleSelectAll(bool),
    /// Update-selected message.
    UpdateSelectedPackages,
    /// Update progress message.
    UpdateProgress {
        /// Number of finished packages.
        completed: usize,
        /// Total packages to update.
        total: usize,
        /// Manager currently executing command.
        manager: PackageManagerType,
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
    RetryLoad(PackageManagerType),
    /// Show an update in the package inspector.
    InspectPackage(PackageManagerType, String),
    /// Copy text from the inspector.
    CopyInspectorText(String),
    /// Refresh all sources and prepare an Update All plan.
    PrepareUpdateAll,
    /// Execute the frozen Update All plan.
    ConfirmUpdateAll,
    /// Dismiss the Update All confirmation.
    CancelUpdateAll,
    /// Re-scan the failed update source before retrying.
    PrepareFailedUpdateRetry,
}

#[derive(Debug, Clone, Default)]
pub struct UpdatesInfo {
    /// Updates cache by manager `(count, updates)`.
    pub updates_by_manager: HashMap<PackageManagerType, (usize, Vec<PackageUpdate>)>,
    /// Initial update-loading failures grouped by manager.
    pub init_errors: ManagerErrors,
    /// Full update-list loading failures grouped by manager.
    pub load_errors: ManagerErrors,
    /// Managers selected in the filter panel.
    pub selected_managers: HashSet<PackageManagerType>,
    /// Managers currently loading update list.
    pub loading_updates: HashSet<PackageManagerType>,
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
    pub update_progress: Option<(usize, usize, PackageManagerType, String)>,
    /// Update command logs.
    pub update_logs: Vec<String>,
    /// Last update error shown in UI.
    pub last_update_error: Option<String>,
    /// Source that failed during the most recent update operation.
    pub failed_update_manager: Option<PackageManagerType>,
}

pub enum Action {
    /// No-op action.
    None,
    /// Asynchronous task action.
    Run(iced::Task<Message>),
    /// Cooperative package operation task.
    CancellableRun(iced::Task<Message>, CancellationToken),
    /// Complete a package operation and optionally reload package data.
    PackageOperationFinished {
        outcome: OperationOutcome,
        reload: bool,
    },
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
    ) -> Action {
        match message {
            Message::SelectPackageManager(pm_type, selected) => {
                if selected {
                    // Managers still in init phase are not selectable yet.
                    if info.is_loading_count && !info.updates_by_manager.contains_key(&pm_type) {
                        return Action::None;
                    }

                    info.selected_managers.insert(pm_type);

                    if info.init_errors.contains_key(&pm_type)
                        || info.load_errors.contains_key(&pm_type)
                    {
                        info.init_errors.remove(&pm_type);
                        info.load_errors.remove(&pm_type);
                        info.loading_updates.insert(pm_type);
                        Action::Run(Self::create_load_task(pm_config, pm_type, true))
                    } else if info.loading_updates.contains(&pm_type) {
                        Action::None
                    } else if let Some((count, packages)) = info.updates_by_manager.get(&pm_type) {
                        if *count == packages.len() {
                            Action::None
                        } else {
                            info.loading_updates.insert(pm_type);
                            Action::Run(Self::create_load_task(pm_config, pm_type, false))
                        }
                    } else {
                        info.loading_updates.insert(pm_type);
                        Action::Run(Self::create_load_task(pm_config, pm_type, false))
                    }
                } else {
                    info.selected_managers.remove(&pm_type);
                    info.selected_packages
                        .retain(|(manager, _)| *manager != pm_type);
                    Action::None
                }
            }
            Message::LoadUpdatesResult(pm_type, result) => {
                info.loading_updates.remove(&pm_type);
                apply_manager_counted_items_result(
                    &mut info.updates_by_manager,
                    &mut info.load_errors,
                    pm_type,
                    result,
                );

                if self.update_all_refreshing.remove(&pm_type)
                    && self.update_all_refreshing.is_empty()
                {
                    self.pending_update_all =
                        Some(Self::build_update_plan(info, &self.update_all_scope));
                }
                Action::None
            }
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
                Action::None
            }
            Message::InspectPackage(pm_type, package_name) => {
                self.inspected_package = Some(SharedUi::selection_key(pm_type, &package_name));
                self.inspector_error = None;
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
                if info.is_updating {
                    return Action::None;
                }
                let key = SharedUi::selection_key(pm_type, &package_name);
                if selected {
                    info.selected_packages.insert(key);
                } else {
                    info.selected_packages.remove(&key);
                }
                Action::None
            }
            Message::ToggleSelectAll(select_all) => {
                if info.is_updating {
                    return Action::None;
                }

                let query = self.search_query.trim().to_lowercase();
                for pm_type in &info.selected_managers {
                    if let Some((_, packages)) = info.updates_by_manager.get(pm_type) {
                        for pkg in packages {
                            if !query.is_empty() && !pkg.name.to_lowercase().contains(&query) {
                                continue;
                            }

                            let key = SharedUi::selection_key(*pm_type, &pkg.name);
                            if select_all {
                                info.selected_packages.insert(key);
                            } else {
                                info.selected_packages.remove(&key);
                            }
                        }
                    }
                }
                Action::None
            }
            Message::UpdateSelectedPackages => {
                if info.selected_packages.is_empty()
                    || info.is_updating
                    || !self.update_all_refreshing.is_empty()
                {
                    return Action::None;
                }
                info.is_updating = true;
                info.last_update_error = None;
                info.failed_update_manager = None;
                info.update_logs.clear();
                let initial_manager = info
                    .selected_packages
                    .iter()
                    .next()
                    .map(|(pm_type, _)| *pm_type)
                    .unwrap_or(PackageManagerType::Dnf);
                info.update_progress = Some((
                    0,
                    info.selected_packages.len(),
                    initial_manager,
                    String::new(),
                ));
                Self::update_packages_action(pm_config, info)
            }
            Message::UpdateProgress {
                completed,
                total,
                manager,
                current_package,
                command_message,
            } => {
                info.update_progress = Some((completed, total, manager, current_package));
                if let Some(command_message) = command_message {
                    push_command_log(
                        &mut info.update_logs,
                        PackageBatchAction::Update,
                        manager,
                        info.update_progress
                            .as_ref()
                            .map_or("", |(_, _, _, package)| package.as_str()),
                        command_message,
                    );
                }
                Action::None
            }
            Message::UpdatePackagesResult(outcome) => {
                self.pending_update_all = None;
                self.update_all_scope.clear();
                self.update_all_refreshing.clear();
                info.is_updating = false;
                info.update_progress = None;
                if outcome.is_success() {
                    info.selected_packages.clear();
                    info.last_update_error = None;
                    info.failed_update_manager = None;
                    Action::PackageOperationFinished {
                        outcome,
                        reload: true,
                    }
                } else {
                    info.failed_update_manager = outcome.failed_manager;
                    let error = outcome.error.clone().unwrap_or_else(|| outcome.summary());
                    log::error!("Failed to update packages: {}", error);
                    info.last_update_error = Some(error);
                    Action::PackageOperationFinished {
                        outcome,
                        reload: false,
                    }
                }
            }
            Message::RefreshSelected => {
                if info.is_updating || !self.update_all_refreshing.is_empty() {
                    return Action::None;
                }
                let pm_types: Vec<PackageManagerType> =
                    info.selected_managers.iter().copied().collect();

                if pm_types.is_empty() {
                    return Action::None;
                }

                // Mark selected managers as loading.
                for pm_type in &pm_types {
                    info.loading_updates.insert(*pm_type);
                }

                // Create load tasks for selected managers.
                let tasks: Vec<Task<Message>> = pm_types
                    .into_iter()
                    .map(|pm_type| Self::create_load_task(pm_config, pm_type, true))
                    .collect();

                Action::Run(Task::batch(tasks))
            }
            Message::RefreshAll => {
                if info.is_updating || !self.update_all_refreshing.is_empty() {
                    return Action::None;
                }
                let pm_types = SharedUi::configured_managers(pm_config);

                if pm_types.is_empty() {
                    return Action::None;
                }

                for pm_type in &pm_types {
                    info.loading_updates.insert(*pm_type);
                }

                let tasks: Vec<Task<Message>> = pm_types
                    .into_iter()
                    .map(|pm_type| Self::create_load_task(pm_config, pm_type, true))
                    .collect();

                Action::Run(Task::batch(tasks))
            }
            Message::RetryLoad(pm_type) => {
                if info.is_updating
                    || !self.update_all_refreshing.is_empty()
                    || info.loading_updates.contains(&pm_type)
                {
                    return Action::None;
                }
                info.init_errors.remove(&pm_type);
                info.load_errors.remove(&pm_type);
                info.loading_updates.insert(pm_type);
                Action::Run(Self::create_load_task(pm_config, pm_type, true))
            }
            Message::PrepareUpdateAll => {
                if info.is_updating
                    || !self.update_all_refreshing.is_empty()
                    || !info.loading_updates.is_empty()
                {
                    return Action::None;
                }
                let managers = SharedUi::configured_managers(pm_config);
                if managers.is_empty() {
                    return Action::None;
                }

                self.pending_update_all = None;
                self.update_all_scope = managers.iter().copied().collect();
                self.update_all_refreshing = self.update_all_scope.clone();
                for manager in &managers {
                    info.init_errors.remove(manager);
                    info.load_errors.remove(manager);
                    info.loading_updates.insert(*manager);
                }

                Action::Run(Task::batch(
                    managers
                        .into_iter()
                        .map(|manager| Self::create_load_task(pm_config, manager, true)),
                ))
            }
            Message::ConfirmUpdateAll => {
                let Some(plan) = self.pending_update_all.take() else {
                    return Action::None;
                };
                if plan.manager_groups.is_empty() || info.is_updating {
                    return Action::None;
                }

                let total = plan.package_count();
                let initial_manager = plan.manager_groups[0].0;
                info.is_updating = true;
                info.last_update_error = None;
                info.update_logs.clear();
                info.update_progress = Some((0, total, initial_manager, String::new()));
                Self::update_plan_action(pm_config, plan.manager_groups)
            }
            Message::CancelUpdateAll => {
                self.pending_update_all = None;
                self.update_all_scope.clear();
                Action::None
            }
            Message::PrepareFailedUpdateRetry => {
                if info.is_updating
                    || !self.update_all_refreshing.is_empty()
                    || !info.loading_updates.is_empty()
                {
                    return Action::None;
                }
                let Some(manager) = info.failed_update_manager.take() else {
                    return Action::None;
                };

                self.pending_update_all = None;
                self.update_all_scope = HashSet::from([manager]);
                self.update_all_refreshing = self.update_all_scope.clone();
                info.init_errors.remove(&manager);
                info.load_errors.remove(&manager);
                info.loading_updates.insert(manager);
                Action::Run(Self::create_load_task(pm_config, manager, true))
            }
        }
    }

    pub fn has_inspector_selection(&self) -> bool {
        self.inspected_package.is_some()
    }

    pub fn dismiss_transient(&mut self) -> bool {
        if self.pending_update_all.take().is_some() {
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
        if self.pending_update_all.is_some() {
            return self
                .pending_update_all
                .as_ref()
                .is_some_and(|plan| plan.package_count() > 0 && !info.is_updating)
                .then_some(Message::ConfirmUpdateAll);
        }
        (!info.is_updating
            && self.update_all_refreshing.is_empty()
            && !info.selected_packages.is_empty())
        .then_some(Message::UpdateSelectedPackages)
    }

    pub fn can_select_packages(&self) -> bool {
        self.update_all_refreshing.is_empty()
    }

    pub fn move_keyboard_selection(
        &self,
        info: &UpdatesInfo,
        direction: crate::shortcut::SelectionDirection,
    ) -> Option<Message> {
        crate::content::shared::next_keyboard_package(
            &self.keyboard_packages(info),
            self.inspected_package.as_ref(),
            direction,
        )
        .map(|(manager, name)| Message::InspectPackage(manager, name))
    }

    pub fn toggle_keyboard_selection(&self, info: &UpdatesInfo) -> Option<Message> {
        let (manager, name) = self.inspected_package.as_ref()?;
        if info.is_updating || !self.update_all_refreshing.is_empty() {
            return None;
        }
        let selected = !info
            .selected_packages
            .contains(&SharedUi::selection_key(*manager, name));
        Some(Message::TogglePackageSelection(
            *manager,
            name.clone(),
            selected,
        ))
    }

    fn keyboard_packages(&self, info: &UpdatesInfo) -> Vec<PackageSelectionKey> {
        let query = self.search_query.trim().to_lowercase();
        let mut managers: Vec<_> = info.selected_managers.iter().copied().collect();
        managers.sort_by_key(|manager| manager.name());
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
                        query.is_empty() || package.name.to_lowercase().contains(&query)
                    })
                    .map(move |package| (manager, package.name.clone()))
            })
            .collect()
    }

    pub fn view<'a>(
        &'a self,
        info: &'a UpdatesInfo,
        pm_config: &updater_core::Config,
        show_inspector: bool,
        inspector_drawer: bool,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, container, row};

        let update_count: usize = info
            .updates_by_manager
            .values()
            .map(|(count, _)| *count)
            .sum();
        let configured_managers = SharedUi::configured_managers(pm_config).len();

        let toolbar = SharedUi::toolbar(
            column![
                row![
                    container(self.search_input_view()).width(iced::Length::Fill),
                    column![
                        SharedUi::section_title("Actions"),
                        self.refresh_actions_view()
                    ]
                    .spacing(theme::spacing::SM),
                ]
                .spacing(theme::spacing::MD)
                .align_y(iced::Alignment::End),
                row![
                    container(self.manager_filter_view(info, pm_config))
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

        column![
            SharedUi::page_header(
                "Updates",
                format!(
                    "{update_count} available updates across {configured_managers} configured managers"
                ),
                theme::colors::UPDATES,
            ),
            SharedUi::summary_row(summary_items),
            toolbar,
            self.batch_actions_view(info),
            self.update_all_confirmation_view(),
            self.updates_list_view(info, show_inspector, inspector_drawer),
        ]
        .spacing(theme::spacing::LG)
        .height(iced::Length::Fill)
        .into()
    }

    // View components.

    fn manager_filter_view<'a>(
        &self,
        info: &'a UpdatesInfo,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'a, Message> {
        let filters_content = if !info.has_loading_count {
            SharedUi::loading_manager_filter_view(
                pm_config,
                if info.is_loading_count {
                    "Loading update information..."
                } else {
                    "Waiting to load update information"
                },
            )
        } else {
            let managers = SharedUi::configured_managers(pm_config);

            if managers.is_empty() {
                return iced::widget::column![
                    SharedUi::section_title("Sources"),
                    SharedUi::empty_filter_view("No package managers detected")
                ]
                .spacing(theme::spacing::SM)
                .into();
            }

            let entries = managers
                .iter()
                .map(|pm_type| {
                    let count = info
                        .updates_by_manager
                        .get(pm_type)
                        .map_or(0, |(count, _)| *count);
                    (*pm_type, count)
                })
                .collect();

            SharedUi::active_manager_filter_view(
                entries,
                &info.selected_managers,
                &info.loading_updates,
                move |pm_type| {
                    info.is_loading_count && !info.updates_by_manager.contains_key(&pm_type)
                },
                Message::SelectPackageManager,
            )
        };

        let mut content = iced::widget::column![SharedUi::section_title("Sources")];
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

    fn refresh_actions_view<'a>(&self) -> iced::Element<'a, Message> {
        use iced::widget::row;

        row![
            SharedUi::refresh_button_with_label("Refresh Selected", Message::RefreshSelected),
            SharedUi::refresh_button_with_label("Refresh All", Message::RefreshAll),
        ]
        .spacing(8)
        .into()
    }

    fn sort_order_view<'a>(&self, info: &'a UpdatesInfo) -> iced::Element<'a, Message> {
        use iced::widget::{column, row};

        let sort_options = row(SortOption::ALL.iter().map(|option| {
            let option = *option;
            SharedUi::segmented_button(
                option.name(),
                option == info.sort_by,
                Message::SortOptionChanged(option),
            )
            .into()
        }))
        .spacing(2)
        .width(iced::Length::Fill);

        column![
            SharedUi::section_title("Sort"),
            SharedUi::segmented_group(sort_options)
        ]
        .spacing(theme::spacing::SM)
        .into()
    }

    fn search_input_view<'a>(&self) -> iced::Element<'a, Message> {
        SharedUi::search_input_view(
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
        show_inspector: bool,
        inspector_drawer: bool,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, container, row, scrollable};

        if !info.has_loading_count {
            return SharedUi::centered_message(if info.is_loading_count {
                "Loading update information..."
            } else {
                "Waiting to load update information"
            });
        }

        if info.selected_managers.is_empty() {
            return SharedUi::centered_message("Please select a package manager to view");
        }

        if info
            .selected_managers
            .iter()
            .any(|pm_type| info.loading_updates.contains(pm_type))
        {
            return SharedUi::centered_message("Loading selected package manager updates...");
        }

        let filtered_managers: Vec<_> = info
            .selected_managers
            .iter()
            .filter_map(|pm_type| {
                info.updates_by_manager
                    .get(pm_type)
                    .map(|entry| (*pm_type, entry))
            })
            .collect();

        let total_updates: usize = filtered_managers.iter().map(|(_, (count, _))| *count).sum();
        let has_visible_errors = filtered_managers
            .iter()
            .any(|(pm_type, _)| info.load_errors.contains_key(pm_type));

        if total_updates == 0 && !has_visible_errors {
            return SharedUi::centered_message("No updates available");
        }

        let search_query = self.search_query.trim().to_lowercase();
        if !search_query.is_empty() {
            let has_any_match = filtered_managers.iter().any(|(_, (_, packages))| {
                packages
                    .iter()
                    .any(|pkg| pkg.name.to_lowercase().contains(&search_query))
            });

            if !has_any_match && !has_visible_errors {
                return SharedUi::centered_message("No updates match your search");
            }
        }

        let updates_sections: Vec<iced::Element<'_, Message>> = filtered_managers
            .into_iter()
            .map(|(pm_type, (count, packages))| {
                self.package_manager_section(pm_type, *count, packages, info)
            })
            .collect();

        let update_list = scrollable(column(updates_sections).spacing(20))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill);
        let inspected = self.inspected_package.as_ref().and_then(|(manager, name)| {
            info.updates_by_manager
                .get(manager)
                .and_then(|(_, packages)| packages.iter().find(|package| package.name == *name))
                .map(|package| crate::content::shared::PackageInspector {
                    manager: *manager,
                    name: &package.name,
                    version: &package.current_version,
                    available_version: Some(&package.new_version),
                    description: None,
                    size: None,
                    install_date: None,
                    homepage: None,
                })
        });
        let inspector = SharedUi::package_inspector(
            inspected,
            Message::CopyInspectorText,
            Message::CopyInspectorText,
            Message::CopyInspectorText,
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
        pm_type: PackageManagerType,
        count: usize,
        packages: &'a [PackageUpdate],
        info: &'a UpdatesInfo,
    ) -> iced::Element<'a, Message> {
        let is_loading = info.loading_updates.contains(&pm_type);
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
                    .map(|pkg| self.package_item_view(pm_type, pkg, info)),
            )
            .spacing(8)
            .into()
        });

        SharedUi::manager_section(
            pm_type,
            subtitle,
            theme::colors::UPDATES,
            "Failed to load updates",
            info.load_errors
                .get(&pm_type)
                .or_else(|| info.init_errors.get(&pm_type))
                .map(String::as_str),
            || Message::RetryLoad(pm_type),
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
                    pkg.name.to_lowercase().contains(&query)
                }
            })
            .collect();

        match sort_by {
            SortOption::Name => {
                filtered.sort_by(|a, b| a.name.cmp(&b.name));
            }
            SortOption::CurrentVersion => {
                filtered.sort_by(|a, b| a.current_version.cmp(&b.current_version));
            }
            SortOption::NewVersion => {
                filtered.sort_by(|a, b| a.new_version.cmp(&b.new_version));
            }
        }

        filtered
    }

    fn package_item_view<'a>(
        &self,
        pm_type: PackageManagerType,
        package: &'a PackageUpdate,
        info: &'a UpdatesInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, checkbox, column, row, text};

        let package_name = package.name.clone();
        let is_selected = info
            .selected_packages
            .contains(&SharedUi::selection_key(pm_type, &package.name));

        let package_checkbox = checkbox(is_selected)
            .on_toggle_maybe((!info.is_updating).then_some({
                let package_name = package_name.clone();
                move |selected| {
                    Message::TogglePackageSelection(pm_type, package_name.clone(), selected)
                }
            }))
            .size(18)
            .spacing(8)
            .style(SharedUi::checkbox_style(false));

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
                text(&package.new_version)
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

        let is_inspected = self
            .inspected_package
            .as_ref()
            .is_some_and(|(manager, name)| *manager == pm_type && name == &package.name);
        let details = button(
            column![
                text(&package.name)
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
        .on_press(Message::InspectPackage(pm_type, package_name));

        row![package_checkbox, details]
            .spacing(theme::spacing::SM)
            .align_y(iced::Alignment::Center)
            .into()
    }

    fn update_all_confirmation_view<'a>(&self) -> iced::Element<'a, Message> {
        use iced::widget::{button, column, container, row, text};

        let Some(plan) = &self.pending_update_all else {
            return container("").height(iced::Length::Shrink).into();
        };

        let package_count = plan.package_count();
        let manager_count = plan.manager_count();
        let failed_count = plan.failed_sources.len();
        let failed_names = plan
            .failed_sources
            .iter()
            .map(PackageManagerType::name)
            .collect::<Vec<_>>()
            .join(", ");
        let detail = if package_count == 0 && failed_count == 0 {
            "No updates were found after refreshing all configured sources.".to_owned()
        } else if failed_count == 0 {
            format!(
                "{package_count} package(s) from {manager_count} source(s). System sources may request authorization."
            )
        } else {
            format!(
                "{package_count} package(s) from {manager_count} source(s). Excluded failed source(s): {failed_names}."
            )
        };
        let title = if package_count == 0 {
            "No updates found"
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
            confirm.on_press(Message::ConfirmUpdateAll)
        } else {
            confirm
        };

        container(
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
                    .on_press(Message::CancelUpdateAll),
                confirm,
            ]
            .spacing(theme::spacing::MD)
            .align_y(iced::Alignment::Center),
        )
        .padding(theme::spacing::MD)
        .width(iced::Length::Fill)
        .style(theme::surface_container)
        .into()
    }

    fn batch_actions_view<'a>(&self, info: &'a UpdatesInfo) -> iced::Element<'a, Message> {
        use iced::widget::{button, checkbox, column, row, text};

        let selected_count = info.selected_packages.len();
        let is_preparing_all = !self.update_all_refreshing.is_empty();
        let is_enabled = selected_count > 0 && !info.is_updating && !is_preparing_all;

        let query = self.search_query.trim().to_lowercase();
        let mut total_visible = 0;
        let mut selected_visible = 0;
        for pm_type in &info.selected_managers {
            if let Some((_, packages)) = info.updates_by_manager.get(pm_type) {
                for package in packages {
                    if !query.is_empty() && !package.name.to_lowercase().contains(&query) {
                        continue;
                    }
                    total_visible += 1;
                    if info
                        .selected_packages
                        .contains(&SharedUi::selection_key(*pm_type, &package.name))
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
                        manager.name()
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
            .on_toggle_maybe((!info.is_updating).then_some(Message::ToggleSelectAll))
            .size(18)
            .spacing(8)
            .text_size(14)
            .style(SharedUi::checkbox_style(false));

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
            update_button.on_press(Message::UpdateSelectedPackages)
        } else {
            update_button
        };

        let update_all_enabled = !info.is_updating && !is_preparing_all;
        let update_all = button(
            text(if is_preparing_all {
                "Preparing Update All..."
            } else {
                "Update All Available"
            })
            .size(13)
            .font(theme::FONT_SEMIBOLD),
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
            let retry = info.failed_update_manager.map(|_| {
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

    fn create_load_task(
        pm_config: &updater_core::Config,
        pm_type: PackageManagerType,
        force_refresh: bool,
    ) -> Task<Message> {
        let pm_config = pm_config.clone();

        Task::future(async move {
            pm_type
                .list_updates_with_refresh(&pm_config, force_refresh)
                .await
                .map_err(|e| format!("Failed to load updates for {}: {}", pm_type.name(), e))
        })
        .then(move |result| Task::done(Message::LoadUpdatesResult(pm_type, result)))
    }

    fn build_update_plan(info: &UpdatesInfo, scope: &HashSet<PackageManagerType>) -> UpdatePlan {
        let mut manager_groups: Vec<_> = info
            .updates_by_manager
            .iter()
            .filter(|(manager, _)| scope.contains(manager))
            .filter(|(manager, _)| {
                !info.init_errors.contains_key(manager) && !info.load_errors.contains_key(manager)
            })
            .filter(|(_, (_, packages))| !packages.is_empty())
            .map(|(manager, (_, packages))| {
                let mut names: Vec<_> = packages
                    .iter()
                    .map(|package| package.name.clone())
                    .collect();
                names.sort();
                (*manager, names)
            })
            .collect();
        manager_groups.sort_by_key(|(manager, _)| manager.name());

        let mut failed_sources: Vec<_> = info
            .init_errors
            .keys()
            .chain(info.load_errors.keys())
            .filter(|manager| scope.contains(manager))
            .copied()
            .collect();
        failed_sources.sort_by_key(|manager| manager.name());
        failed_sources.dedup();

        UpdatePlan {
            manager_groups,
            failed_sources,
        }
    }

    fn update_plan_action(
        pm_config: &updater_core::Config,
        manager_groups: Vec<(PackageManagerType, Vec<String>)>,
    ) -> Action {
        let cancellation = CancellationToken::default();
        let task = run_grouped_package_action(
            pm_config,
            PackageBatchAction::Update,
            manager_groups,
            cancellation.clone(),
            |BatchProgress {
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

    fn update_packages_action(pm_config: &updater_core::Config, info: &UpdatesInfo) -> Action {
        let manager_groups = collect_selected_package_groups(
            info.selected_managers.iter().filter_map(|pm_type| {
                info.updates_by_manager
                    .get(pm_type)
                    .map(|(_, packages)| (*pm_type, packages.as_slice()))
            }),
            &info.selected_packages,
            |package| package.name.as_str(),
        );

        Self::update_plan_action(pm_config, manager_groups)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(name: &str) -> PackageUpdate {
        PackageUpdate {
            name: name.to_owned(),
            current_version: "1.0".to_owned(),
            new_version: "2.0".to_owned(),
        }
    }

    #[test]
    fn update_all_plan_excludes_failed_sources() {
        let mut info = UpdatesInfo::default();
        info.updates_by_manager.insert(
            PackageManagerType::Dnf,
            (2, vec![update("alpha"), update("beta")]),
        );
        info.updates_by_manager
            .insert(PackageManagerType::Flatpak, (1, vec![update("gamma")]));
        info.load_errors
            .insert(PackageManagerType::Flatpak, "network error".to_owned());
        let scope = HashSet::from([PackageManagerType::Dnf, PackageManagerType::Flatpak]);

        let plan = Updates::build_update_plan(&info, &scope);

        assert_eq!(plan.package_count(), 2);
        assert_eq!(plan.manager_count(), 1);
        assert_eq!(plan.manager_groups[0].0, PackageManagerType::Dnf);
        assert_eq!(plan.failed_sources, vec![PackageManagerType::Flatpak]);
    }

    #[test]
    fn failed_source_retry_plan_does_not_repeat_successful_sources() {
        let mut info = UpdatesInfo::default();
        info.updates_by_manager
            .insert(PackageManagerType::Dnf, (1, vec![update("already-done")]));
        info.updates_by_manager
            .insert(PackageManagerType::Flatpak, (1, vec![update("retry-me")]));
        let scope = HashSet::from([PackageManagerType::Flatpak]);

        let plan = Updates::build_update_plan(&info, &scope);

        assert_eq!(plan.package_count(), 1);
        assert_eq!(plan.manager_count(), 1);
        assert_eq!(plan.manager_groups[0].0, PackageManagerType::Flatpak);
        assert_eq!(plan.manager_groups[0].1, vec!["retry-me"]);
    }
}
