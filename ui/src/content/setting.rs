use iced::Task;
use rfd::FileHandle;
use updater_core::{ALL_APP_PACKAGE_MANAGERS, Config, PackageManagerConfig, PackageManagerType};

use crate::{
    app::{self},
    content::{self},
    icon::{ADD_ICON, REFRESH_ICON, SAVE_ICON},
};

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub is_saving: bool,
    pub is_detecting: bool,
    pub selecting_manager: Option<PackageManagerType>,
    pub save_status: Option<SaveStatus>,
}

#[derive(Debug, Clone)]
pub enum SaveStatus {
    Success,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    DetectPackageManagers,
    FinishDetect(Config),
    SaveConfig,
    SaveConfigResult(Result<(), String>),
    ConfigReloaded(Result<(), String>),
    OpenDialog(PackageManagerType),
    SelectedPath(FileHandle),
    CancelSelection,
    OpenGoBinDirDialog,
    SelectedGoBinDir(FileHandle),
    ClearGoBinDir,
}

impl From<Message> for content::Message {
    fn from(msg: Message) -> Self {
        content::Message::Settings(msg)
    }
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(iced::Task<Message>),
}

impl Settings {
    fn section_title(text: &'static str) -> iced::widget::Text<'static> {
        iced::widget::text(text)
            .size(18)
            .color(app::colors::ON_SURFACE)
    }

    fn styled_container<'a>(
        content: impl Into<iced::Element<'a, Message>>,
    ) -> iced::widget::Container<'a, Message> {
        use iced::{Border, widget::container};

        container(content)
            .padding(16)
            .width(iced::Length::Fill)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(app::colors::SURFACE.into()),
                border: Border {
                    color: app::colors::DIVIDER,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                text_color: None,
                shadow: Default::default(),
                snap: false,
            })
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

        let btn = button(
            row![icon, text(label).size(size)]
                .spacing(if size > 14.0 { 8 } else { 6 })
                .align_y(Alignment::Center),
        )
        .padding(if size > 14.0 { [12, 24] } else { [8, 16] })
        .style(|_theme, status| {
            use iced::widget::button::{Status, Style};
            use iced::{Background, Border, Shadow, Vector};

            let is_disabled = matches!(status, Status::Disabled);

            if is_disabled {
                Style {
                    background: Some(Background::Color(app::colors::SURFACE)),
                    text_color: app::colors::ON_SURFACE_MUTED,
                    border: Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    shadow: Shadow::default(),
                    snap: false,
                }
            } else {
                let (bg_color, shadow_offset) = match status {
                    Status::Hovered => (app::colors::PRIMARY_HOVER, 3.0),
                    Status::Pressed => (app::colors::PRIMARY_ACTIVE, 1.0),
                    _ => (app::colors::PRIMARY, 2.0),
                };

                Style {
                    background: Some(Background::Color(bg_color)),
                    text_color: app::colors::ON_PRIMARY,
                    border: Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    shadow: Shadow {
                        color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                        offset: Vector::new(0.0, shadow_offset),
                        blur_radius: 8.0,
                    },
                    snap: false,
                }
            }
        });

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

        let btn = button(text(label).size(size))
            .padding(if size > 14.0 { [12, 24] } else { [8, 16] })
            .style(|_theme, status| {
                use iced::widget::button::{Status, Style};
                use iced::{Background, Border, Shadow, Vector};

                let is_disabled = matches!(status, Status::Disabled);

                if is_disabled {
                    Style {
                        background: Some(Background::Color(app::colors::SURFACE)),
                        text_color: app::colors::ON_SURFACE_MUTED,
                        border: Border {
                            radius: 8.0.into(),
                            ..Default::default()
                        },
                        shadow: Shadow::default(),
                        snap: false,
                    }
                } else {
                    let (bg_color, text_color, shadow_offset) = match status {
                        Status::Hovered => {
                            (app::colors::SURFACE_HOVER, app::colors::ON_SURFACE, 2.0)
                        }
                        Status::Pressed => {
                            (app::colors::SURFACE_PRESSED, app::colors::ON_SURFACE, 0.5)
                        }
                        _ => (app::colors::SURFACE, app::colors::ON_SURFACE, 1.0),
                    };

                    Style {
                        background: Some(Background::Color(bg_color)),
                        text_color,
                        border: Border {
                            color: app::colors::DIVIDER,
                            width: 1.0,
                            radius: 8.0.into(),
                        },
                        shadow: Shadow {
                            color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.08),
                            offset: Vector::new(0.0, shadow_offset),
                            blur_radius: 4.0,
                        },
                        snap: false,
                    }
                }
            });

        if let Some(msg) = message {
            btn.on_press(msg)
        } else {
            btn
        }
    }

    pub fn update(&mut self, message: Message, pm_config: &mut updater_core::Config) -> Action {
        match message {
            Message::DetectPackageManagers => {
                self.is_detecting = true;
                let task = Task::future(Config::detect_package_managers())
                    .then(|detected_config| Task::done(Message::FinishDetect(detected_config)));
                Action::Run(task)
            }
            Message::FinishDetect(result) => {
                self.is_detecting = false;
                *pm_config = result;
                Action::None
            }
            Message::SaveConfig => {
                self.is_saving = true;
                self.save_status = None;
                self.save_config(
                    pm_config.system_manager.as_ref(),
                    pm_config.app_managers.as_ref(),
                    pm_config.go_bin_dir.as_ref(),
                )
            }
            Message::SaveConfigResult(result) => {
                self.is_saving = false;
                match result {
                    Ok(_) => {
                        log::debug!("Configuration saved successfully");
                        self.save_status = Some(SaveStatus::Success);

                        // Reload config after save
                        let mut config = pm_config.clone();
                        let task = Task::perform(
                            async move { config.reload().await.map_err(|e| e.to_string()) },
                            Message::ConfigReloaded,
                        );
                        return Action::Run(task);
                    }
                    Err(e) => {
                        log::error!("Failed to save configuration: {}", e);
                        self.save_status = Some(SaveStatus::Error(e));
                    }
                }
                Action::None
            }
            Message::ConfigReloaded(result) => {
                match result {
                    Ok(_) => {
                        log::debug!("Configuration reloaded successfully");
                    }
                    Err(e) => {
                        log::error!("Failed to reload configuration: {}", e);
                    }
                }
                Action::None
            }
            Message::OpenDialog(manager_type) => {
                self.selecting_manager = Some(manager_type);

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
                if let Some(manager_type) = self.selecting_manager {
                    let path = file_handle.path().to_string_lossy().to_string();

                    pm_config.app_managers.push(PackageManagerConfig {
                        manager_type,
                        custom_path: Some(path),
                    });
                } else {
                    log::error!("No package manager type selected when handling SelectedPath");
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
                pm_config.go_bin_dir = Some(path);
                Action::None
            }
            Message::ClearGoBinDir => {
                pm_config.go_bin_dir = None;
                Action::None
            }
        }
    }

    pub fn view(&self, pm_config: &updater_core::Config) -> iced::Element<'static, Message> {
        use iced::Length;
        use iced::widget::{column, container};

        let content = column![
            self.view_header(),
            self.view_system_manager_section(pm_config.system_manager.as_ref()),
            self.view_app_manager_section(pm_config),
            self.view_selection_list(pm_config),
            self.view_buttons(),
            self.view_status(),
        ]
        .spacing(24)
        .padding(20)
        .width(Length::Fill);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_header(&self) -> iced::Element<'static, Message> {
        use iced::widget::text;

        text("Package Manager Settings").size(24).into()
    }

    fn view_system_manager_section(
        &self,
        system_manager: Option<&PackageManagerConfig>,
    ) -> iced::Element<'static, Message> {
        use iced::widget::{column, row, text};

        let content = if let Some(manager) = system_manager {
            let path_info = manager
                .custom_path
                .as_ref()
                .map(|p| format!("Path: {}", p))
                .unwrap_or_else(|| "Path: $PATH (System Default)".to_string());

            column![
                row![
                    text(manager.manager_type.name()).size(16),
                    text("✓").size(16).color(app::colors::SUCCESS),
                ]
                .spacing(10),
                text(path_info)
                    .size(14)
                    .color(app::colors::ON_SURFACE_MUTED),
            ]
            .spacing(8)
        } else {
            column![
                text("Not detected")
                    .size(16)
                    .color(app::colors::ON_SURFACE_MUTED)
            ]
            .spacing(8)
        };

        column![
            Self::section_title("System Package Manager"),
            Self::styled_container(content)
        ]
        .spacing(12)
        .into()
    }

    /// App Package Manager Section
    fn view_app_manager_section(
        &self,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'static, Message> {
        use iced::widget::{column, text};

        let managers_list = if pm_config.app_managers.is_empty() {
            column![
                text("No application package managers detected")
                    .size(16)
                    .color(app::colors::ON_SURFACE_MUTED)
            ]
        } else {
            column(
                pm_config
                    .app_managers
                    .iter()
                    .map(|manager| self.view_manager_item(manager, true, pm_config))
                    .collect::<Vec<_>>(),
            )
            .spacing(12)
        };

        column![Self::section_title("App Package Manager"), managers_list]
            .spacing(12)
            .width(iced::Length::Fill)
            .into()
    }

    fn view_selection_list(
        &self,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'static, Message> {
        use iced::Alignment;
        use iced::widget::{column, row, svg, text};

        let selection_list: Vec<PackageManagerConfig> = ALL_APP_PACKAGE_MANAGERS
            .iter()
            .filter(|t| !pm_config.app_managers.iter().any(|m| m.manager_type == **t))
            .map(|t| PackageManagerConfig {
                manager_type: *t,
                custom_path: None,
            })
            .collect();

        let managers_list: iced::Element<'_, Message> = if selection_list.is_empty() {
            column![
                text("All available package managers have been added")
                    .size(16)
                    .color(app::colors::ON_SURFACE_MUTED)
            ]
            .into()
        } else {
            column(
                selection_list
                    .iter()
                    .map(|manager| {
                        let add_btn = Self::icon_button(
                            svg::Svg::new(ADD_ICON.clone()).width(16).height(16),
                            "Add",
                            16.0,
                            Some(Message::OpenDialog(manager.manager_type)),
                        );

                        row![self.view_manager_item(manager, false, pm_config), add_btn]
                            .spacing(12)
                            .align_y(Alignment::Center)
                            .into()
                    })
                    .collect::<Vec<_>>(),
            )
            .spacing(12)
            .into()
        };

        column![
            Self::section_title("Add Other Package Manager"),
            managers_list
        ]
        .spacing(12)
        .width(iced::Length::Fill)
        .into()
    }

    fn view_manager_item(
        &self,
        manager: &PackageManagerConfig,
        is_configured: bool,
        pm_config: &updater_core::Config,
    ) -> iced::Element<'static, Message> {
        use iced::widget::{column, row, text};

        let name_row = if is_configured {
            row![
                text(manager.manager_type.name()).size(16),
                text("✓").size(16).color(app::colors::SUCCESS)
            ]
            .spacing(10)
        } else {
            row![text(manager.manager_type.name()).size(16)].spacing(10)
        };

        let info_text = if is_configured {
            manager
                .custom_path
                .as_ref()
                .map(|p| format!("Path: {}", p))
                .unwrap_or_else(|| "Path: $PATH (System Default)".to_string())
        } else {
            manager.manager_type.description().to_string()
        };

        let mut content_items = vec![
            name_row.into(),
            text(info_text)
                .size(14)
                .color(app::colors::ON_SURFACE_MUTED)
                .into(),
        ];

        // GO Binary
        if is_configured && manager.manager_type == PackageManagerType::Go {
            content_items.extend(self.view_go_bin_config(pm_config));
        }

        Self::styled_container(column(content_items).spacing(8)).into()
    }

    /// Just For fun To change return type from Vec to Iterator
    /// Optimized code is often more complex and takes more effort to write than unoptimized code.
    /// For this reason, it is only worth optimizing hot code.
    fn view_go_bin_config(
        &self,
        pm_config: &updater_core::Config,
    ) -> impl Iterator<Item = iced::Element<'static, Message>> {
        // Vec<iced::Element<'static, Message>>
        use iced::Alignment;
        use iced::widget::{row, text};

        let go_bin_info = pm_config
            .go_bin_dir
            .as_ref()
            .map(|dir| format!("Binary Dir: {}", dir))
            .unwrap_or_else(|| {
                "Binary Dir: Auto Detect (GOBIN > GOPATH/bin > ~/go/bin)".to_string()
            });

        let info_elem = text(go_bin_info)
            .size(13)
            .color(app::colors::ON_SURFACE_ALT)
            .into();

        let change_btn =
            Self::secondary_button("Choose Binary Dir", 13.0, Some(Message::OpenGoBinDirDialog));

        let buttons = if pm_config.go_bin_dir.is_some() {
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

    /// Buttons Widgets
    fn view_buttons(&self) -> iced::Element<'static, Message> {
        use iced::widget::{container, row, svg};

        let detect_msg = if self.is_detecting {
            None
        } else {
            Some(Message::DetectPackageManagers)
        };

        let detect_label = if self.is_detecting {
            "Detecting..."
        } else {
            "Detect Package Managers"
        };

        let detect_btn = Self::icon_button(
            svg::Svg::new(REFRESH_ICON.clone()).width(16).height(16),
            detect_label,
            16.0,
            detect_msg,
        );

        let save_msg = if self.is_saving {
            None
        } else {
            Some(Message::SaveConfig)
        };
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

        container(row![detect_btn, save_btn].spacing(16))
            .padding([0, 20])
            .into()
    }

    /// Status View
    fn view_status(&self) -> iced::Element<'static, Message> {
        use iced::{
            Border,
            widget::{container, text},
        };

        if let Some(status) = &self.save_status {
            let (message, color, border_color) = match status {
                SaveStatus::Success => (
                    "✓ Successfully Saved".to_string(),
                    app::colors::SUCCESS,
                    app::colors::SUCCESS,
                ),
                SaveStatus::Error(e) => (
                    format!("✗ Failed To Save: {}", e),
                    app::colors::ERROR,
                    app::colors::ERROR,
                ),
            };

            container(text(message).size(14).color(color))
                .padding(12)
                .width(iced::Length::Fill)
                .style(move |_theme: &iced::Theme| container::Style {
                    background: Some(iced::Color::from_rgba(color.r, color.g, color.b, 0.1).into()),
                    border: Border {
                        color: border_color,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    text_color: None,
                    shadow: Default::default(),
                    snap: false,
                })
                .into()
        } else {
            container(text("")).into()
        }
    }

    fn save_config(
        &self,
        system_manager: Option<&PackageManagerConfig>,
        app_managers: &[PackageManagerConfig],
        go_bin_dir: Option<&String>,
    ) -> Action {
        let system_manager = system_manager.cloned();
        let app_managers = app_managers.to_vec();
        let go_bin_dir = go_bin_dir.cloned();

        let task = iced::Task::perform(
            async move {
                let config = updater_core::Config {
                    system_manager,
                    app_managers,
                    go_bin_dir,
                };

                config.save().await.map_err(|e| e.to_string())
            },
            Message::SaveConfigResult,
        );

        Action::Run(task)
    }
}
