use std::collections::HashMap;

use iced::Task;
use rfd::FileHandle;
use updater_core::ManagerConfig;
use updater_manager_api::{AvailabilityReason, ManagerAvailability, ManagerCategory, ManagerId};

use crate::{
    content::shared,
    icon::{ADD_ICON, REFRESH_ICON, SAVE_ICON},
    manager_catalog::ManagerCatalog,
    theme,
};

#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// Editable configuration shown on the Settings page.
    draft: updater_core::Config,
    /// Last configuration synchronized from or saved to persistent storage.
    baseline: updater_core::Config,
    /// Whether the draft has been initialized from application state.
    is_initialized: bool,
    /// Whether config save is in progress.
    pub is_saving: bool,
    /// Whether package-manager auto detection is in progress.
    pub is_detecting: bool,
    /// Manager currently waiting for custom-path selection.
    pub selecting_manager: Option<ManagerId>,
    /// Managers detected from PATH scan.
    pub detected_in_path: Vec<ManagerId>,
    /// Last availability result for each manager.
    pub availability: HashMap<ManagerId, Result<ManagerAvailability, String>>,
    /// Last save result shown in UI.
    pub save_status: Option<SaveStatus>,
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
    SaveConfigResult(Result<(), String>),
    /// Manager-path dialog message.
    OpenDialog(ManagerId),
    /// Manager-path selection message.
    SelectedPath(FileHandle),
    /// Selection-cancel message.
    CancelSelection,
    /// Go-bin directory dialog message.
    OpenGoBinDirDialog,
    /// Go-bin directory selection message.
    SelectedGoBinDir(FileHandle),
    /// Go-bin directory clear message.
    ClearGoBinDir,
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

    pub fn appearance_value(&self) -> String {
        self.draft.appearance.clone()
    }

    pub fn discard_changes(&mut self) {
        self.draft.clone_from(&self.baseline);
        self.save_status = None;
        self.selecting_manager = None;
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
            Message::DetectPackageManagers => {
                self.is_detecting = true;
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
                        let availability = manager
                            .availability(&manager_config)
                            .await
                            .map_err(|error| error.to_string());
                        results.push((id, availability));
                    }
                    results
                })
                .then(|detected_managers| Task::done(Message::FinishDetect(detected_managers)));
                Action::Run(task)
            }
            Message::FinishDetect(results) => {
                self.is_detecting = false;
                self.detected_in_path = results
                    .iter()
                    .filter_map(|(manager, availability)| {
                        availability
                            .as_ref()
                            .is_ok_and(ManagerAvailability::is_available)
                            .then_some(manager.clone())
                    })
                    .collect();
                self.availability = results.into_iter().collect();
                Action::None
            }
            Message::AddDetectedManager(manager) => {
                let exists = self.draft.manager(&manager).is_some();

                if !exists {
                    self.draft.managers.push(ManagerConfig::new(manager));
                }
                Action::None
            }
            Message::UnloadManager(manager) => {
                self.draft
                    .managers
                    .retain(|configured| configured.id != manager);
                Action::None
            }
            Message::SaveConfig => {
                if !self.is_dirty() || self.is_saving {
                    return Action::None;
                }
                self.is_saving = true;
                self.save_status = None;
                let config = self.draft.clone();
                Action::Run(iced::Task::perform(
                    async move { config.save().await.map_err(|error| error.to_string()) },
                    Message::SaveConfigResult,
                ))
            }
            Message::SaveConfigResult(result) => {
                self.is_saving = false;
                match result {
                    Ok(()) => {
                        log::debug!("Configuration saved successfully");
                        self.baseline.clone_from(&self.draft);
                        self.save_status = Some(SaveStatus::Success);
                        Action::ApplySavedConfig(self.draft.clone())
                    }
                    Err(e) => {
                        log::error!("Failed to save configuration: {}", e);
                        self.save_status = Some(SaveStatus::Error(e));
                        Action::None
                    }
                }
            }
            Message::OpenDialog(manager) => {
                self.selecting_manager = Some(manager);

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
                if let Some(manager) = self.selecting_manager.take() {
                    let path = file_handle.path().to_path_buf();
                    self.availability.remove(&manager);

                    if let Some(existing) = self.draft.manager_mut(&manager) {
                        existing.executable = Some(path);
                    } else {
                        self.draft
                            .managers
                            .push(ManagerConfig::new(manager).with_executable(path));
                    }
                } else {
                    log::error!("No package manager selected when handling SelectedPath");
                }

                Action::None
            }
            Message::CancelSelection => {
                self.selecting_manager = None;
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
                let path = file_handle.path().to_string_lossy().to_string();
                if let Err(error) = self.draft.set_go_bin_dir(Some(path)) {
                    self.save_status = Some(SaveStatus::Error(error.to_string()));
                }
                Action::None
            }
            Message::ClearGoBinDir => {
                if let Err(error) = self.draft.set_go_bin_dir(None) {
                    self.save_status = Some(SaveStatus::Error(error.to_string()));
                }
                Action::None
            }
            Message::RevertChanges => {
                self.discard_changes();
                Action::None
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

    pub fn view(
        &self,
        _active_config: &updater_core::Config,
        catalog: &ManagerCatalog,
    ) -> iced::Element<'static, Message> {
        use iced::Length;
        use iced::widget::{column, container, scrollable};

        let pm_config = &self.draft;

        let content = column![
            shared::page_header(
                "Settings",
                format!("{} package managers configured", pm_config.managers.len()),
                theme::colors::SETTINGS,
            ),
            self.view_system_manager_section(pm_config, catalog),
            self.view_appearance_section(pm_config),
            self.view_app_manager_section(pm_config, catalog),
            self.view_selection_list(pm_config, catalog),
            self.view_buttons(),
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

    fn view_system_manager_section(
        &self,
        pm_config: &updater_core::Config,
        catalog: &ManagerCatalog,
    ) -> iced::Element<'static, Message> {
        use iced::widget::{column, text};

        let system_managers = pm_config
            .managers
            .iter()
            .filter(|manager| {
                catalog
                    .descriptor(&manager.id)
                    .is_some_and(|descriptor| descriptor.category() == ManagerCategory::System)
            })
            .collect::<Vec<_>>();

        let content = if system_managers.is_empty() {
            column![
                text("Not detected")
                    .size(16)
                    .style(theme::text_on_surface_muted)
            ]
            .spacing(8)
        } else {
            column(
                system_managers
                    .into_iter()
                    .map(|manager| {
                        self.view_manager_item(
                            &manager.id,
                            Some(manager),
                            false,
                            pm_config,
                            catalog,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .spacing(12)
        };

        column![
            Self::section_title("System Package Manager"),
            shared::styled_container(content)
        ]
        .spacing(12)
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

    /// Application and development package manager section.
    fn view_app_manager_section(
        &self,
        pm_config: &updater_core::Config,
        catalog: &ManagerCatalog,
    ) -> iced::Element<'static, Message> {
        use iced::Alignment;
        use iced::widget::{column, container, row, text};

        let configured_managers = pm_config
            .managers
            .iter()
            .filter(|manager| {
                catalog
                    .descriptor(&manager.id)
                    .is_none_or(|descriptor| descriptor.category() != ManagerCategory::System)
            })
            .collect::<Vec<_>>();

        let managers_list = if configured_managers.is_empty() {
            column![
                text("No application or development package managers configured")
                    .size(16)
                    .style(theme::text_on_surface_muted)
            ]
        } else {
            column(
                configured_managers
                    .into_iter()
                    .map(|manager| {
                        let unload_btn = Self::secondary_button(
                            "Unload",
                            14.0,
                            Some(Message::UnloadManager(manager.id.clone())),
                        );

                        row![
                            container(self.view_manager_item(
                                &manager.id,
                                Some(manager),
                                false,
                                pm_config,
                                catalog,
                            ))
                            .width(iced::Length::Fill),
                            unload_btn
                        ]
                        .spacing(12)
                        .align_y(Alignment::Center)
                        .width(iced::Length::Fill)
                        .into()
                    })
                    .collect::<Vec<_>>(),
            )
            .spacing(12)
        };

        column![
            Self::section_title("Application & Development Package Managers"),
            managers_list
        ]
        .spacing(12)
        .width(iced::Length::Fill)
        .into()
    }

    fn view_selection_list(
        &self,
        pm_config: &updater_core::Config,
        catalog: &ManagerCatalog,
    ) -> iced::Element<'static, Message> {
        use iced::Alignment;
        use iced::widget::{column, container, row, svg, text};

        let selection_list: Vec<ManagerId> = catalog
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

        let detected_count = selection_list
            .iter()
            .filter(|manager| self.detected_in_path.contains(manager))
            .count();

        let detect_tip = if self.is_detecting {
            "Scanning $PATH...".to_string()
        } else if detected_count == 0 {
            "Click \"Scan $PATH\" to discover available managers.".to_string()
        } else {
            format!(
                "Detected {} manager(s) in $PATH. They will only be added after manual Add.",
                detected_count
            )
        };

        let managers_list: iced::Element<'_, Message> = if selection_list.is_empty() {
            column![
                text("All available package managers have been added")
                    .size(16)
                    .style(theme::text_on_surface_muted)
            ]
            .into()
        } else {
            column(
                selection_list
                    .iter()
                    .map(|manager| {
                        let detected_in_path = self.detected_in_path.contains(manager);

                        let action_message = if detected_in_path {
                            Message::AddDetectedManager(manager.clone())
                        } else {
                            Message::OpenDialog(manager.clone())
                        };

                        let action_label = if detected_in_path {
                            "Add"
                        } else {
                            "Select Path"
                        };

                        let add_btn = Self::icon_button(
                            svg::Svg::new(ADD_ICON.clone()).width(16).height(16),
                            action_label,
                            16.0,
                            Some(action_message),
                        );

                        row![
                            container(self.view_manager_item(
                                manager,
                                None,
                                detected_in_path,
                                pm_config,
                                catalog,
                            ))
                            .width(iced::Length::Fill),
                            add_btn
                        ]
                        .spacing(12)
                        .align_y(Alignment::Center)
                        .width(iced::Length::Fill)
                        .into()
                    })
                    .collect::<Vec<_>>(),
            )
            .spacing(12)
            .into()
        };

        column![
            Self::section_title("Add Other Package Manager"),
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
    ) -> iced::Element<'static, Message> {
        use iced::widget::{column, row, text};

        let is_configured = config.is_some();
        let display_name = catalog.display_name(manager).to_owned();
        let name_row = if is_configured {
            row![
                text(display_name).size(16),
                text("✓").size(16).style(theme::text_success)
            ]
            .spacing(10)
        } else {
            row![text(display_name).size(16)].spacing(10)
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
                .size(13)
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
                    .size(13)
                    .style(theme::text_warning)
                    .into(),
            );
        } else if !catalog.supports_current_platform(manager) {
            content_items.push(
                text("Status: unsupported on this platform")
                    .size(13)
                    .style(theme::text_warning)
                    .into(),
            );
        } else if is_configured || detected_in_path || self.availability.contains_key(manager) {
            let status = match self.availability.get(manager) {
                None => text("Status: not scanned")
                    .size(13)
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
                        .size(13)
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

        // Go binary configuration.
        if is_configured && manager.as_str() == "builtin:go" {
            content_items.extend(self.view_go_bin_config(pm_config));
        }

        shared::styled_container(column(content_items).spacing(8)).into()
    }

    /// Go binary configuration rows.
    fn view_go_bin_config(
        &self,
        pm_config: &updater_core::Config,
    ) -> impl Iterator<Item = iced::Element<'static, Message>> {
        use iced::Alignment;
        use iced::widget::{row, text};

        let go_bin_info = pm_config
            .go_bin_dir()
            .map(|dir| format!("Binary Dir: {}", dir))
            .unwrap_or_else(|| {
                "Binary Dir: Auto Detect (go env GOBIN > go env GOPATH/bin)".to_string()
            });

        let info_elem = text(go_bin_info)
            .size(12)
            .font(theme::FONT_MONO)
            .style(theme::text_on_surface_alt)
            .into();

        let change_btn =
            Self::secondary_button("Choose Binary Dir", 13.0, Some(Message::OpenGoBinDirDialog));

        let buttons = if pm_config.go_bin_dir().is_some() {
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

    /// Action buttons row.
    fn view_buttons(&self) -> iced::Element<'static, Message> {
        use iced::widget::{container, row, svg, text};

        let detect_msg = if self.is_detecting {
            None
        } else {
            Some(Message::DetectPackageManagers)
        };

        let detect_label = if self.is_detecting {
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

        let is_dirty = self.is_dirty();
        let save_msg = (is_dirty && !self.is_saving).then_some(Message::SaveConfig);
        let save_label = if self.is_saving {
            "Saving..."
        } else {
            "Save Configuration"
        };

        let save_btn = Self::icon_button(
            svg::Svg::new(SAVE_ICON.clone()).width(16).height(16),
            save_label,
            16.0,
            save_msg,
        );

        let mut actions = row![detect_btn]
            .spacing(12)
            .align_y(iced::Alignment::Center);
        if is_dirty {
            actions = actions
                .push(
                    text("● Unsaved changes")
                        .size(13)
                        .font(theme::FONT_SEMIBOLD)
                        .color(theme::colors::SETTINGS),
                )
                .push(Self::secondary_button(
                    "Revert",
                    14.0,
                    Some(Message::RevertChanges),
                ));
        }
        actions = actions.push(save_btn);

        container(actions).into()
    }

    /// Save status view.
    fn view_status(&self) -> iced::Element<'static, Message> {
        use iced::{
            Border,
            widget::{container, text},
        };

        if let Some(status) = &self.save_status {
            let (message, is_success) = match status {
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
}
