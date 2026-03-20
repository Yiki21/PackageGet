//! Top-level application composition and message routing.

use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

use futures::channel::mpsc;
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
    /// Sidebar state.
    pub sidebar: SideBar,
    /// Content state.
    pub content: Content,

    /// Package manager configuration.
    pub pm_config: updater_core::Config,
    /// Installed page data.
    pub installed_info: InstalledInfo,
    /// Updates page data.
    pub updates_info: UpdatesInfo,
    /// Finding page data.
    pub finding_info: FindingInfo,
    /// Status panel state.
    pub status_panel: StatusPanel,
}

/// Top-level application messages.
#[derive(Debug, Clone)]
pub enum Message {
    /// Sidebar message.
    SideBar(sidebar::Message),
    /// Content message.
    Content(content::Message),
    /// Status panel message.
    StatusPanel(status_panel::Message),
    /// Configuration load result.
    ConfigLoaded(Result<updater_core::Config, updater_core::error::CoreError>),
    /// Installed initialization progress message.
    InitInstalledProgress {
        /// Completed manager count.
        completed: usize,
        /// Total manager count.
        total: usize,
        /// Reporting manager.
        manager: PackageManagerType,
        /// Progress detail message.
        command_message: String,
    },
    /// Installed count payload for one manager.
    InitInstalledCount {
        /// Source manager.
        manager: PackageManagerType,
        /// Installed package count value.
        count: usize,
    },
    /// Installed initialization completion message.
    InitInstalledFinished,
    /// Updates initialization progress message.
    InitUpdatesProgress {
        /// Completed manager count.
        completed: usize,
        /// Total manager count.
        total: usize,
        /// Reporting manager.
        manager: PackageManagerType,
        /// Progress detail message.
        command_message: String,
    },
    /// Updates payload for one manager.
    InitUpdatesCount {
        /// Source manager.
        manager: PackageManagerType,
        /// Update entries.
        updates: Vec<PackageUpdate>,
    },
    /// Updates initialization completion message.
    InitUpdatesFinished,
}

#[derive(Debug, Clone)]
enum InitInstalledEvent {
    /// Installed worker start event.
    Started {
        /// Total manager count.
        total: usize,
        /// Reporting manager.
        manager: PackageManagerType,
        /// Progress detail message.
        command_message: String,
    },
    /// Installed worker completion event.
    Completed {
        /// Total manager count.
        total: usize,
        /// Reporting manager.
        manager: PackageManagerType,
        /// Progress detail message.
        command_message: String,
    },
    /// Installed count payload event.
    Item {
        /// Source manager.
        manager: PackageManagerType,
        /// Installed package count value.
        count: usize,
    },
    /// Installed workers completion event.
    Finished,
}

#[derive(Debug, Clone)]
enum InitUpdatesEvent {
    /// Updates worker start event.
    Started {
        /// Total manager count.
        total: usize,
        /// Reporting manager.
        manager: PackageManagerType,
        /// Progress detail message.
        command_message: String,
    },
    /// Updates worker completion event.
    Completed {
        /// Total manager count.
        total: usize,
        /// Reporting manager.
        manager: PackageManagerType,
        /// Progress detail message.
        command_message: String,
    },
    /// Updates payload event.
    Item {
        /// Source manager.
        manager: PackageManagerType,
        /// Update entries.
        updates: Vec<PackageUpdate>,
    },
    /// Updates workers completion event.
    Finished,
}

impl App {
    /// Creates app state and starts config loading.
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

    /// Builds app subscriptions.
    pub fn subscription(&self) -> Subscription<Message> {
        self.status_panel
            .subscription(&self.installed_info, &self.updates_info, &self.finding_info)
            .map(Message::StatusPanel)
    }

    /// Handles one app message and returns follow-up tasks.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let at = Instant::now();
        let is_status_panel_message = matches!(&message, Message::StatusPanel(_));
        let mut task = Task::none();

        match message {
            Message::SideBar(sidebar_msg) => task = self.handle_sidebar_message(sidebar_msg),
            Message::Content(content_msg) => task = self.handle_content_message(content_msg),
            Message::StatusPanel(panel_msg) => task = self.handle_status_panel_message(panel_msg),
            Message::ConfigLoaded(result) => task = self.handle_config_loaded(result),
            Message::InitInstalledProgress {
                completed,
                total,
                manager,
                command_message,
            } => self.apply_init_installed_progress(completed, total, manager, command_message),
            Message::InitInstalledCount { manager, count } => {
                self.apply_init_installed_count(manager, count)
            }
            Message::InitInstalledFinished => self.finish_init_installed_counts(),
            Message::InitUpdatesProgress {
                completed,
                total,
                manager,
                command_message,
            } => self.apply_init_updates_progress(completed, total, manager, command_message),
            Message::InitUpdatesCount { manager, updates } => {
                self.apply_init_updates_count(manager, updates)
            }
            Message::InitUpdatesFinished => self.finish_init_updates_counts(),
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

    /// Renders the app UI.
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
                self.installed_info.init_logs.clear();
                self.start_init_installed_counts_task(self.pm_config.clone())
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
                self.installed_info.init_logs.clear();
                self.updates_info.init_logs.clear();

                let installed_task = self.start_init_installed_counts_task(self.pm_config.clone());
                let updates_task = self.start_init_updates_counts_task(self.pm_config.clone());

                Task::batch(vec![installed_task, updates_task])
            }
            Err(e) => {
                log::error!("Failed to load config: {}", e);
                Task::none()
            }
        }
    }

    fn apply_init_installed_count(&mut self, manager: PackageManagerType, count: usize) {
        self.installed_info.has_loading_count = true;
        self.installed_info
            .installed_packages
            .insert(manager, (count, Vec::new()));
    }

    fn finish_init_installed_counts(&mut self) {
        self.installed_info.is_loading_count = false;
        self.installed_info.has_loading_count = true;
        self.installed_info.init_progress = None;
    }

    fn apply_init_updates_count(
        &mut self,
        manager: PackageManagerType,
        updates: Vec<PackageUpdate>,
    ) {
        self.updates_info.has_loading_count = true;
        let count = updates.len();
        self.updates_info
            .updates_by_manager
            .insert(manager, (count, updates));
    }

    fn finish_init_updates_counts(&mut self) {
        self.updates_info.is_loading_count = false;
        self.updates_info.has_loading_count = true;
        self.updates_info.init_progress = None;
    }

    fn apply_init_installed_progress(
        &mut self,
        completed: usize,
        total: usize,
        manager: PackageManagerType,
        command_message: String,
    ) {
        self.installed_info.init_progress = Some((completed.min(total), total));
        Self::push_init_log(
            &mut self.installed_info.init_logs,
            "InitInstalled",
            manager,
            command_message,
        );
    }

    fn apply_init_updates_progress(
        &mut self,
        completed: usize,
        total: usize,
        manager: PackageManagerType,
        command_message: String,
    ) {
        self.updates_info.init_progress = Some((completed.min(total), total));
        Self::push_init_log(
            &mut self.updates_info.init_logs,
            "InitUpdates",
            manager,
            command_message,
        );
    }

    fn push_init_log(
        logs: &mut Vec<String>,
        phase: &str,
        manager: PackageManagerType,
        command_message: String,
    ) {
        let command_message = command_message.trim();
        if command_message.is_empty() {
            return;
        }

        logs.push(format!(
            "[{}][{}] {}",
            phase,
            manager.name(),
            command_message
        ));

        const MAX_INIT_LOGS: usize = 120;
        if logs.len() > MAX_INIT_LOGS {
            let overflow = logs.len() - MAX_INIT_LOGS;
            logs.drain(0..overflow);
        }
    }

    fn configured_managers(config: &updater_core::Config) -> Vec<PackageManagerType> {
        config
            .system_manager
            .iter()
            .map(|pm| pm.manager_type)
            .chain(config.app_managers.iter().map(|pm| pm.manager_type))
            .collect()
    }

    fn start_init_installed_counts_task(&mut self, config: updater_core::Config) -> Task<Message> {
        let managers = Self::configured_managers(&config);
        let manager_set: HashSet<_> = managers.iter().copied().collect();
        self.installed_info
            .installed_packages
            .retain(|pm_type, _| manager_set.contains(pm_type));
        self.installed_info
            .selected_managers
            .retain(|pm_type| manager_set.contains(pm_type));
        let total = managers.len();
        self.installed_info.init_progress = Some((0, total));
        if total == 0 {
            self.finish_init_installed_counts();
            return Task::none();
        }

        let (sender, receiver) = mpsc::unbounded::<InitInstalledEvent>();
        let finished_count = Arc::new(AtomicUsize::new(0));

        // The receiver tracks progress so worker tasks stay stateless.
        let completed_count = Arc::new(AtomicUsize::new(0));
        let completed_count_for_progress = Arc::clone(&completed_count);
        let progress_task = Task::run(receiver, move |event| match event {
            InitInstalledEvent::Started {
                total,
                manager,
                command_message,
            } => Message::InitInstalledProgress {
                completed: completed_count_for_progress.load(Ordering::Relaxed),
                total,
                manager,
                command_message,
            },
            InitInstalledEvent::Completed {
                total,
                manager,
                command_message,
            } => {
                let completed = completed_count_for_progress.fetch_add(1, Ordering::Relaxed) + 1;
                Message::InitInstalledProgress {
                    completed,
                    total,
                    manager,
                    command_message,
                }
            }
            InitInstalledEvent::Item { manager, count } => {
                Message::InitInstalledCount { manager, count }
            }
            InitInstalledEvent::Finished => Message::InitInstalledFinished,
        });

        // One task is spawned per package manager, and all run in parallel.
        let mut tasks = Vec::with_capacity(total + 1);
        for pm in managers {
            let sender_for_task = sender.clone();
            let config = config.clone();
            let finished_count_for_task = Arc::clone(&finished_count);

            let task = Task::future(async move {
                let _ = sender_for_task.unbounded_send(InitInstalledEvent::Started {
                    total,
                    manager: pm,
                    command_message: "Running count_installed".to_string(),
                });

                let count = match pm.count_installed(&config).await {
                    Ok(value) => value,
                    Err(e) => {
                        log::warn!("count_installed failed for {}: {}", pm.name(), e);
                        0
                    }
                };

                let _ =
                    sender_for_task.unbounded_send(InitInstalledEvent::Item { manager: pm, count });

                let _ = sender_for_task.unbounded_send(InitInstalledEvent::Completed {
                    total,
                    manager: pm,
                    command_message: format!("Done count_installed -> {}", count),
                });

                // Emit a terminal event after the last worker reports completion.
                let finished = finished_count_for_task.fetch_add(1, Ordering::AcqRel) + 1;
                if finished == total {
                    let _ = sender_for_task.unbounded_send(InitInstalledEvent::Finished);
                }
            })
            .discard();

            tasks.push(task);
        }

        tasks.push(progress_task);
        Task::batch(tasks)
    }

    fn start_init_updates_counts_task(&mut self, config: updater_core::Config) -> Task<Message> {
        let managers = Self::configured_managers(&config);
        let manager_set: HashSet<_> = managers.iter().copied().collect();
        self.updates_info
            .updates_by_manager
            .retain(|pm_type, _| manager_set.contains(pm_type));
        self.updates_info
            .selected_managers
            .retain(|pm_type| manager_set.contains(pm_type));
        let total = managers.len();
        self.updates_info.init_progress = Some((0, total));
        if total == 0 {
            self.finish_init_updates_counts();
            return Task::none();
        }

        let (sender, receiver) = mpsc::unbounded::<InitUpdatesEvent>();
        let finished_count = Arc::new(AtomicUsize::new(0));

        // The receiver tracks progress so worker tasks stay stateless.
        let completed_count = Arc::new(AtomicUsize::new(0));
        let completed_count_for_progress = Arc::clone(&completed_count);
        let progress_task = Task::run(receiver, move |event| match event {
            InitUpdatesEvent::Started {
                total,
                manager,
                command_message,
            } => Message::InitUpdatesProgress {
                completed: completed_count_for_progress.load(Ordering::Relaxed),
                total,
                manager,
                command_message,
            },
            InitUpdatesEvent::Completed {
                total,
                manager,
                command_message,
            } => {
                let completed = completed_count_for_progress.fetch_add(1, Ordering::Relaxed) + 1;
                Message::InitUpdatesProgress {
                    completed,
                    total,
                    manager,
                    command_message,
                }
            }
            InitUpdatesEvent::Item { manager, updates } => {
                Message::InitUpdatesCount { manager, updates }
            }
            InitUpdatesEvent::Finished => Message::InitUpdatesFinished,
        });

        // One task is spawned per package manager, and all run in parallel.
        let mut tasks = Vec::with_capacity(total + 1);
        for pm in managers {
            let sender_for_task = sender.clone();
            let config = config.clone();
            let finished_count_for_task = Arc::clone(&finished_count);

            let task = Task::future(async move {
                let _ = sender_for_task.unbounded_send(InitUpdatesEvent::Started {
                    total,
                    manager: pm,
                    command_message: "Running list_updates".to_string(),
                });

                let updates = match pm.list_updates(&config).await {
                    Ok(value) => value,
                    Err(e) => {
                        log::warn!("list_updates failed for {}: {}", pm.name(), e);
                        Vec::new()
                    }
                };
                let update_count = updates.len();

                let _ = sender_for_task.unbounded_send(InitUpdatesEvent::Item {
                    manager: pm,
                    updates,
                });

                let _ = sender_for_task.unbounded_send(InitUpdatesEvent::Completed {
                    total,
                    manager: pm,
                    command_message: format!("Done list_updates -> {} updates", update_count),
                });

                // Emit a terminal event after the last worker reports completion.
                let finished = finished_count_for_task.fetch_add(1, Ordering::AcqRel) + 1;
                if finished == total {
                    let _ = sender_for_task.unbounded_send(InitUpdatesEvent::Finished);
                }
            })
            .discard();

            tasks.push(task);
        }

        tasks.push(progress_task);
        Task::batch(tasks)
    }
}
