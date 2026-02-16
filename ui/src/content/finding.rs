// Finding/Search packages view with filtering, sorting and installation capabilities

use std::collections::{HashMap, HashSet};

use iced::{Border, Task};
use updater_core::{PackageInfo, PackageManagerType};

use crate::{app, content, content::shared::SharedUi};

#[derive(Debug, Clone, Default)]
pub struct Finding {
    search_query: String,
    last_search_query: String, // Track last executed search
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectPackageManager(PackageManagerType, bool),
    SearchQueryChanged(String),
    ExecuteSearch,
    SearchResult(PackageManagerType, Result<Vec<PackageInfo>, String>),
    SortOptionChanged(SortOption),
    TogglePackageSelection(String, bool),
    InstallSelectedPackages,
    InstallPackagesResult(Result<(), String>),
}

#[derive(Debug, Clone, Default)]
pub struct FindingInfo {
    pub search_results: HashMap<PackageManagerType, Vec<PackageInfo>>,
    pub selected_managers: HashSet<PackageManagerType>,
    pub searching_managers: HashSet<PackageManagerType>,
    pub sort_by: SortOption,
    pub selected_packages: HashSet<String>,
    pub is_installing: bool,
}

impl From<Message> for content::Message {
    fn from(msg: Message) -> Self {
        content::Message::Finding(msg)
    }
}

pub enum Action {
    None,
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
    pub fn update(&mut self, message: Message, info: &mut FindingInfo) -> Action {
        match message {
            Message::SelectPackageManager(pm_type, selected) => {
                if selected {
                    info.selected_managers.insert(pm_type);
                } else {
                    info.selected_managers.remove(&pm_type);
                    info.searching_managers.remove(&pm_type);
                    if let Some(packages) = info.search_results.remove(&pm_type) {
                        for package in packages {
                            info.selected_packages.remove(&package.name);
                        }
                    }
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

                // Only search in selected managers
                if info.selected_managers.is_empty() {
                    return Action::None;
                }

                // Clear previous results before running a new search
                info.search_results.clear();
                info.selected_packages.clear();
                info.searching_managers.clear();
                self.last_search_query = query.to_string();

                // Mark all selected managers as searching
                for pm_type in info.selected_managers.iter() {
                    info.searching_managers.insert(*pm_type);
                }

                Self::execute_search_action(
                    &updater_core::Config::default(),
                    &info.selected_managers,
                    query,
                )
            }
            Message::SearchResult(pm_type, result) => {
                info.searching_managers.remove(&pm_type);
                match result {
                    Ok(packages) => {
                        info.search_results.insert(pm_type, packages);
                    }
                    Err(_) => {
                        info.search_results.insert(pm_type, Vec::new());
                    }
                }
                Action::None
            }
            Message::SortOptionChanged(sort_option) => {
                info.sort_by = sort_option;
                Action::None
            }
            Message::TogglePackageSelection(package_name, selected) => {
                if selected {
                    info.selected_packages.insert(package_name);
                } else {
                    info.selected_packages.remove(&package_name);
                }
                Action::None
            }
            Message::InstallSelectedPackages => {
                if info.selected_packages.is_empty() {
                    return Action::None;
                }
                info.is_installing = true;
                Self::install_packages_action(&updater_core::Config::default(), info)
            }
            Message::InstallPackagesResult(result) => {
                info.is_installing = false;
                match result {
                    Ok(_) => {
                        info.selected_packages.clear();
                        // Re-execute search to update package status
                        if !self.last_search_query.is_empty() {
                            // Mark all selected managers as searching
                            for pm_type in info.selected_managers.iter() {
                                info.searching_managers.insert(*pm_type);
                            }
                            return Self::execute_search_action(
                                &updater_core::Config::default(),
                                &info.selected_managers,
                                &self.last_search_query,
                            );
                        }
                        Action::None
                    }
                    Err(e) => {
                        eprintln!("Failed to install packages: {}", e);
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

    // === View components ===

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

        let mut all_managers = Vec::new();
        if let Some(system_manager) = &pm_config.system_manager {
            all_managers.push(system_manager.manager_type);
        }
        for app_manager in &pm_config.app_managers {
            all_managers.push(app_manager.manager_type);
        }

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

        let total_results: usize = info
            .search_results
            .values()
            .map(|packages| packages.len())
            .sum();

        if total_results == 0 {
            return SharedUi::centered_message("No packages found");
        }

        let results_sections: Vec<iced::Element<'_, Message>> = info
            .selected_managers
            .iter()
            .filter_map(|pm_type| {
                info.search_results
                    .get(pm_type)
                    .map(|packages| (*pm_type, packages))
            })
            .filter(|(_, packages)| !packages.is_empty())
            .map(|(pm_type, packages)| self.package_manager_section(pm_type, packages, info))
            .collect();

        scrollable(column(results_sections).spacing(20))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
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
                .map(|pkg| self.package_item_view(pkg, info)),
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
                // For relevance, keep original order (usually search engines return by relevance)
                // or do a more sophisticated relevance sort
            }
        }

        sorted
    }

    fn package_item_view<'a>(
        &self,
        package: &'a PackageInfo,
        info: &'a FindingInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{checkbox, column, container, row, text};

        let package_name = package.name.clone();
        let is_selected = info.selected_packages.contains(&package.name);
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
                    move |selected| Message::TogglePackageSelection(package_name.clone(), selected)
                })
            } else {
                None
            })
            .size(18)
            .spacing(8)
            .style(SharedUi::checkbox_style(false));

        let version_text = package.version.trim();

        let main_row = if is_not_installed {
            // Show "Not Installed" badge with distinct styling
            row![
                checkbox,
                name_with_desc,
                container(
                    text("Not Installed")
                        .size(12)
                        .color(iced::Color::from_rgb8(180, 180, 180))
                )
                .padding([4, 8])
                .style(|_theme: &iced::Theme| {
                    use iced::widget::container::Style;
                    Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb8(50, 50, 50))),
                        border: Border {
                            color: iced::Color::from_rgb8(80, 80, 80),
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
            // Show installed version with green badge
            row![
                checkbox,
                name_with_desc,
                container(
                    text(version_text)
                        .size(12)
                        .color(iced::Color::from_rgb8(200, 255, 200))
                )
                .padding([4, 8])
                .style(|_theme: &iced::Theme| {
                    use iced::widget::container::Style;
                    Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb8(30, 70, 30))),
                        border: Border {
                            color: iced::Color::from_rgb8(50, 120, 50),
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
            "Installing...".to_string()
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

    // === Action creators ===

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
        let pm_config = pm_config.clone();
        let selected_packages = info.selected_packages.clone();

        // Group packages by their package manager
        let mut packages_by_manager: HashMap<PackageManagerType, Vec<String>> = HashMap::new();

        for (pm_type, packages) in info.search_results.iter() {
            for pkg in packages {
                if selected_packages.contains(&pkg.name) {
                    packages_by_manager
                        .entry(*pm_type)
                        .or_default()
                        .push(pkg.name.clone());
                }
            }
        }

        let task = Task::future(async move {
            for (pm_type, package_names) in packages_by_manager {
                if let Err(e) = pm_type.install_packages(&pm_config, &package_names).await {
                    return Err(format!(
                        "Failed to install packages from {}: {}",
                        pm_type.name(),
                        e
                    ));
                }
            }
            Ok(())
        })
        .then(|result| Task::done(Message::InstallPackagesResult(result)));

        Action::Run(task)
    }
}
