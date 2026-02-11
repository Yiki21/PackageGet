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

use iced::{Border, Task, widget::button};
use updater_core::{PackageInfo, PackageManagerType};

use crate::{app, content, content::shared::SharedUi};

#[derive(Debug, Clone, Default)]
pub struct Installed {
    search_query: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    SelectPackageManager(PackageManagerType, bool),
    LoadInstalledResult(PackageManagerType, Result<Vec<PackageInfo>, String>),
    RefreshInfo,
    SearchQueryChanged(String),
    SortOptionChanged(SortOption),
    TogglePackageSelection(String, bool),
    RemoveSelectedPackages,
    RemovePackagesResult(Result<(), String>),
}

/// Information about installed packages passed from app state
#[derive(Debug, Clone, Default)]
pub struct InstalledInfo {
    pub installed_packages: HashMap<PackageManagerType, (usize, Vec<PackageInfo>)>,
    pub selected_managers: HashSet<PackageManagerType>,
    pub loading_installed: HashSet<PackageManagerType>,
    pub is_loading_count: bool,
    pub has_loading_count: bool,
    pub sort_by: SortOption,
    pub selected_packages: HashSet<String>,
    pub is_removing: bool,
}

impl From<Message> for content::Message {
    fn from(msg: Message) -> Self {
        content::Message::Installed(msg)
    }
}

pub enum Action {
    None,
    Run(iced::Task<Message>),
    ClearCacheAndReload,
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

    pub fn description(&self) -> &'static str {
        match self {
            SortOption::Name => "Sort by package name",
            SortOption::Version => "Sort by version number",
            SortOption::InstallDate => "Sort by installation date",
        }
    }

    pub const ALL: [SortOption; 3] = [
        SortOption::Name,
        SortOption::Version,
        SortOption::InstallDate,
    ];
}

impl Installed {
    pub fn update(&mut self, message: Message, info: &mut InstalledInfo) -> Action {
        match message {
            Message::SelectPackageManager(pm_type, selected) => {
                if selected {
                    info.selected_managers.insert(pm_type);
                    if info.installed_packages[&pm_type].0
                        == info.installed_packages[&pm_type].1.len()
                    {
                        Action::None
                    } else {
                        info.loading_installed.insert(pm_type);
                        Self::load_installed_packages_action(
                            &updater_core::Config::default(),
                            pm_type,
                        )
                    }
                } else {
                    info.selected_managers.remove(&pm_type);
                    Action::None
                }
            }
            Message::LoadInstalledResult(pm_type, result) => {
                info.loading_installed.remove(&pm_type);
                match result {
                    Ok(packages) => {
                        let count = packages.len();
                        info.installed_packages.insert(pm_type, (count, packages));
                    }
                    Err(_) => {
                        info.installed_packages.insert(pm_type, (0, Vec::new()));
                    }
                }
                Action::None
            }
            Message::RefreshInfo => {
                for pm_type in info.installed_packages.keys() {
                    info.loading_installed.insert(*pm_type);
                }
                Action::ClearCacheAndReload
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
            Message::RemoveSelectedPackages => {
                if info.selected_packages.is_empty() {
                    return Action::None;
                }
                info.is_removing = true;
                Self::remove_packages_action(&updater_core::Config::default(), info)
            }
            Message::RemovePackagesResult(result) => {
                info.is_removing = false;
                match result {
                    Ok(_) => {
                        info.selected_packages.clear();
                        // Reload packages after removal
                        Action::ClearCacheAndReload
                    }
                    Err(e) => {
                        eprintln!("Failed to remove packages: {}", e);
                        Action::None
                    }
                }
            }
        }
    }

    pub fn view<'a>(
        &self,
        info: &'a InstalledInfo,
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
                    self.packages_list_view(info)
                ]
                .spacing(20)
            )
            .width(iced::Length::FillPortion(3))
        ]
        .spacing(24)
        .into()
    }

    // === Styling helpers ===

    fn section_title(text: &'static str) -> iced::widget::Text<'static> {
        SharedUi::section_title(text)
    }

    fn styled_container<'a>(
        content: impl Into<iced::Element<'a, Message>>,
    ) -> iced::widget::Container<'a, Message> {
        SharedUi::styled_container(content)
    }

    fn checkbox_style(
        is_loading: bool,
    ) -> impl Fn(&iced::Theme, iced::widget::checkbox::Status) -> iced::widget::checkbox::Style
    {
        SharedUi::checkbox_style(is_loading)
    }

    fn radio_style(
        _theme: &iced::Theme,
        status: iced::widget::radio::Status,
    ) -> iced::widget::radio::Style {
        SharedUi::radio_style(_theme, status)
    }

    // === View components ===

    fn manager_filter_view<'a>(
        &self,
        info: &'a InstalledInfo,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, text};

        let filters_content = if info.is_loading_count || !info.has_loading_count {
            self.loading_filter_view(info, pm_config)
        } else if info.installed_packages.is_empty() {
            self.empty_filter_view()
        } else {
            self.active_filter_view(info)
        };

        column![
            Self::section_title("Filter Package Managers"),
            Self::styled_container(filters_content)
        ]
        .spacing(12)
        .into()
    }

    fn loading_filter_view<'a>(
        &self,
        info: &'a InstalledInfo,
        pm_config: &updater_core::Config,
    ) -> iced::widget::Column<'a, Message> {
        use iced::widget::{column, text};

        let loading_text = if info.is_loading_count {
            "Loading package information..."
        } else {
            "Waiting to load package information"
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

    fn empty_filter_view(&self) -> iced::widget::Column<'static, Message> {
        use iced::widget::{column, text};

        column![
            text("No installed packages found")
                .size(14)
                .color(app::colors::ON_SURFACE_MUTED)
        ]
        .spacing(8)
    }

    fn active_filter_view<'a>(&self, info: &'a InstalledInfo) -> iced::widget::Column<'a, Message> {
        use iced::widget::column;

        column(info.installed_packages.iter().map(|(pm_type, (count, _))| {
            let pm_type = *pm_type;
            let is_selected = info.selected_managers.contains(&pm_type);
            let is_loading = info.loading_installed.contains(&pm_type);

            let label = if is_loading {
                format!("{} (Loading...)", pm_type.name())
            } else {
                format!("{} ({})", pm_type.name(), count)
            };

            let checkbox = iced::widget::checkbox(is_selected)
                .label(label)
                .spacing(10)
                .text_size(15)
                .style(Self::checkbox_style(is_loading));

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

    fn sort_order_view<'a>(&self, info: &'a InstalledInfo) -> iced::Element<'a, Message> {
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
            .style(Self::radio_style)
            .into()
        }))
        .spacing(12);

        column![
            Self::section_title("Sort By"),
            Self::styled_container(sort_options)
        ]
        .spacing(12)
        .into()
    }

    // === Package list views ===

    fn search_input_view(&self) -> iced::Element<'static, Message> {
        SharedUi::search_input_view(
            "Search",
            "Search packages...",
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

    fn packages_list_view<'a>(&self, info: &'a InstalledInfo) -> iced::Element<'a, Message> {
        use iced::widget::{column, scrollable};

        if info.is_loading_count || !info.has_loading_count {
            return self.centered_message(if info.is_loading_count {
                "Loading package information..."
            } else {
                "Waiting to load package information"
            });
        }

        let filtered_managers: Vec<_> = info
            .installed_packages
            .iter()
            .filter(|(pm_type, _)| info.selected_managers.contains(pm_type))
            .collect();

        if filtered_managers.is_empty() {
            return self.centered_message("Please select a package manager to view");
        }

        let search_query = self.search_query.trim().to_lowercase();
        if !search_query.is_empty() {
            let has_any_match = filtered_managers.iter().any(|(_, (_, packages))| {
                packages
                    .iter()
                    .any(|pkg| pkg.name.to_lowercase().contains(&search_query))
            });

            if !has_any_match {
                return self.centered_message("No packages match your search");
            }
        }

        let packages_sections: Vec<iced::Element<'_, Message>> = filtered_managers
            .into_iter()
            .map(|(pm_type, (count, packages))| {
                self.package_manager_section(*pm_type, *count, packages, info)
            })
            .collect();

        scrollable(column(packages_sections).spacing(20))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
    }

    fn centered_message<'a>(&self, message: &'a str) -> iced::Element<'a, Message> {
        SharedUi::centered_message(message)
    }

    fn package_manager_section<'a>(
        &self,
        pm_type: PackageManagerType,
        count: usize,
        packages: &'a [PackageInfo],
        info: &'a InstalledInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{column, row, text};

        let is_loading = info.loading_installed.contains(&pm_type);

        let header = row![
            text(pm_type.name()).size(18).color(app::colors::SECONDARY),
            text(if is_loading {
                "(Loading...)".to_owned()
            } else {
                format!("({} packages)", count)
            })
            .size(16)
            .color(app::colors::ON_SURFACE_MUTED)
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        let filtered_packages = self.filter_and_sort_packages(packages, info.sort_by);

        if filtered_packages.is_empty() {
            return column![].into();
        }

        let packages_list = column(
            filtered_packages
                .into_iter()
                .map(|pkg| self.package_item_view(pkg, info)),
        )
        .spacing(8);

        column![header, Self::styled_container(packages_list)]
            .spacing(12)
            .into()
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
        package: &'a PackageInfo,
        info: &'a InstalledInfo,
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
                .style(Self::checkbox_style(false)),
            text(&package.name)
                .size(15)
                .color(app::colors::ON_SURFACE)
                .width(iced::Length::Fill),
            text(&package.version)
                .size(14)
                .color(app::colors::ON_SURFACE_MUTED),
        ]
        .spacing(16)
        .align_y(iced::Alignment::Center)
        .padding([8, 0])
        .into()
    }

    fn batch_actions_view<'a>(&self, info: &'a InstalledInfo) -> iced::Element<'a, Message> {
        use iced::widget::{button, row, text};

        let selected_count = info.selected_packages.len();
        let is_enabled = selected_count > 0 && !info.is_removing;

        let button_text = if info.is_removing {
            "Removing...".to_string()
        } else if selected_count > 0 {
            format!("Remove {} package(s)", selected_count)
        } else {
            "Remove Selected".to_string()
        };

        let remove_button = button(text(button_text).size(14).color(if is_enabled {
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
                let base_color = iced::Color::from_rgb8(220, 53, 69);
                match status {
                    iced::widget::button::Status::Hovered => Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb8(
                            200, 35, 51,
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

        let remove_button = if is_enabled {
            remove_button.on_press(Message::RemoveSelectedPackages)
        } else {
            remove_button
        };

        row![remove_button]
            .spacing(12)
            .align_y(iced::Alignment::Center)
            .into()
    }

    fn load_installed_packages_action(
        pm_config: &updater_core::Config,
        pm_type: PackageManagerType,
    ) -> Action {
        let pm_config = pm_config.clone();

        let task = Task::future(async move {
            pm_type.list_installed(&pm_config).await.map_err(|e| {
                format!(
                    "Failed to load installed packages for {}: {}",
                    pm_type.name(),
                    e
                )
            })
        })
        .then(move |result| Task::done(Message::LoadInstalledResult(pm_type, result)));
        Action::Run(task)
    }

    fn remove_packages_action(pm_config: &updater_core::Config, info: &InstalledInfo) -> Action {
        let pm_config = pm_config.clone();
        let selected_packages = info.selected_packages.clone();

        // Group packages by their package manager
        let mut packages_by_manager: HashMap<PackageManagerType, Vec<String>> = HashMap::new();

        for pm_type in info.selected_managers.iter() {
            if let Some((_, packages)) = info.installed_packages.get(pm_type) {
                for pkg in packages {
                    if selected_packages.contains(&pkg.name) {
                        packages_by_manager
                            .entry(*pm_type)
                            .or_insert_with(Vec::new)
                            .push(pkg.name.clone());
                    }
                }
            }
        }

        let task = Task::future(async move {
            for (pm_type, package_names) in packages_by_manager {
                if let Err(e) = pm_type.uninstall_packages(&pm_config, &package_names).await {
                    return Err(format!(
                        "Failed to remove packages from {}: {}",
                        pm_type.name(),
                        e
                    ));
                }
            }
            Ok(())
        })
        .then(|result| Task::done(Message::RemovePackagesResult(result)));

        Action::Run(task)
    }
}
