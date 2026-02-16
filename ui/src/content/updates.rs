// Updates view with filtering, sorting, and search capabilities.

use std::collections::{HashMap, HashSet};

use iced::{Border, Task};
use updater_core::{PackageManagerType, PackageUpdate};

use crate::{app, content, content::shared::SharedUi};

#[derive(Debug, Clone, Default)]
pub struct Updates {
    search_query: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectPackageManager(PackageManagerType, bool),
    LoadUpdatesResult(PackageManagerType, Result<Vec<PackageUpdate>, String>),
    SearchQueryChanged(String),
    SortOptionChanged(SortOption),
    TogglePackageSelection(String, bool),
    ToggleSelectAll(bool),
    UpdateSelectedPackages,
    UpdatePackagesResult(Result<(), String>),
    RefreshInfo,
}

#[derive(Debug, Clone, Default)]
pub struct UpdatesInfo {
    pub updates_by_manager: HashMap<PackageManagerType, (usize, Vec<PackageUpdate>)>,
    pub selected_managers: HashSet<PackageManagerType>,
    pub loading_updates: HashSet<PackageManagerType>,
    pub is_loading_count: bool,
    pub has_loading_count: bool,
    pub sort_by: SortOption,
    pub selected_packages: HashSet<String>,
    pub is_updating: bool,
}

impl From<Message> for content::Message {
    fn from(msg: Message) -> Self {
        content::Message::Updates(msg)
    }
}

pub enum Action {
    None,
    Run(iced::Task<Message>),
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

    #[allow(dead_code)]
    pub fn description(&self) -> &'static str {
        match self {
            SortOption::Name => "Sort by package name",
            SortOption::CurrentVersion => "Sort by current version",
            SortOption::NewVersion => "Sort by new version",
        }
    }

    pub const ALL: [SortOption; 3] = [
        SortOption::Name,
        SortOption::CurrentVersion,
        SortOption::NewVersion,
    ];
}

impl Updates {
    pub fn update(&mut self, message: Message, info: &mut UpdatesInfo) -> Action {
        match message {
            Message::SelectPackageManager(pm_type, selected) => {
                if selected {
                    info.selected_managers.insert(pm_type);

                    if info.loading_updates.contains(&pm_type) {
                        Action::None
                    } else if let Some((count, packages)) = info.updates_by_manager.get(&pm_type) {
                        if *count == packages.len() {
                            Action::None
                        } else {
                            info.loading_updates.insert(pm_type);
                            Self::load_updates_action(&updater_core::Config::default(), pm_type)
                        }
                    } else {
                        info.loading_updates.insert(pm_type);
                        Self::load_updates_action(&updater_core::Config::default(), pm_type)
                    }
                } else {
                    info.selected_managers.remove(&pm_type);
                    Action::None
                }
            }
            Message::LoadUpdatesResult(pm_type, result) => {
                info.loading_updates.remove(&pm_type);
                match result {
                    Ok(packages) => {
                        let count = packages.len();
                        info.updates_by_manager.insert(pm_type, (count, packages));
                    }
                    Err(_) => {
                        info.updates_by_manager.insert(pm_type, (0, Vec::new()));
                    }
                }
                Action::None
            }
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
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
            Message::ToggleSelectAll(select_all) => {
                if select_all {
                    // Select all visible packages from selected managers
                    for pm_type in &info.selected_managers {
                        if let Some((_, packages)) = info.updates_by_manager.get(pm_type) {
                            for pkg in packages {
                                info.selected_packages.insert(pkg.name.clone());
                            }
                        }
                    }
                } else {
                    // Deselect all
                    info.selected_packages.clear();
                }
                Action::None
            }
            Message::UpdateSelectedPackages => {
                if info.selected_packages.is_empty() {
                    return Action::None;
                }
                info.is_updating = true;
                Self::update_packages_action(&updater_core::Config::default(), info)
            }
            Message::UpdatePackagesResult(result) => {
                info.is_updating = false;
                match result {
                    Ok(_) => {
                        info.selected_packages.clear();
                        // Reload updates after successful update
                        // Trigger refresh to reload all package manager data
                        let pm_types: Vec<PackageManagerType> = info.selected_managers.iter().copied().collect();
                        
                        if pm_types.is_empty() {
                            return Action::None;
                        }
                        
                        // Set loading state for selected package managers
                        for pm_type in &pm_types {
                            info.loading_updates.insert(*pm_type);
                        }
                        
                        // Create loading tasks for selected package managers
                        let tasks: Vec<Task<Message>> = pm_types
                            .into_iter()
                            .map(|pm_type| {
                                Self::create_load_task(&updater_core::Config::default(), pm_type)
                            })
                            .collect();
                        
                        Action::Run(Task::batch(tasks))
                    }
                    Err(e) => {
                        eprintln!("Failed to update packages: {}", e);
                        Action::None
                    }
                }
            }
            Message::RefreshInfo => {
                let pm_types: Vec<PackageManagerType> = info.updates_by_manager.keys().copied().collect();
                
                if pm_types.is_empty() {
                    return Action::None;
                }
                
                // Set loading state for all package managers
                for pm_type in &pm_types {
                    info.loading_updates.insert(*pm_type);
                }
                
                // Create loading tasks for all package managers
                let tasks: Vec<Task<Message>> = pm_types
                    .into_iter()
                    .map(|pm_type| {
                        Self::create_load_task(&updater_core::Config::default(), pm_type)
                    })
                    .collect();
                
                Action::Run(Task::batch(tasks))
            }
        }
    }

    pub fn view<'a>(
        &self,
        info: &'a UpdatesInfo,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, container, row};

        row![
            container(
                column![
                    self.manager_filter_view(info, pm_config),
                    self.sort_order_view(info),
                    self.refresh_button_view()
                ]
                .spacing(24)
            )
            .width(iced::Length::FillPortion(1)),
            container(
                column![
                    self.search_input_view(),
                    self.batch_actions_view(info),
                    self.updates_list_view(info)
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
        info: &'a UpdatesInfo,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'a, Message> {
        use iced::widget::column;

        let filters_content = if info.is_loading_count || !info.has_loading_count {
            self.loading_filter_view(info, pm_config)
        } else if info.updates_by_manager.is_empty() {
            self.empty_filter_view()
        } else {
            self.active_filter_view(info)
        };

        column![
            SharedUi::section_title("Filter Package Managers"),
            SharedUi::styled_container(filters_content)
        ]
        .spacing(12)
        .into()
    }

    fn loading_filter_view<'a>(
        &self,
        info: &'a UpdatesInfo,
        pm_config: &updater_core::Config,
    ) -> iced::widget::Column<'a, Message> {
        use iced::widget::{column, text};

        let loading_text = if info.is_loading_count {
            "Loading update information..."
        } else {
            "Waiting to load update information"
        };

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

        let mut col_items = vec![
            text(loading_text)
                .size(13)
                .color(app::colors::ON_SURFACE_MUTED)
                .into(),
        ];

        let checkboxes: Vec<iced::Element<'static, Message>> = all_managers
            .iter()
            .map(|pm_type| {
                iced::widget::checkbox(false)
                    .label(pm_type.name())
                    .spacing(10)
                    .text_size(15)

                    .style(move |_theme, _status| {
                        use iced::widget::checkbox::Style;
                        Style {
                            background: app::colors::SURFACE.into(),
                            icon_color: app::colors::ON_SURFACE_MUTED,
                            border: Border {
                                color: app::colors::DIVIDER,
                                width: 2.0,
                                radius: 4.0.into(),
                            },
                            text_color: Some(app::colors::ON_SURFACE_MUTED),
                        }
                    })
                    .into()
            })
            .collect();

        col_items.extend(checkboxes);
        column(col_items).spacing(8)
    }

    fn empty_filter_view<'a>(&self) -> iced::widget::Column<'a, Message> {
        use iced::widget::{column, text};

        column![
            text("No updates found")
                .size(14)
                .color(app::colors::ON_SURFACE_MUTED)
        ]
        .spacing(8)
    }

    fn active_filter_view<'a>(&self, info: &'a UpdatesInfo) -> iced::widget::Column<'a, Message> {
        use iced::widget::column;

        column(info.updates_by_manager.iter().map(|(pm_type, (count, _))| {
            let pm_type = *pm_type;
            let is_selected = info.selected_managers.contains(&pm_type);
            let is_loading = info.loading_updates.contains(&pm_type);

            let label = if is_loading {
                format!("{} (Loading...)", pm_type.name())
            } else {
                format!("{} ({})", pm_type.name(), count)
            };

            let checkbox = iced::widget::checkbox(is_selected)
                .label(label)
                .spacing(10)
                .text_size(15)
                .style(SharedUi::checkbox_style(is_loading));

            if is_loading {
                checkbox.into()
            } else {
                checkbox
                    .on_toggle(move |selected| Message::SelectPackageManager(pm_type, selected))
                    .into()
            }
        }))
        .spacing(12)
    }

    fn sort_order_view<'a>(&self, info: &'a UpdatesInfo) -> iced::Element<'a, Message> {
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

    fn search_input_view<'a>(&self) -> iced::Element<'a, Message> {
        SharedUi::search_input_view(
            "Search",
            "Search updates...",
            &self.search_query,
            Message::SearchQueryChanged,
        )
    }

    fn refresh_button_view<'a>(&self) -> iced::Element<'a, Message> {
        use iced::widget::{button, text};

        button(text("Refresh").size(14).color(iced::Color::WHITE))
            .padding([8, 16])
            .style(|_theme, status| {
                use iced::widget::button::Style;
                let base_color = app::colors::SECONDARY;
                match status {
                    iced::widget::button::Status::Hovered => Style {
                        background: Some(iced::Background::Color(app::colors::SECONDARY_HOVER)),
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
            })
            .on_press(Message::RefreshInfo)
            .into()
    }

    fn updates_list_view<'a>(&self, info: &'a UpdatesInfo) -> iced::Element<'a, Message> {
        use iced::widget::{column, scrollable};

        if info.is_loading_count || !info.has_loading_count {
            return SharedUi::centered_message(if info.is_loading_count {
                "Loading update information..."
            } else {
                "Waiting to load update information"
            });
        }

        let filtered_managers: Vec<_> = info
            .updates_by_manager
            .iter()
            .filter(|(pm_type, _)| info.selected_managers.contains(pm_type))
            .collect();

        if filtered_managers.is_empty() {
            return SharedUi::centered_message("Please select a package manager to view");
        }

        let total_updates: usize = filtered_managers.iter().map(|(_, (count, _))| *count).sum();

        if total_updates == 0 {
            return SharedUi::centered_message("No updates available");
        }

        let search_query = self.search_query.trim().to_lowercase();
        if !search_query.is_empty() {
            let has_any_match = filtered_managers.iter().any(|(_, (_, packages))| {
                packages
                    .iter()
                    .any(|pkg| pkg.name.to_lowercase().contains(&search_query))
            });

            if !has_any_match {
                return SharedUi::centered_message("No updates match your search");
            }
        }

        let updates_sections: Vec<iced::Element<'_, Message>> = filtered_managers
            .into_iter()
            .map(|(pm_type, (count, packages))| {
                self.package_manager_section(*pm_type, *count, packages, info)
            })
            .collect();

        scrollable(column(updates_sections).spacing(20))
            .width(iced::Length::Fill)
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
        use iced::widget::{column, row, text};

        let is_loading = info.loading_updates.contains(&pm_type);

        let header = row![
            text(pm_type.name()).size(18).color(app::colors::SECONDARY),
            text(if is_loading {
                "(Loading...)".to_owned()
            } else {
                format!("({} updates)", count)
            })
            .size(16)
            .color(app::colors::ON_SURFACE_MUTED)
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        let filtered_packages = self.filter_and_sort_updates(packages, info.sort_by);

        if filtered_packages.is_empty() {
            return column![].into();
        }

        let updates_list = column(
            filtered_packages
                .into_iter()
                .map(|pkg| self.package_item_view(pkg, info)),
        )
        .spacing(8);

        column![header, SharedUi::styled_container(updates_list)]
            .spacing(12)
            .into()
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
        package: &'a PackageUpdate,
        info: &'a UpdatesInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{checkbox, row, text};

        let package_name = package.name.clone();
        let is_selected = info.selected_packages.contains(&package.name);

        row![
            checkbox(is_selected)
                .on_toggle({
                    let package_name = package_name.clone();
                    move |selected| Message::TogglePackageSelection(package_name.clone(), selected)
                })
                .size(18)
                .spacing(8)
                .style(SharedUi::checkbox_style(false)),
            text(&package.name)
                .size(15)
                .color(app::colors::ON_SURFACE)
                .width(iced::Length::Fill),
            text(&package.current_version)
                .size(14)
                .color(app::colors::ON_SURFACE_MUTED),
            text("→").size(14).color(app::colors::ON_SURFACE_MUTED),
            text(&package.new_version)
                .size(14)
                .color(app::colors::ON_SURFACE_MUTED),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center)
        .padding([8, 0])
        .into()
    }

    fn batch_actions_view<'a>(&self, info: &'a UpdatesInfo) -> iced::Element<'a, Message> {
        use iced::widget::{button, checkbox, row, text};

        let selected_count = info.selected_packages.len();
        let is_enabled = selected_count > 0 && !info.is_updating;

        // Count total visible packages from selected managers
        let total_visible: usize = info
            .selected_managers
            .iter()
            .filter_map(|pm_type| info.updates_by_manager.get(pm_type))
            .map(|(_, packages)| packages.len())
            .sum();

        let all_selected = total_visible > 0 && selected_count == total_visible;

        let button_text = if info.is_updating {
            "Updating...".to_string()
        } else if selected_count > 0 {
            format!("Update {} package(s)", selected_count)
        } else {
            "Update Selected".to_string()
        };

        let select_all_checkbox = checkbox(all_selected)
            .label("Select All")
            .on_toggle(Message::ToggleSelectAll)
            .size(18)
            .spacing(8)
            .text_size(14)
            .style(SharedUi::checkbox_style(false));

        let update_button = button(text(button_text).size(14).color(if is_enabled {
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
                let base_color = iced::Color::from_rgb8(40, 167, 69);
                match status {
                    iced::widget::button::Status::Hovered => Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb8(
                            33, 136, 56,
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

        let update_button = if is_enabled {
            update_button.on_press(Message::UpdateSelectedPackages)
        } else {
            update_button
        };

        row![select_all_checkbox, update_button]
            .spacing(12)
            .align_y(iced::Alignment::Center)
            .into()
    }

    fn create_load_task(
        pm_config: &updater_core::Config,
        pm_type: PackageManagerType,
    ) -> Task<Message> {
        let pm_config = pm_config.clone();

        Task::future(async move {
            pm_type
                .list_updates(&pm_config)
                .await
                .map_err(|e| format!("Failed to load updates for {}: {}", pm_type.name(), e))
        })
        .then(move |result| Task::done(Message::LoadUpdatesResult(pm_type, result)))
    }

    fn load_updates_action(
        pm_config: &updater_core::Config,
        pm_type: PackageManagerType,
    ) -> Action {
        Action::Run(Self::create_load_task(pm_config, pm_type))
    }

    fn update_packages_action(pm_config: &updater_core::Config, info: &UpdatesInfo) -> Action {
        let pm_config = pm_config.clone();
        let selected_packages = info.selected_packages.clone();

        // Group packages by their package manager
        let mut packages_by_manager: HashMap<PackageManagerType, Vec<String>> = HashMap::new();

        for pm_type in info.selected_managers.iter() {
            if let Some((_, packages)) = info.updates_by_manager.get(pm_type) {
                for pkg in packages {
                    if selected_packages.contains(&pkg.name) {
                        packages_by_manager
                            .entry(*pm_type)
                            .or_default()
                            .push(pkg.name.clone());
                    }
                }
            }
        }

        let task = Task::future(async move {
            for (pm_type, package_names) in packages_by_manager {
                if let Err(e) = pm_type.update_packages(&pm_config, &package_names).await {
                    return Err(format!(
                        "Failed to update packages from {}: {}",
                        pm_type.name(),
                        e
                    ));
                }
            }
            Ok(())
        })
        .then(|result| Task::done(Message::UpdatePackagesResult(result)));

        Action::Run(task)
    }
}
