//! Top-level application composition and message routing.

use std::collections::HashSet;
use std::time::Instant;

use iced::{Length, Subscription, Task};
use updater_core::{PackageManagerType, PackageUpdate};
use updater_manager_api::ManagerId;

use crate::{
    content::{self, Content, FindingInfo, InstalledInfo, UpdatesInfo},
    init_workflows::{InitProgress, ManagerInitTask, run_manager_init_task},
    manager_catalog::ManagerCatalog,
    shortcut::Shortcut,
    sidebar::{self, SideBar},
    status_panel::{self, StatusPanel},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutMode {
    Wide,
    Medium,
    Narrow,
}

impl LayoutMode {
    fn from_width(width: f32) -> Self {
        if width >= 1100.0 {
            Self::Wide
        } else if width >= 760.0 {
            Self::Medium
        } else {
            Self::Narrow
        }
    }
}

/// Root GUI state for the updater application.
#[derive(Debug, Clone)]
pub struct App {
    /// Sidebar state.
    pub sidebar: SideBar,
    /// Content state.
    pub content: Content,
    /// Registered manager metadata used for UI identity and display.
    manager_catalog: ManagerCatalog,

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
    /// Bounded structured operation history.
    activity_history: crate::activity::ActivityHistory,
    /// Whether the Activity Center is open.
    activity_center_open: bool,
    /// Monotonic operation record identifier.
    next_operation_id: u64,
    /// Cooperative task-abort handle for the active operation.
    active_operation_handle: Option<iced::task::Handle>,
    /// Cooperative cancellation token for the active package operation.
    active_operation_cancellation: Option<content::CancellationToken>,
    /// Current application window size.
    window_size: iced::Size,
    /// Whether the medium-width sidebar is explicitly expanded.
    sidebar_expanded: bool,
    /// Whether the selected package inspector is open at constrained widths.
    inspector_drawer_open: bool,
    /// Current desktop system appearance.
    system_theme: iced::theme::Mode,
    /// Whether navigation/closing is waiting for an unsaved-settings decision.
    pending_settings_exit: Option<PendingSettingsExit>,
}

#[derive(Debug, Clone, Copy)]
enum PendingSettingsExit {
    Navigate(content::ActiveContentPage),
    Close(iced::window::Id),
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
    /// Request cooperative cancellation for the active operation.
    CancelActiveOperation,
    /// Show or hide the Activity Center.
    ToggleActivityCenter,
    /// Clear the bounded operation history.
    ClearActivityHistory,
    /// Activity-history load result.
    ActivityHistoryLoaded(crate::activity::ActivityHistory),
    /// Activity-history save result.
    ActivityHistorySaved(Result<(), String>),
    /// Native notification task finished.
    NotificationFinished(Result<(), String>),
    /// Toggle the constrained-width package inspector drawer.
    ToggleInspectorDrawer,
    /// Toggle the responsive sidebar.
    ToggleSidebar,
    /// Window size changed.
    WindowResized(iced::Size),
    /// Desktop system appearance changed.
    SystemThemeChanged(iced::theme::Mode),
    /// Window close request.
    CloseRequested(iced::window::Id),
    /// Save pending Settings changes before leaving.
    SavePendingSettings,
    /// Discard pending Settings changes before leaving.
    DiscardPendingSettings,
    /// Keep editing Settings.
    CancelPendingSettingsExit,
    /// Application-level keyboard shortcut.
    Shortcut(Shortcut),
    /// Configuration load result.
    ConfigLoaded(Result<updater_core::Config, updater_core::error::CoreError>),
    /// Installed initialization progress message.
    InitInstalledProgress {
        /// Completed manager count.
        completed: usize,
        /// Total manager count.
        total: usize,
        /// Reporting manager.
        manager: ManagerId,
        /// Progress detail message.
        command_message: String,
    },
    /// Installed count payload for one manager.
    InitInstalledCount {
        /// Source manager.
        manager: ManagerId,
        /// Installed package count value or failure detail.
        result: Result<usize, String>,
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
        manager: ManagerId,
        /// Progress detail message.
        command_message: String,
    },
    /// Updates payload for one manager.
    InitUpdatesCount {
        /// Source manager.
        manager: ManagerId,
        /// Update entries or failure detail.
        result: Result<Vec<PackageUpdate>, String>,
    },
    /// Updates initialization completion message.
    InitUpdatesFinished,
}

impl App {
    /// Creates app state and starts config loading.
    pub fn new() -> (Self, Task<Message>) {
        let now = Instant::now();

        let app = Self {
            sidebar: SideBar::default(),
            content: Content::default(),
            manager_catalog: ManagerCatalog::builtin(),
            pm_config: updater_core::Config::default(),
            installed_info: InstalledInfo::default(),
            updates_info: UpdatesInfo::default(),
            finding_info: FindingInfo::default(),
            status_panel: StatusPanel::new(now),
            activity_history: crate::activity::ActivityHistory::default(),
            activity_center_open: false,
            next_operation_id: 1,
            active_operation_handle: None,
            active_operation_cancellation: None,
            window_size: iced::Size::new(1200.0, 800.0),
            sidebar_expanded: false,
            inspector_drawer_open: false,
            system_theme: iced::theme::Mode::Light,
            pending_settings_exit: None,
        };

        let task = Task::batch(vec![
            Task::perform(updater_core::Config::load(), Message::ConfigLoaded),
            Task::perform(
                crate::activity::ActivityHistory::load(),
                Message::ActivityHistoryLoaded,
            ),
            iced::system::theme().map(Message::SystemThemeChanged),
        ]);

        (app, task)
    }

    /// Builds app subscriptions.
    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            self.status_panel
                .subscription(&self.installed_info, &self.updates_info, &self.finding_info)
                .map(Message::StatusPanel),
            iced::window::close_requests().map(Message::CloseRequested),
            iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            iced::system::theme_changes().map(Message::SystemThemeChanged),
        ])
    }

    /// Handles one app message and returns follow-up tasks.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let at = Instant::now();
        let is_animation_message = matches!(&message, Message::StatusPanel(_));
        let task = if let Message::Shortcut(shortcut) = message {
            self.handle_shortcut(shortcut)
        } else {
            self.update_message(message)
        };

        if !is_animation_message {
            self.status_panel.update(
                status_panel::Message::Sync(at),
                &self.installed_info,
                &self.updates_info,
                &self.finding_info,
                &self.manager_catalog,
            );
        }
        task
    }

    fn update_message(&mut self, message: Message) -> Task<Message> {
        let mut task = Task::none();

        match message {
            Message::SideBar(sidebar_msg) => match self.sidebar.update(sidebar_msg) {
                sidebar::Action::ChangeContent(target) => {
                    if self.content.active_content == content::ActiveContentPage::Settings
                        && target != content::ActiveContentPage::Settings
                        && self.content.settings.is_dirty()
                    {
                        self.sidebar.active_tab = self.content.active_content.into();
                        self.pending_settings_exit = Some(PendingSettingsExit::Navigate(target));
                    } else {
                        self.content.active_content = target;
                        self.pending_settings_exit = None;
                        if self.layout_mode() == LayoutMode::Narrow {
                            self.sidebar_expanded = false;
                        }
                    }
                }
                sidebar::Action::CloseRequested => self.sidebar_expanded = false,
                sidebar::Action::None => {}
            },
            Message::Content(content_msg) => {
                let action = self.content.update(
                    content_msg,
                    &mut self.pm_config,
                    &mut self.installed_info,
                    &mut self.updates_info,
                    &mut self.finding_info,
                    &self.manager_catalog,
                );

                task = match action {
                    content::Action::Run(content_task) => content_task.map(Message::Content),
                    content::Action::CancellableRun(content_task, cancellation) => {
                        let (task, handle) = content_task.map(Message::Content).abortable();
                        self.active_operation_handle = Some(handle);
                        self.active_operation_cancellation = Some(cancellation);
                        task
                    }
                    content::Action::ReloadPackageData { reason, follow_up } => {
                        let reload = self.reload_package_data(reason);
                        let pending_exit = if reason == content::ReloadReason::ConfigurationChanged
                            && !self.content.settings.is_dirty()
                        {
                            self.complete_pending_settings_exit()
                        } else {
                            Task::none()
                        };
                        Task::batch(vec![reload, follow_up.map(Message::Content), pending_exit])
                    }
                    content::Action::PackageOperationFinished {
                        outcome,
                        reload,
                        follow_up,
                    } => {
                        self.active_operation_handle = None;
                        self.active_operation_cancellation = None;
                        let notification = self.record_operation(&outcome);
                        self.status_panel.record_outcome(outcome);
                        let completion = if reload {
                            Task::batch(vec![
                                self.reload_package_data(content::ReloadReason::PackageOperation),
                                follow_up.map(Message::Content),
                            ])
                        } else {
                            follow_up.map(Message::Content)
                        };
                        Task::batch(vec![completion, notification])
                    }
                    content::Action::None => Task::none(),
                };
            }
            Message::StatusPanel(panel_msg) => {
                self.status_panel.update(
                    panel_msg,
                    &self.installed_info,
                    &self.updates_info,
                    &self.finding_info,
                    &self.manager_catalog,
                );
            }
            Message::CancelActiveOperation => {
                if let Some(handle) = self.active_operation_handle.take() {
                    let progress = self.cancelled_operation_progress();
                    if let Some(cancellation) = self.active_operation_cancellation.take() {
                        cancellation.cancel();
                    }
                    handle.abort();
                    self.cancel_active_operation_state();
                    if let Some(progress) = progress {
                        let id = self.next_operation_id;
                        self.next_operation_id = self.next_operation_id.saturating_add(1);
                        self.activity_history
                            .push(crate::activity::ActivityRecord::cancelled(id, progress));
                        task = self.save_activity_history();
                    }
                }
            }
            Message::ToggleActivityCenter => {
                self.activity_center_open = !self.activity_center_open;
            }
            Message::ClearActivityHistory => {
                self.activity_history.clear();
                task = self.save_activity_history();
            }
            Message::ActivityHistoryLoaded(history) => {
                self.next_operation_id = history
                    .iter()
                    .map(|record| record.id)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                self.activity_history = history;
            }
            Message::ActivityHistorySaved(result) => {
                if let Err(error) = result {
                    log::warn!("Failed to persist Activity Center history: {error}");
                }
            }
            Message::NotificationFinished(result) => {
                if let Err(error) = result {
                    log::warn!("Failed to show completion notification: {error}");
                }
            }
            Message::ToggleSidebar => self.sidebar_expanded = !self.sidebar_expanded,
            Message::ToggleInspectorDrawer => {
                self.inspector_drawer_open = !self.inspector_drawer_open;
            }
            Message::WindowResized(size) => {
                self.window_size = size;
                if size.width >= 1100.0 {
                    self.sidebar_expanded = false;
                    self.inspector_drawer_open = false;
                }
            }
            Message::SystemThemeChanged(mode) => self.system_theme = mode,
            Message::CloseRequested(window_id) => {
                if self.content.settings.is_dirty() {
                    self.pending_settings_exit = Some(PendingSettingsExit::Close(window_id));
                    if self.content.active_content != content::ActiveContentPage::Settings {
                        self.content.active_content = content::ActiveContentPage::Settings;
                        self.sidebar.active_tab = sidebar::Tab::Settings;
                    }
                } else {
                    task = iced::window::close(window_id);
                }
            }
            Message::SavePendingSettings => {
                task = Task::done(Message::Content(content::Message::Settings(
                    content::SettingsMessage::SaveConfig,
                )));
            }
            Message::DiscardPendingSettings => {
                self.content.settings.discard_changes();
                task = self.complete_pending_settings_exit();
            }
            Message::CancelPendingSettingsExit => self.pending_settings_exit = None,
            Message::ConfigLoaded(result) => {
                task = match result {
                    Ok(config) => {
                        self.content.settings.sync_from_config(&config);
                        self.pm_config = config;
                        self.reload_package_data(content::ReloadReason::Startup)
                    }
                    Err(e) => {
                        log::error!("Failed to load config: {}", e);
                        Task::none()
                    }
                };
            }
            Message::InitInstalledProgress {
                completed,
                total,
                manager,
                command_message,
            } => self.apply_init_installed_progress(completed, total, manager, command_message),
            Message::InitInstalledCount { manager, result } => {
                self.apply_init_installed_count(manager, result)
            }
            Message::InitInstalledFinished => task = self.finish_init_installed_counts(),
            Message::InitUpdatesProgress {
                completed,
                total,
                manager,
                command_message,
            } => self.apply_init_updates_progress(completed, total, manager, command_message),
            Message::InitUpdatesCount { manager, result } => {
                self.apply_init_updates_count(manager, result)
            }
            Message::InitUpdatesFinished => self.finish_init_updates_counts(),
            Message::Shortcut(_) => unreachable!("shortcuts are handled before routed messages"),
        }

        task
    }

    fn handle_shortcut(&mut self, shortcut: Shortcut) -> Task<Message> {
        use content::ActiveContentPage;

        if matches!(shortcut, Shortcut::Dismiss) {
            let dismissed = self.pending_settings_exit.take().is_some()
                || self
                    .content
                    .dismiss_active_transient(&mut self.installed_info)
                || self.status_panel.dismiss_top_surface()
                || std::mem::replace(&mut self.activity_center_open, false);

            return if dismissed {
                self.restore_page_focus()
            } else {
                Task::none()
            };
        }

        match shortcut {
            Shortcut::GlobalSearch => {
                let navigation = self.navigate_to(ActiveContentPage::Finding);
                if self.content.active_content == ActiveContentPage::Finding {
                    Task::batch(vec![
                        navigation,
                        self.focus_search(ActiveContentPage::Finding),
                    ])
                } else {
                    navigation
                }
            }
            Shortcut::Refresh => self.route_content_shortcut(self.content.refresh_current_page(
                &self.installed_info,
                &self.updates_info,
                &self.finding_info,
            )),
            Shortcut::NavigateFinding => self.navigate_to(ActiveContentPage::Finding),
            Shortcut::NavigateUpdates => self.navigate_to(ActiveContentPage::Updates),
            Shortcut::NavigateInstalled => self.navigate_to(ActiveContentPage::Installed),
            Shortcut::NavigateSettings => self.navigate_to(ActiveContentPage::Settings),
            Shortcut::FocusPageSearch => self.focus_search(self.content.active_content),
            Shortcut::PrimaryAction => {
                self.route_content_shortcut(self.content.prepare_primary_action(
                    &self.installed_info,
                    &self.updates_info,
                    &self.finding_info,
                ))
            }
            Shortcut::SelectAll => self.route_content_shortcut(self.content.select_all_visible(
                &self.installed_info,
                &self.updates_info,
                &self.finding_info,
            )),
            Shortcut::MoveSelection(direction) => {
                self.route_content_shortcut(self.content.move_keyboard_selection(
                    direction,
                    &self.installed_info,
                    &self.updates_info,
                    &self.finding_info,
                    &self.manager_catalog,
                ))
            }
            Shortcut::ToggleSelection => {
                self.route_content_shortcut(self.content.toggle_keyboard_selection(
                    &self.installed_info,
                    &self.updates_info,
                    &self.finding_info,
                ))
            }
            Shortcut::FocusNext => iced::widget::operation::focus_next(),
            Shortcut::FocusPrevious => iced::widget::operation::focus_previous(),
            Shortcut::Dismiss => Task::none(),
        }
    }

    fn navigate_to(&mut self, target: content::ActiveContentPage) -> Task<Message> {
        if self.content.active_content == target {
            return Task::none();
        }

        if self.content.active_content == content::ActiveContentPage::Settings
            && target != content::ActiveContentPage::Settings
            && self.content.settings.is_dirty()
        {
            self.pending_settings_exit = Some(PendingSettingsExit::Navigate(target));
            return Task::none();
        }

        self.content.active_content = target;
        self.sidebar.active_tab = target.into();
        self.pending_settings_exit = None;
        Task::none()
    }

    fn focus_search(&self, page: content::ActiveContentPage) -> Task<Message> {
        if page == content::ActiveContentPage::Settings {
            return Task::none();
        }

        let id = crate::content::search_input_id(page);
        Task::batch(vec![
            iced::widget::operation::focus(id.clone()),
            iced::widget::operation::select_all(id),
        ])
    }

    fn restore_page_focus(&self) -> Task<Message> {
        match self.content.active_content {
            content::ActiveContentPage::Finding
            | content::ActiveContentPage::Updates
            | content::ActiveContentPage::Installed => {
                self.focus_search(self.content.active_content)
            }
            content::ActiveContentPage::Settings => Task::none(),
        }
    }

    fn cancelled_operation_progress(&self) -> Option<crate::activity::CancelledProgress> {
        if self.finding_info.is_installing {
            let (completed, total) = self
                .finding_info
                .install_progress
                .as_ref()
                .map(|(completed, total, _, _)| (*completed, *total))
                .unwrap_or((0, self.finding_info.selected_packages.len()));
            return Some(crate::activity::CancelledProgress {
                action: "Install",
                completed_packages: completed,
                total_packages: total,
                completed_managers: 0,
                total_managers: self.finding_info.selected_managers.len(),
            });
        }
        if self.updates_info.is_updating {
            let (completed, total) = self
                .updates_info
                .update_progress
                .as_ref()
                .map(|(completed, total, _, _)| (*completed, *total))
                .unwrap_or((0, self.updates_info.selected_packages.len()));
            return Some(crate::activity::CancelledProgress {
                action: "Update",
                completed_packages: completed,
                total_packages: total,
                completed_managers: 0,
                total_managers: self.updates_info.selected_managers.len(),
            });
        }
        if self.installed_info.is_removing {
            let (completed, total) = self
                .installed_info
                .remove_progress
                .as_ref()
                .map(|(completed, total, _, _)| (*completed, *total))
                .unwrap_or((0, self.installed_info.selected_packages.len()));
            return Some(crate::activity::CancelledProgress {
                action: "Remove",
                completed_packages: completed,
                total_packages: total,
                completed_managers: 0,
                total_managers: self.installed_info.selected_managers.len(),
            });
        }
        None
    }

    fn save_activity_history(&self) -> Task<Message> {
        let history = self.activity_history.clone();
        Task::perform(
            async move { history.save().await },
            Message::ActivityHistorySaved,
        )
    }

    fn cancel_active_operation_state(&mut self) {
        if self.finding_info.is_installing {
            self.finding_info.is_installing = false;
            self.finding_info.install_progress = None;
            self.finding_info.last_install_error = Some("Operation cancelled by user".to_owned());
        }
        if self.updates_info.is_updating {
            self.updates_info.is_updating = false;
            self.updates_info.update_progress = None;
            self.updates_info.last_update_error = Some("Operation cancelled by user".to_owned());
        }
        if self.installed_info.is_removing {
            self.installed_info.is_removing = false;
            self.installed_info.remove_progress = None;
            self.installed_info.last_remove_error = Some("Operation cancelled by user".to_owned());
        }
        self.installed_info.confirming_remove = false;
    }

    fn record_operation(&mut self, outcome: &content::OperationOutcome) -> Task<Message> {
        let id = self.next_operation_id;
        self.next_operation_id = self.next_operation_id.saturating_add(1);
        self.activity_history
            .push(crate::activity::ActivityRecord::from_outcome(id, outcome));
        let persist = self.save_activity_history();

        if !self.pm_config.notifications_enabled {
            return persist;
        }

        let title = if outcome.is_success() {
            "Updater operation completed"
        } else {
            "Updater operation stopped"
        };
        let body = outcome.summary();
        let notification = Task::future(async move {
            tokio::task::spawn_blocking(move || {
                notify_rust::Notification::new()
                    .summary(title)
                    .body(&body)
                    .appname("Updater")
                    .show()
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())?
        })
        .then(|result| Task::done(Message::NotificationFinished(result)));
        Task::batch(vec![persist, notification])
    }

    fn route_content_shortcut(
        &mut self,
        content_message: Option<content::Message>,
    ) -> Task<Message> {
        content_message.map_or_else(Task::none, |message| {
            self.update_message(Message::Content(message))
        })
    }

    pub fn theme(&self) -> iced::Theme {
        crate::theme::application_theme(
            crate::theme::Appearance::from_config(&self.content.settings.appearance_value())
                .resolve(self.system_theme),
        )
    }

    fn layout_mode(&self) -> LayoutMode {
        LayoutMode::from_width(self.window_size.width)
    }

    /// Renders the app UI.
    pub fn view(&self) -> iced::Element<'_, Message> {
        use iced::widget::{column, container, row};

        let update_count = self
            .updates_info
            .updates_by_manager
            .values()
            .map(|(count, _)| *count)
            .sum();
        let sidebar_summary = sidebar::Summary {
            update_count,
            updates_loading: self.updates_info.is_loading_count
                || !self.updates_info.loading_updates.is_empty(),
            updates_failed: !self.updates_info.init_errors.is_empty()
                || !self.updates_info.load_errors.is_empty(),
            settings_dirty: self.content.settings.is_dirty(),
        };
        let mode = self.layout_mode();
        let compact_sidebar = mode == LayoutMode::Medium;
        let show_sidebar = mode != LayoutMode::Narrow || self.sidebar_expanded;
        let sidebar_width = if compact_sidebar { 72.0 } else { 200.0 };
        let sidebar = container(
            self.sidebar
                .view(sidebar_summary, compact_sidebar, mode == LayoutMode::Narrow)
                .map(Message::SideBar),
        )
        .padding(if compact_sidebar { [18, 8] } else { [18, 10] })
        .width(Length::Fixed(sidebar_width))
        .height(Length::Fill)
        .style(crate::theme::sidebar_container);

        let vertical_separator = container("")
            .width(Length::Fixed(1.0))
            .height(Length::Fill)
            .style(crate::theme::separator);

        let content_padding = if mode == LayoutMode::Narrow { 12 } else { 24 };
        let show_inspector = mode == LayoutMode::Wide || self.inspector_drawer_open;
        let content = self
            .content
            .view(
                &self.pm_config,
                &self.installed_info,
                &self.updates_info,
                &self.finding_info,
                &self.manager_catalog,
                content::ViewOptions {
                    show_inspector,
                    inspector_drawer: mode != LayoutMode::Wide,
                },
            )
            .map(Message::Content);
        let content_area = container(if mode == LayoutMode::Wide {
            content
        } else {
            iced::widget::scrollable(content).into()
        })
        .padding(content_padding)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(crate::theme::content_container);

        let show_menu = mode == LayoutMode::Narrow && !show_sidebar;
        let menu = show_menu.then(|| {
            iced::widget::button(iced::widget::text("Menu"))
                .padding([7, 10])
                .style(crate::theme::secondary_button(true))
                .on_press(Message::ToggleSidebar)
        });
        let has_inspector = self.content.has_active_inspector();
        let show_inspector_control = mode != LayoutMode::Wide && has_inspector;
        let inspector_control = show_inspector_control.then(|| {
            iced::widget::button(iced::widget::text(if self.inspector_drawer_open {
                "Hide Details"
            } else {
                "Show Details"
            }))
            .padding([7, 10])
            .style(crate::theme::secondary_button(true))
            .on_press(Message::ToggleInspectorDrawer)
        });
        let mut compact_controls = row![].spacing(8);
        if let Some(menu) = menu {
            compact_controls = compact_controls.push(menu);
        }
        if let Some(inspector_control) = inspector_control {
            compact_controls = compact_controls.push(inspector_control);
        }
        let content_with_controls: iced::Element<'_, Message> =
            if show_menu || show_inspector_control {
                column![compact_controls, content_area]
                    .spacing(8)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                content_area.into()
            };
        let top_layout: iced::Element<'_, Message> = if show_sidebar {
            row![sidebar, vertical_separator, content_with_controls]
                .spacing(0)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            content_with_controls
        };

        let mut layout = column![top_layout].width(Length::Fill).height(Length::Fill);

        let shortcuts = iced::widget::container(
            row![
                iced::widget::text(match self.content.active_content {
                    content::ActiveContentPage::Finding => {
                        "Ctrl+K Search  ·  Ctrl+R Refresh  ·  / Focus  ·  Ctrl+Enter Install"
                    }
                    content::ActiveContentPage::Updates => {
                        "Ctrl+R Refresh  ·  / Focus  ·  Ctrl+A Select All  ·  Ctrl+Enter Update"
                    }
                    content::ActiveContentPage::Installed => {
                        "Ctrl+R Refresh  ·  / Focus  ·  Ctrl+A Select All  ·  Ctrl+Enter Remove"
                    }
                    content::ActiveContentPage::Settings => {
                        "Alt+1–4 Navigate  ·  Tab Move Focus  ·  Ctrl+Enter Save"
                    }
                })
                .size(11)
                .style(crate::theme::text_on_surface_alt)
                .width(Length::Fill),
                iced::widget::button(iced::widget::text("Activity").size(11))
                    .padding([3, 8])
                    .style(crate::theme::secondary_button(true))
                    .on_press(Message::ToggleActivityCenter),
                iced::widget::button(iced::widget::text("Cancel Task").size(11))
                    .padding([3, 8])
                    .style(crate::theme::secondary_button(
                        self.active_operation_handle.is_some()
                    ))
                    .on_press_maybe(
                        self.active_operation_handle
                            .is_some()
                            .then_some(Message::CancelActiveOperation)
                    ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        )
        .padding([4, 16])
        .width(Length::Fill)
        .style(crate::theme::toolbar_container);
        layout = layout.push(shortcuts);

        if self.activity_center_open {
            let history = if self.activity_history.is_empty() {
                column![
                    iced::widget::text("No completed operations yet")
                        .size(13)
                        .style(crate::theme::text_on_surface_muted)
                ]
            } else {
                column(self.activity_history.iter().map(|record| {
                    column![
                        iced::widget::text(record.title())
                            .size(13)
                            .font(crate::theme::FONT_SEMIBOLD),
                        iced::widget::text(record.summary(&self.manager_catalog))
                            .size(12)
                            .style(crate::theme::text_on_surface_muted),
                    ]
                    .spacing(3)
                    .into()
                }))
                .spacing(10)
            };
            let actions = row![
                iced::widget::text("Activity Center")
                    .size(14)
                    .font(crate::theme::FONT_SEMIBOLD)
                    .width(Length::Fill),
                iced::widget::button(iced::widget::text("Clear").size(12))
                    .padding([5, 9])
                    .style(crate::theme::secondary_button(
                        !self.activity_history.is_empty()
                    ))
                    .on_press_maybe(
                        (!self.activity_history.is_empty())
                            .then_some(Message::ClearActivityHistory)
                    ),
                iced::widget::button(iced::widget::text("Close").size(12))
                    .padding([5, 9])
                    .style(crate::theme::secondary_button(true))
                    .on_press(Message::ToggleActivityCenter),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);
            let center = container(
                column![actions, iced::widget::scrollable(history).height(160)].spacing(10),
            )
            .padding([10, 16])
            .width(Length::Fill)
            .style(crate::theme::surface_container);
            layout = layout.push(center);
        }

        if self.status_panel.is_visible() {
            let horizontal_separator = container("")
                .width(Length::Fill)
                .height(Length::Fixed(1.0))
                .style(crate::theme::separator);
            layout = layout
                .push(horizontal_separator)
                .push(self.status_panel.view().map(Message::StatusPanel));
        }

        if self.pending_settings_exit.is_some() {
            let prompt = container(
                row![
                    iced::widget::text("Save changes before leaving Settings?")
                        .size(13)
                        .font(crate::theme::FONT_SEMIBOLD)
                        .style(crate::theme::text_on_surface)
                        .width(Length::Fill),
                    iced::widget::button(iced::widget::text("Cancel").size(13))
                        .padding([7, 12])
                        .style(crate::theme::secondary_button(true))
                        .on_press(Message::CancelPendingSettingsExit),
                    iced::widget::button(iced::widget::text("Discard").size(13))
                        .padding([7, 12])
                        .style(crate::theme::secondary_button(true))
                        .on_press(Message::DiscardPendingSettings),
                    iced::widget::button(
                        iced::widget::text("Save")
                            .size(13)
                            .font(crate::theme::FONT_SEMIBOLD)
                            .style(crate::theme::text_on_primary)
                    )
                    .padding([7, 14])
                    .style(crate::theme::action_button(
                        true,
                        crate::theme::colors::ACCENT,
                        crate::theme::colors::ACCENT_HOVER,
                        crate::theme::colors::ACCENT_ACTIVE,
                    ))
                    .on_press(Message::SavePendingSettings),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .padding([10, 16])
            .width(Length::Fill)
            .style(crate::theme::toolbar_container);
            let horizontal_separator = container("")
                .width(Length::Fill)
                .height(Length::Fixed(1.0))
                .style(crate::theme::separator);
            layout = layout.push(horizontal_separator).push(prompt);
        }

        crate::shortcut::capture(layout.into())
    }

    fn apply_init_installed_count(&mut self, manager: ManagerId, result: Result<usize, String>) {
        self.installed_info.has_loading_count = true;
        match result {
            Ok(count) => {
                self.installed_info.init_errors.remove(&manager);
                self.installed_info
                    .installed_packages
                    .insert(manager, (count, Vec::new()));
            }
            Err(error) => {
                self.installed_info
                    .installed_packages
                    .entry(manager.clone())
                    .or_insert_with(|| (0, Vec::new()));
                self.installed_info.init_errors.insert(manager, error);
            }
        }
    }

    fn finish_init_installed_counts(&mut self) -> Task<Message> {
        self.installed_info.is_loading_count = false;
        self.installed_info.has_loading_count = true;
        self.installed_info.init_progress = None;

        let managers: Vec<_> = self
            .installed_info
            .selected_managers
            .iter()
            .filter(|manager| !self.installed_info.init_errors.contains_key(manager))
            .cloned()
            .collect();
        if managers.is_empty() {
            return Task::none();
        }

        for manager in &managers {
            self.installed_info
                .loading_installed
                .insert(manager.clone());
        }

        Task::batch(managers.into_iter().map(|manager| {
            let config = self.pm_config.clone();
            let task_manager = manager.clone();
            Task::future(async move {
                let runtime =
                    PackageManagerType::from_manager_id(&task_manager).ok_or_else(|| {
                        format!("Manager is not available in this build: {task_manager}")
                    })?;
                runtime.list_installed(&config).await.map_err(|error| {
                    format!(
                        "Failed to load installed packages for {}: {}",
                        task_manager, error
                    )
                })
            })
            .then(move |result| {
                Task::done(Message::Content(content::Message::Installed(
                    content::InstalledMessage::LoadInstalledResult(manager.clone(), result),
                )))
            })
        }))
    }

    fn apply_init_updates_count(
        &mut self,
        manager: ManagerId,
        result: Result<Vec<PackageUpdate>, String>,
    ) {
        self.updates_info.has_loading_count = true;
        match result {
            Ok(updates) => {
                self.updates_info.init_errors.remove(&manager);
                let count = updates.len();
                self.updates_info
                    .updates_by_manager
                    .insert(manager, (count, updates));
            }
            Err(error) => {
                self.updates_info
                    .updates_by_manager
                    .entry(manager.clone())
                    .or_insert_with(|| (0, Vec::new()));
                self.updates_info.init_errors.insert(manager, error);
            }
        }
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
        manager: ManagerId,
        command_message: String,
    ) {
        self.installed_info.init_progress = Some((completed.min(total), total));
        let manager_name = self.manager_catalog.display_name(&manager).to_owned();
        Self::push_init_log(
            &mut self.installed_info.init_logs,
            "InitInstalled",
            &manager_name,
            command_message,
        );
    }

    fn apply_init_updates_progress(
        &mut self,
        completed: usize,
        total: usize,
        manager: ManagerId,
        command_message: String,
    ) {
        self.updates_info.init_progress = Some((completed.min(total), total));
        let manager_name = self.manager_catalog.display_name(&manager).to_owned();
        Self::push_init_log(
            &mut self.updates_info.init_logs,
            "InitUpdates",
            &manager_name,
            command_message,
        );
    }

    fn push_init_log(
        logs: &mut Vec<String>,
        phase: &str,
        manager_name: &str,
        command_message: String,
    ) {
        let command_message = command_message.trim();
        if command_message.is_empty() {
            return;
        }

        logs.push(format!("[{phase}][{manager_name}] {command_message}"));

        const MAX_INIT_LOGS: usize = 120;
        if logs.len() > MAX_INIT_LOGS {
            let overflow = logs.len() - MAX_INIT_LOGS;
            logs.drain(0..overflow);
        }
    }

    fn configured_managers(config: &updater_core::Config) -> Vec<ManagerId> {
        config
            .managers
            .iter()
            .map(|manager| manager.id.clone())
            .collect()
    }

    fn complete_pending_settings_exit(&mut self) -> Task<Message> {
        match self.pending_settings_exit.take() {
            Some(PendingSettingsExit::Navigate(target)) => {
                self.content.active_content = target;
                self.sidebar.active_tab = target.into();
                Task::none()
            }
            Some(PendingSettingsExit::Close(window_id)) => iced::window::close(window_id),
            None => Task::none(),
        }
    }

    fn reconcile_selected_managers(
        selected: &mut HashSet<ManagerId>,
        configured: &HashSet<ManagerId>,
        preserve: bool,
    ) {
        if preserve {
            selected.retain(|manager| configured.contains(manager));
        } else {
            selected.clear();
        }
    }

    fn reload_package_data(&mut self, reason: content::ReloadReason) -> Task<Message> {
        let configured: HashSet<_> = Self::configured_managers(&self.pm_config)
            .into_iter()
            .collect();
        let preserve_context = reason.preserves_page_context();

        if preserve_context {
            Self::reconcile_selected_managers(
                &mut self.installed_info.selected_managers,
                &configured,
                true,
            );
            Self::reconcile_selected_managers(
                &mut self.updates_info.selected_managers,
                &configured,
                true,
            );
            Self::reconcile_selected_managers(
                &mut self.finding_info.selected_managers,
                &configured,
                true,
            );
        } else {
            Self::reconcile_selected_managers(
                &mut self.installed_info.selected_managers,
                &configured,
                false,
            );
            self.updates_info.selected_managers = configured.clone();
            Self::reconcile_selected_managers(
                &mut self.finding_info.selected_managers,
                &configured,
                false,
            );
        }

        self.installed_info.installed_packages.clear();
        self.installed_info.selected_packages.clear();
        self.installed_info.loading_installed.clear();
        self.installed_info.load_errors.clear();
        self.installed_info.init_errors.clear();
        self.installed_info.init_logs.clear();
        self.installed_info.has_loading_count = false;
        self.installed_info.is_loading_count = true;
        self.installed_info.confirming_remove = false;

        self.updates_info.updates_by_manager.clear();
        self.updates_info.selected_packages.clear();
        self.updates_info.loading_updates.clear();
        self.updates_info.load_errors.clear();
        self.updates_info.init_errors.clear();
        self.updates_info.init_logs.clear();
        self.updates_info.has_loading_count = false;
        self.updates_info.is_loading_count = true;

        self.finding_info.search_results.clear();
        self.finding_info.search_errors.clear();
        self.finding_info.selected_packages.clear();

        Task::batch(vec![
            self.start_init_installed_counts_task(self.pm_config.clone()),
            self.start_init_updates_counts_task(self.pm_config.clone()),
        ])
    }

    fn start_init_installed_counts_task(&mut self, config: updater_core::Config) -> Task<Message> {
        let managers = Self::configured_managers(&config);
        let manager_set: HashSet<_> = managers.iter().cloned().collect();
        self.installed_info
            .installed_packages
            .retain(|pm_type, _| manager_set.contains(pm_type));
        self.installed_info
            .selected_managers
            .retain(|pm_type| manager_set.contains(pm_type));
        let total = managers.len();
        self.installed_info.init_progress = Some((0, total));
        if total == 0 {
            return self.finish_init_installed_counts();
        }

        run_manager_init_task(
            config,
            managers,
            ManagerInitTask {
                start_label: |_| "Running count_installed".to_string(),
                complete_label: |manager, result: &Result<usize, String>| match result {
                    Ok(count) => format!("Done count_installed -> {}", count),
                    Err(error) => format!("count_installed failed for {manager} -> {error}"),
                },
                work: |manager: ManagerId, config| async move {
                    let runtime =
                        PackageManagerType::from_manager_id(&manager).ok_or_else(|| {
                            format!("Manager is not available in this build: {manager}")
                        })?;
                    runtime
                        .count_installed(&config)
                        .await
                        .map_err(|error| error.to_string())
                },
                item_message: |manager, result| Message::InitInstalledCount { manager, result },
                progress_message: |progress: InitProgress| Message::InitInstalledProgress {
                    completed: progress.completed,
                    total: progress.total,
                    manager: progress.manager,
                    command_message: progress.command_message,
                },
                done_message: || Message::InitInstalledFinished,
            },
        )
    }

    fn start_init_updates_counts_task(&mut self, config: updater_core::Config) -> Task<Message> {
        let managers = Self::configured_managers(&config);
        let manager_set: HashSet<_> = managers.iter().cloned().collect();
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

        run_manager_init_task(
            config,
            managers,
            ManagerInitTask {
                start_label: |_| "Running list_updates".to_string(),
                complete_label: |manager, result: &Result<Vec<PackageUpdate>, String>| match result
                {
                    Ok(updates) => {
                        format!("Done list_updates -> {} updates", updates.len())
                    }
                    Err(error) => {
                        format!("list_updates failed for {manager} -> {error}")
                    }
                },
                work: |manager: ManagerId, config| async move {
                    let runtime =
                        PackageManagerType::from_manager_id(&manager).ok_or_else(|| {
                            format!("Manager is not available in this build: {manager}")
                        })?;
                    runtime
                        .list_updates_with_refresh(&config, false)
                        .await
                        .map_err(|error| error.to_string())
                },
                item_message: |manager, result| Message::InitUpdatesCount { manager, result },
                progress_message: |progress: InitProgress| Message::InitUpdatesProgress {
                    completed: progress.completed,
                    total: progress.total,
                    manager: progress.manager,
                    command_message: progress.command_message,
                },
                done_message: || Message::InitUpdatesFinished,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_id(value: &str) -> ManagerId {
        ManagerId::parse(value).unwrap()
    }

    #[test]
    fn preserving_context_prunes_unconfigured_managers() {
        let configured = HashSet::from([manager_id("builtin:dnf"), manager_id("builtin:flatpak")]);
        let mut selected = HashSet::from([
            manager_id("builtin:dnf"),
            manager_id("builtin:flatpak"),
            manager_id("builtin:cargo"),
        ]);

        App::reconcile_selected_managers(&mut selected, &configured, true);

        assert_eq!(selected, configured);
    }

    #[test]
    fn resetting_context_clears_manager_selection() {
        let configured = HashSet::from([manager_id("builtin:dnf")]);
        let mut selected = HashSet::from([manager_id("builtin:dnf")]);

        App::reconcile_selected_managers(&mut selected, &configured, false);

        assert!(selected.is_empty());
    }

    #[test]
    fn preserving_context_keeps_configured_unknown_manager() {
        let unknown = manager_id("org.example:custom");
        let configured = HashSet::from([unknown.clone()]);
        let mut selected = HashSet::from([unknown.clone(), manager_id("builtin:dnf")]);

        App::reconcile_selected_managers(&mut selected, &configured, true);

        assert_eq!(selected, HashSet::from([unknown]));
    }
}
