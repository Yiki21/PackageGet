// Installed packages view with filtering and sorting capabilities
//
// Layout structure:
// ┌─────────────────────────────────────┐
// │ Left Sidebar       │  Main Area     │
// │ □ All (194)        │  ┌─ DNF ──┐    │
// │ □ DNF (123)        │  │ pkg1   │    │
// │ □ Flatpak (15)     │  │ pkg2   │    │
// │ □ Homebrew(45)     │  └────────┘    │
// │ □ Cargo (8)        │  ┌─Flatpak┐    │
// │ □ Go (3)           │  │ app1   │    │
// │ Sort by: [Name]    │  └────────┘    │
// └─────────────────────────────────────┘

use std::collections::{HashMap, HashSet};

use iced::Task;
use updater_core::{CancellationToken, OperationOutcome, OperationProgress};
use updater_manager_api::{
    ManagerCapability, ManagerId, PackageAction, PackageInfo, PackageTarget,
};

use crate::{
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
pub struct Installed {
    /// Search text for filtering installed packages in UI.
    search_query: String,
    /// Whether the package-manager source picker is expanded.
    sources_expanded: bool,
    /// Search text inside the package-manager source picker.
    source_query: String,
    /// Package currently shown in the details inspector.
    inspected_package: Option<PackageSelectionKey>,
    /// On-demand metadata request and result for the current inspector package.
    package_detail: shared::PackageDetailState,
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
    /// Installed-load result message.
    LoadInstalledResult {
        /// Request generation assigned when this manager load started.
        request_id: u64,
        /// Source manager.
        manager: ManagerId,
        /// Installed packages or failure detail.
        result: Result<Vec<PackageInfo>, String>,
    },
    /// Installed refresh message.
    RefreshInfo,
    /// Retry loading one package manager.
    RetryLoad(ManagerId),
    /// Search-query change message.
    SearchQueryChanged(String),
    /// Sort-option change message.
    SortOptionChanged(SortOption),
    /// Package-inspector selection message.
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
    /// Open a validated homepage URL.
    OpenHomepage(String),
    /// Homepage opener result.
    HomepageOpened(Result<(), String>),
    /// Package-selection toggle message.
    TogglePackageSelection(ManagerId, String, bool),
    /// Select-all toggle message.
    ToggleSelectAll(bool),
    /// Remove-selected message.
    RemoveSelectedPackages,
    /// Confirm package removal.
    ConfirmRemovePackages,
    /// Cancel package removal.
    CancelRemovePackages,
    /// Remove progress message.
    RemoveProgress {
        /// Number of finished packages.
        completed: usize,
        /// Total packages to remove.
        total: usize,
        /// Manager currently executing command.
        manager: ManagerId,
        /// Current package being processed.
        current_package: String,
        /// Optional command output/status line.
        command_message: Option<String>,
    },
    /// Remove result message.
    RemovePackagesResult(OperationOutcome),
}

/// Information about installed packages passed from app state
#[derive(Debug, Clone, Default)]
pub struct InstalledInfo {
    /// Installed package cache by manager `(count, packages)`.
    pub installed_packages: HashMap<ManagerId, (usize, Vec<PackageInfo>)>,
    /// Initial count-loading failures grouped by manager.
    pub init_errors: ManagerErrors,
    /// Full installed-list loading failures grouped by manager.
    pub load_errors: ManagerErrors,
    /// Managers selected in the filter panel.
    pub selected_managers: HashSet<ManagerId>,
    /// Managers currently loading full installed package list.
    pub loading_installed: HashMap<ManagerId, u64>,
    /// Last allocated installed-load request generation.
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
    /// Whether remove operation is in progress.
    pub is_removing: bool,
    /// Remove progress `(completed, total, manager, package)`.
    pub remove_progress: Option<(usize, usize, ManagerId, String)>,
    /// Remove command logs.
    pub remove_logs: Vec<String>,
    /// Removal plan frozen when the confirmation opened; Confirm executes exactly this.
    pub pending_remove: Option<PackageActionPlan>,
    /// Last removal error shown in UI.
    pub last_remove_error: Option<String>,
    /// Last inspector action error shown in UI.
    pub inspector_error: Option<String>,
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
    Version,
    InstallDate,
}

impl SortOption {
    pub fn name(&self) -> &'static str {
        match self {
            SortOption::Name => "Name",
            SortOption::Version => "Version",
            SortOption::InstallDate => "Install Date",
        }
    }

    pub const ALL: [SortOption; 3] = [
        SortOption::Name,
        SortOption::Version,
        SortOption::InstallDate,
    ];
}

impl Installed {
    pub fn update(
        &mut self,
        message: Message,
        pm_config: &updater_core::Config,
        info: &mut InstalledInfo,
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
                if !selected {
                    for manager in managers {
                        self.set_source_selection(false, manager, pm_config, info, catalog);
                    }
                    return Action::None;
                }

                let mut tasks = Vec::new();
                for manager in managers {
                    if let Action::Run(task) =
                        self.set_source_selection(true, manager, pm_config, info, catalog)
                    {
                        tasks.push(task);
                    }
                }
                if tasks.is_empty() {
                    Action::None
                } else {
                    Action::Run(Task::batch(tasks))
                }
            }
            Message::SelectPackageManager(manager, selected) => {
                self.set_source_selection(selected, manager, pm_config, info, catalog)
            }
            Message::LoadInstalledResult {
                request_id,
                manager,
                result,
            } => {
                if info.loading_installed.get(&manager) != Some(&request_id) {
                    return Action::None;
                }
                info.loading_installed.remove(&manager);
                let inspected_package_missing = self.inspected_package.as_ref().is_some_and(
                    |(inspected_manager, inspected_name)| {
                        inspected_manager == &manager
                            && result.as_ref().is_ok_and(|packages| {
                                !packages
                                    .iter()
                                    .any(|package| package.name == *inspected_name)
                            })
                    },
                );
                if inspected_package_missing {
                    self.inspected_package = None;
                }
                apply_manager_counted_items_result(
                    &mut info.installed_packages,
                    &mut info.load_errors,
                    manager,
                    result,
                );
                Action::None
            }
            Message::RefreshInfo => {
                let managers: Vec<ManagerId> = info.installed_packages.keys().cloned().collect();

                if managers.is_empty() {
                    return Action::None;
                }

                let tasks: Vec<Task<Message>> = managers
                    .into_iter()
                    .map(|manager| Self::start_load(pm_config, info, manager, catalog))
                    .collect();

                Action::Run(Task::batch(tasks))
            }
            Message::RetryLoad(manager) => {
                if info.loading_installed.contains_key(&manager) {
                    return Action::None;
                }
                info.init_errors.remove(&manager);
                info.load_errors.remove(&manager);
                Action::Run(Self::start_load(pm_config, info, manager, catalog))
            }
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
                Action::None
            }
            Message::SortOptionChanged(sort_option) => {
                info.sort_by = sort_option;
                Action::None
            }
            Message::InspectPackage(manager, package_name)
            | Message::RetryPackageInfo(manager, package_name) => {
                let key = shared::selection_key(&manager, &package_name);
                self.inspected_package = Some(key.clone());
                info.inspector_error = None;
                let Some(target) = info
                    .installed_packages
                    .get(&manager)
                    .and_then(|(_, packages)| {
                        packages.iter().find(|package| package.name == package_name)
                    })
                    .map(PackageInfo::target)
                else {
                    return Action::None;
                };
                let Some(config) = pm_config.manager(&manager).cloned() else {
                    info.inspector_error = Some(format!("manager is not configured: {manager}"));
                    return Action::None;
                };
                let generation = self.package_detail.begin(key);
                let registry = catalog.registry();
                Action::Run(
                    Task::future(shared::load_package_info(registry, config, target)).then(
                        move |result| {
                            Task::done(Message::PackageInfoLoaded {
                                generation,
                                manager: manager.clone(),
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
                info.inspector_error = None;
                Action::Run(iced::clipboard::write(value))
            }
            Message::OpenHomepage(homepage) => {
                info.inspector_error = None;
                Action::Run(
                    Task::future(crate::content::shared::open_http_url(homepage))
                        .then(|result| Task::done(Message::HomepageOpened(result))),
                )
            }
            Message::HomepageOpened(result) => {
                info.inspector_error = result.err();
                Action::None
            }
            Message::TogglePackageSelection(manager, package_name, selected) => {
                if info.is_removing {
                    return Action::None;
                }
                let key = shared::selection_key(&manager, &package_name);
                if selected {
                    info.selected_packages.insert(key);
                } else {
                    info.selected_packages.remove(&key);
                }
                info.pending_remove = None;
                Action::None
            }
            Message::ToggleSelectAll(select_all) => {
                if info.is_removing {
                    return Action::None;
                }

                let query = self.search_query.trim().to_lowercase();
                let visible = info
                    .selected_managers
                    .iter()
                    .filter_map(|manager| {
                        info.installed_packages
                            .get(manager)
                            .map(|(_, packages)| (manager, packages))
                    })
                    .flat_map(|(manager, packages)| {
                        packages
                            .iter()
                            .filter(|package| {
                                query.is_empty()
                                    || package.name.to_lowercase().contains(query.as_str())
                            })
                            .map(move |package| shared::selection_key(manager, &package.name))
                    });
                if select_all {
                    info.selected_packages.extend(visible);
                } else {
                    visible.for_each(|key| {
                        info.selected_packages.remove(&key);
                    });
                }
                info.pending_remove = None;
                Action::None
            }
            Message::RemoveSelectedPackages => {
                if info.selected_packages.is_empty() || info.is_removing {
                    return Action::None;
                }
                info.last_remove_error = None;
                let manager_groups = collect_selected_package_groups(
                    info.selected_managers.iter().filter_map(|manager| {
                        info.installed_packages
                            .get(manager)
                            .map(|(_, packages)| (manager.clone(), packages.as_slice()))
                    }),
                    &info.selected_packages,
                    catalog,
                    PackageInfo::target,
                );
                if manager_groups.is_empty() {
                    info.pending_remove = None;
                    info.last_remove_error =
                        Some("Selected packages are no longer available to remove".to_owned());
                    return Action::None;
                }
                info.pending_remove = Some(PackageActionPlan { manager_groups });
                Action::None
            }
            Message::ConfirmRemovePackages => {
                let Some(manager_groups) = Self::begin_frozen_remove(info) else {
                    return Action::None;
                };
                let cancellation = CancellationToken::default();
                let task = run_grouped_package_action(
                    catalog.registry(),
                    pm_config,
                    PackageAction::Uninstall,
                    manager_groups,
                    cancellation.clone(),
                    |OperationProgress {
                         completed,
                         total,
                         manager,
                         current_package,
                         command_message,
                     }| Message::RemoveProgress {
                        completed,
                        total,
                        manager,
                        current_package,
                        command_message,
                    },
                    Message::RemovePackagesResult,
                );
                Action::CancellableRun(task, cancellation)
            }
            Message::CancelRemovePackages => {
                info.pending_remove = None;
                Action::None
            }
            Message::RemoveProgress {
                completed,
                total,
                manager,
                current_package,
                command_message,
            } => {
                info.remove_progress = Some((completed, total, manager.clone(), current_package));
                if let Some(command_message) = command_message {
                    push_command_log(
                        &mut info.remove_logs,
                        PackageAction::Uninstall,
                        &manager,
                        catalog,
                        info.remove_progress
                            .as_ref()
                            .map_or("", |(_, _, _, package)| package.as_str()),
                        command_message,
                    );
                }
                Action::None
            }
            Message::RemovePackagesResult(outcome) => {
                info.is_removing = false;
                info.remove_progress = None;
                if outcome.is_success() {
                    info.selected_packages.clear();
                    info.last_remove_error = None;
                    Action::PackageOperationFinished { outcome }
                } else {
                    let error = outcome.error.clone().unwrap_or_else(|| outcome.summary());
                    log::error!("Failed to remove packages: {}", error);
                    info.last_remove_error = Some(error);
                    Action::PackageOperationFinished { outcome }
                }
            }
        }
    }

    fn set_source_selection(
        &mut self,
        selected: bool,
        manager: ManagerId,
        pm_config: &updater_core::Config,
        info: &mut InstalledInfo,
        catalog: &ManagerCatalog,
    ) -> Action {
        if !selected {
            info.selected_managers.remove(&manager);
            info.selected_packages
                .retain(|(selected_manager, _)| selected_manager != &manager);
            if self
                .inspected_package
                .as_ref()
                .is_some_and(|(inspected_manager, _)| inspected_manager == &manager)
            {
                self.inspected_package = None;
            }
            info.pending_remove = None;
            return Action::None;
        }

        if !info.has_loading_count
            || (info.is_loading_count && !info.installed_packages.contains_key(&manager))
        {
            return Action::None;
        }
        info.selected_managers.insert(manager.clone());
        let should_load =
            if info.init_errors.contains_key(&manager) || info.load_errors.contains_key(&manager) {
                info.init_errors.remove(&manager);
                info.load_errors.remove(&manager);
                true
            } else if info.loading_installed.contains_key(&manager) {
                false
            } else if let Some((count, packages)) = info.installed_packages.get(&manager) {
                *count != packages.len()
            } else {
                true
            };
        if should_load {
            Action::Run(Self::start_load(pm_config, info, manager, catalog))
        } else {
            Action::None
        }
    }

    pub fn has_inspector_selection(&self) -> bool {
        self.inspected_package.is_some()
    }

    pub fn dismiss_transient(&mut self, info: &mut InstalledInfo) -> bool {
        if info.pending_remove.take().is_some() {
            true
        } else {
            self.inspected_package.take().is_some()
        }
    }

    /// Consumes the frozen removal plan and seeds progress from it.
    ///
    /// Returns the exact manager groups to uninstall, independent of the live selection.
    fn begin_frozen_remove(
        info: &mut InstalledInfo,
    ) -> Option<Vec<(ManagerId, Vec<PackageTarget>)>> {
        if info.is_removing {
            return None;
        }
        let plan = info.pending_remove.take()?;
        let Some((initial_manager, _)) = plan.manager_groups.first() else {
            info.last_remove_error =
                Some("The removal plan does not contain any packages".to_owned());
            return None;
        };
        info.is_removing = true;
        info.last_remove_error = None;
        info.remove_logs.clear();
        info.remove_progress = Some((
            0,
            plan.package_count(),
            initial_manager.clone(),
            String::new(),
        ));
        Some(plan.manager_groups)
    }

    /// Counts frozen removal targets that the current search filter hides.
    fn hidden_remove_count(&self, plan: &PackageActionPlan) -> usize {
        let query = self.search_query.trim().to_lowercase();
        if query.is_empty() {
            return 0;
        }
        plan.manager_groups
            .iter()
            .flat_map(|(_, targets)| targets)
            .filter(|target| !target.name.to_lowercase().contains(query.as_str()))
            .count()
    }

    pub fn primary_action(&self, info: &InstalledInfo) -> Option<Message> {
        if info.pending_remove.is_some() {
            // Removal is irreversible: the keyboard default dismisses the plan;
            // only the explicit Remove button confirms it.
            return Some(Message::CancelRemovePackages);
        }
        (!info.is_removing && !info.selected_packages.is_empty())
            .then_some(Message::RemoveSelectedPackages)
    }

    pub fn can_select_packages(&self) -> bool {
        true
    }

    pub fn move_keyboard_selection(
        &self,
        info: &InstalledInfo,
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

    pub fn toggle_keyboard_selection(&self, info: &InstalledInfo) -> Option<Message> {
        let (manager, name) = self.inspected_package.as_ref()?;
        if info.is_removing {
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
        info: &InstalledInfo,
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
                let packages = info
                    .installed_packages
                    .get(&manager)
                    .map_or(&[][..], |(_, packages)| packages.as_slice());
                self.filter_and_sort_packages(packages, info.sort_by)
                    .into_iter()
                    .filter(|package| {
                        query.is_empty() || package.name.to_lowercase().contains(&query)
                    })
                    .map(move |package| (manager.clone(), package.name.clone()))
            })
            .collect()
    }

    pub fn view<'a>(
        &'a self,
        info: &'a InstalledInfo,
        pm_config: &updater_core::Config,
        catalog: &'a ManagerCatalog,
        show_inspector: bool,
        inspector_drawer: bool,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, container, row};

        let total_packages: usize = info
            .installed_packages
            .values()
            .map(|(count, _)| *count)
            .sum();
        let configured_managers = shared::configured_managers(pm_config).len();

        let toolbar = shared::toolbar(
            column![
                row![
                    container(self.search_input_view()).width(iced::Length::Fill),
                    column![
                        shared::section_title("Actions"),
                        shared::refresh_button_with_label(
                            "Refresh",
                            !info.is_removing,
                            Message::RefreshInfo
                        )
                    ]
                    .spacing(theme::spacing::SM),
                ]
                .spacing(theme::spacing::MD)
                .align_y(iced::Alignment::End),
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

        column![
            shared::page_header(
                "Installed",
                format!(
                    "{total_packages} packages across {configured_managers} configured managers"
                ),
                theme::colors::INSTALLED,
            ),
            shared::summary_row([
                (
                    format!("{total_packages} installed"),
                    theme::colors::INSTALLED,
                ),
                (
                    format!("{} sources selected", info.selected_managers.len()),
                    theme::colors::ON_SURFACE_MUTED,
                ),
                (
                    format!("{} packages selected", info.selected_packages.len()),
                    theme::colors::DISCOVER,
                ),
            ]),
            toolbar,
            self.batch_actions_view(info, catalog),
            self.packages_list_view(info, catalog, show_inspector, inspector_drawer),
        ]
        .spacing(theme::spacing::LG)
        .height(iced::Length::Fill)
        .into()
    }

    // View components.

    fn manager_filter_view<'a>(
        &'a self,
        info: &'a InstalledInfo,
        pm_config: &updater_core::Config,
        catalog: &'a ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        use iced::widget::column;

        let managers = shared::configured_managers_with_capability(
            pm_config,
            catalog,
            ManagerCapability::Installed,
        );
        if managers.is_empty() {
            return column![
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
                    .installed_packages
                    .get(&manager)
                    .map(|(count, _)| *count),
                status: if info.loading_installed.contains_key(&manager) {
                    shared::ManagerSourceStatus::Loading
                } else if info.init_errors.contains_key(&manager)
                    || info.load_errors.contains_key(&manager)
                {
                    shared::ManagerSourceStatus::Failed
                } else if !info.has_loading_count
                    || (info.is_loading_count && !info.installed_packages.contains_key(&manager))
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
                count_label: "installed",
                disabled: info.is_removing || !info.has_loading_count,
            },
            shared::ManagerSourcePickerMessages {
                toggle_picker: Message::ToggleSourcePicker,
                query_changed: Message::SourceQueryChanged,
                set_visible_selection: Message::SetSourceSelection,
                toggle_manager: Message::SelectPackageManager,
            },
        );

        let mut content = column![shared::section_title("Sources")];
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

    fn sort_order_view<'a>(&self, info: &'a InstalledInfo) -> iced::Element<'a, Message> {
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

    // Package list views.

    fn search_input_view(&self) -> iced::Element<'static, Message> {
        shared::search_input_view(
            crate::content::shared::search_input_id(crate::content::ActiveContentPage::Installed),
            "Search",
            "Search packages...",
            &self.search_query,
            Message::SearchQueryChanged,
        )
    }

    fn packages_list_view<'a>(
        &'a self,
        info: &'a InstalledInfo,
        catalog: &'a ManagerCatalog,
        show_inspector: bool,
        inspector_drawer: bool,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, container, row, scrollable};

        if !info.has_loading_count {
            return shared::centered_message(if info.is_loading_count {
                "Loading package information..."
            } else {
                "Waiting to load package information"
            });
        }

        let filtered_managers: Vec<_> = info
            .installed_packages
            .iter()
            .filter(|(manager, _)| info.selected_managers.contains(*manager))
            .collect();

        if filtered_managers.is_empty() {
            return shared::centered_message("Please select a package manager to view");
        }

        let search_query = self.search_query.trim().to_lowercase();
        let has_visible_errors = filtered_managers
            .iter()
            .any(|(manager, _)| info.load_errors.contains_key(*manager));

        if !search_query.is_empty() {
            let has_any_match = filtered_managers.iter().any(|(_, (_, packages))| {
                packages
                    .iter()
                    .any(|pkg| pkg.name.to_lowercase().contains(&search_query))
            });

            if !has_any_match && !has_visible_errors {
                return shared::centered_message("No packages match your search");
            }
        }

        let packages_sections: Vec<iced::Element<'_, Message>> = filtered_managers
            .into_iter()
            .map(|(manager, (count, packages))| {
                self.package_manager_section(manager, *count, packages, info, catalog)
            })
            .collect();

        let package_list = scrollable(column(packages_sections).spacing(20))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill);

        let inspected_package = self.inspected_package.as_ref().and_then(|(manager, name)| {
            info.installed_packages
                .get(manager)
                .and_then(|(_, packages)| packages.iter().find(|package| package.name == *name))
                .map(|package| (manager.clone(), package))
        });

        if !show_inspector {
            return container(package_list)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into();
        }

        let inspector = container(self.package_inspector_view(
            inspected_package,
            info.inspector_error.as_deref(),
            catalog,
        ))
        .padding(theme::spacing::LG)
        .width(if inspector_drawer {
            iced::Length::Fill
        } else {
            iced::Length::Fixed(268.0)
        })
        .height(iced::Length::Fill)
        .style(theme::surface_container);
        if inspector_drawer {
            return column![container(package_list).width(iced::Length::Fill), inspector]
                .spacing(theme::spacing::LG)
                .into();
        }

        row![
            container(package_list)
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
        manager: &'a ManagerId,
        count: usize,
        packages: &'a [PackageInfo],
        info: &'a InstalledInfo,
        catalog: &'a ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        let is_loading = info.loading_installed.contains_key(manager);
        let filtered_packages = self.filter_and_sort_packages(packages, info.sort_by);
        let subtitle = if is_loading {
            "(Loading...)".to_owned()
        } else {
            format!("({} packages)", count)
        };

        let body = (!filtered_packages.is_empty()).then(|| {
            iced::widget::column(
                filtered_packages
                    .into_iter()
                    .map(|pkg| self.package_item_view(manager, pkg, info)),
            )
            .spacing(8)
            .into()
        });

        shared::manager_section(
            manager.clone(),
            catalog,
            subtitle,
            ManagerSectionStyle {
                accent: theme::colors::INSTALLED,
                error_prefix: "Failed to load installed packages",
            },
            info.load_errors
                .get(manager)
                .or_else(|| info.init_errors.get(manager))
                .map(String::as_str),
            || Message::RetryLoad(manager.clone()),
            Message::CopyInspectorText,
            body,
        )
    }

    fn filter_and_sort_packages<'a>(
        &self,
        packages: &'a [PackageInfo],
        sort_by: SortOption,
    ) -> Vec<&'a PackageInfo> {
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
            SortOption::Version => {
                filtered.sort_by(|a, b| a.version.cmp(&b.version));
            }
            SortOption::InstallDate => {
                filtered.sort_by(|a, b| match (&b.install_date, &a.install_date) {
                    (Some(b_date), Some(a_date)) => b_date.cmp(a_date),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                });
            }
        }

        filtered
    }

    fn package_item_view<'a>(
        &self,
        manager: &'a ManagerId,
        package: &'a PackageInfo,
        info: &'a InstalledInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, checkbox, row};

        let package_name = package.name.clone();
        let is_selected = info
            .selected_packages
            .contains(&shared::selection_key(manager, &package.name));
        let is_inspected =
            self.inspected_package
                .as_ref()
                .is_some_and(|(inspected_manager, name)| {
                    inspected_manager == manager && name == &package.name
                });

        let package_checkbox = checkbox(is_selected)
            .on_toggle_maybe((!info.is_removing).then_some({
                let package_name = package_name.clone();
                let manager = manager.clone();
                move |selected| {
                    Message::TogglePackageSelection(manager.clone(), package_name.clone(), selected)
                }
            }))
            .size(18)
            .spacing(8)
            .style(shared::checkbox_style(false));

        let details = button(
            row![
                shared::package_summary(package),
                shared::muted_badge(&package.version)
            ]
            .spacing(theme::spacing::MD)
            .align_y(iced::Alignment::Center),
        )
        .padding([8, 10])
        .width(iced::Length::Fill)
        .style(theme::list_row(is_inspected))
        .on_press(Message::InspectPackage(manager.clone(), package_name));

        row![package_checkbox, details]
            .spacing(theme::spacing::SM)
            .align_y(iced::Alignment::Center)
            .into()
    }

    fn package_inspector_view<'a>(
        &'a self,
        inspected: Option<(ManagerId, &'a PackageInfo)>,
        inspector_error: Option<&'a str>,
        catalog: &'a ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        use crate::content::shared::PackageInspector;
        use iced::widget::{column, text};

        let retry_info = inspected.as_ref().and_then(|(manager, package)| {
            let key = shared::selection_key(manager, &package.name);
            self.package_detail
                .error(&key)
                .map(|_| Message::RetryPackageInfo(manager.clone(), package.name.clone()))
        });
        let package = inspected.map(|(manager, package)| {
            let key = shared::selection_key(&manager, &package.name);
            let displayed = self.package_detail.package(&key).unwrap_or(package);
            PackageInspector {
                manager,
                name: &displayed.name,
                version: &displayed.version,
                available_version: None,
                description: displayed.description.as_deref(),
                size: displayed.size,
                install_date: displayed.install_date.as_deref(),
                homepage: displayed.homepage.as_deref(),
                scope: displayed.scope,
                origin: displayed.origin.as_ref(),
                is_loading: self.package_detail.is_loading(&key),
                detail_error: self.package_detail.error(&key),
            }
        });
        let mut content = column![shared::package_inspector(
            package,
            catalog,
            Message::CopyInspectorText,
            Message::CopyInspectorText,
            Message::OpenHomepage,
            retry_info,
        )]
        .height(iced::Length::Fill);
        if let Some(error) = inspector_error {
            content = content.push(
                text(error)
                    .size(12)
                    .style(theme::text_error)
                    .wrapping(text::Wrapping::WordOrGlyph),
            );
        }
        content.into()
    }

    fn batch_actions_view<'a>(
        &self,
        info: &'a InstalledInfo,
        catalog: &'a ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, checkbox, column, row, text};

        let selected_count = info.selected_packages.len();
        let is_enabled = selected_count > 0 && !info.is_removing;

        let query = self.search_query.trim().to_lowercase();
        let mut total_visible = 0;
        let mut selected_visible = 0;
        for manager in &info.selected_managers {
            if let Some((_, packages)) = info.installed_packages.get(manager) {
                for package in packages {
                    if !query.is_empty() && !package.name.to_lowercase().contains(&query) {
                        continue;
                    }
                    total_visible += 1;
                    if info
                        .selected_packages
                        .contains(&shared::selection_key(manager, &package.name))
                    {
                        selected_visible += 1;
                    }
                }
            }
        }

        let all_selected = total_visible > 0 && selected_visible == total_visible;

        let button_text = if info.is_removing {
            if let Some((completed, total, manager, package)) = &info.remove_progress {
                if package.is_empty() {
                    format!("Removing {}/{}...", completed, total)
                } else {
                    format!(
                        "Removing {}/{}: {} ({})",
                        completed,
                        total,
                        package,
                        catalog.display_name(manager)
                    )
                }
            } else {
                "Removing...".to_string()
            }
        } else if selected_count > 0 {
            format!("Remove {} package(s)", selected_count)
        } else {
            "Remove Selected".to_string()
        };

        let select_all_checkbox = checkbox(all_selected)
            .label("Select All")
            .on_toggle_maybe((!info.is_removing).then_some(Message::ToggleSelectAll))
            .size(18)
            .spacing(8)
            .text_size(14)
            .style(shared::checkbox_style(false));

        let remove_button = button(text(button_text).size(14).font(theme::FONT_SEMIBOLD).style(
            if is_enabled {
                theme::text_on_primary
            } else {
                theme::text_on_surface_muted
            },
        ))
        .padding([8, 16])
        .style(theme::action_button(
            is_enabled,
            theme::colors::REMOVE_ACTION,
            theme::colors::REMOVE_ACTION_HOVER,
            theme::colors::REMOVE_ACTION_ACTIVE,
        ));

        let remove_button = if is_enabled {
            remove_button.on_press(Message::RemoveSelectedPackages)
        } else {
            remove_button
        };

        let actions_row = row![select_all_checkbox, remove_button]
            .spacing(12)
            .align_y(iced::Alignment::Center);

        let mut content = column![actions_row].spacing(8);
        if info.pending_remove.is_some() {
            content = content.push(self.remove_confirmation_view(info, catalog));
        }
        if let Some(error) = &info.last_remove_error {
            content = content.push(
                text(format!("Removal failed: {error}"))
                    .size(13)
                    .style(theme::text_error),
            );
        }

        content.into()
    }

    fn remove_confirmation_view<'a>(
        &self,
        info: &'a InstalledInfo,
        catalog: &'a ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, column, container, row, text};

        let Some(plan) = &info.pending_remove else {
            return container("").height(iced::Length::Shrink).into();
        };
        let package_count = plan.package_count();
        let manager_count = plan.manager_groups.len();
        let hidden_count = self.hidden_remove_count(plan);

        let hidden_detail = (hidden_count > 0).then(|| {
            format!(
                "{hidden_count} package(s) are hidden by the current filter and will also be removed"
            )
        });

        let mut content = column![
            row![
                column![
                    text(format!(
                        "Remove {package_count} package{} from {manager_count} source{}",
                        if package_count == 1 { "" } else { "s" },
                        if manager_count == 1 { "" } else { "s" },
                    ))
                    .size(14)
                    .font(theme::FONT_SEMIBOLD)
                    .style(theme::text_on_surface),
                    text("This cannot be undone")
                        .size(13)
                        .style(theme::text_error),
                ]
                .spacing(theme::spacing::XS)
                .width(iced::Length::Fill),
                button(text("Cancel").size(13))
                    .padding([8, 12])
                    .style(theme::secondary_button(true))
                    .on_press(Message::CancelRemovePackages),
                button(
                    text(format!("Remove {package_count} Packages"))
                        .size(13)
                        .font(theme::FONT_SEMIBOLD)
                        .style(theme::text_on_primary)
                )
                .padding([8, 14])
                .style(theme::action_button(
                    true,
                    theme::colors::REMOVE_ACTION,
                    theme::colors::REMOVE_ACTION_HOVER,
                    theme::colors::REMOVE_ACTION_ACTIVE,
                ))
                .on_press(Message::ConfirmRemovePackages),
            ]
            .spacing(theme::spacing::MD)
            .align_y(iced::Alignment::Center)
            .wrap(),
        ]
        .spacing(theme::spacing::MD);
        content = content.push(shared::package_action_plan_view(
            &plan.manager_groups,
            catalog,
        ));
        if let Some(hidden_detail) = hidden_detail {
            content = content.push(
                text(hidden_detail)
                    .size(12)
                    .style(theme::text_warning)
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

    pub(crate) fn start_load(
        pm_config: &updater_core::Config,
        info: &mut InstalledInfo,
        manager: ManagerId,
        catalog: &ManagerCatalog,
    ) -> Task<Message> {
        info.request_generation = info.request_generation.wrapping_add(1);
        let request_id = info.request_generation;
        info.loading_installed.insert(manager.clone(), request_id);

        let pm_config = pm_config.clone();
        let registry = catalog.registry();
        let result_manager = manager.clone();

        Task::future(async move {
            let runtime = registry
                .manager_for(&manager, ManagerCapability::Installed)
                .map_err(|error| error.to_string())?;
            let manager_config = pm_config
                .manager(&manager)
                .ok_or_else(|| format!("Manager is not configured: {manager}"))?;
            runtime
                .installed(manager_config)
                .await
                .map_err(|error| shared::describe_manager_error(&error))
        })
        .then(move |result| {
            Task::done(Message::LoadInstalledResult {
                request_id,
                manager: result_manager.clone(),
                result,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed_with_packages(manager: &ManagerId, names: &[&str]) -> (Installed, InstalledInfo) {
        let mut info = InstalledInfo::default();
        info.selected_managers.insert(manager.clone());
        info.installed_packages.insert(
            manager.clone(),
            (
                names.len(),
                names
                    .iter()
                    .map(|name| PackageInfo::new(manager.clone(), *name, "1.0"))
                    .collect(),
            ),
        );
        (Installed::default(), info)
    }

    #[test]
    fn remove_confirmation_freezes_plan_including_hidden_selection() {
        let cargo = ManagerId::parse("builtin:cargo").unwrap();
        let (mut installed, mut info) =
            installed_with_packages(&cargo, &["alpha", "python-one", "python-two"]);
        let config = updater_core::Config::default();
        let catalog = ManagerCatalog::builtin();

        let _ = installed.update(
            Message::TogglePackageSelection(cargo.clone(), "alpha".to_owned(), true),
            &config,
            &mut info,
            &catalog,
        );
        let _ = installed.update(
            Message::SearchQueryChanged("python".to_owned()),
            &config,
            &mut info,
            &catalog,
        );
        let _ = installed.update(Message::ToggleSelectAll(true), &config, &mut info, &catalog);
        let action = installed.update(
            Message::RemoveSelectedPackages,
            &config,
            &mut info,
            &catalog,
        );

        assert!(matches!(action, Action::None));
        assert!(!info.is_removing);
        let plan = info
            .pending_remove
            .as_ref()
            .expect("removal plan is frozen");
        assert_eq!(
            plan.manager_groups,
            vec![(
                cargo.clone(),
                vec![
                    PackageTarget::new(cargo.clone(), "alpha"),
                    PackageTarget::new(cargo.clone(), "python-one"),
                    PackageTarget::new(cargo, "python-two"),
                ],
            )]
        );
        assert_eq!(plan.package_count(), 3);
        assert_eq!(installed.hidden_remove_count(plan), 1);

        let _ = installed.update(Message::CancelRemovePackages, &config, &mut info, &catalog);
        assert!(info.pending_remove.is_none());
    }

    #[test]
    fn confirm_remove_executes_frozen_plan() {
        let cargo = ManagerId::parse("builtin:cargo").unwrap();
        let (mut installed, mut info) =
            installed_with_packages(&cargo, &["alpha", "beta", "gamma"]);
        info.selected_packages
            .insert(shared::selection_key(&cargo, "alpha"));
        let config = updater_core::Config::default();
        let catalog = ManagerCatalog::builtin();

        let _ = installed.update(
            Message::RemoveSelectedPackages,
            &config,
            &mut info,
            &catalog,
        );
        let frozen = info.pending_remove.clone().expect("removal plan is frozen");

        // The live selection drifts after Prepare; the frozen plan must win.
        info.selected_packages.clear();
        info.selected_packages
            .insert(shared::selection_key(&cargo, "beta"));
        info.selected_packages
            .insert(shared::selection_key(&cargo, "gamma"));

        let mut probe = info.clone();
        let executed = Installed::begin_frozen_remove(&mut probe);
        assert_eq!(executed, Some(frozen.manager_groups.clone()));
        assert_eq!(
            executed,
            Some(vec![(
                cargo.clone(),
                vec![PackageTarget::new(cargo.clone(), "alpha")]
            )])
        );

        let action = installed.update(Message::ConfirmRemovePackages, &config, &mut info, &catalog);

        assert!(matches!(action, Action::CancellableRun(_, _)));
        assert!(info.is_removing);
        assert!(info.pending_remove.is_none());
        assert_eq!(
            info.remove_progress,
            Some((0, frozen.package_count(), cargo, String::new()))
        );
    }

    #[test]
    fn remove_confirmation_keyboard_default_cancels() {
        let cargo = ManagerId::parse("builtin:cargo").unwrap();
        let (installed, mut info) = installed_with_packages(&cargo, &["alpha"]);
        info.selected_packages
            .insert(shared::selection_key(&cargo, "alpha"));

        assert!(matches!(
            installed.primary_action(&info),
            Some(Message::RemoveSelectedPackages)
        ));

        info.pending_remove = Some(PackageActionPlan {
            manager_groups: vec![(cargo.clone(), vec![PackageTarget::new(cargo, "alpha")])],
        });

        assert!(matches!(
            installed.primary_action(&info),
            Some(Message::CancelRemovePackages)
        ));
    }

    #[test]
    fn confirm_remove_without_plan_does_nothing() {
        let cargo = ManagerId::parse("builtin:cargo").unwrap();
        let (mut installed, mut info) = installed_with_packages(&cargo, &["alpha"]);
        info.selected_packages
            .insert(shared::selection_key(&cargo, "alpha"));

        let action = installed.update(
            Message::ConfirmRemovePackages,
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        assert!(matches!(action, Action::None));
        assert!(!info.is_removing);
        assert!(info.remove_progress.is_none());
    }

    #[test]
    fn clearing_visible_sources_preserves_hidden_selection_and_inspector() {
        let mut installed = Installed::default();
        let mut info = InstalledInfo::default();
        let cargo = ManagerId::parse("builtin:cargo").unwrap();
        let flatpak = ManagerId::parse("builtin:flatpak").unwrap();
        info.selected_managers = HashSet::from([cargo.clone(), flatpak.clone()]);
        info.selected_packages
            .insert(shared::selection_key(&cargo, "cargo-edit"));
        info.selected_packages
            .insert(shared::selection_key(&flatpak, "org.example.App"));
        installed.inspected_package = Some(shared::selection_key(&flatpak, "org.example.App"));

        let action = installed.update(
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
        assert_eq!(
            installed.inspected_package,
            Some(shared::selection_key(&flatpak, "org.example.App"))
        );
    }

    #[test]
    fn installed_result_only_applies_to_the_active_request() {
        let mut installed = Installed::default();
        let mut info = InstalledInfo::default();
        let manager = ManagerId::parse("builtin:cargo").unwrap();
        info.loading_installed.insert(manager.clone(), 2);

        let _ = installed.update(
            Message::LoadInstalledResult {
                request_id: 1,
                manager: manager.clone(),
                result: Err("stale result".to_owned()),
            },
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        assert_eq!(info.loading_installed.get(&manager), Some(&2));
        assert!(info.load_errors.is_empty());

        let _ = installed.update(
            Message::LoadInstalledResult {
                request_id: 2,
                manager: manager.clone(),
                result: Err("current result".to_owned()),
            },
            &updater_core::Config::default(),
            &mut info,
            &ManagerCatalog::builtin(),
        );

        assert!(!info.loading_installed.contains_key(&manager));
        assert_eq!(
            info.load_errors.get(&manager).map(String::as_str),
            Some("current result")
        );
    }
}
