//! Top-level application composition and message routing.

use std::sync::Arc;
use std::time::Instant;

use futures::future::join_all;
use iced::{Length, Subscription, Task};
use updater_core::{PackageManagerType, PackageUpdate};

use crate::{
    content::{self, Content, FindingInfo, InstalledInfo, UpdatesInfo},
    sidebar::{self, SideBar},
    status_panel::{self, StatusPanel},
};

#[allow(dead_code)]
pub mod colors {
    use iced::Color;

    // Primary - Light Green
    pub const PRIMARY: Color = Color::from_rgb8(211, 249, 216); // #d3f9d8
    pub const PRIMARY_HOVER: Color = Color::from_rgb8(196, 240, 204); // #c4f0cc
    pub const PRIMARY_ACTIVE: Color = Color::from_rgb8(173, 231, 190); // #ade7be
    pub const PRIMARY_LIGHT: Color = Color::from_rgb8(228, 252, 228); // #e4fce4
    pub const PRIMARY_MUTED: Color = Color::from_rgb8(180, 220, 180); // #b4dcb4

    // Secondary - Cyan Blue
    pub const SECONDARY: Color = Color::from_rgb8(59, 201, 219); // #3bc9db
    pub const SECONDARY_HOVER: Color = Color::from_rgb8(34, 184, 207); // #22b8cf
    pub const SECONDARY_ACTIVE: Color = Color::from_rgb8(21, 170, 191); // #15aabf
    pub const SECONDARY_SOFT: Color = Color::from_rgb8(150, 219, 230); // #96dbe6

    // Surface
    pub const SURFACE: Color = Color::from_rgb8(247, 248, 250); // #f7f8fa
    pub const SURFACE_HOVER: Color = Color::from_rgb8(238, 240, 243); // #eef0f3
    pub const SURFACE_PRESSED: Color = Color::from_rgb8(222, 226, 230); // #dee2e6
    pub const SURFACE_MUTED: Color = Color::from_rgb8(245, 246, 248); // #f5f6f8
    pub const SURFACE_ALT: Color = Color::from_rgb8(250, 251, 253); // #fafbfd

    // Foreground
    pub const ON_PRIMARY: Color = Color::from_rgb8(34, 52, 40); // #223428
    pub const ON_SURFACE: Color = Color::from_rgb8(52, 58, 64); // #343a40
    pub const ON_SURFACE_IDLE: Color = Color::from_rgb8(77, 85, 92); // #4d555c
    pub const ON_SURFACE_MUTED: Color = Color::from_rgb8(130, 138, 145); // #828a91
    pub const ON_SURFACE_ALT: Color = Color::from_rgb8(95, 102, 110); // #5f666e

    // Accent
    pub const ACCENT: Color = Color::from_rgb8(173, 231, 190); // #ade7be
    pub const ACCENT_HOVER: Color = Color::from_rgb8(150, 210, 170); // #96d2aa
    pub const ACCENT_MUTED: Color = Color::from_rgb8(200, 240, 210); // #c8f0d2

    // Status
    pub const SUCCESS: Color = Color::from_rgb8(76, 175, 80); // #4caf50
    pub const WARNING: Color = Color::from_rgb8(255, 193, 7); // #ffc107
    pub const ERROR: Color = Color::from_rgb8(244, 67, 54); // #f44336

    // Helpers
    pub const DIVIDER: Color = Color::from_rgb8(220, 224, 228); // #dce0e4
    pub const DIVIDER_LIGHT: Color = Color::from_rgb8(235, 238, 242); // #ebeef2
    pub const SHADOW: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.08);
    pub const SHADOW_LIGHT: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.05);
    pub const SHADOW_HEAVY: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.2);
    pub const OVERLAY: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.3);
    pub const OVERLAY_LIGHT: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.08);
    pub const FOCUS: Color = Color::from_rgb8(173, 231, 190); // #ade7be
    pub const DISABLED: Color = Color::from_rgb8(200, 205, 210); // #c8cdd2
}

/// Root GUI state for the updater application.
#[derive(Debug, Clone)]
pub struct App {
    /// Left navigation.
    pub sidebar: SideBar,
    /// Main content area state.
    pub content: Content,

    /// Loaded package manager configuration.
    pub pm_config: updater_core::Config,
    /// State for installed packages page.
    pub installed_info: InstalledInfo,
    /// State for updates page.
    pub updates_info: UpdatesInfo,
    /// State for finding/search page.
    pub finding_info: FindingInfo,
    /// Bottom status/progress panel state.
    pub status_panel: StatusPanel,
}

/// Top-level application messages.
#[derive(Debug, Clone)]
pub enum Message {
    SideBar(sidebar::Message),
    Content(content::Message),
    StatusPanel(status_panel::Message),
    ConfigLoaded(Result<updater_core::Config, updater_core::error::CoreError>),
    InitInstalledCounts(Vec<(PackageManagerType, usize)>),
    InitUpdatesCounts(Vec<(PackageManagerType, Vec<PackageUpdate>)>),
}

impl App {
    /// Initializes app state and starts async config loading.
    pub fn new() -> (Self, Task<Message>) {
        let now = Instant::now();

        let app = Self {
            sidebar: SideBar::default(),
            content: Content::default(),
            pm_config: updater_core::Config::default(),
            installed_info: InstalledInfo::default(),
            updates_info: UpdatesInfo::default(),
            finding_info: FindingInfo::default(),
            status_panel: StatusPanel::new(now),
        };

        let task = Task::perform(updater_core::Config::load(), Message::ConfigLoaded);

        (app, task)
    }

    /// Combines application subscriptions.
    pub fn subscription(&self) -> Subscription<Message> {
        self.status_panel
            .subscription(&self.installed_info, &self.updates_info, &self.finding_info)
            .map(Message::StatusPanel)
    }

    /// Handles one application message and returns follow-up tasks.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let at = Instant::now();
        let is_status_panel_message = matches!(&message, Message::StatusPanel(_));
        let mut task = Task::none();

        match message {
            Message::SideBar(sidebar_msg) => task = self.handle_sidebar_message(sidebar_msg),
            Message::Content(content_msg) => task = self.handle_content_message(content_msg),
            Message::StatusPanel(panel_msg) => task = self.handle_status_panel_message(panel_msg),
            Message::ConfigLoaded(result) => task = self.handle_config_loaded(result),
            Message::InitInstalledCounts(counts) => self.apply_installed_counts(counts),
            Message::InitUpdatesCounts(updates) => self.apply_updates_counts(updates),
        }

        if !is_status_panel_message {
            let _ = self.status_panel.update(
                status_panel::Message::Sync(at),
                &self.installed_info,
                &self.updates_info,
                &self.finding_info,
            );
        }
        task
    }

    /// Renders the full application UI.
    pub fn view(&self) -> iced::Element<'_, Message> {
        use iced::{
            Border, Shadow, Vector,
            widget::{column, container, row},
        };

        let sidebar = container(self.sidebar.view().map(Message::SideBar))
            .padding(16)
            .width(Length::Fixed(220.0))
            .height(Length::Fill)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(colors::SURFACE_ALT.into()),
                border: Border {
                    color: colors::DIVIDER_LIGHT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                text_color: None,
                shadow: Shadow {
                    color: colors::SHADOW_LIGHT,
                    offset: Vector::new(2.0, 0.0),
                    blur_radius: 8.0,
                },
                snap: false,
            });

        let content_area = container(
            self.content
                .view(
                    &self.pm_config,
                    &self.installed_info,
                    &self.updates_info,
                    &self.finding_info,
                )
                .map(Message::Content),
        )
        .padding(24)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(iced::Color::WHITE.into()),
            border: Border::default(),
            text_color: None,
            shadow: Shadow::default(),
            snap: false,
        });

        let top_layout = container(
            row![sidebar, content_area]
                .spacing(0)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(colors::SURFACE_MUTED.into()),
            border: Border::default(),
            text_color: None,
            shadow: Shadow::default(),
            snap: false,
        });

        column![
            top_layout,
            self.status_panel.view().map(Message::StatusPanel)
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn handle_sidebar_message(&mut self, sidebar_msg: sidebar::Message) -> Task<Message> {
        match self.sidebar.update(sidebar_msg) {
            sidebar::Action::ChangeContent(content) => {
                self.content.active_content = content;
                Task::none()
            }
            sidebar::Action::Run(sidebar_task) => sidebar_task.map(Message::SideBar),
            sidebar::Action::None => Task::none(),
        }
    }

    fn handle_content_message(&mut self, content_msg: content::Message) -> Task<Message> {
        let action = self.content.update(
            content_msg,
            &mut self.pm_config,
            &mut self.installed_info,
            &mut self.updates_info,
            &mut self.finding_info,
        );

        match action {
            content::Action::Run(content_task) => content_task.map(Message::Content),
            content::Action::ReloadInstalledData => {
                self.installed_info.is_loading_count = true;
                Task::future(Self::init_installed_counts(self.pm_config.clone())).then(
                    |installed_counts| Task::done(Message::InitInstalledCounts(installed_counts)),
                )
            }
            content::Action::None => Task::none(),
        }
    }

    fn handle_status_panel_message(&mut self, panel_msg: status_panel::Message) -> Task<Message> {
        match self.status_panel.update(
            panel_msg,
            &self.installed_info,
            &self.updates_info,
            &self.finding_info,
        ) {
            status_panel::Action::Run(panel_task) => panel_task.map(Message::StatusPanel),
            status_panel::Action::None => Task::none(),
        }
    }

    fn handle_config_loaded(
        &mut self,
        result: Result<updater_core::Config, updater_core::error::CoreError>,
    ) -> Task<Message> {
        match result {
            Ok(config) => {
                self.pm_config = config;
                self.installed_info.is_loading_count = true;
                self.updates_info.is_loading_count = true;

                let installed_task = Task::future(Self::init_installed_counts(
                    self.pm_config.clone(),
                ))
                .then(|installed_counts| {
                    Task::done(Message::InitInstalledCounts(installed_counts))
                });

                let updates_task = Task::future(Self::init_updates_counts(self.pm_config.clone()))
                    .then(|update_counts| Task::done(Message::InitUpdatesCounts(update_counts)));

                Task::batch(vec![installed_task, updates_task])
            }
            Err(e) => {
                log::error!("Failed to load config: {}", e);
                Task::none()
            }
        }
    }

    fn apply_installed_counts(&mut self, counts: Vec<(PackageManagerType, usize)>) {
        self.installed_info.is_loading_count = false;
        self.installed_info.has_loading_count = true;

        self.installed_info.installed_packages = counts
            .into_iter()
            .map(|(pm_type, count)| (pm_type, (count, Vec::new())))
            .collect();
    }

    fn apply_updates_counts(&mut self, updates: Vec<(PackageManagerType, Vec<PackageUpdate>)>) {
        self.updates_info.is_loading_count = false;
        self.updates_info.has_loading_count = true;

        self.updates_info.updates_by_manager = updates
            .into_iter()
            .map(|(pm_type, packages)| {
                let count = packages.len();
                (pm_type, (count, packages))
            })
            .collect();
    }

    async fn init_installed_counts(
        config: updater_core::Config,
    ) -> Vec<(PackageManagerType, usize)> {
        let config = Arc::new(config);

        let pms: Vec<PackageManagerType> = config
            .system_manager
            .iter()
            .map(|pm| pm.manager_type)
            .chain(config.app_managers.iter().map(|pm| pm.manager_type))
            .collect();

        join_all(pms.into_iter().map(|pm| {
            let config = config.clone();
            async move {
                let count = pm.count_installed(&config).await.unwrap_or(0);
                (pm, count)
            }
        }))
        .await
    }

    async fn init_updates_counts(
        config: updater_core::Config,
    ) -> Vec<(PackageManagerType, Vec<PackageUpdate>)> {
        let config = Arc::new(config);

        let pms: Vec<PackageManagerType> = config
            .system_manager
            .iter()
            .map(|pm| pm.manager_type)
            .chain(config.app_managers.iter().map(|pm| pm.manager_type))
            .collect();

        join_all(pms.into_iter().map(|pm| {
            let config = config.clone();
            async move {
                let updates = pm.list_updates(&config).await.unwrap_or_default();
                (pm, updates)
            }
        }))
        .await
    }
}
