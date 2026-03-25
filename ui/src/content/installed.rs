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

use iced::{Border, Task};
use updater_core::{PackageInfo, PackageManagerType};

use crate::{
    app, content,
    content::errors::{ManagerErrors, apply_manager_counted_items_result, joined_manager_names},
    content::shared::{PackageSelectionKey, SharedUi},
    content::workflows::{
        BatchProgress, PackageBatchAction, collect_selected_package_groups, push_command_log,
        run_grouped_package_action,
    },
};

#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// Search text for filtering installed packages in UI.
    search_query: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Package-manager selection message.
    SelectPackageManager(PackageManagerType, bool),
    /// Installed-load result message.
    LoadInstalledResult(PackageManagerType, Result<Vec<PackageInfo>, String>),
    /// Installed refresh message.
    RefreshInfo,
    /// Search-query change message.
    SearchQueryChanged(String),
    /// Sort-option change message.
    SortOptionChanged(SortOption),
    /// Package-selection toggle message.
    TogglePackageSelection(PackageManagerType, String, bool),
    /// Select-all toggle message.
    ToggleSelectAll(bool),
    /// Remove-selected message.
    RemoveSelectedPackages,
    /// Remove progress message.
    RemoveProgress {
        /// Number of finished packages.
        completed: usize,
        /// Total packages to remove.
        total: usize,
        /// Manager currently executing command.
        manager: PackageManagerType,
        /// Current package being processed.
        current_package: String,
        /// Optional command output/status line.
        command_message: Option<String>,
    },
    /// Remove result message.
    RemovePackagesResult(Result<(), String>),
}

/// Information about installed packages passed from app state
#[derive(Debug, Clone, Default)]
pub struct InstalledInfo {
    /// Installed package cache by manager `(count, packages)`.
    pub installed_packages: HashMap<PackageManagerType, (usize, Vec<PackageInfo>)>,
    /// Initial count-loading failures grouped by manager.
    pub init_errors: ManagerErrors,
    /// Full installed-list loading failures grouped by manager.
    pub load_errors: ManagerErrors,
    /// Managers selected in the filter panel.
    pub selected_managers: HashSet<PackageManagerType>,
    /// Managers currently loading full installed package list.
    pub loading_installed: HashSet<PackageManagerType>,
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
    pub remove_progress: Option<(usize, usize, PackageManagerType, String)>,
    /// Remove command logs.
    pub remove_logs: Vec<String>,
}

impl From<Message> for content::Message {
    fn from(msg: Message) -> Self {
        content::Message::Installed(msg)
    }
}

pub enum Action {
    /// No-op action.
    None,
    /// Asynchronous task action.
    Run(iced::Task<Message>),
    /// Cache-clear and reload request action.
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
    ) -> Action {
        match message {
            Message::SelectPackageManager(pm_type, selected) => {
                if selected {
                    // Managers still in init phase are not selectable yet.
                    if info.is_loading_count && !info.installed_packages.contains_key(&pm_type) {
                        return Action::None;
                    }

                    info.selected_managers.insert(pm_type);
                    if let Some((count, packages)) = info.installed_packages.get(&pm_type) {
                        if *count == packages.len() {
                            Action::None
                        } else {
                            info.loading_installed.insert(pm_type);
                            Self::load_installed_packages_action(pm_config, pm_type)
                        }
                    } else {
                        info.loading_installed.insert(pm_type);
                        Self::load_installed_packages_action(pm_config, pm_type)
                    }
                } else {
                    info.selected_managers.remove(&pm_type);
                    info.selected_packages
                        .retain(|(manager, _)| *manager != pm_type);
                    Action::None
                }
            }
            Message::LoadInstalledResult(pm_type, result) => {
                info.loading_installed.remove(&pm_type);
                apply_manager_counted_items_result(
                    &mut info.installed_packages,
                    &mut info.load_errors,
                    pm_type,
                    result,
                );
                Action::None
            }
            Message::RefreshInfo => {
                let pm_types: Vec<PackageManagerType> =
                    info.installed_packages.keys().copied().collect();

                if pm_types.is_empty() {
                    return Action::None;
                }

                // Mark all managers as loading.
                for pm_type in &pm_types {
                    info.loading_installed.insert(*pm_type);
                }

                // Create load tasks for all managers.
                let tasks: Vec<Task<Message>> = pm_types
                    .into_iter()
                    .map(|pm_type| Self::create_load_task(pm_config, pm_type))
                    .collect();

                Action::Run(Task::batch(tasks))
            }
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
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
            Message::ToggleSelectAll(select_all) => {
                if select_all {
                    // Select all visible packages from selected managers.
                    for pm_type in &info.selected_managers {
                        if let Some((_, packages)) = info.installed_packages.get(pm_type) {
                            for pkg in packages {
                                info.selected_packages
                                    .insert(SharedUi::selection_key(*pm_type, &pkg.name));
                            }
                        }
                    }
                } else {
                    // Clear all selected packages.
                    info.selected_packages.clear();
                }
                Action::None
            }
            Message::RemoveSelectedPackages => {
                if info.selected_packages.is_empty() {
                    return Action::None;
                }
                info.is_removing = true;
                info.remove_logs.clear();
                let initial_manager = info
                    .selected_packages
                    .iter()
                    .next()
                    .map(|(pm_type, _)| *pm_type)
                    .unwrap_or(PackageManagerType::Dnf);
                info.remove_progress = Some((
                    0,
                    info.selected_packages.len(),
                    initial_manager,
                    String::new(),
                ));
                Self::remove_packages_action(pm_config, info)
            }
            Message::RemoveProgress {
                completed,
                total,
                manager,
                current_package,
                command_message,
            } => {
                info.remove_progress = Some((completed, total, manager, current_package));
                if let Some(command_message) = command_message {
                    push_command_log(
                        &mut info.remove_logs,
                        PackageBatchAction::Remove,
                        manager,
                        info.remove_progress
                            .as_ref()
                            .map_or("", |(_, _, _, package)| package.as_str()),
                        command_message,
                    );
                }
                Action::None
            }
            Message::RemovePackagesResult(result) => {
                info.is_removing = false;
                info.remove_progress = None;
                match result {
                    Ok(_) => {
                        info.selected_packages.clear();
                        // Reload package data after removal.
                        Action::ClearCacheAndReload
                    }
                    Err(e) => {
                        log::error!("Failed to remove packages: {}", e);
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
                    SharedUi::refresh_button(Message::RefreshInfo)
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

    // View components.

    fn manager_filter_view<'a>(
        &self,
        info: &'a InstalledInfo,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'a, Message> {
        let filters_content = if !info.has_loading_count {
            SharedUi::loading_manager_filter_view(
                pm_config,
                if info.is_loading_count {
                    "Loading package information..."
                } else {
                    "Waiting to load package information"
                },
            )
        } else {
            let managers = SharedUi::configured_managers(pm_config);
            if managers.is_empty() {
                return SharedUi::filter_section(
                    "Filter Package Managers",
                    SharedUi::empty_filter_view("No package managers detected"),
                );
            }

            let entries = managers
                .iter()
                .map(|pm_type| {
                    let count = info
                        .installed_packages
                        .get(pm_type)
                        .map_or(0, |(count, _)| *count);
                    (*pm_type, count)
                })
                .collect();

            SharedUi::active_manager_filter_view(
                entries,
                &info.selected_managers,
                &info.loading_installed,
                move |pm_type| {
                    info.is_loading_count && !info.installed_packages.contains_key(&pm_type)
                },
                Message::SelectPackageManager,
            )
        };

        let init_error_note = (!info.init_errors.is_empty()).then(|| {
            iced::widget::text(format!(
                "Initialization failed for: {}",
                joined_manager_names(&info.init_errors)
            ))
            .size(13)
            .color(app::colors::ERROR)
        });

        let mut section = iced::widget::column![SharedUi::section_title("Filter Package Managers")];
        if let Some(note) = init_error_note {
            section = section.push(note);
        }
        section
            .push(SharedUi::styled_container(filters_content))
            .spacing(12)
            .into()
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

    // Package list views.

    fn search_input_view(&self) -> iced::Element<'static, Message> {
        SharedUi::search_input_view(
            "Search",
            "Search packages...",
            &self.search_query,
            Message::SearchQueryChanged,
        )
    }

    fn packages_list_view<'a>(&self, info: &'a InstalledInfo) -> iced::Element<'a, Message> {
        use iced::widget::{column, scrollable};

        if !info.has_loading_count {
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
        let has_visible_errors = filtered_managers
            .iter()
            .any(|(pm_type, _)| info.load_errors.contains_key(*pm_type));

        if !search_query.is_empty() {
            let has_any_match = filtered_managers.iter().any(|(_, (_, packages))| {
                packages
                    .iter()
                    .any(|pkg| pkg.name.to_lowercase().contains(&search_query))
            });

            if !has_any_match && !has_visible_errors {
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

        if let Some(error) = info.load_errors.get(&pm_type) {
            return column![
                header,
                SharedUi::styled_container(
                    text(format!("Failed to load installed packages: {}", error))
                        .size(14)
                        .color(app::colors::ERROR)
                )
            ]
            .spacing(12)
            .into();
        }

        if filtered_packages.is_empty() {
            return column![].into();
        }

        let packages_list = column(
            filtered_packages
                .into_iter()
                .map(|pkg| self.package_item_view(pm_type, pkg, info)),
        )
        .spacing(8);

        column![header, SharedUi::styled_container(packages_list)]
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
        pm_type: PackageManagerType,
        package: &'a PackageInfo,
        info: &'a InstalledInfo,
    ) -> iced::Element<'a, Message> {
        use iced::widget::{checkbox, row, text};

        let package_name = package.name.clone();
        let is_selected = info
            .selected_packages
            .contains(&SharedUi::selection_key(pm_type, &package.name));

        row![
            checkbox(is_selected)
                .on_toggle({
                    let package_name = package_name.clone();
                    move |selected| {
                        Message::TogglePackageSelection(pm_type, package_name.clone(), selected)
                    }
                })
                .size(18)
                .spacing(8)
                .style(SharedUi::checkbox_style(false)),
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
        use iced::widget::{button, checkbox, row, text};

        let selected_count = info.selected_packages.len();
        let is_enabled = selected_count > 0 && !info.is_removing;

        // Count visible packages in selected managers.
        let total_visible: usize = info
            .selected_managers
            .iter()
            .filter_map(|pm_type| info.installed_packages.get(pm_type))
            .map(|(_, packages)| packages.len())
            .sum();

        let all_selected = total_visible > 0 && selected_count == total_visible;

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
                        manager.name()
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
            .on_toggle(Message::ToggleSelectAll)
            .size(18)
            .spacing(8)
            .text_size(14)
            .style(SharedUi::checkbox_style(false));

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

        row![select_all_checkbox, remove_button]
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
            pm_type.list_installed(&pm_config).await.map_err(|e| {
                format!(
                    "Failed to load installed packages for {}: {}",
                    pm_type.name(),
                    e
                )
            })
        })
        .then(move |result| Task::done(Message::LoadInstalledResult(pm_type, result)))
    }

    fn load_installed_packages_action(
        pm_config: &updater_core::Config,
        pm_type: PackageManagerType,
    ) -> Action {
        Action::Run(Self::create_load_task(pm_config, pm_type))
    }

    fn remove_packages_action(pm_config: &updater_core::Config, info: &InstalledInfo) -> Action {
        let manager_groups = collect_selected_package_groups(
            info.selected_managers.iter().filter_map(|pm_type| {
                info.installed_packages
                    .get(pm_type)
                    .map(|(_, packages)| (*pm_type, packages.as_slice()))
            }),
            &info.selected_packages,
            |package| package.name.as_str(),
        );

        Action::Run(run_grouped_package_action(
            pm_config,
            PackageBatchAction::Remove,
            manager_groups,
            |BatchProgress {
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
        ))
    }
}
