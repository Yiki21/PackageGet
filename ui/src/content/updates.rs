// Updates view with filtering, sorting, and search capabilities.

use std::collections::{HashMap, HashSet};

use iced::{Border, Task};
use updater_core::{PackageManagerType, PackageUpdate};

use crate::{
    app, content,
    content::errors::{ManagerErrors, apply_manager_counted_items_result},
    content::shared::{PackageSelectionKey, SharedUi},
    content::workflows::{
        BatchProgress, PackageBatchAction, collect_selected_package_groups, push_command_log,
        run_grouped_package_action,
    },
};

#[derive(Debug, Clone, Default)]
pub struct Updates {
    /// Search text for filtering updates in UI.
    search_query: String,
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
    UpdatePackagesResult(Result<(), String>),
    /// Selected-managers refresh message.
    RefreshSelected,
    /// Full refresh message.
    RefreshAll,
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
}

impl From<Message> for content::Message {
    fn from(msg: Message) -> Self {
        content::Message::Updates(msg)
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

                    if info.loading_updates.contains(&pm_type) {
                        Action::None
                    } else if let Some((count, packages)) = info.updates_by_manager.get(&pm_type) {
                        if *count == packages.len() {
                            Action::None
                        } else {
                            info.loading_updates.insert(pm_type);
                            Self::load_updates_action(pm_config, pm_type, false)
                        }
                    } else {
                        info.loading_updates.insert(pm_type);
                        Self::load_updates_action(pm_config, pm_type, false)
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
                        if let Some((_, packages)) = info.updates_by_manager.get(pm_type) {
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
            Message::UpdateSelectedPackages => {
                if info.selected_packages.is_empty() {
                    return Action::None;
                }
                info.is_updating = true;
                info.last_update_error = None;
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
            Message::UpdatePackagesResult(result) => {
                info.is_updating = false;
                info.update_progress = None;
                match result {
                    Ok(_) => {
                        info.selected_packages.clear();
                        info.last_update_error = None;
                        // Reload updates after a successful update run.
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
                            .map(|pm_type| Self::create_load_task(pm_config, pm_type, false))
                            .collect();

                        Action::Run(Task::batch(tasks))
                    }
                    Err(e) => {
                        log::error!("Failed to update packages: {}", e);
                        info.last_update_error = Some(e);
                        Action::None
                    }
                }
            }
            Message::RefreshSelected => {
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
        }
    }

    pub fn view<'a>(
        &self,
        info: &'a UpdatesInfo,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'a, Message> {
        use iced::widget::column;

        SharedUi::content_page_layout(
            column![
                self.manager_filter_view(info, pm_config),
                self.sort_order_view(info),
                self.refresh_actions_view()
            ]
            .spacing(24),
            column![
                self.search_input_view(),
                self.batch_actions_view(info),
                self.updates_list_view(info)
            ]
            .spacing(20),
        )
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
                return SharedUi::filter_section(
                    "Filter Package Managers",
                    SharedUi::empty_filter_view("No package managers detected"),
                );
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

        SharedUi::manager_filter_section(
            "Filter Package Managers",
            filters_content,
            &info.init_errors,
        )
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

    fn updates_list_view<'a>(&self, info: &'a UpdatesInfo) -> iced::Element<'a, Message> {
        use iced::widget::{column, scrollable};

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
            "Failed to load updates",
            info.load_errors.get(&pm_type).map(String::as_str),
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
        use iced::widget::{button, checkbox, column, row, text};

        let selected_count = info.selected_packages.len();
        let is_enabled = selected_count > 0 && !info.is_updating;

        // Count visible packages in selected managers.
        let total_visible: usize = info
            .selected_managers
            .iter()
            .filter_map(|pm_type| info.updates_by_manager.get(pm_type))
            .map(|(_, packages)| packages.len())
            .sum();

        let all_selected = total_visible > 0 && selected_count == total_visible;

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

        let actions_row = row![select_all_checkbox, update_button]
            .spacing(12)
            .align_y(iced::Alignment::Center);

        if let Some(error) = &info.last_update_error {
            column![
                actions_row,
                text(format!("Update failed: {}", error))
                    .size(13)
                    .color(app::colors::ERROR)
            ]
            .spacing(8)
            .into()
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

    fn load_updates_action(
        pm_config: &updater_core::Config,
        pm_type: PackageManagerType,
        force_refresh: bool,
    ) -> Action {
        Action::Run(Self::create_load_task(pm_config, pm_type, force_refresh))
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

        Action::Run(run_grouped_package_action(
            pm_config,
            PackageBatchAction::Update,
            manager_groups,
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
        ))
    }
}
