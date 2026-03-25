// Finding/Search packages view with filtering, sorting and installation capabilities

use std::collections::{HashMap, HashSet};

use iced::{Border, Task};
use updater_core::{PackageInfo, PackageManagerType};

use crate::{
    app, content,
    content::errors::{ManagerErrors, apply_manager_items_result},
    content::shared::{PackageSelectionKey, SharedUi},
    content::workflows::{
        BatchProgress, PackageBatchAction, collect_selected_package_groups, push_command_log,
        run_grouped_package_action,
    },
};

#[derive(Debug, Clone, Default)]
pub struct Finding {
    /// Search query being edited by user.
    search_query: String,
    /// Last executed query used for post-install refresh.
    last_search_query: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Package-manager selection message.
    SelectPackageManager(PackageManagerType, bool),
    /// Search-query change message.
    SearchQueryChanged(String),
    /// Search execution message.
    ExecuteSearch,
    /// Search result message.
    SearchResult(PackageManagerType, Result<Vec<PackageInfo>, String>),
    /// Sort-option change message.
    SortOptionChanged(SortOption),
    /// Package-selection toggle message.
    TogglePackageSelection(PackageManagerType, String, bool),
    /// Install-selected message.
    InstallSelectedPackages,
    /// Install progress message.
    InstallProgress {
        /// Number of finished packages.
        completed: usize,
        /// Total packages to install.
        total: usize,
        /// Manager currently executing command.
        manager: PackageManagerType,
        /// Current package being processed.
        current_package: String,
        /// Optional command output/status line.
        command_message: Option<String>,
    },
    /// Install result message.
    InstallPackagesResult(Result<(), String>),
}

#[derive(Debug, Clone, Default)]
pub struct FindingInfo {
    /// Search results grouped by manager.
    pub search_results: HashMap<PackageManagerType, Vec<PackageInfo>>,
    /// Search errors grouped by manager.
    pub search_errors: ManagerErrors,
    /// Managers selected in the filter panel.
    pub selected_managers: HashSet<PackageManagerType>,
    /// Managers currently running search.
    pub searching_managers: HashSet<PackageManagerType>,
    /// Current sort option.
    pub sort_by: SortOption,
    /// Selected package keys for batch operations.
    pub selected_packages: HashSet<PackageSelectionKey>,
    /// Whether install operation is in progress.
    pub is_installing: bool,
    /// Install progress `(completed, total, manager, package)`.
    pub install_progress: Option<(usize, usize, PackageManagerType, String)>,
    /// Install command logs.
    pub install_logs: Vec<String>,
}

impl From<Message> for content::Message {
    fn from(msg: Message) -> Self {
        content::Message::Finding(msg)
    }
}

pub enum Action {
    /// No-op action.
    None,
    /// Asynchronous task action.
    Run(iced::Task<Message>),
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
                        .retain(|(manager, _)| *manager != pm_type);
                    info.search_results.remove(&pm_type);
                }
                Action::None
            }
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
                Action::None
            }
            Message::ExecuteSearch => {
                let query = self.search_query.trim();
                if query.is_empty() {
                    return Action::None;
                }

                // Search only in selected managers.
                if info.selected_managers.is_empty() {
                    return Action::None;
                }

                // Clear previous results before running a new search.
                info.search_results.clear();
                info.selected_packages.clear();
                info.searching_managers.clear();
                info.search_errors.clear();
                self.last_search_query = query.to_string();

                // Mark selected managers as searching.
                for pm_type in info.selected_managers.iter() {
                    info.searching_managers.insert(*pm_type);
                }

                Self::execute_search_action(pm_config, &info.selected_managers, query)
            }
            Message::SearchResult(pm_type, result) => {
                info.searching_managers.remove(&pm_type);
                apply_manager_items_result(
                    &mut info.search_results,
                    &mut info.search_errors,
                    pm_type,
                    result,
                );
                Action::None
            }
            Message::SortOptionChanged(sort_option) => {
                info.sort_by = sort_option;
                Action::None
            }
            Message::TogglePackageSelection(pm_type, package_name, selected) => {
                let key = SharedUi::selection_key(pm_type, &package_name);
                if selected {
                    info.selected_packages.insert(key);
                } else {
                    info.selected_packages.remove(&key);
                }
                Action::None
            }
            Message::InstallSelectedPackages => {
                if info.selected_packages.is_empty() {
                    return Action::None;
                }
                info.is_installing = true;
                info.install_logs.clear();
                let initial_manager = info
                    .selected_packages
                    .iter()
                    .next()
                    .map(|(pm_type, _)| *pm_type)
                    .unwrap_or(PackageManagerType::Dnf);
                info.install_progress = Some((
                    0,
                    info.selected_packages.len(),
                    initial_manager,
                    String::new(),
                ));
                Self::install_packages_action(pm_config, info)
            }
            Message::InstallProgress {
                completed,
                total,
                manager,
                current_package,
                command_message,
            } => {
                info.install_progress = Some((completed, total, manager, current_package));
                if let Some(command_message) = command_message {
                    push_command_log(
                        &mut info.install_logs,
                        PackageBatchAction::Install,
                        manager,
                        info.install_progress
                            .as_ref()
                            .map_or("", |(_, _, _, package)| package.as_str()),
                        command_message,
                    );
                }
                Action::None
            }
            Message::InstallPackagesResult(result) => {
                info.is_installing = false;
                info.install_progress = None;
                match result {
                    Ok(_) => {
                        info.selected_packages.clear();
                        // Re-run search to refresh package status.
                        if !self.last_search_query.is_empty() {
                            // Mark selected managers as searching.
                            for pm_type in info.selected_managers.iter() {
                                info.searching_managers.insert(*pm_type);
                            }
                            return Self::execute_search_action(
                                pm_config,
                                &info.selected_managers,
                                &self.last_search_query,
                            );
                        }
                        Action::None
                    }
                    Err(e) => {
                        log::error!("Failed to install packages: {}", e);
                        Action::None
                    }
                }
            }
        }
    }

    pub fn view<'a>(
        &self,
        info: &'a FindingInfo,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, container, row};

        row![
            container(
                column![
                    self.manager_filter_view(info, pm_config),
                    self.sort_order_view(info),
                ]
                .spacing(24)
            )
            .width(iced::Length::FillPortion(1)),
            container(
                column![
                    self.search_input_view(),
                    self.batch_actions_view(info),
                    self.search_results_view(info)
                ]
                .spacing(20)
            )
            .width(iced::Length::FillPortion(3))
        ]
        .spacing(24)
        .into()
    }

    // View components.

    fn manager_filter_view<'a>(
        &self,
        info: &'a FindingInfo,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'a, Message> {
        use iced::widget::column;

        let filters_content = self.active_filter_view(info, pm_config);

        column![
            SharedUi::section_title("Search Sources"),
            SharedUi::styled_container(filters_content)
        ]
        .spacing(12)
        .into()
    }

    fn active_filter_view<'a>(
        &self,
        info: &'a FindingInfo,
        pm_config: &updater_core::Config,
    ) -> iced::widget::Column<'a, Message> {
        use iced::widget::{column, text};

        let all_managers = SharedUi::configured_managers(pm_config);

        if all_managers.is_empty() {
            return column![
                text("No package managers detected")
                    .size(14)
                    .color(app::colors::ON_SURFACE_MUTED)
            ]
            .spacing(8);
        }

        column(all_managers.iter().map(|pm_type| {
            let pm_type = *pm_type;
            let is_selected = info.selected_managers.contains(&pm_type);
            let is_searching = info.searching_managers.contains(&pm_type);

            let label = if is_searching {
                format!("{} (Searching...)", pm_type.name())
            } else if info.search_errors.contains_key(&pm_type) {
                format!("{} (Failed)", pm_type.name())
            } else if let Some(results) = info.search_results.get(&pm_type) {
                format!("{} ({} results)", pm_type.name(), results.len())
            } else {
                pm_type.name().to_string()
            };

            let checkbox = iced::widget::checkbox(is_selected)
                .label(label)
                .spacing(10)
                .text_size(15)
                .style(SharedUi::checkbox_style(is_searching));

            if is_searching {
                checkbox.into()
            } else {
                checkbox
                    .on_toggle(move |selected| Message::SelectPackageManager(pm_type, selected))
                    .into()
            }
        }))
        .spacing(12)
    }

    fn sort_order_view<'a>(&self, info: &'a FindingInfo) -> iced::Element<'a, Message> {
        use iced::widget::column;

        let sort_options = column(SortOption::ALL.iter().map(|option| {
            let option = *option;
            iced::widget::radio(
                option.name(),
                option,
                Some(info.sort_by),
                Message::SortOptionChanged,
            )
            .size(15)
            .spacing(10)
            .text_size(15)
            .style(SharedUi::radio_style)
            .into()
        }))
        .spacing(12);

        column![
            SharedUi::section_title("Sort By"),
            SharedUi::styled_container(sort_options)
        ]
        .spacing(12)
        .into()
    }

    fn search_input_view(&self) -> iced::Element<'static, Message> {
        SharedUi::search_input_view_with_submit(
            "Search",
            "Enter package name to search...",
            &self.search_query,
            Message::SearchQueryChanged,
            Message::ExecuteSearch,
        )
    }

    fn search_results_view<'a>(&self, info: &'a FindingInfo) -> iced::Element<'a, Message> {
        use iced::widget::{column, scrollable};

        if info.selected_managers.is_empty() {
            return SharedUi::centered_message("Please select package managers to search from");
        }

        if self.last_search_query.is_empty() {
            return SharedUi::centered_message("Enter a package name and click Search");
        }

        if !info.searching_managers.is_empty() {
            return SharedUi::centered_message("Searching...");
        }

        let results_sections: Vec<iced::Element<'_, Message>> = info
            .selected_managers
            .iter()
            .filter_map(|pm_type| {
                if let Some(error) = info.search_errors.get(pm_type) {
                    Some(self.error_section(*pm_type, error))
                } else {
                    info.search_results
                        .get(pm_type)
                        .filter(|packages| !packages.is_empty())
                        .map(|packages| self.package_manager_section(*pm_type, packages, info))
                }
            })
            .collect();

        if results_sections.is_empty() {
            return SharedUi::centered_message("No packages found");
        }

        scrollable(column(results_sections).spacing(20))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    fn error_section<'a>(
        &self,
        pm_type: PackageManagerType,
        error: &'a str,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, text};

        column![
            text(pm_type.name()).size(18).color(app::colors::SECONDARY),
            SharedUi::styled_container(
                text(format!("Search failed: {}", error))
                    .size(14)
                    .color(app::colors::ERROR),
            )
        ]
        .spacing(12)
        .into()
    }

    fn package_manager_section<'a>(
        &self,
        pm_type: PackageManagerType,
        packages: &'a [PackageInfo],
        info: &'a FindingInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, row, text};

        let header = row![
            text(pm_type.name()).size(18).color(app::colors::SECONDARY),
            text(format!("({} results)", packages.len()))
                .size(16)
                .color(app::colors::ON_SURFACE_MUTED)
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        let sorted_packages = self.sort_packages(packages, info.sort_by);

        let packages_list = column(
            sorted_packages
                .into_iter()
                .map(|pkg| self.package_item_view(pm_type, pkg, info)),
        )
        .spacing(8);

        column![header, SharedUi::styled_container(packages_list)]
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
        pm_type: PackageManagerType,
        package: &'a PackageInfo,
        info: &'a FindingInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{checkbox, column, container, row, text};

        let package_name = package.name.clone();
        let is_selected = info
            .selected_packages
            .contains(&SharedUi::selection_key(pm_type, &package.name));
        let is_not_installed = package.version.trim() == "Not Installed";

        let mut name_with_desc =
            column![text(&package.name).size(15).color(app::colors::ON_SURFACE),]
                .spacing(4)
                .width(iced::Length::Fill);

        if let Some(description) = &package.description {
            name_with_desc = name_with_desc.push(
                text(description)
                    .size(13)
                    .color(app::colors::ON_SURFACE_MUTED),
            );
        };

        let enable_install = !info.is_installing && is_not_installed;

        let checkbox = checkbox(is_selected)
            .on_toggle_maybe(if enable_install {
                Some({
                    let package_name = package_name.clone();
                    move |selected| {
                        Message::TogglePackageSelection(pm_type, package_name.clone(), selected)
                    }
                })
            } else {
                None
            })
            .size(18)
            .spacing(8)
            .style(SharedUi::checkbox_style(false));

        let version_text = package.version.trim();

        let main_row = if is_not_installed {
            // Render a dedicated badge for not-installed packages.
            row![
                checkbox,
                name_with_desc,
                container(
                    text("Not Installed")
                        .size(12)
                        .color(app::colors::ON_SURFACE_MUTED)
                )
                .padding([4, 8])
                .style(|_theme: &iced::Theme| {
                    use iced::widget::container::Style;
                    Style {
                        background: Some(iced::Background::Color(app::colors::SURFACE_MUTED)),
                        border: Border {
                            color: app::colors::DIVIDER,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        text_color: None,
                        shadow: Default::default(),
                        snap: Default::default(),
                    }
                })
            ]
        } else if !version_text.is_empty() && version_text != "unknown" {
            // Render result version (not necessarily installed version).
            row![
                checkbox,
                name_with_desc,
                container(
                    text(version_text)
                        .size(12)
                        .color(app::colors::ON_SURFACE_MUTED)
                )
                .padding([4, 8])
                .style(|_theme: &iced::Theme| {
                    use iced::widget::container::Style;
                    Style {
                        background: Some(iced::Background::Color(app::colors::SURFACE_MUTED)),
                        border: Border {
                            color: app::colors::DIVIDER,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        text_color: None,
                        shadow: Default::default(),
                        snap: Default::default(),
                    }
                })
            ]
        } else {
            row![checkbox, name_with_desc, text("")]
        };

        main_row
            .spacing(16)
            .align_y(iced::Alignment::Center)
            .padding([8, 0])
            .into()
    }

    fn batch_actions_view<'a>(&self, info: &'a FindingInfo) -> iced::Element<'a, Message> {
        use iced::widget::{button, row, text};

        let selected_count = info.selected_packages.len();
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
                        manager.name()
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

        let install_button = button(text(button_text).size(14).color(if is_enabled {
            iced::Color::WHITE
        } else {
            app::colors::ON_SURFACE_MUTED
        }))
        .padding([8, 16])
        .style(move |_theme, status| {
            use iced::widget::button::Style;
            if !is_enabled {
                Style {
                    background: Some(iced::Background::Color(app::colors::SURFACE_MUTED)),
                    text_color: app::colors::ON_SURFACE_MUTED,
                    border: Border {
                        color: app::colors::DIVIDER,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..Default::default()
                }
            } else {
                let base_color = iced::Color::from_rgb8(13, 110, 253);
                match status {
                    iced::widget::button::Status::Hovered => Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb8(
                            11, 94, 215,
                        ))),
                        text_color: iced::Color::WHITE,
                        border: Border {
                            color: iced::Color::TRANSPARENT,
                            width: 0.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    },
                    _ => Style {
                        background: Some(iced::Background::Color(base_color)),
                        text_color: iced::Color::WHITE,
                        border: Border {
                            color: iced::Color::TRANSPARENT,
                            width: 0.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    },
                }
            }
        });

        let install_button = if is_enabled {
            install_button.on_press(Message::InstallSelectedPackages)
        } else {
            install_button
        };

        row![install_button]
            .spacing(12)
            .align_y(iced::Alignment::Center)
            .into()
    }

    // Action creators.

    fn execute_search_action(
        pm_config: &updater_core::Config,
        selected_managers: &HashSet<PackageManagerType>,
        query: &str,
    ) -> Action {
        let pm_config = pm_config.clone();
        let query = query.to_string();
        let managers: Vec<_> = selected_managers.iter().copied().collect();

        let tasks: Vec<_> = managers
            .into_iter()
            .map(|pm_type| {
                let pm_config = pm_config.clone();
                let query = query.clone();
                Task::future(async move {
                    let result = pm_type
                        .search_package(&pm_config, &query)
                        .await
                        .map_err(|e| format!("Failed to search in {}: {}", pm_type.name(), e));
                    (pm_type, result)
                })
                .then(move |(pm_type, result)| Task::done(Message::SearchResult(pm_type, result)))
            })
            .collect();

        Action::Run(Task::batch(tasks))
    }

    fn install_packages_action(pm_config: &updater_core::Config, info: &FindingInfo) -> Action {
        let manager_groups = collect_selected_package_groups(
            info.search_results
                .iter()
                .map(|(pm_type, packages)| (*pm_type, packages.as_slice())),
            &info.selected_packages,
            |package| package.name.as_str(),
        );

        Action::Run(run_grouped_package_action(
            pm_config,
            PackageBatchAction::Install,
            manager_groups,
            |BatchProgress {
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
        ))
    }
}
