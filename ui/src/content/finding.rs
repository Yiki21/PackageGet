// Finding/Search packages view with filtering, sorting and installation capabilities

use std::collections::{HashMap, HashSet};

use iced::Task;
use updater_core::{CancellationToken, OperationOutcome, OperationProgress};
use updater_manager_api::{ManagerCapability, ManagerId, PackageAction, PackageInfo};

use crate::{
    content::errors::{ManagerErrors, apply_manager_items_result},
    content::shared::{self, PackageSelectionKey},
    content::workflows::{
        collect_selected_package_groups, push_command_log, run_grouped_package_action,
    },
    theme,
};

#[derive(Debug, Clone, Default)]
pub struct Finding {
    /// Search query being edited by user.
    search_query: String,
    /// Last executed query used for post-install refresh.
    last_search_query: String,
    /// Package currently shown in the details inspector.
    inspected_package: Option<PackageSelectionKey>,
    /// Last inspector action error shown in UI.
    inspector_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Package-manager selection message.
    SelectPackageManager(ManagerId, bool),
    /// Search-query change message.
    SearchQueryChanged(String),
    /// Search execution message.
    ExecuteSearch,
    /// Repeat the last completed query after package-data reload.
    RepeatLastSearch,
    /// Search result message.
    SearchResult {
        /// Request generation assigned when this manager search started.
        request_id: u64,
        /// Source manager.
        manager: ManagerId,
        /// Matching packages or failure detail.
        result: Result<Vec<PackageInfo>, String>,
    },
    /// Retry search for one package manager.
    RetrySearch(ManagerId),
    /// Show a search result in the package inspector.
    InspectPackage(ManagerId, String),
    /// Copy text from the inspector.
    CopyInspectorText(String),
    /// Open a validated homepage URL.
    OpenHomepage(String),
    /// Homepage opener result.
    HomepageOpened(Result<(), String>),
    /// Sort-option change message.
    SortOptionChanged(SortOption),
    /// Package-selection toggle message.
    TogglePackageSelection(ManagerId, String, bool),
    /// Select-all visible packages toggle message.
    ToggleSelectAll(bool),
    /// Install-selected message.
    InstallSelectedPackages,
    /// Install progress message.
    InstallProgress {
        /// Number of finished packages.
        completed: usize,
        /// Total packages to install.
        total: usize,
        /// Manager currently executing command.
        manager: ManagerId,
        /// Current package being processed.
        current_package: String,
        /// Optional command output/status line.
        command_message: Option<String>,
    },
    /// Install result message.
    InstallPackagesResult(OperationOutcome),
}

#[derive(Debug, Clone, Default)]
pub struct FindingInfo {
    /// Search results grouped by manager.
    pub search_results: HashMap<ManagerId, Vec<PackageInfo>>,
    /// Search errors grouped by manager.
    pub search_errors: ManagerErrors,
    /// Managers selected in the filter panel.
    pub selected_managers: HashSet<ManagerId>,
    /// Managers currently running search.
    pub searching_managers: HashMap<ManagerId, u64>,
    /// Last allocated search request generation.
    pub request_generation: u64,
    /// Current sort option.
    pub sort_by: SortOption,
    /// Selected package keys for batch operations.
    pub selected_packages: HashSet<PackageSelectionKey>,
    /// Whether install operation is in progress.
    pub is_installing: bool,
    /// Install progress `(completed, total, manager, package)`.
    pub install_progress: Option<(usize, usize, ManagerId, String)>,
    /// Install command logs.
    pub install_logs: Vec<String>,
    /// Last install error shown in UI.
    pub last_install_error: Option<String>,
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
        follow_up: iced::Task<Message>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortOption {
    Name,
    #[default]
    Relevance,
}

impl SortOption {
    pub fn name(&self) -> &'static str {
        match self {
            SortOption::Name => "Name",
            SortOption::Relevance => "Relevance",
        }
    }

    pub const ALL: [SortOption; 2] = [SortOption::Name, SortOption::Relevance];
}

impl Finding {
    pub fn update(
        &mut self,
        message: Message,
        pm_config: &updater_core::Config,
        info: &mut FindingInfo,
        catalog: &crate::manager_catalog::ManagerCatalog,
    ) -> Action {
        match message {
            Message::SelectPackageManager(pm_type, selected) => {
                if selected {
                    info.selected_managers.insert(pm_type);
                } else {
                    info.selected_managers.remove(&pm_type);
                    info.searching_managers.remove(&pm_type);
                    info.search_errors.remove(&pm_type);
                    info.selected_packages
                        .retain(|(manager, _)| manager != &pm_type);
                    info.search_results.remove(&pm_type);
                }
                Action::None
            }
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
                Action::None
            }
            Message::ExecuteSearch => {
                let query = self.search_query.trim().to_owned();
                if query.is_empty() || info.is_installing || !info.searching_managers.is_empty() {
                    return Action::None;
                }

                // Search only in selected managers.
                if info.selected_managers.is_empty() {
                    return Action::None;
                }

                self.start_search(pm_config, info, &query, catalog)
            }
            Message::RepeatLastSearch => {
                let query = self.last_search_query.clone();
                if query.is_empty()
                    || info.is_installing
                    || !info.searching_managers.is_empty()
                    || info.selected_managers.is_empty()
                {
                    return Action::None;
                }
                self.start_search(pm_config, info, &query, catalog)
            }
            Message::SearchResult {
                request_id,
                manager: pm_type,
                result,
            } => {
                if info.searching_managers.get(&pm_type) != Some(&request_id) {
                    return Action::None;
                }
                info.searching_managers.remove(&pm_type);
                apply_manager_items_result(
                    &mut info.search_results,
                    &mut info.search_errors,
                    pm_type,
                    result,
                );
                Action::None
            }
            Message::RetrySearch(pm_type) => {
                if self.last_search_query.is_empty()
                    || info.searching_managers.contains_key(&pm_type)
                    || !info.selected_managers.contains(&pm_type)
                {
                    return Action::None;
                }
                info.search_errors.remove(&pm_type);
                info.request_generation = info.request_generation.wrapping_add(1);
                let request_id = info.request_generation;
                info.searching_managers.insert(pm_type.clone(), request_id);
                Action::Run(Self::execute_search_task(
                    pm_config,
                    &HashSet::from([pm_type]),
                    &self.last_search_query,
                    catalog,
                    request_id,
                ))
            }
            Message::InspectPackage(pm_type, package_name) => {
                self.inspected_package = Some(shared::selection_key(&pm_type, &package_name));
                self.inspector_error = None;
                Action::None
            }
            Message::CopyInspectorText(value) => {
                self.inspector_error = None;
                Action::Run(iced::clipboard::write(value))
            }
            Message::OpenHomepage(homepage) => {
                self.inspector_error = None;
                Action::Run(
                    Task::future(crate::content::shared::open_http_url(homepage))
                        .then(|result| Task::done(Message::HomepageOpened(result))),
                )
            }
            Message::HomepageOpened(result) => {
                self.inspector_error = result.err();
                Action::None
            }
            Message::SortOptionChanged(sort_option) => {
                info.sort_by = sort_option;
                Action::None
            }
            Message::TogglePackageSelection(pm_type, package_name, selected) => {
                if info.is_installing {
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
                if info.is_installing {
                    return Action::None;
                }

                let visible = info
                    .search_results
                    .iter()
                    .filter(|(manager, _)| info.selected_managers.contains(*manager))
                    .flat_map(|(manager, packages)| {
                        packages
                            .iter()
                            .filter(|package| package.version.trim() == "Not Installed")
                            .map(move |package| shared::selection_key(manager, &package.name))
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
            Message::InstallSelectedPackages => {
                if info.selected_packages.is_empty()
                    || info.is_installing
                    || !info.searching_managers.is_empty()
                {
                    return Action::None;
                }
                info.is_installing = true;
                info.last_install_error = None;
                info.install_logs.clear();
                let initial_manager = info
                    .selected_packages
                    .iter()
                    .next()
                    .map(|(manager_id, _)| manager_id.clone())
                    .expect("non-empty package selection must contain a manager ID");
                info.install_progress = Some((
                    0,
                    info.selected_packages.len(),
                    initial_manager,
                    String::new(),
                ));
                let manager_groups = collect_selected_package_groups(
                    info.search_results
                        .iter()
                        .map(|(manager, packages)| (manager.clone(), packages.as_slice())),
                    &info.selected_packages,
                    catalog,
                    |package| package.name.as_str(),
                );
                let cancellation = CancellationToken::default();
                let task = run_grouped_package_action(
                    catalog.registry(),
                    pm_config,
                    PackageAction::Install,
                    manager_groups,
                    cancellation.clone(),
                    |OperationProgress {
                         completed,
                         total,
                         manager,
                         current_package,
                         command_message,
                     }| Message::InstallProgress {
                        completed,
                        total,
                        manager,
                        current_package,
                        command_message,
                    },
                    Message::InstallPackagesResult,
                );
                Action::CancellableRun(task, cancellation)
            }
            Message::InstallProgress {
                completed,
                total,
                manager,
                current_package,
                command_message,
            } => {
                info.install_progress = Some((completed, total, manager.clone(), current_package));
                if let Some(command_message) = command_message {
                    push_command_log(
                        &mut info.install_logs,
                        PackageAction::Install,
                        &manager,
                        catalog,
                        info.install_progress
                            .as_ref()
                            .map_or("", |(_, _, _, package)| package.as_str()),
                        command_message,
                    );
                }
                Action::None
            }
            Message::InstallPackagesResult(outcome) => {
                info.is_installing = false;
                info.install_progress = None;
                if outcome.is_success() {
                    info.selected_packages.clear();
                    info.last_install_error = None;
                    let follow_up = if self.last_search_query.is_empty() {
                        Task::none()
                    } else {
                        Task::done(Message::RepeatLastSearch)
                    };
                    Action::PackageOperationFinished {
                        outcome,
                        reload: true,
                        follow_up,
                    }
                } else {
                    let error = outcome.error.clone().unwrap_or_else(|| outcome.summary());
                    log::error!("Failed to install packages: {}", error);
                    info.last_install_error = Some(error);
                    Action::PackageOperationFinished {
                        outcome,
                        reload: false,
                        follow_up: Task::none(),
                    }
                }
            }
        }
    }

    pub fn has_inspector_selection(&self) -> bool {
        self.inspected_package.is_some()
    }

    pub fn dismiss_transient(&mut self) -> bool {
        if self.inspected_package.take().is_some() {
            self.inspector_error = None;
            true
        } else {
            false
        }
    }

    pub fn refresh(&self, info: &FindingInfo) -> Option<Message> {
        (!self.last_search_query.is_empty()
            && !info.is_installing
            && info.searching_managers.is_empty())
        .then_some(Message::ExecuteSearch)
    }

    pub fn primary_action(&self, info: &FindingInfo) -> Option<Message> {
        (!info.selected_packages.is_empty()
            && !info.is_installing
            && info.searching_managers.is_empty())
        .then_some(Message::InstallSelectedPackages)
    }

    pub fn can_select_packages(&self, info: &FindingInfo) -> bool {
        !info.is_installing && !info.search_results.is_empty()
    }

    pub fn move_keyboard_selection(
        &self,
        info: &FindingInfo,
        catalog: &crate::manager_catalog::ManagerCatalog,
        direction: crate::shortcut::SelectionDirection,
    ) -> Option<Message> {
        let packages = self.keyboard_packages(info, catalog);
        crate::content::shared::next_keyboard_package(
            &packages,
            self.inspected_package.as_ref(),
            direction,
        )
        .map(|(manager, name)| Message::InspectPackage(manager, name))
    }

    pub fn toggle_keyboard_selection(&self, info: &FindingInfo) -> Option<Message> {
        let (manager, name) = self.inspected_package.as_ref()?;
        let package = info
            .search_results
            .get(manager)?
            .iter()
            .find(|package| package.name == *name)?;
        if info.is_installing || package.version.trim() != "Not Installed" {
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
        info: &FindingInfo,
        catalog: &crate::manager_catalog::ManagerCatalog,
    ) -> Vec<PackageSelectionKey> {
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
                let mut packages = info
                    .search_results
                    .get(&manager)
                    .into_iter()
                    .flatten()
                    .map(|package| package.name.clone())
                    .collect::<Vec<_>>();
                if info.sort_by == SortOption::Name {
                    packages.sort();
                }
                packages
                    .into_iter()
                    .map(move |name| (manager.clone(), name))
            })
            .collect()
    }

    pub fn view<'a>(
        &'a self,
        info: &'a FindingInfo,
        pm_config: &updater_core::Config,
        catalog: &'a crate::manager_catalog::ManagerCatalog,
        show_inspector: bool,
        inspector_drawer: bool,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, column, container, row, text};

        let result_count: usize = info.search_results.values().map(Vec::len).sum();
        let selected_sources = info.selected_managers.len();
        let can_search = !self.search_query.trim().is_empty()
            && selected_sources > 0
            && info.searching_managers.is_empty();
        let search_button = button(
            text(if info.searching_managers.is_empty() {
                "Search"
            } else {
                "Searching..."
            })
            .size(13)
            .font(theme::FONT_SEMIBOLD)
            .style(if can_search {
                theme::text_on_primary
            } else {
                theme::text_on_surface_alt
            }),
        )
        .padding([9, 16])
        .style(theme::action_button(
            can_search,
            theme::colors::ACCENT,
            theme::colors::ACCENT_HOVER,
            theme::colors::ACCENT_ACTIVE,
        ));
        let search_button = if can_search {
            search_button.on_press(Message::ExecuteSearch)
        } else {
            search_button
        };

        let search_input: iced::Element<'_, Message> = {
            let input =
                iced::widget::text_input("Enter package name to search...", &self.search_query)
                    .id(shared::search_input_id(
                        crate::content::ActiveContentPage::Finding,
                    ))
                    .on_input(Message::SearchQueryChanged)
                    .on_submit(Message::ExecuteSearch)
                    .padding([9, 11])
                    .size(14)
                    .style(theme::text_input_style);

            column![shared::section_title("Search"), input]
                .spacing(theme::spacing::SM)
                .into()
        };

        let manager_filter: iced::Element<'_, Message> = {
            let all_managers = shared::configured_managers(pm_config);
            let filters: iced::Element<'_, Message> = if all_managers.is_empty() {
                iced::widget::text("No package managers detected")
                    .size(13)
                    .style(theme::text_on_surface_muted)
                    .into()
            } else {
                row(all_managers.iter().map(|manager| {
                    let manager = manager.clone();
                    let display_name = catalog.display_name(&manager);
                    let is_selected = info.selected_managers.contains(&manager);
                    let is_searching = info.searching_managers.contains_key(&manager);
                    let label = if is_searching {
                        format!("{display_name} (Searching...)")
                    } else if info.search_errors.contains_key(&manager) {
                        format!("{display_name} (Failed)")
                    } else if let Some(results) = info.search_results.get(&manager) {
                        format!("{display_name} ({} results)", results.len())
                    } else {
                        display_name.to_owned()
                    };
                    let checkbox = iced::widget::checkbox(is_selected)
                        .label(label)
                        .spacing(8)
                        .text_size(13)
                        .style(shared::checkbox_style(is_searching));

                    if is_searching {
                        checkbox.into()
                    } else {
                        checkbox
                            .on_toggle(move |selected| {
                                Message::SelectPackageManager(manager.clone(), selected)
                            })
                            .into()
                    }
                }))
                .spacing(18)
                .width(iced::Length::Fill)
                .wrap()
                .vertical_spacing(10)
                .into()
            };

            column![shared::section_title("Sources"), filters]
                .spacing(theme::spacing::SM)
                .into()
        };

        let sort_order: iced::Element<'_, Message> = {
            let options = row(SortOption::ALL.iter().map(|option| {
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
                shared::segmented_group(options)
            ]
            .spacing(theme::spacing::SM)
            .into()
        };

        let toolbar = shared::toolbar(
            column![
                row![
                    container(search_input).width(iced::Length::Fill),
                    column![shared::section_title("Actions"), search_button]
                        .spacing(theme::spacing::SM),
                ]
                .spacing(theme::spacing::MD)
                .align_y(iced::Alignment::End),
                row![
                    container(manager_filter).width(iced::Length::FillPortion(2)),
                    container(sort_order).width(iced::Length::FillPortion(1)),
                ]
                .spacing(theme::spacing::LG)
                .align_y(iced::Alignment::Start),
            ]
            .spacing(theme::spacing::MD),
        );

        column![
            shared::page_header(
                "Find Packages",
                format!("{result_count} results from {selected_sources} selected sources"),
                theme::colors::DISCOVER,
            ),
            shared::summary_row([
                (format!("{result_count} results"), theme::colors::DISCOVER),
                (
                    format!("{selected_sources} sources selected"),
                    theme::colors::ON_SURFACE_MUTED,
                ),
                (
                    format!("{} packages selected", info.selected_packages.len()),
                    theme::colors::INSTALLED,
                ),
            ]),
            toolbar,
            self.batch_actions_view(info, catalog),
            self.search_results_view(info, catalog, show_inspector, inspector_drawer),
        ]
        .spacing(theme::spacing::LG)
        .height(iced::Length::Fill)
        .into()
    }

    fn search_results_view<'a>(
        &'a self,
        info: &'a FindingInfo,
        catalog: &'a crate::manager_catalog::ManagerCatalog,
        show_inspector: bool,
        inspector_drawer: bool,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, container, row, scrollable};

        if info.selected_managers.is_empty() {
            return shared::centered_message("Please select package managers to search from");
        }

        if self.last_search_query.is_empty() {
            return shared::centered_message("Enter a package name and click Search");
        }

        if !info.searching_managers.is_empty() {
            return shared::centered_message("Searching...");
        }

        let results_sections: Vec<iced::Element<'_, Message>> = info
            .selected_managers
            .iter()
            .filter_map(|pm_type| {
                if let Some(error) = info.search_errors.get(pm_type) {
                    Some(self.error_section(pm_type.clone(), error, catalog))
                } else {
                    info.search_results
                        .get(pm_type)
                        .filter(|packages| !packages.is_empty())
                        .map(|packages| {
                            self.package_manager_section(pm_type.clone(), packages, info, catalog)
                        })
                }
            })
            .collect();

        if results_sections.is_empty() {
            return shared::centered_message("No packages found");
        }

        let result_list = scrollable(column(results_sections).spacing(20))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill);
        let inspected = self.inspected_package.as_ref().and_then(|(manager, name)| {
            info.search_results
                .get(manager)
                .and_then(|packages| packages.iter().find(|package| package.name == *name))
                .map(|package| crate::content::shared::PackageInspector {
                    manager: manager.clone(),
                    name: &package.name,
                    version: &package.version,
                    available_version: None,
                    description: package.description.as_deref(),
                    size: package.size,
                    install_date: package.install_date.as_deref(),
                    homepage: package.homepage.as_deref(),
                })
        });
        let mut inspector = column![shared::package_inspector(
            inspected,
            catalog,
            Message::CopyInspectorText,
            Message::CopyInspectorText,
            Message::OpenHomepage,
        )]
        .height(iced::Length::Fill);
        if let Some(error) = &self.inspector_error {
            inspector = inspector.push(iced::widget::text(error).size(12).style(theme::text_error));
        }

        if !show_inspector {
            return container(result_list)
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
            return column![container(result_list).width(iced::Length::Fill), inspector]
                .spacing(theme::spacing::LG)
                .into();
        }

        row![
            container(result_list)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill),
            inspector,
        ]
        .spacing(theme::spacing::LG)
        .height(iced::Length::Fill)
        .into()
    }

    fn error_section<'a>(
        &self,
        manager_id: ManagerId,
        error: &'a str,
        catalog: &'a crate::manager_catalog::ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, text};

        let display_name = catalog.display_name(&manager_id).to_owned();
        column![
            text(display_name.clone())
                .size(18)
                .color(theme::colors::DISCOVER),
            shared::error_card(
                format!("Search failed in {display_name}"),
                error,
                Message::RetrySearch(manager_id),
            )
        ]
        .spacing(12)
        .into()
    }

    fn package_manager_section<'a>(
        &self,
        manager_id: ManagerId,
        packages: &'a [PackageInfo],
        info: &'a FindingInfo,
        catalog: &'a crate::manager_catalog::ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, row, text};

        let header = row![
            text(catalog.display_name(&manager_id).to_owned())
                .size(18)
                .color(theme::colors::DISCOVER),
            text(format!("({} results)", packages.len()))
                .size(16)
                .style(theme::text_on_surface_muted)
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        let sorted_packages = self.sort_packages(packages, info.sort_by);

        let packages_list = column(
            sorted_packages
                .into_iter()
                .map(|pkg| self.package_item_view(manager_id.clone(), pkg, info)),
        )
        .spacing(8);

        column![header, shared::styled_container(packages_list)]
            .spacing(12)
            .into()
    }

    fn sort_packages<'a>(
        &self,
        packages: &'a [PackageInfo],
        sort_by: SortOption,
    ) -> Vec<&'a PackageInfo> {
        let mut sorted: Vec<_> = packages.iter().collect();

        match sort_by {
            SortOption::Name => {
                sorted.sort_by(|a, b| a.name.cmp(&b.name));
            }
            SortOption::Relevance => {
                // Keep provider order for relevance sorting.
            }
        }

        sorted
    }

    fn package_item_view<'a>(
        &self,
        manager_id: ManagerId,
        package: &'a PackageInfo,
        info: &'a FindingInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, checkbox, row};

        let package_name = package.name.clone();
        let is_selected = info
            .selected_packages
            .contains(&shared::selection_key(&manager_id, &package.name));
        let is_not_installed = package.version.trim() == "Not Installed";

        let enable_install = !info.is_installing && is_not_installed;

        let checkbox = checkbox(is_selected)
            .on_toggle_maybe(if enable_install {
                Some({
                    let package_name = package_name.clone();
                    let manager_id = manager_id.clone();
                    move |selected| {
                        Message::TogglePackageSelection(
                            manager_id.clone(),
                            package_name.clone(),
                            selected,
                        )
                    }
                })
            } else {
                None
            })
            .size(18)
            .spacing(8)
            .style(shared::checkbox_style(false));

        let version_text = package.version.trim();

        let is_inspected = self
            .inspected_package
            .as_ref()
            .is_some_and(|(manager, name)| manager == &manager_id && name == &package.name);
        let mut summary = row![shared::package_summary(package)];
        if is_not_installed {
            summary = summary.push(shared::muted_badge("Not Installed"));
        } else if !version_text.is_empty() && version_text != "unknown" {
            summary = summary.push(shared::muted_badge(version_text));
        }
        let details = button(summary.spacing(16).align_y(iced::Alignment::Center))
            .padding([8, 10])
            .width(iced::Length::Fill)
            .style(theme::list_row(is_inspected))
            .on_press(Message::InspectPackage(manager_id, package_name));

        row![checkbox, details]
            .spacing(theme::spacing::SM)
            .align_y(iced::Alignment::Center)
            .into()
    }

    fn batch_actions_view<'a>(
        &self,
        info: &'a FindingInfo,
        catalog: &'a crate::manager_catalog::ManagerCatalog,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{button, checkbox, column, row, text};

        let selected_count = info.selected_packages.len();
        let selectable_count: usize = info
            .selected_managers
            .iter()
            .filter_map(|manager| info.search_results.get(manager))
            .flatten()
            .filter(|package| package.version.trim() == "Not Installed")
            .count();
        let all_selected = selectable_count > 0 && selected_count == selectable_count;
        let is_enabled = selected_count > 0 && !info.is_installing;

        let button_text = if info.is_installing {
            if let Some((completed, total, manager, package)) = &info.install_progress {
                if package.is_empty() {
                    format!("Installing {}/{}...", completed, total)
                } else {
                    format!(
                        "Installing {}/{}: {} ({})",
                        completed,
                        total,
                        package,
                        catalog.display_name(manager)
                    )
                }
            } else {
                "Installing...".to_string()
            }
        } else if selected_count > 0 {
            format!("Install {} package(s)", selected_count)
        } else {
            "Install Selected".to_string()
        };

        let install_button = button(text(button_text).size(14).font(theme::FONT_SEMIBOLD).style(
            if is_enabled {
                theme::text_on_primary
            } else {
                theme::text_on_surface_muted
            },
        ))
        .padding([8, 16])
        .style(theme::action_button(
            is_enabled,
            theme::colors::INSTALL_ACTION,
            theme::colors::INSTALL_ACTION_HOVER,
            theme::colors::INSTALL_ACTION_ACTIVE,
        ));

        let install_button = if is_enabled {
            install_button.on_press(Message::InstallSelectedPackages)
        } else {
            install_button
        };

        let select_all_checkbox = checkbox(all_selected)
            .label("Select All")
            .on_toggle_maybe((!info.is_installing).then_some(Message::ToggleSelectAll))
            .size(18)
            .spacing(8)
            .text_size(14)
            .style(shared::checkbox_style(false));

        let actions_row = row![select_all_checkbox, install_button]
            .spacing(12)
            .align_y(iced::Alignment::Center);

        if let Some(error) = &info.last_install_error {
            column![
                actions_row,
                text(format!("Install failed: {error}"))
                    .size(13)
                    .style(theme::text_error)
            ]
            .spacing(8)
            .into()
        } else {
            actions_row.into()
        }
    }

    fn start_search(
        &mut self,
        pm_config: &updater_core::Config,
        info: &mut FindingInfo,
        query: &str,
        catalog: &crate::manager_catalog::ManagerCatalog,
    ) -> Action {
        info.search_results.clear();
        info.selected_packages.clear();
        info.searching_managers.clear();
        info.search_errors.clear();
        self.last_search_query = query.to_owned();
        info.request_generation = info.request_generation.wrapping_add(1);
        let request_id = info.request_generation;
        for manager in &info.selected_managers {
            info.searching_managers.insert(manager.clone(), request_id);
        }

        Action::Run(Self::execute_search_task(
            pm_config,
            &info.selected_managers,
            query,
            catalog,
            request_id,
        ))
    }

    fn execute_search_task(
        pm_config: &updater_core::Config,
        selected_managers: &HashSet<ManagerId>,
        query: &str,
        catalog: &crate::manager_catalog::ManagerCatalog,
        request_id: u64,
    ) -> Task<Message> {
        let pm_config = pm_config.clone();
        let query = query.to_string();
        let managers: Vec<_> = selected_managers.iter().cloned().collect();
        let registry = catalog.registry();

        let tasks: Vec<_> = managers
            .into_iter()
            .map(|manager_id| {
                let pm_config = pm_config.clone();
                let query = query.clone();
                let registry = registry.clone();
                Task::future(async move {
                    let result = async {
                        let manager = registry
                            .manager_for(&manager_id, ManagerCapability::Search)
                            .map_err(|error| error.to_string())?;
                        let manager_config = pm_config
                            .manager(&manager_id)
                            .ok_or_else(|| format!("Manager is not configured: {manager_id}"))?;
                        manager
                            .search(manager_config, &query)
                            .await
                            .map_err(|error| {
                                format!("Failed to search in {}: {error}", manager_id.as_str())
                            })
                    }
                    .await;
                    (manager_id, result)
                })
                .then(move |(manager_id, result)| {
                    Task::done(Message::SearchResult {
                        request_id,
                        manager: manager_id,
                        result,
                    })
                })
            })
            .collect();

        Task::batch(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_id(value: &str) -> ManagerId {
        ManagerId::parse(value).unwrap()
    }

    #[test]
    fn search_result_only_applies_to_the_active_request() {
        let mut finding = Finding::default();
        let mut info = FindingInfo::default();
        let manager = manager_id("builtin:cargo");
        info.searching_managers.insert(manager.clone(), 2);

        let _ = finding.update(
            Message::SearchResult {
                request_id: 1,
                manager: manager.clone(),
                result: Err("stale result".to_owned()),
            },
            &updater_core::Config::default(),
            &mut info,
            &crate::manager_catalog::ManagerCatalog::builtin(),
        );

        assert_eq!(info.searching_managers.get(&manager), Some(&2));
        assert!(info.search_errors.is_empty());

        let _ = finding.update(
            Message::SearchResult {
                request_id: 2,
                manager: manager.clone(),
                result: Err("current result".to_owned()),
            },
            &updater_core::Config::default(),
            &mut info,
            &crate::manager_catalog::ManagerCatalog::builtin(),
        );

        assert!(!info.searching_managers.contains_key(&manager));
        assert_eq!(
            info.search_errors.get(&manager).map(String::as_str),
            Some("current result")
        );
    }

    #[test]
    fn deselected_manager_cannot_reinsert_a_late_search_result() {
        let mut finding = Finding::default();
        let mut info = FindingInfo::default();
        let manager = manager_id("builtin:cargo");
        info.selected_managers.insert(manager.clone());
        info.searching_managers.insert(manager.clone(), 1);

        let _ = finding.update(
            Message::SelectPackageManager(manager.clone(), false),
            &updater_core::Config::default(),
            &mut info,
            &crate::manager_catalog::ManagerCatalog::builtin(),
        );
        let _ = finding.update(
            Message::SearchResult {
                request_id: 1,
                manager: manager.clone(),
                result: Err("late result".to_owned()),
            },
            &updater_core::Config::default(),
            &mut info,
            &crate::manager_catalog::ManagerCatalog::builtin(),
        );

        assert!(!info.selected_managers.contains(&manager));
        assert!(!info.searching_managers.contains_key(&manager));
        assert!(!info.search_errors.contains_key(&manager));
        assert!(!info.search_results.contains_key(&manager));
    }

    #[test]
    fn repeating_last_search_allocates_a_fresh_request() {
        let mut finding = Finding {
            last_search_query: "ripgrep".to_owned(),
            ..Finding::default()
        };
        let mut info = FindingInfo::default();
        let manager = manager_id("builtin:cargo");
        info.selected_managers.insert(manager.clone());

        let action = finding.update(
            Message::RepeatLastSearch,
            &updater_core::Config::default(),
            &mut info,
            &crate::manager_catalog::ManagerCatalog::builtin(),
        );

        assert!(matches!(action, Action::Run(_)));
        assert_eq!(info.request_generation, 1);
        assert_eq!(info.searching_managers.get(&manager), Some(&1));
        assert_eq!(finding.last_search_query, "ripgrep");
    }
}
