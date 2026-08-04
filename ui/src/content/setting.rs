use std::collections::HashMap;

use iced::Task;
use rfd::FileHandle;
use updater_core::ManagerConfig;
use updater_manager_api::{AvailabilityReason, ManagerAvailability, ManagerCategory, ManagerId};
use updater_managers::{
    configured_go_bin_dir, configured_nix_profile, set_configured_go_bin_dir,
    set_configured_nix_profile,
};

use crate::{
    content::{ManagerHealthInfo, shared},
    icon::{self, ADD_ICON, REFRESH_ICON, SAVE_ICON},
    manager_catalog::ManagerCatalog,
    theme,
};

#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// Shared draft configuration used by Settings and Package Managers.
    draft: updater_core::Config,
    /// Last configuration synchronized from or saved to persistent storage.
    baseline: updater_core::Config,
    /// Whether the draft has been initialized from application state.
    is_initialized: bool,
    /// Whether config save is in progress.
    pub is_saving: bool,
    /// Last save result shown in UI.
    pub save_status: Option<SaveStatus>,
    /// State owned by the Package Managers page.
    pub manager_page: ManagerPageState,
}

#[derive(Debug, Clone, Default)]
pub struct ManagerPageState {
    /// Search text for filtering configured and available managers.
    pub manager_query: String,
    /// Whether package-manager auto detection is in progress.
    pub is_detecting: bool,
    /// Manager currently waiting for custom-path selection.
    pub selecting_manager: Option<ManagerId>,
    /// Managers detected from PATH scan.
    pub detected_in_path: Vec<ManagerId>,
    /// Availability results from the last package-manager detection or path validation.
    pub detection_results: HashMap<ManagerId, Result<ManagerAvailability, String>>,
}

#[derive(Debug, Clone)]
pub enum SaveStatus {
    /// Save completed successfully.
    Success,
    /// Save failed with error detail.
    Error(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Filter package managers on the Package Managers page.
    ManagerQueryChanged(String),
    /// Package-manager detection message.
    DetectPackageManagers,
    /// Detection result message.
    FinishDetect(Vec<(ManagerId, Result<ManagerAvailability, String>)>),
    /// Manager-add message.
    AddDetectedManager(ManagerId),
    /// Manager-remove message.
    UnloadManager(ManagerId),
    /// Config-save message.
    SaveConfig,
    /// Config-save result message.
    SaveConfigResult {
        /// Configuration snapshot passed to persistent storage.
        config: updater_core::Config,
        /// Availability results for custom executable paths.
        validation: Vec<(ManagerId, Result<ManagerAvailability, String>)>,
        /// Validation or persistence result.
        result: Result<(), String>,
    },
    /// Manager-path dialog message.
    OpenDialog(ManagerId),
    /// Manager-path selection message.
    SelectedPath(FileHandle),
    /// Restore a manager to executable discovery through PATH.
    ResetExecutable(ManagerId),
    /// Selection-cancel message.
    CancelSelection,
    /// Go-bin directory dialog message.
    OpenGoBinDirDialog,
    /// Go-bin directory selection message.
    SelectedGoBinDir(FileHandle),
    /// Go-bin directory clear message.
    ClearGoBinDir,
    /// Nix profile path dialog message.
    OpenNixProfileDialog,
    /// Nix profile path selection message.
    SelectedNixProfile(FileHandle),
    /// Revert all unsaved settings changes.
    RevertChanges,
    /// Change the preferred application appearance.
    AppearanceChanged(theme::Appearance),
    /// Enable or disable native completion notifications.
    NotificationsChanged(bool),
}

#[derive(Debug)]
pub enum Action {
    /// No-op action.
    None,
    /// Asynchronous task action.
    Run(iced::Task<Message>),
    /// Apply the successfully saved draft and reload package data.
    ApplySavedConfig(updater_core::Config),
    /// A manager-owned setting changed, invalidating prior health results.
    ManagerConfigChanged,
}

impl Settings {
    pub fn sync_from_config(&mut self, config: &updater_core::Config) {
        if !self.is_initialized || !self.is_dirty() {
            self.draft = config.clone();
            self.baseline = config.clone();
            self.is_initialized = true;
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.is_initialized && self.draft != self.baseline
    }

    pub fn has_manager_changes(&self) -> bool {
        self.is_initialized && self.draft.managers != self.baseline.managers
    }

    pub fn draft_config(&self) -> &updater_core::Config {
        &self.draft
    }

    pub fn appearance_value(&self) -> String {
        self.draft.appearance.clone()
    }

    pub fn discard_changes(&mut self) {
        self.draft.clone_from(&self.baseline);
        self.manager_page.detection_results.clear();
        self.save_status = None;
        self.manager_page.selecting_manager = None;
    }

    fn section_title(text: &'static str) -> iced::widget::Text<'static> {
        iced::widget::text(text)
            .size(16)
            .font(theme::FONT_SEMIBOLD)
            .style(theme::text_on_surface)
    }

    fn icon_button(
        icon: iced::widget::Svg<'static>,
        label: &'static str,
        size: f32,
        message: Option<Message>,
    ) -> iced::widget::Button<'static, Message> {
        use iced::{
            Alignment,
            widget::{button, row, text},
        };

        let enabled = message.is_some();
        let btn = button(
            row![icon, text(label).size(size).font(theme::FONT_SEMIBOLD)]
                .spacing(if size > 14.0 { 8 } else { 6 })
                .align_y(Alignment::Center),
        )
        .padding(if size > 14.0 { [10, 16] } else { [8, 12] })
        .style(theme::action_button(
            enabled,
            theme::colors::ACCENT,
            theme::colors::ACCENT_HOVER,
            theme::colors::ACCENT_ACTIVE,
        ));

        if let Some(msg) = message {
            btn.on_press(msg)
        } else {
            btn
        }
    }

    fn secondary_button(
        label: &'static str,
        size: f32,
        message: Option<Message>,
    ) -> iced::widget::Button<'static, Message> {
        use iced::widget::{button, text};

        let enabled = message.is_some();
        let btn = button(text(label).size(size))
            .padding(if size > 14.0 { [10, 16] } else { [8, 12] })
            .style(theme::secondary_button(enabled));

        if let Some(msg) = message {
            btn.on_press(msg)
        } else {
            btn
        }
    }

    pub fn update(
        &mut self,
        message: Message,
        active_config: &updater_core::Config,
        catalog: &ManagerCatalog,
    ) -> Action {
        self.sync_from_config(active_config);

        match message {
            Message::ManagerQueryChanged(query) => {
                self.manager_page.manager_query = query;
                Action::None
            }
            Message::DetectPackageManagers => {
                self.manager_page.is_detecting = true;
                let config = self.draft.clone();
                let registry = catalog.registry();
                let task = Task::future(async move {
                    let mut results = Vec::with_capacity(registry.len());
                    for manager in registry.managers() {
                        let id = manager.descriptor().id().clone();
                        let manager_config = config
                            .manager(&id)
                            .cloned()
                            .unwrap_or_else(|| ManagerConfig::new(id.clone()));
                        let detection_results = manager
                            .availability(&manager_config)
                            .await
                            .map_err(|error| error.to_string());
                        results.push((id, detection_results));
                    }
                    results
                })
                .then(|detected_managers| Task::done(Message::FinishDetect(detected_managers)));
                Action::Run(task)
            }
            Message::FinishDetect(results) => {
                self.manager_page.is_detecting = false;
                self.manager_page.detected_in_path = results
                    .iter()
                    .filter_map(|(manager, detection_results)| {
                        detection_results
                            .as_ref()
                            .is_ok_and(ManagerAvailability::is_available)
                            .then_some(manager.clone())
                    })
                    .collect();
                self.manager_page.detection_results = results
                    .into_iter()
                    .filter(|(manager, _)| {
                        manager.as_str() != "builtin:nix-profile"
                            || self.draft.manager(manager).is_some()
                    })
                    .collect();
                Action::None
            }
            Message::AddDetectedManager(manager) => {
                let exists = self.draft.manager(&manager).is_some();

                if !exists {
                    self.draft.managers.push(ManagerConfig::new(manager));
                    return Action::ManagerConfigChanged;
                }
                Action::None
            }
            Message::UnloadManager(manager) => {
                let previous_len = self.draft.managers.len();
                self.draft
                    .managers
                    .retain(|configured| configured.id != manager);
                self.manager_page.detection_results.remove(&manager);
                self.save_status = None;
                if self.draft.managers.len() == previous_len {
                    Action::None
                } else {
                    Action::ManagerConfigChanged
                }
            }
            Message::SaveConfig => {
                if !self.is_dirty() || self.is_saving {
                    return Action::None;
                }
                self.is_saving = true;
                self.save_status = None;
                let config = self.draft.clone();
                let registry = catalog.registry();
                let managers_to_validate = config
                    .managers
                    .iter()
                    .filter_map(|configured| {
                        registry
                            .get(&configured.id)
                            .map(|manager| (manager, configured.clone()))
                    })
                    .collect::<Vec<_>>();

                Action::Run(Task::future(async move {
                    let mut validation = Vec::with_capacity(managers_to_validate.len());
                    let mut invalid_count = 0;
                    for (manager, configured) in managers_to_validate {
                        let id = configured.id.clone();
                        if let Err(error) = manager.validate_config(&configured) {
                            invalid_count += 1;
                            validation.push((id, Err(error.to_string())));
                            continue;
                        }
                        if configured.executable().is_none() {
                            continue;
                        }
                        let detection_results = manager
                            .availability(&configured)
                            .await
                            .map_err(|error| error.to_string());
                        if !detection_results
                            .as_ref()
                            .is_ok_and(ManagerAvailability::is_available)
                        {
                            invalid_count += 1;
                        }
                        validation.push((id, detection_results));
                    }

                    let result = if invalid_count == 0 {
                        config.save().await.map_err(|error| error.to_string())
                    } else {
                        Err(format!(
                            "{invalid_count} manager configuration(s) failed validation"
                        ))
                    };

                    Message::SaveConfigResult {
                        config,
                        validation,
                        result,
                    }
                }))
            }
            Message::SaveConfigResult {
                config,
                validation,
                result,
            } => {
                self.is_saving = false;
                for (manager, detection_results) in validation {
                    if self.draft.manager(&manager) == config.manager(&manager) {
                        self.manager_page
                            .detection_results
                            .insert(manager, detection_results);
                    }
                }
                match result {
                    Ok(()) => {
                        log::debug!("Configuration saved successfully");
                        self.baseline.clone_from(&config);
                        self.save_status = Some(SaveStatus::Success);
                        Action::ApplySavedConfig(config)
                    }
                    Err(e) => {
                        log::error!("Configuration save rejected: {}", e);
                        self.save_status = Some(SaveStatus::Error(e));
                        Action::None
                    }
                }
            }
            Message::OpenDialog(manager) => {
                self.manager_page.selecting_manager = Some(manager);

                let task = Task::future(
                    rfd::AsyncFileDialog::new()
                        .set_title("Select Package Manager Executable")
                        .pick_file(),
                )
                .then(|handle| match handle {
                    Some(file_handle) => Task::done(Message::SelectedPath(file_handle)),
                    None => Task::done(Message::CancelSelection),
                });

                Action::Run(task)
            }
            Message::SelectedPath(file_handle) => {
                let mut changed = false;
                if let Some(manager) = self.manager_page.selecting_manager.take() {
                    let path = file_handle.path().to_path_buf();
                    self.manager_page.detection_results.remove(&manager);
                    self.save_status = None;

                    if let Some(existing) = self.draft.manager_mut(&manager) {
                        changed = existing.executable() != Some(path.as_path());
                        existing.executable = Some(path);
                    } else {
                        self.draft
                            .managers
                            .push(ManagerConfig::new(manager).with_executable(path));
                        changed = true;
                    }
                } else {
                    log::error!("No package manager selected when handling SelectedPath");
                }

                if changed {
                    Action::ManagerConfigChanged
                } else {
                    Action::None
                }
            }
            Message::ResetExecutable(manager) => {
                let mut changed = false;
                if let Some(configured) = self.draft.manager_mut(&manager) {
                    changed = configured.executable.take().is_some();
                    self.manager_page.detection_results.remove(&manager);
                    self.save_status = None;
                }
                if changed {
                    Action::ManagerConfigChanged
                } else {
                    Action::None
                }
            }
            Message::CancelSelection => {
                self.manager_page.selecting_manager = None;
                Action::None
            }
            Message::OpenGoBinDirDialog => {
                let task = Task::future(
                    rfd::AsyncFileDialog::new()
                        .set_title("Choose Go Binary Directory")
                        .pick_folder(),
                )
                .then(|handle| match handle {
                    Some(file_handle) => Task::done(Message::SelectedGoBinDir(file_handle)),
                    None => Task::done(Message::CancelSelection),
                });

                Action::Run(task)
            }
            Message::SelectedGoBinDir(file_handle) => {
                let path = file_handle.path().to_path_buf();
                let id = ManagerId::parse("builtin:go")
                    .expect("built-in Go manager ID must remain valid");
                let previous = self
                    .draft
                    .manager(&id)
                    .and_then(|manager| configured_go_bin_dir(manager).ok().flatten());
                let Some(manager) = self.draft.manager_mut(&id) else {
                    self.save_status = Some(SaveStatus::Error(
                        "Go manager must be enabled before changing its binary directory".into(),
                    ));
                    return Action::None;
                };
                if let Err(error) = set_configured_go_bin_dir(manager, Some(path.clone())) {
                    self.save_status = Some(SaveStatus::Error(error.to_string()));
                    return Action::None;
                }
                self.save_status = None;
                if previous.as_deref() != Some(path.as_path()) {
                    self.manager_page.detection_results.remove(&id);
                    Action::ManagerConfigChanged
                } else {
                    Action::None
                }
            }
            Message::ClearGoBinDir => {
                let id = ManagerId::parse("builtin:go")
                    .expect("built-in Go manager ID must remain valid");
                let Some(manager) = self.draft.manager_mut(&id) else {
                    return Action::None;
                };
                let changed = configured_go_bin_dir(manager).ok().flatten().is_some();
                if let Err(error) = set_configured_go_bin_dir(manager, None) {
                    self.save_status = Some(SaveStatus::Error(error.to_string()));
                    return Action::None;
                }
                self.save_status = None;
                if changed {
                    self.manager_page.detection_results.remove(&id);
                    Action::ManagerConfigChanged
                } else {
                    Action::None
                }
            }
            Message::OpenNixProfileDialog => {
                let task = Task::future(
                    rfd::AsyncFileDialog::new()
                        .set_title("Select Nix User Profile")
                        .pick_file(),
                )
                .then(|handle| match handle {
                    Some(file_handle) => Task::done(Message::SelectedNixProfile(file_handle)),
                    None => Task::done(Message::CancelSelection),
                });
                Action::Run(task)
            }
            Message::SelectedNixProfile(file_handle) => {
                let id = ManagerId::parse("builtin:nix-profile")
                    .expect("built-in Nix profile manager ID must remain valid");
                let previous_profile = self
                    .draft
                    .manager(&id)
                    .and_then(|manager| configured_nix_profile(manager).ok());
                let added_manager = self.draft.manager(&id).is_none();
                if added_manager {
                    self.draft.managers.push(ManagerConfig::new(id.clone()));
                }
                let changed = match file_handle.path().to_str() {
                    Some(path) => match self
                        .draft
                        .manager_mut(&id)
                        .ok_or_else(|| "Nix profile manager was not added".to_owned())
                        .and_then(|manager| {
                            set_configured_nix_profile(manager, path.to_owned().into())
                                .map_err(|error| error.to_string())
                        }) {
                        Ok(()) => {
                            self.manager_page.detection_results.remove(&id);
                            self.save_status = None;
                            added_manager
                                || previous_profile
                                    .as_deref()
                                    .and_then(|profile| profile.to_str())
                                    != Some(path)
                        }
                        Err(error) => {
                            if added_manager {
                                self.draft.managers.retain(|manager| manager.id != id);
                            }
                            self.save_status = Some(SaveStatus::Error(error.to_string()));
                            false
                        }
                    },
                    None => {
                        if added_manager {
                            self.draft.managers.retain(|manager| manager.id != id);
                        }
                        self.save_status = Some(SaveStatus::Error(
                            "Nix profile path must be valid UTF-8".to_owned(),
                        ));
                        false
                    }
                };
                if changed {
                    Action::ManagerConfigChanged
                } else {
                    Action::None
                }
            }
            Message::RevertChanges => {
                let manager_config_changed = self.draft.managers != self.baseline.managers;
                self.discard_changes();
                if manager_config_changed {
                    Action::ManagerConfigChanged
                } else {
                    Action::None
                }
            }
            Message::AppearanceChanged(appearance) => {
                self.draft.appearance = appearance.config_value().to_owned();
                self.save_status = None;
                Action::None
            }
            Message::NotificationsChanged(enabled) => {
                self.draft.notifications_enabled = enabled;
                self.save_status = None;
                Action::None
            }
        }
    }

    pub fn view(&self) -> iced::Element<'static, Message> {
        use iced::Length;
        use iced::widget::{column, container, scrollable};

        let pm_config = &self.draft;

        let content = column![
            shared::page_header(
                "Settings",
                "Appearance, notifications, and application preferences",
                theme::colors::SETTINGS,
            ),
            self.view_appearance_section(pm_config),
            self.view_preference_buttons(),
            self.view_status(),
        ]
        .spacing(theme::spacing::LG)
        .width(Length::Fill);

        let scrollable_content = scrollable(
            container(content)
                .padding(iced::Padding {
                    top: 0.0,
                    right: theme::spacing::LG,
                    bottom: theme::spacing::LG,
                    left: 0.0,
                })
                .width(Length::Fill),
        )
        .direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::new()
                .width(8)
                .scroller_width(4)
                .margin(2),
        ))
        .style(theme::scrollable_style)
        .width(Length::Fill)
        .height(Length::Fill);

        container(scrollable_content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn view_managers(
        &self,
        catalog: &ManagerCatalog,
        health_info: &ManagerHealthInfo,
    ) -> iced::Element<'static, Message> {
        use iced::Length;
        use iced::widget::{column, container, scrollable};

        let pm_config = &self.draft;
        let content = column![
            self.view_configured_managers_section(pm_config, catalog, health_info),
            self.view_selection_list(pm_config, catalog, health_info),
            self.view_manager_buttons(),
            self.view_status(),
        ]
        .spacing(theme::spacing::LG)
        .width(Length::Fill);

        let scrollable_content = scrollable(
            container(content)
                .padding(iced::Padding {
                    top: 0.0,
                    right: theme::spacing::LG,
                    bottom: theme::spacing::LG,
                    left: 0.0,
                })
                .width(Length::Fill),
        )
        .direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::new()
                .width(8)
                .scroller_width(4)
                .margin(2),
        ))
        .style(theme::scrollable_style)
        .width(Length::Fill)
        .height(Length::Fill);

        container(scrollable_content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_configured_managers_section(
        &self,
        pm_config: &updater_core::Config,
        catalog: &ManagerCatalog,
        health_info: &ManagerHealthInfo,
    ) -> iced::Element<'static, Message> {
        use iced::Alignment;
        use iced::widget::{column, container, row, text, text_input};

        let mut groups = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for manager in pm_config.managers.iter().filter(|manager| {
            shared::manager_matches_query(&manager.id, catalog, &self.manager_page.manager_query)
        }) {
            let category = catalog
                .descriptor(&manager.id)
                .map_or(ManagerCategory::Other, |descriptor| descriptor.category());
            groups[manager_category_rank(category)].push(manager);
        }

        let mut grouped_lists = Vec::new();
        for (index, managers) in groups.into_iter().enumerate() {
            if managers.is_empty() {
                continue;
            }
            let category = category_from_rank(index);
            let count = managers.len();
            let rows = column(managers.into_iter().map(|manager| {
                let item = container(self.view_manager_item(
                    &manager.id,
                    Some(manager),
                    false,
                    pm_config,
                    catalog,
                    health_info,
                ))
                .width(iced::Length::Fill);
                let mut manager_row = row![item]
                    .spacing(theme::spacing::MD)
                    .align_y(Alignment::Center)
                    .width(iced::Length::Fill);
                if category != ManagerCategory::System {
                    manager_row = manager_row.push(Self::secondary_button(
                        "Unload",
                        14.0,
                        Some(Message::UnloadManager(manager.id.clone())),
                    ));
                }
                manager_row.into()
            }))
            .spacing(theme::spacing::SM);
            grouped_lists.push(
                column![
                    row![
                        text(shared::manager_category_label(category))
                            .size(14)
                            .font(theme::FONT_SEMIBOLD)
                            .style(theme::text_on_surface),
                        text(format!("{count}"))
                            .size(13)
                            .style(theme::text_on_surface_muted),
                    ]
                    .spacing(theme::spacing::SM)
                    .align_y(Alignment::Center),
                    rows,
                ]
                .spacing(theme::spacing::SM)
                .into(),
            );
        }

        let content: iced::Element<'_, Message> = if grouped_lists.is_empty() {
            let message = if pm_config.managers.is_empty() {
                "No package managers configured"
            } else {
                "No configured package managers match this filter"
            };
            text(message)
                .size(14)
                .style(theme::text_on_surface_muted)
                .into()
        } else {
            column(grouped_lists).spacing(theme::spacing::LG).into()
        };

        column![
            Self::section_title("Package Managers"),
            text("Configured sources are grouped by the kind of software they manage.")
                .size(14)
                .style(theme::text_on_surface_muted),
            text_input(
                "Filter package managers...",
                &self.manager_page.manager_query
            )
            .id(shared::search_input_id(
                crate::content::ActiveContentPage::Health,
            ))
            .on_input(Message::ManagerQueryChanged)
            .padding([8, 10])
            .size(14)
            .style(theme::text_input_style),
            content,
        ]
        .spacing(theme::spacing::SM)
        .width(iced::Length::Fill)
        .into()
    }

    fn view_appearance_section(
        &self,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'static, Message> {
        use iced::widget::{checkbox, column, row};

        let selected = theme::Appearance::from_config(&pm_config.appearance);
        let options = row(theme::Appearance::ALL.into_iter().map(|appearance| {
            shared::segmented_button(
                appearance.name(),
                appearance == selected,
                Message::AppearanceChanged(appearance),
            )
            .into()
        }))
        .spacing(2)
        .width(iced::Length::Fill);

        let notifications = checkbox(pm_config.notifications_enabled)
            .label("Native completion and failure notifications")
            .on_toggle(Message::NotificationsChanged)
            .size(18)
            .spacing(8)
            .text_size(14)
            .style(shared::checkbox_style(false));

        column![
            Self::section_title("Appearance & Notifications"),
            shared::segmented_group(options),
            notifications,
        ]
        .spacing(12)
        .width(iced::Length::Fill)
        .into()
    }

    fn view_selection_list(
        &self,
        pm_config: &updater_core::Config,
        catalog: &ManagerCatalog,
        health_info: &ManagerHealthInfo,
    ) -> iced::Element<'static, Message> {
        use iced::Alignment;
        use iced::widget::{column, container, row, svg, text};

        let available_managers: Vec<ManagerId> = catalog
            .registry()
            .managers()
            .into_iter()
            .map(|manager| manager.descriptor().id().clone())
            .filter(|manager| {
                catalog
                    .descriptor(manager)
                    .is_some_and(|descriptor| descriptor.category() != ManagerCategory::System)
            })
            .filter(|manager| pm_config.manager(manager).is_none())
            .collect();

        let detected_count = available_managers
            .iter()
            .filter(|manager| self.manager_page.detected_in_path.contains(manager))
            .count();

        let detect_tip = if self.manager_page.is_detecting {
            "Scanning $PATH...".to_string()
        } else if detected_count == 0 {
            "Click \"Scan $PATH\" to discover available managers.".to_string()
        } else {
            format!(
                "Detected {} manager(s) in $PATH. They will only be added after manual Add.",
                detected_count
            )
        };

        let mut groups = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for manager in &available_managers {
            let descriptor = catalog.descriptor(manager);
            if !shared::manager_matches_query(manager, catalog, &self.manager_page.manager_query) {
                continue;
            }
            let category =
                descriptor.map_or(ManagerCategory::Other, |descriptor| descriptor.category());
            groups[manager_category_rank(category)].push(manager.clone());
        }

        let mut grouped_lists = Vec::new();
        for (index, managers) in groups.into_iter().enumerate() {
            if managers.is_empty() {
                continue;
            }
            let category = category_from_rank(index);
            if category == ManagerCategory::System {
                continue;
            }
            let count = managers.len();
            let rows = column(managers.into_iter().map(|manager| {
                let detected_in_path = self.manager_page.detected_in_path.contains(&manager);
                let is_nix_profile = manager.as_str() == "builtin:nix-profile";
                let action_message = if is_nix_profile {
                    Message::OpenNixProfileDialog
                } else if detected_in_path {
                    Message::AddDetectedManager(manager.clone())
                } else {
                    Message::OpenDialog(manager.clone())
                };
                let action_label = if is_nix_profile {
                    "Choose Profile"
                } else if detected_in_path {
                    "Add"
                } else {
                    "Select Path"
                };
                let add_btn = Self::icon_button(
                    svg::Svg::new(ADD_ICON.clone()).width(16).height(16),
                    action_label,
                    14.0,
                    Some(action_message),
                );

                row![
                    container(self.view_manager_item(
                        &manager,
                        None,
                        detected_in_path,
                        pm_config,
                        catalog,
                        health_info,
                    ))
                    .width(iced::Length::Fill),
                    add_btn,
                ]
                .spacing(theme::spacing::MD)
                .align_y(Alignment::Center)
                .width(iced::Length::Fill)
                .into()
            }))
            .spacing(theme::spacing::SM);
            grouped_lists.push(
                column![
                    row![
                        text(shared::manager_category_label(category))
                            .size(14)
                            .font(theme::FONT_SEMIBOLD)
                            .style(theme::text_on_surface),
                        text(format!("{count}"))
                            .size(13)
                            .style(theme::text_on_surface_muted),
                    ]
                    .spacing(theme::spacing::SM)
                    .align_y(Alignment::Center),
                    rows,
                ]
                .spacing(theme::spacing::SM)
                .into(),
            );
        }

        let managers_list: iced::Element<'_, Message> = if available_managers.is_empty() {
            column![
                text("All available package managers have been added")
                    .size(16)
                    .style(theme::text_on_surface_muted)
            ]
            .into()
        } else if grouped_lists.is_empty() {
            text("No package managers match this filter")
                .size(14)
                .style(theme::text_on_surface_muted)
                .into()
        } else {
            column(grouped_lists).spacing(theme::spacing::LG).into()
        };

        column![
            Self::section_title("Add Package Managers"),
            text(detect_tip)
                .size(14)
                .style(theme::text_on_surface_muted),
            managers_list
        ]
        .spacing(12)
        .width(iced::Length::Fill)
        .into()
    }

    fn view_manager_item(
        &self,
        manager: &ManagerId,
        config: Option<&ManagerConfig>,
        detected_in_path: bool,
        pm_config: &updater_core::Config,
        catalog: &ManagerCatalog,
        health_info: &ManagerHealthInfo,
    ) -> iced::Element<'static, Message> {
        use iced::Alignment;
        use iced::widget::{column, row, text};

        let is_configured = config.is_some();
        let display_name = catalog.display_name(manager).to_owned();
        let category = catalog
            .descriptor(manager)
            .map_or(ManagerCategory::Other, |descriptor| descriptor.category());
        let name_row = if is_configured {
            row![
                text(display_name.clone())
                    .size(17)
                    .font(theme::FONT_SEMIBOLD),
                text(shared::manager_category_label(category))
                    .size(13)
                    .style(theme::text_on_surface_muted),
                text("✓").size(14).style(theme::text_success)
            ]
            .spacing(theme::spacing::SM)
            .align_y(Alignment::Center)
        } else {
            row![
                text(display_name.clone())
                    .size(17)
                    .font(theme::FONT_SEMIBOLD),
                text(shared::manager_category_label(category))
                    .size(13)
                    .style(theme::text_on_surface_muted),
            ]
            .spacing(theme::spacing::SM)
            .align_y(Alignment::Center)
        };

        let info_text = if let Some(config) = config {
            config
                .executable()
                .map(|path| format!("Path: {}", path.display()))
                .unwrap_or_else(|| "Path: $PATH (System Default)".to_string())
        } else if detected_in_path {
            "Detected in $PATH. Click Add to use system default path.".to_string()
        } else {
            catalog.descriptor(manager).map_or_else(
                || manager.as_str().to_owned(),
                |descriptor| descriptor.description().to_owned(),
            )
        };

        let mut content_items = vec![
            name_row.into(),
            text(info_text)
                .size(14)
                .font(if is_configured {
                    theme::FONT_MONO
                } else {
                    theme::FONT_REGULAR
                })
                .style(theme::text_on_surface_muted)
                .into(),
        ];

        if !catalog.is_registered(manager) {
            content_items.push(
                text("Status: unregistered / unavailable")
                    .size(14)
                    .style(theme::text_warning)
                    .into(),
            );
        } else if !catalog.supports_current_platform(manager) {
            content_items.push(
                text("Status: unsupported on this platform")
                    .size(14)
                    .style(theme::text_warning)
                    .into(),
            );
        } else if is_configured
            || detected_in_path
            || self.manager_page.detection_results.contains_key(manager)
            || health_info.result(manager).is_some()
        {
            let status_result = if is_configured {
                health_info.result(manager)
            } else {
                self.manager_page.detection_results.get(manager)
            };
            let status = match status_result {
                None => text("Status: not scanned")
                    .size(14)
                    .style(theme::text_on_surface_alt)
                    .into(),
                Some(availability) => {
                    let (message, is_available) = match availability {
                        Ok(ManagerAvailability::Available { version }) => (
                            version.as_ref().map_or_else(
                                || "Available".to_owned(),
                                |version| format!("Available ({version})"),
                            ),
                            true,
                        ),
                        Ok(ManagerAvailability::Unavailable { reason }) => (
                            match reason {
                                AvailabilityReason::UnsupportedPlatform { .. } => {
                                    "Unsupported on this platform".to_owned()
                                }
                                AvailabilityReason::CommandMissing { command } => {
                                    format!("Not found: {command}")
                                }
                                AvailabilityReason::NotExecutable { path } => {
                                    format!("Not executable: {}", path.display())
                                }
                                AvailabilityReason::VersionCheckFailed { detail } => {
                                    format!("Version check failed: {detail}")
                                }
                                _ => "Unavailable".to_owned(),
                            },
                            false,
                        ),
                        Ok(_) => ("Unavailable".to_owned(), false),
                        Err(error) => (format!("Check failed: {error}"), false),
                    };

                    text(format!("Status: {message}"))
                        .size(14)
                        .style(if is_available {
                            theme::text_success
                        } else {
                            theme::text_warning
                        })
                        .into()
                }
            };
            content_items.push(status);
        }

        if let Some(config) = config {
            use iced::Alignment;

            let choose_label = if config.executable().is_some() {
                "Change Path"
            } else {
                "Select Path"
            };
            let mut path_actions = row![Self::secondary_button(
                choose_label,
                13.0,
                Some(Message::OpenDialog(manager.clone())),
            )]
            .spacing(10)
            .align_y(Alignment::Center);

            if config.executable().is_some() {
                path_actions = path_actions.push(Self::secondary_button(
                    "Use $PATH",
                    13.0,
                    Some(Message::ResetExecutable(manager.clone())),
                ));
            }
            content_items.push(path_actions.into());
        }

        // Go binary configuration.
        if is_configured && manager.as_str() == "builtin:go" {
            content_items.extend(self.view_go_bin_config(pm_config));
        }

        if is_configured && manager.as_str() == "builtin:nix-profile" {
            content_items.extend(self.view_nix_profile_config(pm_config));
        }

        shared::styled_container(
            row![
                icon::manager_logo(manager, &display_name, 42.0),
                column(content_items)
                    .spacing(theme::spacing::SM)
                    .width(iced::Length::Fill),
            ]
            .spacing(theme::spacing::MD)
            .align_y(Alignment::Start),
        )
        .into()
    }

    /// Go binary configuration rows.
    fn view_go_bin_config(
        &self,
        pm_config: &updater_core::Config,
    ) -> impl Iterator<Item = iced::Element<'static, Message>> {
        use iced::Alignment;
        use iced::widget::{row, text};

        let go_bin_info = pm_config
            .manager(&ManagerId::parse("builtin:go").expect("valid Go manager ID"))
            .and_then(|manager| configured_go_bin_dir(manager).ok().flatten())
            .map(|dir| format!("Binary Dir: {}", dir.display()))
            .unwrap_or_else(|| {
                "Binary Dir: Auto Detect (go env GOBIN > go env GOPATH/bin)".to_string()
            });

        let info_elem = text(go_bin_info)
            .size(14)
            .font(theme::FONT_MONO)
            .style(theme::text_on_surface_alt)
            .into();

        let change_btn =
            Self::secondary_button("Choose Binary Dir", 13.0, Some(Message::OpenGoBinDirDialog));

        let buttons = if pm_config
            .manager(&ManagerId::parse("builtin:go").expect("valid Go manager ID"))
            .and_then(|manager| configured_go_bin_dir(manager).ok().flatten())
            .is_some()
        {
            row![
                change_btn,
                Self::secondary_button("Reset As Auto Detect", 13.0, Some(Message::ClearGoBinDir))
            ]
            .spacing(10)
            .align_y(Alignment::Center)
        } else {
            row![change_btn].spacing(10).align_y(Alignment::Center)
        };

        [info_elem, buttons.into()].into_iter()
    }

    fn view_nix_profile_config(
        &self,
        pm_config: &updater_core::Config,
    ) -> impl Iterator<Item = iced::Element<'static, Message>> {
        use iced::widget::{row, text};

        let profile = pm_config
            .manager(&ManagerId::parse("builtin:nix-profile").expect("valid Nix manager ID"))
            .and_then(|manager| configured_nix_profile(manager).ok())
            .map(|path| format!("Profile: {}", path.display()))
            .unwrap_or_else(|| "Profile: not configured".to_owned());
        let info = text(profile)
            .size(14)
            .font(theme::FONT_MONO)
            .style(theme::text_on_surface_alt)
            .into();
        let actions = row![Self::secondary_button(
            "Change Profile",
            13.0,
            Some(Message::OpenNixProfileDialog),
        )]
        .spacing(10)
        .into();
        [info, actions].into_iter()
    }

    fn view_manager_buttons(&self) -> iced::Element<'static, Message> {
        use iced::widget::{container, row, svg};

        let detect_msg = if self.manager_page.is_detecting {
            None
        } else {
            Some(Message::DetectPackageManagers)
        };

        let detect_label = if self.manager_page.is_detecting {
            "Scanning $PATH..."
        } else {
            "Scan $PATH"
        };

        let detect_btn = Self::icon_button(
            svg::Svg::new(REFRESH_ICON.clone()).width(16).height(16),
            detect_label,
            16.0,
            detect_msg,
        );

        let mut actions = vec![detect_btn.into()];
        actions.extend(self.save_action_elements());

        container(
            row(actions)
                .spacing(12)
                .align_y(iced::Alignment::Center)
                .wrap(),
        )
        .into()
    }

    fn view_preference_buttons(&self) -> iced::Element<'static, Message> {
        use iced::widget::{container, row};

        container(
            row(self.save_action_elements())
                .spacing(12)
                .align_y(iced::Alignment::Center)
                .wrap(),
        )
        .into()
    }

    fn save_action_elements(&self) -> Vec<iced::Element<'static, Message>> {
        use iced::widget::{svg, text};

        let is_dirty = self.is_dirty();
        let mut actions = Vec::with_capacity(3);
        if is_dirty {
            actions.push(
                text("● Unsaved changes")
                    .size(14)
                    .font(theme::FONT_SEMIBOLD)
                    .color(theme::colors::SETTINGS)
                    .into(),
            );
            actions
                .push(Self::secondary_button("Revert", 14.0, Some(Message::RevertChanges)).into());
        }

        let save_msg = (is_dirty && !self.is_saving).then_some(Message::SaveConfig);
        let save_label = if self.is_saving {
            "Saving..."
        } else {
            "Save Configuration"
        };
        actions.push(
            Self::icon_button(
                svg::Svg::new(SAVE_ICON.clone()).width(16).height(16),
                save_label,
                16.0,
                save_msg,
            )
            .into(),
        );
        actions
    }

    /// Save status view.
    fn view_status(&self) -> iced::Element<'static, Message> {
        use iced::{
            Border,
            widget::{container, text},
        };

        if let Some(status) = &self.save_status {
            let (message, is_success) = match status {
                SaveStatus::Success if self.is_dirty() => (
                    "✓ Saved previous changes; newer changes remain unsaved".to_string(),
                    true,
                ),
                SaveStatus::Success => ("✓ Successfully Saved".to_string(), true),
                SaveStatus::Error(e) => (format!("✗ Failed To Save: {e}"), false),
            };

            container(text(message).size(14).style(move |iced_theme| {
                let semantic = theme::semantic_colors(iced_theme);
                iced::widget::text::Style {
                    color: Some(if is_success {
                        semantic.success
                    } else {
                        semantic.error
                    }),
                }
            }))
            .padding(12)
            .width(iced::Length::Fill)
            .style(move |iced_theme: &iced::Theme| {
                let semantic = theme::semantic_colors(iced_theme);
                let color = if is_success {
                    semantic.success
                } else {
                    semantic.error
                };
                container::Style {
                    background: Some(iced::Color::from_rgba(color.r, color.g, color.b, 0.1).into()),
                    border: Border {
                        color,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    text_color: None,
                    shadow: Default::default(),
                    snap: false,
                }
            })
            .into()
        } else {
            container(text("")).into()
        }
    }
}

const fn manager_category_rank(category: ManagerCategory) -> usize {
    match category {
        ManagerCategory::System => 0,
        ManagerCategory::Application => 1,
        ManagerCategory::Development => 2,
        ManagerCategory::Other => 3,
        _ => 3,
    }
}

const fn category_from_rank(rank: usize) -> ManagerCategory {
    match rank {
        0 => ManagerCategory::System,
        1 => ManagerCategory::Application,
        2 => ManagerCategory::Development,
        _ => ManagerCategory::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_id(value: &str) -> ManagerId {
        ManagerId::parse(value).unwrap()
    }

    fn config_with_manager(manager: &str) -> updater_core::Config {
        updater_core::Config {
            managers: vec![ManagerConfig::new(manager_id(manager))],
            ..updater_core::Config::default()
        }
    }

    #[test]
    fn draft_changes_do_not_mutate_active_config() {
        let active = config_with_manager("builtin:cargo");
        let mut settings = Settings::default();
        settings.sync_from_config(&active);

        settings
            .draft
            .managers
            .push(ManagerConfig::new(manager_id("builtin:flatpak")));

        assert!(settings.is_dirty());
        assert_eq!(active.managers.len(), 1);
        assert_eq!(settings.draft.managers.len(), 2);
    }

    #[test]
    fn discard_restores_last_baseline() {
        let active = config_with_manager("builtin:cargo");
        let mut settings = Settings::default();
        settings.sync_from_config(&active);
        settings.draft.managers[0].executable = Some("/tmp/cargo".into());

        settings.discard_changes();

        assert!(!settings.is_dirty());
        assert_eq!(settings.draft, active);
    }

    #[test]
    fn external_sync_does_not_overwrite_dirty_draft() {
        let active = config_with_manager("builtin:cargo");
        let replacement = config_with_manager("builtin:flatpak");
        let mut settings = Settings::default();
        settings.sync_from_config(&active);
        settings.draft.managers[0].executable = Some("/tmp/cargo".into());

        settings.sync_from_config(&replacement);

        assert!(settings.is_dirty());
        assert_eq!(settings.baseline, active);
        assert_eq!(
            settings.draft.managers[0].executable(),
            Some(std::path::Path::new("/tmp/cargo"))
        );
    }

    #[test]
    fn unknown_configured_manager_is_preserved_in_draft() {
        let active = config_with_manager("org.example:custom");
        let mut settings = Settings::default();

        settings.sync_from_config(&active);
        settings.draft.notifications_enabled = true;

        assert_eq!(settings.draft.managers, active.managers);
        assert!(settings.is_dirty());
    }

    #[test]
    fn reset_executable_restores_path_discovery_and_clears_stale_status() {
        let mut active = config_with_manager("builtin:cargo");
        active.managers[0].executable = Some("/custom/cargo".into());
        let manager = manager_id("builtin:cargo");
        let catalog = ManagerCatalog::builtin();
        let mut settings = Settings::default();
        settings.sync_from_config(&active);
        settings.manager_page.detection_results.insert(
            manager.clone(),
            Ok(ManagerAvailability::Unavailable {
                reason: AvailabilityReason::CommandMissing {
                    command: "/custom/cargo".to_owned(),
                },
            }),
        );

        let action = settings.update(Message::ResetExecutable(manager.clone()), &active, &catalog);

        assert!(matches!(action, Action::ManagerConfigChanged));
        assert_eq!(settings.draft.manager_executable(&manager), None);
        assert!(
            !settings
                .manager_page
                .detection_results
                .contains_key(&manager)
        );
        assert!(settings.is_dirty());
    }

    #[test]
    fn unconfigured_nix_scan_waits_for_explicit_profile_selection() {
        let active = updater_core::Config::default();
        let manager = manager_id("builtin:nix-profile");
        let catalog = ManagerCatalog::builtin();
        let mut settings = Settings {
            manager_page: ManagerPageState {
                is_detecting: true,
                ..ManagerPageState::default()
            },
            ..Settings::default()
        };

        let action = settings.update(
            Message::FinishDetect(vec![(
                manager.clone(),
                Err("Nix profile settings are invalid".to_owned()),
            )]),
            &active,
            &catalog,
        );

        assert!(matches!(action, Action::None));
        assert!(!settings.manager_page.is_detecting);
        assert!(!settings.manager_page.detected_in_path.contains(&manager));
        assert!(
            !settings
                .manager_page
                .detection_results
                .contains_key(&manager)
        );
        assert!(settings.draft.manager(&manager).is_none());
    }

    #[test]
    fn failed_executable_validation_keeps_draft_and_baseline() {
        let active = config_with_manager("builtin:cargo");
        let manager = manager_id("builtin:cargo");
        let catalog = ManagerCatalog::builtin();
        let mut settings = Settings::default();
        settings.sync_from_config(&active);
        settings.draft.managers[0].executable = Some("/invalid/cargo".into());
        settings.is_saving = true;
        let attempted = settings.draft.clone();

        let action = settings.update(
            Message::SaveConfigResult {
                config: attempted.clone(),
                validation: vec![(
                    manager.clone(),
                    Ok(ManagerAvailability::Unavailable {
                        reason: AvailabilityReason::NotExecutable {
                            path: "/invalid/cargo".into(),
                        },
                    }),
                )],
                result: Err("1 custom executable path(s) failed validation".to_owned()),
            },
            &active,
            &catalog,
        );

        assert!(matches!(action, Action::None));
        assert!(!settings.is_saving);
        assert_eq!(settings.baseline, active);
        assert_eq!(settings.draft, attempted);
        assert!(settings.is_dirty());
        assert!(
            settings
                .manager_page
                .detection_results
                .get(&manager)
                .is_some_and(|result| {
                    result
                        .as_ref()
                        .is_ok_and(|detection_results| !detection_results.is_available())
                })
        );
        assert!(matches!(settings.save_status, Some(SaveStatus::Error(_))));
    }

    #[test]
    fn successful_save_applies_snapshot_without_absorbing_newer_draft() {
        let active = config_with_manager("builtin:cargo");
        let manager = manager_id("builtin:cargo");
        let catalog = ManagerCatalog::builtin();
        let mut settings = Settings::default();
        settings.sync_from_config(&active);

        let mut saved = active.clone();
        saved.appearance = "dark".to_owned();
        saved.managers[0].executable = Some("/saved/cargo".into());
        settings.draft.clone_from(&saved);
        settings.draft.notifications_enabled = true;
        settings.draft.managers[0].executable = Some("/newer/cargo".into());
        settings.is_saving = true;

        let action = settings.update(
            Message::SaveConfigResult {
                config: saved.clone(),
                validation: vec![(
                    manager.clone(),
                    Ok(ManagerAvailability::Available {
                        version: Some("1.0.0".to_owned()),
                    }),
                )],
                result: Ok(()),
            },
            &active,
            &catalog,
        );

        let Action::ApplySavedConfig(applied) = action else {
            panic!("successful save must apply the persisted snapshot");
        };
        assert_eq!(applied, saved);
        assert_eq!(settings.baseline, saved);
        assert!(settings.draft.notifications_enabled);
        assert_eq!(
            settings.draft.manager_executable(&manager),
            Some("/newer/cargo".into())
        );
        assert!(
            !settings
                .manager_page
                .detection_results
                .contains_key(&manager)
        );
        assert!(settings.is_dirty());
        assert!(!settings.is_saving);
    }
}
