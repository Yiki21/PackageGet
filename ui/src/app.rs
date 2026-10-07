//! Top-level application composition and message routing.

use std::collections::HashSet;
use std::time::Instant;

use iced::{Length, Subscription, Task};
use updater_core::ManagerOperationStatus;
use updater_manager_api::{ManagerCapability, ManagerId, PackageUpdate};

use crate::{
    content::{self, Content, FindingInfo, InstalledInfo, ManagerHealthInfo, UpdatesInfo},
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

/// How often the background read-only update count check runs.
const UPDATE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

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
    /// Shared package-manager health state.
    pub manager_health: ManagerHealthInfo,
    /// Status panel state.
    pub status_panel: StatusPanel,
    /// Bounded structured operation history.
    activity_history: crate::activity::ActivityHistory,
    /// Whether the Activity Center is open.
    activity_center_open: bool,
    /// Monotonic operation record identifier.
    next_operation_id: u64,
    /// Cooperative cancellation token for the active package operation.
    active_operation_cancellation: Option<content::CancellationToken>,
    active_operation_started_at: Option<String>,
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
    /// Current configuration loading or recovery state.
    config_load_state: ConfigLoadState,
    /// Current package-data reload generation.
    package_data_generation: u64,
    /// Managers whose in-flight installed initialization must not overwrite a
    /// newer post-operation refresh.
    installed_refresh_overrides: HashSet<ManagerId>,
    /// Managers whose in-flight updates initialization must not overwrite a
    /// newer post-operation refresh.
    updates_refresh_overrides: HashSet<ManagerId>,
    /// Update count that the user has already been notified about.
    ///
    /// `None` means no non-zero count has been announced yet, so the next
    /// non-zero discovery notifies once.
    notified_update_count: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
enum PendingSettingsExit {
    Navigate(content::ActiveContentPage),
    Close(iced::window::Id),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigLoadState {
    Loading,
    Ready,
    Failed {
        load_error: String,
        recovery_error: Option<String>,
    },
    ConfirmReset {
        load_error: String,
    },
    Resetting {
        load_error: String,
    },
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
    /// Scheduled read-only check for newly available updates.
    ScheduledUpdateCheck,
    /// Configuration load result.
    ConfigLoaded(Result<updater_core::Config, updater_core::error::CoreError>),
    /// Retry strict configuration loading.
    RetryConfigLoad,
    /// Open the configuration directory in the desktop file manager.
    OpenConfigDirectory,
    /// Desktop configuration-directory opener result.
    ConfigDirectoryOpened(Result<(), String>),
    /// Ask for confirmation before replacing the configuration.
    RequestConfigReset,
    /// Return to the configuration load failure without resetting.
    CancelConfigReset,
    /// Confirm manager detection and configuration replacement.
    ConfirmConfigReset,
    /// Installed initialization progress message.
    InitInstalledProgress {
        /// Package-data reload generation.
        generation: u64,
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
        /// Package-data reload generation.
        generation: u64,
        /// Source manager.
        manager: ManagerId,
        /// Installed package count value or failure detail.
        result: Result<usize, String>,
    },
    /// Installed initialization completion message.
    InitInstalledFinished {
        /// Package-data reload generation.
        generation: u64,
    },
    /// Updates initialization progress message.
    InitUpdatesProgress {
        /// Package-data reload generation.
        generation: u64,
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
        /// Package-data reload generation.
        generation: u64,
        /// Source manager.
        manager: ManagerId,
        /// Update entries or failure detail.
        result: Result<Vec<PackageUpdate>, String>,
    },
    /// Updates initialization completion message.
    InitUpdatesFinished {
        /// Package-data reload generation.
        generation: u64,
    },
}

/// Keyboard hint shown in the footer for one page.
///
/// Every page advertises the page-switch shortcut and how to close the current
/// transient surface; list pages additionally advertise moving and selecting,
/// and only pages with a search box advertise `/` as the focus control.
pub(crate) fn footer_shortcuts(page: content::ActiveContentPage) -> &'static str {
    match page {
        content::ActiveContentPage::Finding => {
            "Ctrl+K Search  ·  ↑↓ Move  ·  Space Select  ·  Esc Close  ·  / Focus  ·  Alt+1–5 Pages  ·  Ctrl+Enter Install"
        }
        content::ActiveContentPage::Updates => {
            "Ctrl+R Refresh  ·  ↑↓ Move  ·  Space Select  ·  Esc Close  ·  / Focus  ·  Alt+1–5 Pages  ·  Ctrl+Enter Update"
        }
        content::ActiveContentPage::Installed => {
            "Ctrl+R Refresh  ·  ↑↓ Move  ·  Space Select  ·  Esc Close  ·  / Focus  ·  Alt+1–5 Pages  ·  Ctrl+Enter Remove"
        }
        content::ActiveContentPage::Health => {
            "Ctrl+R Recheck  ·  Esc Close  ·  Alt+1–5 Pages  ·  Ctrl+Enter Save"
        }
        content::ActiveContentPage::Settings => {
            "Tab Move Focus  ·  Esc Close  ·  Alt+1–5 Pages  ·  Ctrl+Enter Save"
        }
    }
}

impl App {
    /// Creates app state and starts config loading.
    pub fn new() -> (Self, Task<Message>) {
        let now = Instant::now();
        let manager_catalog = ManagerCatalog::builtin();
        let config_registry = manager_catalog.registry();

        let app = Self {
            sidebar: SideBar::default(),
            content: Content::default(),
            manager_catalog,
            pm_config: updater_core::Config::default(),
            installed_info: InstalledInfo::default(),
            updates_info: UpdatesInfo::default(),
            finding_info: FindingInfo::default(),
            manager_health: ManagerHealthInfo::default(),
            status_panel: StatusPanel::new(now),
            activity_history: crate::activity::ActivityHistory::default(),
            activity_center_open: false,
            next_operation_id: 1,
            active_operation_cancellation: None,
            active_operation_started_at: None,
            window_size: iced::Size::new(1200.0, 800.0),
            sidebar_expanded: false,
            inspector_drawer_open: false,
            system_theme: iced::theme::Mode::Light,
            pending_settings_exit: None,
            config_load_state: ConfigLoadState::Loading,
            package_data_generation: 0,
            installed_refresh_overrides: HashSet::new(),
            updates_refresh_overrides: HashSet::new(),
            notified_update_count: None,
        };

        let task = Task::batch(vec![
            app.load_config_task(config_registry),
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
            // A fixed-interval read-only check so the Updates badge can appear
            // without the user opening the Updates page first.
            iced::time::every(UPDATE_CHECK_INTERVAL).map(|_| Message::ScheduledUpdateCheck),
            iced::window::close_requests().map(Message::CloseRequested),
            iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            iced::system::theme_changes().map(Message::SystemThemeChanged),
        ])
    }

    /// Handles one app message and returns follow-up tasks.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let at = Instant::now();
        // `StopOperation` changes panel state that the synced label must
        // reflect, so it is routed like an ordinary message rather than an
        // animation-only tick.
        let is_animation_message = matches!(
            &message,
            Message::StatusPanel(panel_msg)
                if !matches!(panel_msg, status_panel::Message::StopOperation)
        );
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
                    if Self::is_configuration_page(self.content.active_content)
                        && !Self::is_configuration_page(target)
                        && self.content.settings.is_dirty()
                    {
                        self.sidebar.active_tab = self.content.active_content.into();
                        self.pending_settings_exit = Some(PendingSettingsExit::Navigate(target));
                    } else {
                        task = self.activate_page(target);
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
                    &mut self.manager_health,
                    &self.manager_catalog,
                );
                if self.updates_info.background_loading_count {
                    // A source the user reloads during a scheduled check must
                    // keep that fresher result over the check's older read.
                    self.updates_refresh_overrides
                        .extend(self.updates_info.loading_updates.keys().cloned());
                }

                task = match action {
                    content::Action::Run(content_task) => content_task.map(Message::Content),
                    content::Action::CancellableRun(content_task, cancellation) => {
                        self.status_panel.begin_package_operation();
                        self.active_operation_cancellation = Some(cancellation);
                        self.active_operation_started_at = Some(crate::activity::now_timestamp());
                        content_task.map(Message::Content)
                    }
                    content::Action::Navigate(target) => self.navigate_to(target),
                    content::Action::ReloadPackageData { reload, follow_up } => {
                        let configuration_changed = reload.is_configuration_change();
                        let package_reload = self.reload_package_data(reload);
                        let pending_exit =
                            if configuration_changed && !self.content.settings.is_dirty() {
                                self.complete_pending_settings_exit()
                            } else {
                                Task::none()
                            };
                        Task::batch(vec![
                            package_reload,
                            follow_up.map(Message::Content),
                            pending_exit,
                        ])
                    }
                    content::Action::PackageOperationFinished { outcome, follow_up } => {
                        self.active_operation_cancellation = None;
                        let started_at = self
                            .active_operation_started_at
                            .take()
                            .unwrap_or_else(crate::activity::now_timestamp);
                        let notification = self.record_operation(&outcome, started_at);
                        let refresh = self.refresh_package_managers(&outcome);
                        self.status_panel.record_outcome(outcome);
                        let completion =
                            Task::batch(vec![refresh, follow_up.map(Message::Content)]);
                        Task::batch(vec![completion, notification])
                    }
                    content::Action::None => Task::none(),
                };
            }
            Message::StatusPanel(panel_msg) => {
                if matches!(panel_msg, status_panel::Message::StopOperation) {
                    self.cancel_active_operation();
                }
                self.status_panel.update(
                    panel_msg,
                    &self.installed_info,
                    &self.updates_info,
                    &self.finding_info,
                    &self.manager_catalog,
                );
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
            Message::ScheduledUpdateCheck => {
                task = self.scheduled_update_check();
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
                    if !Self::is_configuration_page(self.content.active_content) {
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
                let manager_changes = content::ManagerConfigChanges::between(
                    self.content.settings.draft_config(),
                    &self.pm_config,
                );
                self.content.settings.discard_changes();
                self.manager_health
                    .reconcile_configuration(&self.pm_config, &manager_changes.affected_managers());
                task = self.complete_pending_settings_exit();
            }
            Message::CancelPendingSettingsExit => self.pending_settings_exit = None,
            Message::ConfigLoaded(result) => {
                task = match result {
                    Ok(config) => {
                        self.config_load_state = ConfigLoadState::Ready;
                        self.content.settings.sync_from_config(&config);
                        self.pm_config = config;
                        self.manager_health.invalidate();
                        // `reload_package_data` mutates state synchronously, so
                        // the scheduled check that follows always sees the new
                        // package-data generation.
                        Task::batch(vec![
                            self.reload_package_data(content::PackageDataReload::Startup),
                            Task::done(Message::ScheduledUpdateCheck),
                        ])
                    }
                    Err(error) => {
                        let (load_error, recovery_error) = match &self.config_load_state {
                            ConfigLoadState::Resetting { load_error } => (
                                load_error.clone(),
                                Some(format!("Configuration reset failed: {error}")),
                            ),
                            _ => (error.to_string(), None),
                        };
                        log::error!("Failed to load config: {error}");
                        self.config_load_state = ConfigLoadState::Failed {
                            load_error,
                            recovery_error,
                        };
                        Task::none()
                    }
                };
            }
            Message::RetryConfigLoad => {
                if matches!(self.config_load_state, ConfigLoadState::Failed { .. }) {
                    self.config_load_state = ConfigLoadState::Loading;
                    task = self.load_config_task(self.manager_catalog.registry());
                }
            }
            Message::OpenConfigDirectory => {
                if matches!(self.config_load_state, ConfigLoadState::Failed { .. }) {
                    task = Task::perform(
                        async {
                            let file = updater_core::Config::file_path()
                                .map_err(|error| error.to_string())?;
                            let directory = file
                                .parent()
                                .ok_or_else(|| {
                                    "Configuration path has no parent directory".to_owned()
                                })?
                                .to_path_buf();
                            tokio::fs::create_dir_all(&directory)
                                .await
                                .map_err(|error| {
                                    format!("Could not create {}: {error}", directory.display())
                                })?;
                            crate::content::open_directory(directory).await
                        },
                        Message::ConfigDirectoryOpened,
                    );
                }
            }
            Message::ConfigDirectoryOpened(result) => {
                if let ConfigLoadState::Failed { recovery_error, .. } = &mut self.config_load_state
                {
                    *recovery_error = result.err();
                }
            }
            Message::RequestConfigReset => {
                if let ConfigLoadState::Failed { load_error, .. } = &self.config_load_state {
                    self.config_load_state = ConfigLoadState::ConfirmReset {
                        load_error: load_error.clone(),
                    };
                }
            }
            Message::CancelConfigReset => {
                if let ConfigLoadState::ConfirmReset { load_error } = &self.config_load_state {
                    self.config_load_state = ConfigLoadState::Failed {
                        load_error: load_error.clone(),
                        recovery_error: None,
                    };
                }
            }
            Message::ConfirmConfigReset => {
                if let ConfigLoadState::ConfirmReset { load_error } = &self.config_load_state {
                    let load_error = load_error.clone();
                    self.config_load_state = ConfigLoadState::Resetting { load_error };
                    let registry = self.manager_catalog.registry();
                    task = Task::perform(
                        async move {
                            let config =
                                updater_core::Config::detect_package_managers(&registry).await;
                            config.save().await?;
                            Ok(config)
                        },
                        Message::ConfigLoaded,
                    );
                }
            }
            Message::InitInstalledProgress {
                generation,
                completed,
                total,
                manager,
                command_message,
            } => {
                if generation == self.package_data_generation
                    && !self.installed_refresh_overrides.contains(&manager)
                {
                    self.installed_info.init_progress = Some((completed.min(total), total));
                    let manager_name = self.manager_catalog.display_name(&manager).to_owned();
                    Self::push_init_log(
                        &mut self.installed_info.init_logs,
                        "InitInstalled",
                        &manager_name,
                        command_message,
                    );
                }
            }
            Message::InitInstalledCount {
                generation,
                manager,
                result,
            } => {
                if generation == self.package_data_generation
                    && !self.installed_refresh_overrides.contains(&manager)
                {
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
            }
            Message::InitInstalledFinished { generation } => {
                if generation == self.package_data_generation {
                    task = self.finish_init_installed_counts();
                }
            }
            Message::InitUpdatesProgress {
                generation,
                completed,
                total,
                manager,
                command_message,
            } => {
                if generation == self.package_data_generation
                    && !self.updates_refresh_overrides.contains(&manager)
                {
                    self.updates_info.init_progress = Some((completed.min(total), total));
                    let manager_name = self.manager_catalog.display_name(&manager).to_owned();
                    Self::push_init_log(
                        &mut self.updates_info.init_logs,
                        "InitUpdates",
                        &manager_name,
                        command_message,
                    );
                }
            }
            Message::InitUpdatesCount {
                generation,
                manager,
                result,
            } => {
                if generation == self.package_data_generation
                    && !self.updates_refresh_overrides.contains(&manager)
                {
                    self.updates_info.has_loading_count = true;
                    match result {
                        Ok(updates) => {
                            self.updates_info.init_errors.remove(&manager);
                            self.updates_info
                                .mark_checked(manager.clone(), crate::activity::now_timestamp());
                            self.updates_info
                                .updates_by_manager
                                .insert(manager, (updates.len(), updates));
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
            }
            Message::InitUpdatesFinished { generation } => {
                if generation == self.package_data_generation {
                    task = self.finish_init_updates_counts();
                }
            }
            Message::Shortcut(_) => unreachable!("shortcuts are handled before routed messages"),
        }

        task
    }

    fn handle_shortcut(&mut self, shortcut: Shortcut) -> Task<Message> {
        use content::ActiveContentPage;

        if !matches!(self.config_load_state, ConfigLoadState::Ready) {
            return if matches!(shortcut, Shortcut::Dismiss) {
                self.update_message(Message::CancelConfigReset)
            } else {
                Task::none()
            };
        }

        if matches!(shortcut, Shortcut::Dismiss) {
            // Innermost surface first; the chain short-circuits, so one press
            // closes one thing. Dismiss must not steal focus back into the page
            // search box, so it never refocuses anything: the user asked to
            // close something, not to start typing again, and `/` remains the
            // documented way to focus search.
            let _dismissed = self.pending_settings_exit.take().is_some()
                || self
                    .content
                    .dismiss_active_transient(&mut self.installed_info)
                || self.status_panel.dismiss_top_surface()
                || std::mem::replace(&mut self.activity_center_open, false);

            return Task::none();
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
            Shortcut::NavigateHealth => self.navigate_to(ActiveContentPage::Health),
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
            // The focused text input already dropped its own focus before the
            // shortcut was published, and iced exposes no `unfocus` task
            // operation, so there is nothing left to do but leave the page's
            // inspected row and drawers alone.
            Shortcut::BlurInput => Task::none(),
            Shortcut::Dismiss => Task::none(),
        }
    }

    fn navigate_to(&mut self, target: content::ActiveContentPage) -> Task<Message> {
        if self.content.active_content == target {
            return Task::none();
        }

        if Self::is_configuration_page(self.content.active_content)
            && !Self::is_configuration_page(target)
            && self.content.settings.is_dirty()
        {
            self.pending_settings_exit = Some(PendingSettingsExit::Navigate(target));
            return Task::none();
        }

        self.pending_settings_exit = None;
        self.activate_page(target)
    }

    fn activate_page(&mut self, target: content::ActiveContentPage) -> Task<Message> {
        self.content.active_content = target;
        self.sidebar.active_tab = target.into();

        let mut tasks = Vec::new();
        match target {
            content::ActiveContentPage::Updates
                if !self.updates_info.has_loading_count && !self.updates_info.is_loading_count =>
            {
                // A scheduled check may already be running. Adopt it, so the
                // page shows its loading state without a second concurrent
                // scan of the same managers.
                let scheduled = self.updates_info.background_loading_count;
                self.updates_info.is_loading_count = true;
                if !scheduled {
                    tasks.push(self.start_init_updates_counts_task(
                        self.pm_config.clone(),
                        self.package_data_generation,
                    ));
                }
            }
            content::ActiveContentPage::Installed
                if !self.installed_info.has_loading_count
                    && !self.installed_info.is_loading_count =>
            {
                self.installed_info.is_loading_count = true;
                tasks.push(self.start_init_installed_counts_task(
                    self.pm_config.clone(),
                    self.package_data_generation,
                ));
            }
            content::ActiveContentPage::Health
                if self
                    .manager_health
                    .should_scan_on_open(self.content.settings.draft_config()) =>
            {
                tasks.push(Task::done(Message::Content(content::Message::Health(
                    content::HealthMessage::StartScan,
                ))));
            }
            _ => {}
        }

        Task::batch(tasks)
    }

    fn is_configuration_page(page: content::ActiveContentPage) -> bool {
        matches!(
            page,
            content::ActiveContentPage::Health | content::ActiveContentPage::Settings
        )
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

    fn save_activity_history(&self) -> Task<Message> {
        let history = self.activity_history.clone();
        Task::perform(
            async move { history.save().await },
            Message::ActivityHistorySaved,
        )
    }

    fn cancel_active_operation(&mut self) {
        if let Some(cancellation) = &self.active_operation_cancellation {
            cancellation.cancel();
            self.status_panel.request_cancellation();
        }
    }

    /// Runs the read-only update count check when nothing else is in flight.
    ///
    /// This deliberately reuses the lazy per-page initialization path, which
    /// calls `list_updates(.., refresh = false)`, so it can never trigger a
    /// privileged database refresh or a write.
    fn scheduled_update_check(&mut self) -> Task<Message> {
        if !matches!(self.config_load_state, ConfigLoadState::Ready) {
            return Task::none();
        }
        if !self.has_idle_update_scan() {
            return Task::none();
        }

        // A scheduled check is background work: it must not pull the status
        // panel open with a progress bar the user never asked for.
        self.updates_info.background_loading_count = true;
        self.start_init_updates_counts_task(self.pm_config.clone(), self.package_data_generation)
    }

    /// Whether a background update count scan may start right now.
    ///
    /// Reuses the same guards the Updates page uses: an in-flight count scan or
    /// per-source load blocks a new one, and so does any other active work so a
    /// background scan cannot race an operation's own manager refresh.
    fn has_idle_update_scan(&self) -> bool {
        !self.updates_info.is_loading_count
            && !self.updates_info.background_loading_count
            && self.updates_info.loading_updates.is_empty()
            && !self.installed_info.is_loading_count
            && self.installed_info.loading_installed.is_empty()
            && !self.finding_info.is_installing
            && !self.updates_info.is_updating
            && !self.installed_info.is_removing
            && !self.installed_info.is_updating
            && self.finding_info.searching_managers.is_empty()
    }

    /// Builds the native notification task for a completed or discovered result.
    fn notification_task(title: &'static str, body: String) -> Task<Message> {
        Task::future(async move {
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
        .then(|result| Task::done(Message::NotificationFinished(result)))
    }

    fn record_operation(
        &mut self,
        outcome: &content::OperationOutcome,
        started_at: String,
    ) -> Task<Message> {
        let id = self.next_operation_id;
        self.next_operation_id = self.next_operation_id.saturating_add(1);
        self.activity_history
            .push(crate::activity::ActivityRecord::from_outcome(
                id,
                outcome,
                started_at,
                crate::activity::now_timestamp(),
            ));
        let persist = self.save_activity_history();

        if !self.pm_config.notifications_enabled {
            return persist;
        }

        let title = if outcome.is_success() {
            "Updater operation completed"
        } else {
            "Updater operation stopped"
        };
        let notification = Self::notification_task(title, outcome.summary());
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

    /// Builds the sidebar summary, including the at-a-glance update badge count.
    pub fn sidebar_summary(&self) -> sidebar::Summary {
        let update_count = self.updates_info.current_update_count();
        sidebar::Summary {
            update_count,
            // The loading glyph belongs to the first-ever scan. A later
            // background check must not hide a count the user can already see,
            // while a load the user asked for looks the same as it always did.
            updates_loading: self.updates_info.is_loading_count
                || (self.updates_info.background_loading_count
                    && !self.updates_info.has_loading_count)
                || !self.updates_info.loading_updates.is_empty(),
            updates_failed: !self.updates_info.init_errors.is_empty()
                || !self.updates_info.load_errors.is_empty(),
            health_checking: self.manager_health.is_checking(),
            health_has_issues: self.manager_health.has_issues(
                &self.pm_config,
                &self.installed_info,
                &self.updates_info,
            ),
            settings_dirty: self.content.settings.is_dirty(),
        }
    }

    fn load_config_task(
        &self,
        registry: std::sync::Arc<updater_core::ManagerRegistry>,
    ) -> Task<Message> {
        Task::perform(
            async move { updater_core::Config::load(&registry).await },
            Message::ConfigLoaded,
        )
    }

    /// Renders the app UI.
    pub fn view(&self) -> iced::Element<'_, Message> {
        use iced::widget::{button, column, container, row, text};

        if !matches!(self.config_load_state, ConfigLoadState::Ready) {
            let recovery: iced::Element<'_, Message> = match &self.config_load_state {
                ConfigLoadState::Loading => column![
                    text("Loading configuration")
                        .size(24)
                        .font(crate::theme::FONT_SEMIBOLD),
                    text("Checking config.json...")
                        .size(14)
                        .style(crate::theme::text_on_surface_muted),
                ]
                .spacing(8)
                .align_x(iced::Alignment::Center)
                .into(),
                ConfigLoadState::Failed {
                    load_error,
                    recovery_error,
                } => {
                    let retry = button(
                        text("Retry")
                            .size(14)
                            .font(crate::theme::FONT_SEMIBOLD)
                            .style(crate::theme::text_on_primary),
                    )
                    .padding([9, 16])
                    .style(crate::theme::action_button(
                        true,
                        crate::theme::colors::ACCENT,
                        crate::theme::colors::ACCENT_HOVER,
                        crate::theme::colors::ACCENT_ACTIVE,
                    ))
                    .on_press(Message::RetryConfigLoad);
                    let open_folder = button(text("Open Config Folder").size(14))
                        .padding([9, 14])
                        .style(crate::theme::secondary_button(true))
                        .on_press(Message::OpenConfigDirectory);
                    let reset = button(
                        text("Reset Configuration")
                            .size(14)
                            .font(crate::theme::FONT_SEMIBOLD)
                            .style(crate::theme::text_on_primary),
                    )
                    .padding([9, 14])
                    .style(crate::theme::action_button(
                        true,
                        crate::theme::colors::REMOVE_ACTION,
                        crate::theme::colors::REMOVE_ACTION_HOVER,
                        crate::theme::colors::REMOVE_ACTION_ACTIVE,
                    ))
                    .on_press(Message::RequestConfigReset);
                    let mut content = column![
                        text("Configuration unavailable")
                            .size(24)
                            .font(crate::theme::FONT_SEMIBOLD)
                            .style(crate::theme::text_error),
                        text("Updater left config.json unchanged.")
                            .size(14)
                            .style(crate::theme::text_on_surface_muted),
                        container(
                            text(load_error)
                                .size(12)
                                .font(crate::theme::FONT_MONO)
                                .style(crate::theme::text_on_surface)
                                .width(Length::Fill)
                                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                        )
                        .padding(14)
                        .width(Length::Fill)
                        .style(crate::theme::surface_container),
                        row![retry, open_folder, reset]
                            .spacing(10)
                            .width(Length::Fill)
                            .wrap(),
                    ]
                    .spacing(16)
                    .width(Length::Fill);
                    if let Some(error) = recovery_error {
                        content = content.push(
                            text(error)
                                .size(13)
                                .style(crate::theme::text_error)
                                .width(Length::Fill)
                                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                        );
                    }
                    content.into()
                }
                ConfigLoadState::ConfirmReset { load_error } => {
                    let cancel = button(text("Cancel").size(14))
                        .padding([9, 14])
                        .style(crate::theme::secondary_button(true))
                        .on_press(Message::CancelConfigReset);
                    let confirm = button(
                        text("Reset Configuration")
                            .size(14)
                            .font(crate::theme::FONT_SEMIBOLD)
                            .style(crate::theme::text_on_primary),
                    )
                    .padding([9, 14])
                    .style(crate::theme::action_button(
                        true,
                        crate::theme::colors::REMOVE_ACTION,
                        crate::theme::colors::REMOVE_ACTION_HOVER,
                        crate::theme::colors::REMOVE_ACTION_ACTIVE,
                    ))
                    .on_press(Message::ConfirmConfigReset);
                    column![
                        text("Reset configuration?")
                            .size(24)
                            .font(crate::theme::FONT_SEMIBOLD),
                        text(
                            "This replaces config.json with detected managers and default application settings. Existing settings will be lost.",
                        )
                        .size(14)
                        .style(crate::theme::text_warning)
                        .width(Length::Fill)
                        .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                        container(
                            text(load_error)
                                .size(12)
                                .font(crate::theme::FONT_MONO)
                                .style(crate::theme::text_on_surface)
                                .width(Length::Fill)
                                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
                        )
                        .padding(14)
                        .width(Length::Fill)
                        .style(crate::theme::surface_container),
                        row![cancel, confirm].spacing(10).wrap(),
                    ]
                    .spacing(16)
                    .width(Length::Fill)
                    .into()
                }
                ConfigLoadState::Resetting { .. } => column![
                    text("Resetting configuration")
                        .size(24)
                        .font(crate::theme::FONT_SEMIBOLD),
                    text("Detecting available package managers...")
                        .size(14)
                        .style(crate::theme::text_on_surface_muted),
                ]
                .spacing(8)
                .align_x(iced::Alignment::Center)
                .into(),
                ConfigLoadState::Ready => unreachable!("ready state renders the main workspace"),
            };
            let page = container(container(recovery).width(Length::Fill).max_width(760))
                .padding(32)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(crate::theme::content_container);
            return crate::shortcut::capture(page.into());
        }

        let sidebar_summary = self.sidebar_summary();
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
                &self.manager_health,
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
                iced::widget::text(footer_shortcuts(self.content.active_content))
                    .size(11)
                    .style(crate::theme::text_on_surface_alt)
                    .width(Length::Fill),
                iced::widget::button(
                    iced::widget::text(status_panel::FOOTER_HISTORY_LABEL).size(11)
                )
                .padding([3, 8])
                .style(crate::theme::secondary_button(true))
                .on_press(Message::ToggleActivityCenter),
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
                    let manager_details = record
                        .manager_summaries(&self.manager_catalog)
                        .into_iter()
                        .fold(column![], |details, summary| {
                            details.push(
                                iced::widget::text(summary)
                                    .size(11)
                                    .style(crate::theme::text_on_surface_muted),
                            )
                        });
                    column![
                        iced::widget::text(record.title())
                            .size(13)
                            .font(crate::theme::FONT_SEMIBOLD),
                        iced::widget::text(record.time_summary())
                            .size(11)
                            .style(crate::theme::text_on_surface_muted),
                        iced::widget::text(format!("Scope: {}", record.scope_summary()))
                            .size(11)
                            .style(crate::theme::text_on_surface_muted),
                        iced::widget::text(record.summary(&self.manager_catalog))
                            .size(12)
                            .style(crate::theme::text_on_surface_muted),
                        manager_details,
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
                    iced::widget::text("Save configuration changes before leaving?")
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

    fn finish_init_installed_counts(&mut self) -> Task<Message> {
        self.installed_info.is_loading_count = false;
        self.installed_info.has_loading_count = true;
        self.installed_info.init_progress = None;

        let refresh_overrides = std::mem::take(&mut self.installed_refresh_overrides);
        let managers: Vec<_> = self
            .installed_info
            .selected_managers
            .iter()
            .filter(|manager| !self.installed_info.init_errors.contains_key(manager))
            .filter(|manager| !refresh_overrides.contains(manager))
            .cloned()
            .collect();
        if managers.is_empty() {
            return Task::none();
        }

        let mut tasks = Vec::with_capacity(managers.len());
        for manager in managers {
            tasks.push(
                content::Installed::start_load(
                    &self.pm_config,
                    &mut self.installed_info,
                    manager,
                    &self.manager_catalog,
                )
                .map(content::Message::Installed)
                .map(Message::Content),
            );
        }
        Task::batch(tasks)
    }

    fn finish_init_updates_counts(&mut self) -> Task<Message> {
        self.updates_info.is_loading_count = false;
        self.updates_info.background_loading_count = false;
        self.updates_info.has_loading_count = true;
        self.updates_info.init_progress = None;
        self.updates_refresh_overrides.clear();

        self.notify_new_update_count()
    }

    /// Raises one notification when a scan discovers a non-zero update count.
    ///
    /// Gated on the existing notification preference, and raised only on the
    /// transition from nothing to something: a repeated background check, or
    /// one that finds a few more updates than the previous scan, stays silent.
    fn notify_new_update_count(&mut self) -> Task<Message> {
        let update_count: usize = self
            .updates_info
            .updates_by_manager
            .values()
            .map(|(count, _)| *count)
            .sum();

        if update_count == 0 {
            // A later non-zero discovery should be able to notify again.
            self.notified_update_count = None;
            return Task::none();
        }
        if !self.pm_config.notifications_enabled || self.notified_update_count.is_some() {
            return Task::none();
        }

        self.notified_update_count = Some(update_count);
        let body = if update_count == 1 {
            "1 package update is available".to_owned()
        } else {
            format!("{update_count} package updates are available")
        };
        Self::notification_task("Updater found package updates", body)
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
            Some(PendingSettingsExit::Navigate(target)) => self.activate_page(target),
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

    fn reload_package_data(&mut self, reload: content::PackageDataReload) -> Task<Message> {
        let configured: HashSet<_> = Self::configured_managers(&self.pm_config)
            .into_iter()
            .collect();
        let update_managers: HashSet<_> = content::configured_managers_with_capability(
            &self.pm_config,
            &self.manager_catalog,
            ManagerCapability::Updates,
        )
        .into_iter()
        .collect();
        let search_managers: HashSet<_> = content::configured_managers_with_capability(
            &self.pm_config,
            &self.manager_catalog,
            ManagerCapability::Search,
        )
        .into_iter()
        .collect();
        let preserve_context = reload.preserves_page_context();

        if preserve_context {
            Self::reconcile_selected_managers(
                &mut self.installed_info.selected_managers,
                &configured,
                true,
            );
            Self::reconcile_selected_managers(
                &mut self.updates_info.selected_managers,
                &update_managers,
                true,
            );
            Self::reconcile_selected_managers(
                &mut self.finding_info.selected_managers,
                &search_managers,
                true,
            );
        } else {
            Self::reconcile_selected_managers(
                &mut self.installed_info.selected_managers,
                &configured,
                false,
            );
            self.updates_info.selected_managers = update_managers;
            Self::reconcile_selected_managers(
                &mut self.finding_info.selected_managers,
                &search_managers,
                false,
            );
        }

        match reload {
            content::PackageDataReload::Startup => {
                self.package_data_generation = self.package_data_generation.wrapping_add(1);

                self.installed_info.installed_packages.clear();
                self.installed_info.selected_packages.clear();
                self.installed_info.loading_installed.clear();
                self.installed_info.load_errors.clear();
                self.installed_info.init_errors.clear();
                self.installed_info.init_logs.clear();
                self.installed_info.has_loading_count = false;
                self.installed_info.is_loading_count = false;
                self.installed_info.pending_remove = None;
                self.installed_refresh_overrides.clear();

                self.updates_info.updates_by_manager.clear();
                self.updates_info.selected_packages.clear();
                self.updates_info.loading_updates.clear();
                self.updates_info.refresh_modes.clear();
                self.updates_info.checked_at.clear();
                self.updates_info.load_errors.clear();
                self.updates_info.init_errors.clear();
                self.updates_info.init_logs.clear();
                self.updates_info.has_loading_count = false;
                self.updates_info.is_loading_count = false;
                self.updates_info.background_loading_count = false;
                self.content.updates.reset_pending_updates();
                self.updates_refresh_overrides.clear();

                self.finding_info.search_results.clear();
                self.finding_info.search_errors.clear();
                self.finding_info.selected_packages.clear();
                self.finding_info.searching_managers.clear();
                self.content.finding.reset_pending_install();

                Task::none()
            }
            content::PackageDataReload::ManagersChanged(changes) => {
                self.reload_changed_package_managers(changes)
            }
        }
    }

    fn reload_changed_package_managers(
        &mut self,
        changes: content::ManagerConfigChanges,
    ) -> Task<Message> {
        let affected = changes.affected_managers();
        let installed_initialized =
            self.installed_info.has_loading_count || self.installed_info.is_loading_count;
        let updates_initialized = self.updates_info.has_loading_count
            || self.updates_info.is_loading_count
            || self.updates_info.background_loading_count;

        self.installed_info
            .installed_packages
            .retain(|manager, _| !affected.contains(manager));
        self.installed_info
            .selected_packages
            .retain(|(manager, _)| !affected.contains(manager));
        self.installed_info
            .loading_installed
            .retain(|manager, _| !affected.contains(manager));
        self.installed_info
            .load_errors
            .retain(|manager, _| !affected.contains(manager));
        self.installed_info
            .init_errors
            .retain(|manager, _| !affected.contains(manager));
        self.installed_info.pending_remove = None;

        self.updates_info
            .updates_by_manager
            .retain(|manager, _| !affected.contains(manager));
        self.updates_info
            .selected_packages
            .retain(|(manager, _)| !affected.contains(manager));
        self.updates_info
            .loading_updates
            .retain(|manager, _| !affected.contains(manager));
        self.updates_info
            .refresh_modes
            .retain(|manager, _| !affected.contains(manager));
        self.updates_info
            .checked_at
            .retain(|manager, _| !affected.contains(manager));
        self.updates_info
            .load_errors
            .retain(|manager, _| !affected.contains(manager));
        self.updates_info
            .init_errors
            .retain(|manager, _| !affected.contains(manager));
        self.content.updates.reset_pending_updates();

        self.finding_info
            .search_results
            .retain(|manager, _| !affected.contains(manager));
        self.finding_info
            .search_errors
            .retain(|manager, _| !affected.contains(manager));
        self.finding_info
            .selected_packages
            .retain(|(manager, _)| !affected.contains(manager));
        self.finding_info
            .searching_managers
            .retain(|manager, _| !affected.contains(manager));
        self.content.finding.reset_pending_install();

        if self.installed_info.is_loading_count {
            self.installed_refresh_overrides
                .extend(affected.iter().cloned());
        }
        if self.updates_info.is_loading_count || self.updates_info.background_loading_count {
            self.updates_refresh_overrides
                .extend(affected.iter().cloned());
        }

        let mut tasks = Vec::new();
        for configured in &self.pm_config.managers {
            let manager = &configured.id;
            if !changes.reload_managers().contains(manager) {
                continue;
            }

            let capabilities = self
                .manager_catalog
                .descriptor(manager)
                .map(|descriptor| descriptor.capabilities());
            if installed_initialized
                && capabilities
                    .is_some_and(|capabilities| capabilities.contains(ManagerCapability::Installed))
            {
                tasks.push(
                    content::Installed::start_load(
                        &self.pm_config,
                        &mut self.installed_info,
                        manager.clone(),
                        &self.manager_catalog,
                    )
                    .map(content::Message::Installed)
                    .map(Message::Content),
                );
            }
            if updates_initialized
                && capabilities
                    .is_some_and(|capabilities| capabilities.contains(ManagerCapability::Updates))
            {
                tasks.push(
                    content::Updates::start_load(
                        &self.pm_config,
                        &mut self.updates_info,
                        manager.clone(),
                        &self.manager_catalog,
                        content::RefreshMode::Privileged,
                    )
                    .map(content::Message::Updates)
                    .map(Message::Content),
                );
            }
        }

        Task::batch(tasks)
    }

    fn refresh_package_managers(&mut self, outcome: &content::OperationOutcome) -> Task<Message> {
        let managers: HashSet<_> = outcome
            .manager_outcomes
            .iter()
            .filter(|manager| manager.status == ManagerOperationStatus::Succeeded)
            .map(|manager| manager.manager_id.clone())
            .collect();
        if managers.is_empty() {
            return Task::none();
        }

        self.installed_info
            .selected_packages
            .retain(|(manager, _)| !managers.contains(manager));
        self.updates_info
            .selected_packages
            .retain(|(manager, _)| !managers.contains(manager));
        self.finding_info
            .selected_packages
            .retain(|(manager, _)| !managers.contains(manager));

        let mut tasks = Vec::new();
        for manager in managers {
            self.finding_info.search_results.remove(&manager);
            self.finding_info.search_errors.remove(&manager);
            self.finding_info.searching_managers.remove(&manager);

            let capabilities = self
                .manager_catalog
                .descriptor(&manager)
                .map(|descriptor| descriptor.capabilities());
            if (self.installed_info.has_loading_count || self.installed_info.is_loading_count)
                && capabilities
                    .is_some_and(|capabilities| capabilities.contains(ManagerCapability::Installed))
            {
                self.installed_info.init_errors.remove(&manager);
                self.installed_info.load_errors.remove(&manager);
                if self.installed_info.is_loading_count {
                    self.installed_refresh_overrides.insert(manager.clone());
                }
                tasks.push(
                    content::Installed::start_load(
                        &self.pm_config,
                        &mut self.installed_info,
                        manager.clone(),
                        &self.manager_catalog,
                    )
                    .map(content::Message::Installed)
                    .map(Message::Content),
                );
            }

            if (self.updates_info.has_loading_count
                || self.updates_info.is_loading_count
                || self.updates_info.background_loading_count)
                && capabilities
                    .is_some_and(|capabilities| capabilities.contains(ManagerCapability::Updates))
            {
                self.updates_info.init_errors.remove(&manager);
                self.updates_info.load_errors.remove(&manager);
                if self.updates_info.is_loading_count || self.updates_info.background_loading_count
                {
                    self.updates_refresh_overrides.insert(manager.clone());
                }
                tasks.push(
                    content::Updates::start_load(
                        &self.pm_config,
                        &mut self.updates_info,
                        manager,
                        &self.manager_catalog,
                        content::RefreshMode::Local,
                    )
                    .map(content::Message::Updates)
                    .map(Message::Content),
                );
            }
        }

        Task::batch(tasks)
    }

    fn start_init_installed_counts_task(
        &mut self,
        config: updater_core::Config,
        generation: u64,
    ) -> Task<Message> {
        let managers = content::configured_managers_with_capability(
            &config,
            &self.manager_catalog,
            ManagerCapability::Installed,
        );
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

        let registry = self.manager_catalog.registry();
        run_manager_init_task(
            config,
            managers,
            ManagerInitTask {
                start_label: |_| "Running count_installed".to_string(),
                complete_label: |manager, result: &Result<usize, String>| match result {
                    Ok(count) => format!("Done count_installed -> {}", count),
                    Err(error) => format!("count_installed failed for {manager} -> {error}"),
                },
                work: move |manager: ManagerId, config: updater_core::Config| {
                    let registry = registry.clone();
                    async move {
                        let runtime = registry
                            .manager_for(&manager, ManagerCapability::Installed)
                            .map_err(|error| error.to_string())?;
                        let manager_config = config
                            .manager(&manager)
                            .ok_or_else(|| format!("Manager is not configured: {manager}"))?;
                        runtime
                            .count_installed(manager_config)
                            .await
                            .map_err(|error| content::describe_manager_error(&error))
                    }
                },
                item_message: move |manager, result| Message::InitInstalledCount {
                    generation,
                    manager,
                    result,
                },
                progress_message: move |progress: InitProgress| Message::InitInstalledProgress {
                    generation,
                    completed: progress.completed,
                    total: progress.total,
                    manager: progress.manager,
                    command_message: progress.command_message,
                },
                done_message: move || Message::InitInstalledFinished { generation },
            },
        )
    }

    fn start_init_updates_counts_task(
        &mut self,
        config: updater_core::Config,
        generation: u64,
    ) -> Task<Message> {
        let managers = content::configured_managers_with_capability(
            &config,
            &self.manager_catalog,
            ManagerCapability::Updates,
        );
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
            return self.finish_init_updates_counts();
        }

        let registry = self.manager_catalog.registry();
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
                work: move |manager: ManagerId, config: updater_core::Config| {
                    let registry = registry.clone();
                    async move {
                        let runtime = registry
                            .manager_for(&manager, ManagerCapability::Updates)
                            .map_err(|error| error.to_string())?;
                        let manager_config = config
                            .manager(&manager)
                            .ok_or_else(|| format!("Manager is not configured: {manager}"))?;
                        runtime
                            .updates(manager_config, false)
                            .await
                            .map_err(|error| content::describe_manager_error(&error))
                    }
                },
                item_message: move |manager, result| Message::InitUpdatesCount {
                    generation,
                    manager,
                    result,
                },
                progress_message: move |progress: InitProgress| Message::InitUpdatesProgress {
                    generation,
                    completed: progress.completed,
                    total: progress.total,
                    manager: progress.manager,
                    command_message: progress.command_message,
                },
                done_message: move || Message::InitUpdatesFinished { generation },
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::new().0
    }

    fn manager_id(value: &str) -> ManagerId {
        ManagerId::parse(value).unwrap()
    }

    fn dirty_configuration_app(page: content::ActiveContentPage) -> App {
        let mut app = app();
        let _ = app.update_message(Message::ConfigLoaded(Ok(updater_core::Config::default())));
        let _ = app.update_message(Message::Content(content::Message::Settings(
            content::SettingsMessage::NotificationsChanged(true),
        )));
        app.content.active_content = page;
        app.sidebar.active_tab = page.into();
        app
    }

    #[test]
    fn dirty_configuration_can_move_between_settings_and_managers() {
        let mut app = dirty_configuration_app(content::ActiveContentPage::Health);

        let _ = app.navigate_to(content::ActiveContentPage::Settings);

        assert_eq!(
            app.content.active_content,
            content::ActiveContentPage::Settings
        );
        assert!(app.pending_settings_exit.is_none());
    }

    #[test]
    fn dirty_manager_configuration_prompts_before_leaving_configuration_pages() {
        let mut app = dirty_configuration_app(content::ActiveContentPage::Health);

        let _ = app.navigate_to(content::ActiveContentPage::Finding);

        assert_eq!(
            app.content.active_content,
            content::ActiveContentPage::Health
        );
        assert!(matches!(
            app.pending_settings_exit,
            Some(PendingSettingsExit::Navigate(
                content::ActiveContentPage::Finding
            ))
        ));
    }

    #[test]
    fn discarding_application_preferences_preserves_health_state() {
        let mut app = app();
        let _ = app.update_message(Message::ConfigLoaded(Ok(updater_core::Config::default())));
        let _ = app.update_message(Message::Content(content::Message::Health(
            content::HealthMessage::StartScan,
        )));
        assert!(app.manager_health.is_checking());
        let _ = app.update_message(Message::Content(content::Message::Settings(
            content::SettingsMessage::NotificationsChanged(true),
        )));
        let _ = app.update_message(Message::DiscardPendingSettings);
        assert!(app.manager_health.is_checking());
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

    #[test]
    fn config_load_failure_is_visible_and_retry_returns_to_loading() {
        let mut app = app();

        let _ = app.update_message(Message::ConfigLoaded(Err(
            updater_core::error::CoreError::ConfigError("broken document".to_owned()),
        )));

        assert!(matches!(
            &app.config_load_state,
            ConfigLoadState::Failed {
                load_error,
                recovery_error: None,
            } if load_error.contains("broken document")
        ));

        let _ = app.update_message(Message::RetryConfigLoad);

        assert_eq!(app.config_load_state, ConfigLoadState::Loading);
    }

    #[test]
    fn config_reset_requires_confirmation_and_escape_cancels() {
        let mut app = app();
        app.config_load_state = ConfigLoadState::Failed {
            load_error: "invalid JSON".to_owned(),
            recovery_error: None,
        };

        let _ = app.update_message(Message::RequestConfigReset);
        assert!(matches!(
            app.config_load_state,
            ConfigLoadState::ConfirmReset { .. }
        ));

        let _ = app.handle_shortcut(Shortcut::Dismiss);
        assert_eq!(
            app.config_load_state,
            ConfigLoadState::Failed {
                load_error: "invalid JSON".to_owned(),
                recovery_error: None,
            }
        );
        assert_eq!(app.pm_config, updater_core::Config::default());

        let _ = app.update_message(Message::RequestConfigReset);
        let _ = app.update_message(Message::ConfirmConfigReset);

        assert_eq!(
            app.config_load_state,
            ConfigLoadState::Resetting {
                load_error: "invalid JSON".to_owned(),
            }
        );
        assert_eq!(app.pm_config, updater_core::Config::default());
    }

    #[test]
    fn recovery_action_error_is_shown_until_config_load_succeeds() {
        let mut app = app();
        app.config_load_state = ConfigLoadState::Failed {
            load_error: "invalid JSON".to_owned(),
            recovery_error: None,
        };

        let _ = app.update_message(Message::ConfigDirectoryOpened(Err(
            "no desktop opener".to_owned()
        )));
        assert!(matches!(
            &app.config_load_state,
            ConfigLoadState::Failed {
                recovery_error: Some(error),
                ..
            } if error == "no desktop opener"
        ));

        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager_id(
                "builtin:cargo",
            ))],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config.clone())));

        assert_eq!(app.config_load_state, ConfigLoadState::Ready);
        assert_eq!(app.pm_config, config);
    }

    #[test]
    fn reset_failure_preserves_the_original_load_error() {
        let mut app = app();
        app.config_load_state = ConfigLoadState::Resetting {
            load_error: "invalid JSON".to_owned(),
        };

        let _ = app.update_message(Message::ConfigLoaded(Err(
            updater_core::error::CoreError::ConfigError("permission denied".to_owned()),
        )));

        assert!(matches!(
            &app.config_load_state,
            ConfigLoadState::Failed {
                load_error,
                recovery_error: Some(recovery_error),
            } if load_error == "invalid JSON" && recovery_error.contains("permission denied")
        ));
    }

    #[test]
    fn init_messages_apply_only_to_the_current_reload_generation() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        app.package_data_generation = 2;
        app.installed_info.is_loading_count = true;
        app.installed_info.init_progress = Some((0, 1));
        app.updates_info.is_loading_count = true;
        app.updates_info.init_progress = Some((0, 1));

        let _ = app.update_message(Message::InitInstalledProgress {
            generation: 1,
            completed: 1,
            total: 1,
            manager: manager.clone(),
            command_message: "stale installed progress".to_owned(),
        });
        let _ = app.update_message(Message::InitInstalledCount {
            generation: 1,
            manager: manager.clone(),
            result: Err("stale installed result".to_owned()),
        });
        let _ = app.update_message(Message::InitInstalledFinished { generation: 1 });
        let _ = app.update_message(Message::InitUpdatesProgress {
            generation: 1,
            completed: 1,
            total: 1,
            manager: manager.clone(),
            command_message: "stale updates progress".to_owned(),
        });
        let _ = app.update_message(Message::InitUpdatesCount {
            generation: 1,
            manager: manager.clone(),
            result: Err("stale updates result".to_owned()),
        });
        let _ = app.update_message(Message::InitUpdatesFinished { generation: 1 });

        assert_eq!(app.installed_info.init_progress, Some((0, 1)));
        assert!(app.installed_info.init_logs.is_empty());
        assert!(app.installed_info.init_errors.is_empty());
        assert!(app.installed_info.is_loading_count);
        assert_eq!(app.updates_info.init_progress, Some((0, 1)));
        assert!(app.updates_info.init_logs.is_empty());
        assert!(app.updates_info.init_errors.is_empty());
        assert!(app.updates_info.is_loading_count);

        let _ = app.update_message(Message::InitInstalledCount {
            generation: 2,
            manager: manager.clone(),
            result: Err("current installed result".to_owned()),
        });
        let _ = app.update_message(Message::InitUpdatesCount {
            generation: 2,
            manager: manager.clone(),
            result: Err("current updates result".to_owned()),
        });
        let _ = app.update_message(Message::InitInstalledFinished { generation: 2 });
        let _ = app.update_message(Message::InitUpdatesFinished { generation: 2 });

        assert_eq!(
            app.installed_info
                .init_errors
                .get(&manager)
                .map(String::as_str),
            Some("current installed result")
        );
        assert_eq!(
            app.updates_info
                .init_errors
                .get(&manager)
                .map(String::as_str),
            Some("current updates result")
        );
        assert!(!app.installed_info.is_loading_count);
        assert!(!app.updates_info.is_loading_count);
    }

    #[test]
    fn successful_init_updates_count_stamps_checked_at() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        app.package_data_generation = 2;
        app.updates_info.is_loading_count = true;

        let _ = app.update_message(Message::InitUpdatesCount {
            generation: 2,
            manager: manager.clone(),
            result: Ok(Vec::new()),
        });

        let checked_at = app
            .updates_info
            .checked_at
            .get(&manager)
            .expect("a successful check records its time");
        assert!(chrono::DateTime::parse_from_rfc3339(checked_at).is_ok());
        assert_eq!(app.updates_info.current_update_count(), 0);
    }

    #[test]
    fn startup_defers_package_scans_until_their_page_is_opened() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager.clone())],
            ..updater_core::Config::default()
        };

        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));

        assert_eq!(app.package_data_generation, 1);
        assert!(!app.installed_info.has_loading_count);
        assert!(!app.installed_info.is_loading_count);
        assert!(!app.updates_info.has_loading_count);
        assert!(!app.updates_info.is_loading_count);
        assert!(app.updates_info.selected_managers.contains(&manager));

        let _ = app.activate_page(content::ActiveContentPage::Updates);

        assert!(app.updates_info.is_loading_count);
        assert_eq!(app.updates_info.init_progress, Some((0, 1)));
        assert!(!app.installed_info.is_loading_count);
    }

    #[test]
    fn scheduled_update_check_is_ignored_while_a_scan_is_in_flight() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager)],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));

        // A count scan already running must not be restarted or have its
        // progress reset by a background tick, whether the page or an earlier
        // scheduled check started it.
        for in_flight in ["page", "schedule"] {
            app.updates_info.background_loading_count = in_flight == "schedule";
            app.updates_info.is_loading_count = in_flight == "page";
            app.updates_info.init_progress = Some((1, 3));

            let _ = app.update_message(Message::ScheduledUpdateCheck);

            assert!(
                app.updates_info.is_loading_count || app.updates_info.background_loading_count,
                "{in_flight} scan was cleared"
            );
            assert_eq!(app.updates_info.init_progress, Some((1, 3)));
        }

        // Once the scan drained, the same message starts a fresh count.
        app.updates_info.is_loading_count = false;
        app.updates_info.background_loading_count = false;
        app.updates_info.init_progress = None;

        let _ = app.update_message(Message::ScheduledUpdateCheck);

        assert!(app.updates_info.background_loading_count);
        assert!(!app.updates_info.is_loading_count);
        assert_eq!(app.updates_info.init_progress, Some((0, 1)));
    }

    #[test]
    fn scheduled_update_check_does_not_start_during_a_package_operation() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager)],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));
        app.updates_info.is_updating = true;

        let _ = app.update_message(Message::ScheduledUpdateCheck);

        assert!(!app.updates_info.is_loading_count);
        assert!(!app.updates_info.background_loading_count);
        assert!(app.updates_info.init_progress.is_none());
    }

    #[test]
    fn scheduled_update_check_leaves_the_status_panel_closed() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager)],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));

        let _ = app.update_message(Message::ScheduledUpdateCheck);

        // The scan runs, but as background work: no panel, no progress bar.
        assert!(app.updates_info.background_loading_count);
        assert!(!app.updates_info.is_loading_count);
        assert!(!app.status_panel.is_visible());
        // The first-ever scan still shows the badge's loading glyph.
        assert!(app.sidebar_summary().updates_loading);

        // Opening the Updates page adopts that scan rather than starting a
        // second one, and only then does it become user-visible work.
        let _ = app.activate_page(content::ActiveContentPage::Updates);

        assert!(app.updates_info.is_loading_count);
        assert!(app.updates_info.background_loading_count);
        assert_eq!(app.updates_info.init_progress, Some((0, 1)));
    }

    #[test]
    fn a_known_update_count_survives_a_background_rescan() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager.clone())],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));
        let generation = app.package_data_generation;

        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager: manager.clone(),
            result: Ok(vec![PackageUpdate::new(
                updater_manager_api::PackageTarget::new(manager.clone(), "firefox"),
                "1.0",
                "2.0",
            )]),
        });
        let _ = app.update_message(Message::InitUpdatesFinished { generation });

        assert!(app.updates_info.has_loading_count);
        let settled = app.sidebar_summary();
        assert_eq!(settled.update_count, 1);
        assert!(!settled.updates_loading);

        // A later scheduled check must not trade the visible count for the
        // loading glyph.
        let _ = app.update_message(Message::ScheduledUpdateCheck);

        assert!(app.updates_info.background_loading_count);
        let rescanning = app.sidebar_summary();
        assert_eq!(rescanning.update_count, 1);
        assert!(
            !rescanning.updates_loading,
            "a background rescan hid the known count"
        );
    }

    #[test]
    fn a_source_reloaded_during_a_background_check_keeps_its_fresh_result() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager.clone())],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));
        let generation = app.package_data_generation;
        let _ = app.update_message(Message::ScheduledUpdateCheck);
        assert!(app.updates_info.background_loading_count);

        // The user reloads the source while the scheduled check is running.
        app.updates_info.loading_updates.insert(manager.clone(), 1);
        let _ = app.update_message(Message::Content(content::Message::Settings(
            content::SettingsMessage::NotificationsChanged(true),
        )));

        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager: manager.clone(),
            result: Ok(vec![PackageUpdate::new(
                updater_manager_api::PackageTarget::new(manager.clone(), "stale"),
                "1.0",
                "2.0",
            )]),
        });

        assert!(
            !app.updates_info.updates_by_manager.contains_key(&manager),
            "the background check's older read replaced the user's reload"
        );
    }

    #[test]
    fn configuration_reload_clears_an_in_flight_background_check() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager)],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config.clone())));
        let _ = app.update_message(Message::ScheduledUpdateCheck);
        assert!(app.updates_info.background_loading_count);

        // The reload bumps the generation, so the old scan's finish message is
        // ignored. A stuck flag would block every later scheduled check.
        let _ = app.reload_package_data(content::PackageDataReload::Startup);

        assert!(!app.updates_info.background_loading_count);
        let _ = app.update_message(Message::ScheduledUpdateCheck);
        assert!(app.updates_info.background_loading_count);
    }

    #[test]
    fn background_check_flag_clears_when_its_scan_finishes() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager.clone())],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));
        let generation = app.package_data_generation;
        let _ = app.update_message(Message::ScheduledUpdateCheck);
        assert!(app.updates_info.background_loading_count);

        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager,
            result: Ok(Vec::new()),
        });
        let _ = app.update_message(Message::InitUpdatesFinished { generation });

        assert!(!app.updates_info.background_loading_count);
        assert!(!app.updates_info.is_loading_count);
        assert!(app.updates_info.has_loading_count);
        assert!(!app.sidebar_summary().updates_loading);
        // The guard released, so the next tick may scan again.
        let _ = app.update_message(Message::ScheduledUpdateCheck);
        assert!(app.updates_info.background_loading_count);
    }

    #[test]
    fn a_user_started_update_load_keeps_the_badge_loading_until_it_finishes() {
        let mut app = app();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        let config = updater_core::Config {
            managers: vec![
                updater_core::ManagerConfig::new(cargo.clone()),
                updater_core::ManagerConfig::new(npm),
            ],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));
        let generation = app.package_data_generation;
        let _ = app.activate_page(content::ActiveContentPage::Updates);
        assert!(app.sidebar_summary().updates_loading);

        // One source reporting does not end a load the user asked for.
        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager: cargo,
            result: Ok(Vec::new()),
        });
        assert!(app.updates_info.has_loading_count);
        assert!(app.sidebar_summary().updates_loading);

        let _ = app.update_message(Message::InitUpdatesFinished { generation });
        assert!(!app.sidebar_summary().updates_loading);
    }

    #[test]
    fn discovered_update_count_reaches_the_sidebar_badge() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager.clone())],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));

        assert_eq!(app.sidebar_summary().update_count, 0);

        let _ = app.update_message(Message::InitUpdatesCount {
            generation: app.package_data_generation,
            manager,
            result: Ok(vec![PackageUpdate::new(
                updater_manager_api::PackageTarget::new(manager_id("builtin:cargo"), "firefox"),
                "1.0",
                "2.0",
            )]),
        });

        assert_eq!(app.sidebar_summary().update_count, 1);
    }

    #[test]
    fn a_failed_count_scan_drops_out_of_the_sidebar_badge() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager.clone())],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));
        let generation = app.package_data_generation;

        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager: manager.clone(),
            result: Ok(vec![PackageUpdate::new(
                updater_manager_api::PackageTarget::new(manager.clone(), "firefox"),
                "1.0",
                "2.0",
            )]),
        });
        assert_eq!(app.sidebar_summary().update_count, 1);

        // The cache keeps the previous count so the source can show it as last
        // known, but the badge must not present it as a current total.
        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager,
            result: Err("package manager command failed".to_owned()),
        });

        assert_eq!(app.updates_info.current_update_count(), 0);
        assert_eq!(app.sidebar_summary().update_count, 0);
    }

    #[test]
    fn update_discovery_notification_is_gated_on_the_preference() {
        let mut app = app();
        let manager = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(manager.clone())],
            ..updater_core::Config::default()
        };
        let _ = app.update_message(Message::ConfigLoaded(Ok(config)));
        let generation = app.package_data_generation;

        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager: manager.clone(),
            result: Ok(Vec::new()),
        });
        let _ = app.update_message(Message::InitUpdatesFinished { generation });
        assert_eq!(app.notified_update_count, None);

        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager: manager.clone(),
            result: Ok(vec![PackageUpdate::new(
                updater_manager_api::PackageTarget::new(manager.clone(), "firefox"),
                "1.0",
                "2.0",
            )]),
        });
        app.updates_info.is_loading_count = true;
        let _ = app.update_message(Message::InitUpdatesFinished { generation });

        // Notifications are disabled by default, so nothing was announced.
        assert_eq!(app.notified_update_count, None);

        app.pm_config.notifications_enabled = true;
        app.updates_info.is_loading_count = true;
        let _ = app.update_message(Message::InitUpdatesFinished { generation });

        assert_eq!(app.notified_update_count, Some(1));

        // A scan that finds a few more updates than the announced set is not a
        // fresh discovery, so it stays silent too.
        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager: manager.clone(),
            result: Ok(vec![
                PackageUpdate::new(
                    updater_manager_api::PackageTarget::new(manager.clone(), "firefox"),
                    "1.0",
                    "2.0",
                ),
                PackageUpdate::new(
                    updater_manager_api::PackageTarget::new(manager.clone(), "vim"),
                    "8.0",
                    "9.0",
                ),
            ]),
        });
        app.updates_info.is_loading_count = true;
        let _ = app.update_message(Message::InitUpdatesFinished { generation });

        assert_eq!(app.notified_update_count, Some(1));

        // Draining back to zero re-arms the next discovery.
        let _ = app.update_message(Message::InitUpdatesCount {
            generation,
            manager,
            result: Ok(Vec::new()),
        });
        app.updates_info.is_loading_count = true;
        let _ = app.update_message(Message::InitUpdatesFinished { generation });

        assert_eq!(app.notified_update_count, None);
    }

    #[test]
    fn blur_keeps_transients_that_dismiss_clears() {
        let mut app = app();
        let _ = app.update_message(Message::ConfigLoaded(Ok(updater_core::Config::default())));
        app.activity_center_open = true;
        app.status_panel
            .record_outcome(updater_core::OperationOutcome {
                action: updater_manager_api::PackageAction::Install,
                completed_packages: 1,
                total_packages: 1,
                completed_managers: 1,
                total_managers: 1,
                failed_manager: None,
                error: None,
                cancelled: false,
                manager_outcomes: Vec::new(),
                scope: updater_manager_api::PackageScope::User,
            });

        // Esc captured by the focused search box only blurs it: the inspected
        // row and the completed-operation surface must survive.
        let _ = app.handle_shortcut(Shortcut::BlurInput);

        assert!(
            app.activity_center_open,
            "blurring the search box must not dismiss the page behind it"
        );
        assert!(
            app.status_panel.is_visible(),
            "blurring the search box must not drop the operation summary"
        );

        // Esc anywhere else still dismisses, innermost surface first. The
        // dismiss chain short-circuits, so each surface needs its own press.
        let _ = app.handle_shortcut(Shortcut::Dismiss);

        assert!(
            !app.status_panel.is_visible(),
            "dismissing outside the search box still clears the operation summary"
        );

        let _ = app.handle_shortcut(Shortcut::Dismiss);

        assert!(!app.activity_center_open);
    }

    #[test]
    fn blur_keeps_the_inspected_row_that_dismiss_clears() {
        let mut app = app();
        let _ = app.update_message(Message::ConfigLoaded(Ok(updater_core::Config::default())));
        app.content.active_content = content::ActiveContentPage::Finding;
        let _ = app.update_message(Message::Content(content::Message::Finding(
            content::FindingMessage::InspectPackage(
                manager_id("builtin:cargo"),
                "alpha".to_owned(),
            ),
        )));
        assert!(app.content.finding.has_inspector_selection());

        // Esc captured by the focused search box only blurs it.
        let _ = app.handle_shortcut(Shortcut::BlurInput);

        assert!(
            app.content.finding.has_inspector_selection(),
            "blurring the search box must keep the inspected row"
        );

        // Esc anywhere else still dismisses the inspected row.
        let _ = app.handle_shortcut(Shortcut::Dismiss);

        assert!(!app.content.finding.has_inspector_selection());
    }

    #[test]
    fn footer_hints_document_the_keyboard_controls() {
        for page in [
            content::ActiveContentPage::Finding,
            content::ActiveContentPage::Updates,
            content::ActiveContentPage::Installed,
            content::ActiveContentPage::Health,
            content::ActiveContentPage::Settings,
        ] {
            let hint = footer_shortcuts(page);
            assert!(
                hint.contains("Esc Close"),
                "{page:?} footer omits Esc: {hint}"
            );
            assert!(
                hint.contains("Alt+1–5 Pages"),
                "{page:?} footer omits the page-switch shortcut: {hint}"
            );
        }

        for page in [
            content::ActiveContentPage::Finding,
            content::ActiveContentPage::Updates,
            content::ActiveContentPage::Installed,
        ] {
            let hint = footer_shortcuts(page);
            assert!(hint.contains("↑↓ Move"), "{page:?} footer omits movement");
            assert!(
                hint.contains("Space Select"),
                "{page:?} footer omits selection"
            );
        }
    }

    #[test]
    fn partial_operation_refresh_only_reloads_successful_managers() {
        let mut app = app();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        app.pm_config = updater_core::Config {
            managers: vec![
                updater_core::ManagerConfig::new(cargo.clone()),
                updater_core::ManagerConfig::new(npm.clone()),
            ],
            ..updater_core::Config::default()
        };
        app.installed_info.has_loading_count = true;
        app.updates_info.has_loading_count = true;
        app.installed_info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        app.updates_info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        app.finding_info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        app.installed_info
            .installed_packages
            .insert(cargo.clone(), (1, Vec::new()));
        app.installed_info
            .installed_packages
            .insert(npm.clone(), (2, Vec::new()));
        app.updates_info
            .updates_by_manager
            .insert(cargo.clone(), (1, Vec::new()));
        app.updates_info
            .updates_by_manager
            .insert(npm.clone(), (2, Vec::new()));
        app.finding_info
            .search_results
            .insert(cargo.clone(), Vec::new());
        app.finding_info
            .search_results
            .insert(npm.clone(), Vec::new());
        app.installed_info
            .selected_packages
            .insert((cargo.clone(), "cargo-edit".to_owned()));
        app.installed_info
            .selected_packages
            .insert((npm.clone(), "typescript".to_owned()));

        let outcome = updater_core::OperationOutcome {
            action: updater_manager_api::PackageAction::Update,
            completed_packages: 1,
            total_packages: 2,
            completed_managers: 1,
            total_managers: 2,
            failed_manager: Some(npm.clone()),
            error: Some("npm update failed".to_owned()),
            cancelled: false,
            manager_outcomes: vec![
                updater_core::ManagerOperationOutcome {
                    manager_id: cargo.clone(),
                    scope: updater_manager_api::PackageScope::User,
                    requested_packages: 1,
                    completed_packages: 1,
                    status: updater_core::ManagerOperationStatus::Succeeded,
                    error: None,
                },
                updater_core::ManagerOperationOutcome {
                    manager_id: npm.clone(),
                    scope: updater_manager_api::PackageScope::User,
                    requested_packages: 1,
                    completed_packages: 0,
                    status: updater_core::ManagerOperationStatus::Failed,
                    error: Some("npm update failed".to_owned()),
                },
            ],
            scope: updater_manager_api::PackageScope::User,
        };

        let _ = app.refresh_package_managers(&outcome);

        assert_eq!(
            app.installed_info
                .loading_installed
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            HashSet::from([cargo.clone()])
        );
        assert_eq!(
            app.updates_info
                .loading_updates
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            HashSet::from([cargo.clone()])
        );
        assert!(app.installed_info.installed_packages.contains_key(&npm));
        assert!(app.updates_info.updates_by_manager.contains_key(&npm));
        assert!(!app.finding_info.search_results.contains_key(&cargo));
        assert!(app.finding_info.search_results.contains_key(&npm));
        assert!(
            !app.installed_info
                .selected_packages
                .contains(&(cargo, "cargo-edit".to_owned()))
        );
        assert!(
            app.installed_info
                .selected_packages
                .contains(&(npm, "typescript".to_owned()))
        );
    }

    #[test]
    fn post_operation_updates_reload_does_not_force_a_privileged_refresh() {
        let mut app = app();
        let cargo = manager_id("builtin:cargo");
        app.pm_config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(cargo.clone())],
            ..updater_core::Config::default()
        };
        app.installed_info.has_loading_count = true;
        app.updates_info.has_loading_count = true;
        app.updates_info
            .updates_by_manager
            .insert(cargo.clone(), (1, Vec::new()));
        let outcome = updater_core::OperationOutcome {
            action: updater_manager_api::PackageAction::Update,
            completed_packages: 1,
            total_packages: 1,
            completed_managers: 1,
            total_managers: 1,
            failed_manager: None,
            error: None,
            cancelled: false,
            manager_outcomes: vec![updater_core::ManagerOperationOutcome {
                manager_id: cargo.clone(),
                scope: updater_manager_api::PackageScope::User,
                requested_packages: 1,
                completed_packages: 1,
                status: updater_core::ManagerOperationStatus::Succeeded,
                error: None,
            }],
            scope: updater_manager_api::PackageScope::User,
        };

        let _ = app.refresh_package_managers(&outcome);

        assert_eq!(
            app.updates_info.refresh_modes.get(&cargo),
            Some(&content::RefreshMode::Local),
            "local state is authoritative after a write, so no second metadata sync"
        );
        assert!(
            app.updates_info
                .refresh_modes
                .values()
                .all(|mode| *mode == content::RefreshMode::Local)
        );
    }

    #[test]
    fn successful_operation_refreshes_each_manager_once() {
        let mut app = app();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        app.pm_config = updater_core::Config {
            managers: vec![
                updater_core::ManagerConfig::new(cargo.clone()),
                updater_core::ManagerConfig::new(npm.clone()),
            ],
            ..updater_core::Config::default()
        };
        app.installed_info.has_loading_count = true;
        app.updates_info.has_loading_count = true;
        let succeeded = |manager_id: ManagerId| updater_core::ManagerOperationOutcome {
            manager_id,
            scope: updater_manager_api::PackageScope::User,
            requested_packages: 1,
            completed_packages: 1,
            status: updater_core::ManagerOperationStatus::Succeeded,
            error: None,
        };
        let outcome = updater_core::OperationOutcome {
            action: updater_manager_api::PackageAction::Update,
            completed_packages: 3,
            total_packages: 3,
            completed_managers: 3,
            total_managers: 3,
            failed_manager: None,
            error: None,
            cancelled: false,
            manager_outcomes: vec![
                succeeded(cargo.clone()),
                succeeded(npm.clone()),
                succeeded(cargo.clone()),
            ],
            scope: updater_manager_api::PackageScope::User,
        };

        let _ = app.refresh_package_managers(&outcome);

        assert_eq!(app.installed_info.request_generation, 2);
        assert_eq!(app.updates_info.request_generation, 2);
        assert_eq!(
            app.installed_info
                .loading_installed
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            HashSet::from([cargo.clone(), npm.clone()])
        );
        assert_eq!(
            app.updates_info
                .loading_updates
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            HashSet::from([cargo, npm])
        );
    }

    #[test]
    fn operation_without_success_does_not_refresh_failed_or_unstarted_managers() {
        let mut app = app();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        app.pm_config = updater_core::Config {
            managers: vec![
                updater_core::ManagerConfig::new(cargo.clone()),
                updater_core::ManagerConfig::new(npm.clone()),
            ],
            ..updater_core::Config::default()
        };
        app.installed_info.has_loading_count = true;
        app.updates_info.has_loading_count = true;
        app.installed_info
            .installed_packages
            .insert(cargo.clone(), (1, Vec::new()));
        app.installed_info
            .installed_packages
            .insert(npm.clone(), (2, Vec::new()));
        app.updates_info
            .updates_by_manager
            .insert(cargo.clone(), (1, Vec::new()));
        app.updates_info
            .updates_by_manager
            .insert(npm.clone(), (2, Vec::new()));
        app.finding_info
            .search_results
            .insert(cargo.clone(), Vec::new());
        app.finding_info
            .search_results
            .insert(npm.clone(), Vec::new());
        let outcome = updater_core::OperationOutcome {
            action: updater_manager_api::PackageAction::Update,
            completed_packages: 0,
            total_packages: 2,
            completed_managers: 0,
            total_managers: 2,
            failed_manager: Some(cargo.clone()),
            error: Some("cargo update failed".to_owned()),
            cancelled: false,
            manager_outcomes: vec![
                updater_core::ManagerOperationOutcome {
                    manager_id: cargo.clone(),
                    scope: updater_manager_api::PackageScope::User,
                    requested_packages: 1,
                    completed_packages: 0,
                    status: updater_core::ManagerOperationStatus::Failed,
                    error: Some("cargo update failed".to_owned()),
                },
                updater_core::ManagerOperationOutcome {
                    manager_id: npm.clone(),
                    scope: updater_manager_api::PackageScope::User,
                    requested_packages: 1,
                    completed_packages: 0,
                    status: updater_core::ManagerOperationStatus::NotStarted,
                    error: None,
                },
            ],
            scope: updater_manager_api::PackageScope::User,
        };

        let _ = app.refresh_package_managers(&outcome);

        assert_eq!(app.installed_info.request_generation, 0);
        assert_eq!(app.updates_info.request_generation, 0);
        assert!(app.installed_info.loading_installed.is_empty());
        assert!(app.updates_info.loading_updates.is_empty());
        assert!(app.installed_info.installed_packages.contains_key(&cargo));
        assert!(app.installed_info.installed_packages.contains_key(&npm));
        assert!(app.updates_info.updates_by_manager.contains_key(&cargo));
        assert!(app.updates_info.updates_by_manager.contains_key(&npm));
        assert!(app.finding_info.search_results.contains_key(&cargo));
        assert!(app.finding_info.search_results.contains_key(&npm));
    }

    #[test]
    fn configuration_reload_only_invalidates_changed_manager_state() {
        let mut app = app();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        let previous = updater_core::Config {
            managers: vec![
                updater_core::ManagerConfig::new(cargo.clone()),
                updater_core::ManagerConfig::new(npm.clone()),
            ],
            ..updater_core::Config::default()
        };
        app.pm_config = updater_core::Config {
            managers: vec![
                updater_core::ManagerConfig::new(cargo.clone()).with_executable("/opt/cargo"),
                updater_core::ManagerConfig::new(npm.clone()),
            ],
            ..updater_core::Config::default()
        };
        app.package_data_generation = 7;
        app.installed_info.has_loading_count = true;
        app.installed_info.request_generation = 42;
        app.updates_info.has_loading_count = true;
        app.updates_info.request_generation = 52;
        app.installed_info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        app.updates_info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        app.finding_info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        app.installed_info
            .installed_packages
            .insert(cargo.clone(), (1, Vec::new()));
        app.installed_info
            .loading_installed
            .insert(cargo.clone(), 41);
        app.installed_info
            .installed_packages
            .insert(npm.clone(), (2, Vec::new()));
        app.installed_info.loading_installed.insert(npm.clone(), 42);
        app.installed_info
            .load_errors
            .insert(cargo.clone(), "stale cargo error".to_owned());
        app.installed_info
            .load_errors
            .insert(npm.clone(), "preserved npm error".to_owned());
        app.updates_info
            .updates_by_manager
            .insert(cargo.clone(), (1, Vec::new()));
        app.updates_info
            .updates_by_manager
            .insert(npm.clone(), (2, Vec::new()));
        app.updates_info.loading_updates.insert(cargo.clone(), 51);
        app.updates_info.loading_updates.insert(npm.clone(), 52);
        app.finding_info
            .search_results
            .insert(cargo.clone(), Vec::new());
        app.finding_info
            .search_results
            .insert(npm.clone(), Vec::new());
        app.finding_info
            .searching_managers
            .insert(cargo.clone(), 61);
        app.finding_info.searching_managers.insert(npm.clone(), 62);
        app.installed_info
            .selected_packages
            .insert((cargo.clone(), "cargo-edit".to_owned()));
        app.installed_info
            .selected_packages
            .insert((npm.clone(), "typescript".to_owned()));

        let changes = content::ManagerConfigChanges::between(&previous, &app.pm_config);
        let _ = app.reload_package_data(content::PackageDataReload::ManagersChanged(changes));

        assert_eq!(app.package_data_generation, 7);
        assert!(!app.installed_info.installed_packages.contains_key(&cargo));
        assert!(app.installed_info.installed_packages.contains_key(&npm));
        assert_eq!(app.installed_info.loading_installed.get(&cargo), Some(&43));
        assert_eq!(app.installed_info.loading_installed.get(&npm), Some(&42));
        assert!(!app.installed_info.load_errors.contains_key(&cargo));
        assert_eq!(
            app.installed_info.load_errors.get(&npm).map(String::as_str),
            Some("preserved npm error")
        );
        assert!(!app.updates_info.updates_by_manager.contains_key(&cargo));
        assert!(app.updates_info.updates_by_manager.contains_key(&npm));
        assert_eq!(app.updates_info.loading_updates.get(&cargo), Some(&53));
        assert_eq!(app.updates_info.loading_updates.get(&npm), Some(&52));
        assert!(!app.finding_info.search_results.contains_key(&cargo));
        assert!(app.finding_info.search_results.contains_key(&npm));
        assert!(!app.finding_info.searching_managers.contains_key(&cargo));
        assert_eq!(app.finding_info.searching_managers.get(&npm), Some(&62));
        assert!(app.installed_info.selected_managers.contains(&cargo));
        assert!(app.installed_info.selected_managers.contains(&npm));
        assert!(
            !app.installed_info
                .selected_packages
                .contains(&(cargo, "cargo-edit".to_owned()))
        );
        assert!(
            app.installed_info
                .selected_packages
                .contains(&(npm, "typescript".to_owned()))
        );
    }

    #[test]
    fn configuration_reload_prunes_removed_manager_without_starting_unopened_pages() {
        let mut app = app();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        let previous = updater_core::Config {
            managers: vec![
                updater_core::ManagerConfig::new(cargo.clone()),
                updater_core::ManagerConfig::new(npm.clone()),
            ],
            ..updater_core::Config::default()
        };
        app.pm_config = updater_core::Config {
            managers: vec![updater_core::ManagerConfig::new(cargo.clone())],
            ..updater_core::Config::default()
        };
        app.package_data_generation = 3;
        app.installed_info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        app.updates_info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        app.finding_info.selected_managers = HashSet::from([cargo.clone(), npm.clone()]);
        app.installed_info
            .installed_packages
            .insert(cargo.clone(), (1, Vec::new()));
        app.installed_info
            .installed_packages
            .insert(npm.clone(), (2, Vec::new()));
        app.updates_info
            .updates_by_manager
            .insert(cargo.clone(), (1, Vec::new()));
        app.updates_info
            .updates_by_manager
            .insert(npm.clone(), (2, Vec::new()));

        let changes = content::ManagerConfigChanges::between(&previous, &app.pm_config);
        let _ = app.reload_package_data(content::PackageDataReload::ManagersChanged(changes));

        assert_eq!(app.package_data_generation, 3);
        assert_eq!(app.installed_info.request_generation, 0);
        assert_eq!(app.updates_info.request_generation, 0);
        assert!(app.installed_info.installed_packages.contains_key(&cargo));
        assert!(!app.installed_info.installed_packages.contains_key(&npm));
        assert!(app.updates_info.updates_by_manager.contains_key(&cargo));
        assert!(!app.updates_info.updates_by_manager.contains_key(&npm));
        assert_eq!(
            app.installed_info.selected_managers,
            HashSet::from([cargo.clone()])
        );
        assert_eq!(
            app.updates_info.selected_managers,
            HashSet::from([cargo.clone()])
        );
        assert_eq!(app.finding_info.selected_managers, HashSet::from([cargo]));
    }

    #[test]
    fn configuration_reload_ignores_only_changed_manager_results_during_initialization() {
        let mut app = app();
        let cargo = manager_id("builtin:cargo");
        let npm = manager_id("builtin:npm");
        let previous = updater_core::Config {
            managers: vec![
                updater_core::ManagerConfig::new(cargo.clone()),
                updater_core::ManagerConfig::new(npm.clone()),
            ],
            ..updater_core::Config::default()
        };
        app.pm_config = updater_core::Config {
            managers: vec![
                updater_core::ManagerConfig::new(cargo.clone()).with_executable("/opt/cargo"),
                updater_core::ManagerConfig::new(npm.clone()),
            ],
            ..updater_core::Config::default()
        };
        app.package_data_generation = 9;
        app.installed_info.is_loading_count = true;
        app.updates_info.is_loading_count = true;

        let changes = content::ManagerConfigChanges::between(&previous, &app.pm_config);
        let _ = app.reload_package_data(content::PackageDataReload::ManagersChanged(changes));

        let _ = app.update_message(Message::InitInstalledCount {
            generation: 9,
            manager: cargo.clone(),
            result: Err("stale cargo count".to_owned()),
        });
        let _ = app.update_message(Message::InitInstalledCount {
            generation: 9,
            manager: npm.clone(),
            result: Ok(4),
        });
        let _ = app.update_message(Message::InitUpdatesCount {
            generation: 9,
            manager: cargo.clone(),
            result: Err("stale cargo updates".to_owned()),
        });
        let _ = app.update_message(Message::InitUpdatesCount {
            generation: 9,
            manager: npm.clone(),
            result: Ok(Vec::new()),
        });

        assert!(!app.installed_info.init_errors.contains_key(&cargo));
        assert_eq!(
            app.installed_info
                .installed_packages
                .get(&npm)
                .map(|(count, _)| *count),
            Some(4)
        );
        assert!(!app.updates_info.init_errors.contains_key(&cargo));
        assert!(app.updates_info.updates_by_manager.contains_key(&npm));
        assert!(app.installed_refresh_overrides.contains(&cargo));
        assert!(app.updates_refresh_overrides.contains(&cargo));
        assert!(!app.installed_refresh_overrides.contains(&npm));
        assert!(!app.updates_refresh_overrides.contains(&npm));
    }
}
