mod errors;
mod finding;
mod health;
mod installed;
mod setting;
mod shared;
mod updates;
mod workflows;

use crate::{
    content::{finding::Finding, health::HealthCenter, setting::Settings, updates::Updates},
    shortcut::SelectionDirection,
};

pub use finding::FindingInfo;
pub use health::ManagerHealthInfo;
pub(crate) use health::Message as HealthMessage;
pub(crate) use installed::Installed;
pub use installed::InstalledInfo;
pub use setting::Message as SettingsMessage;
pub(crate) use shared::{configured_managers_with_capability, open_directory, search_input_id};
pub(crate) use updater_core::CancellationToken;
pub use updater_core::OperationOutcome;
pub use updates::UpdatesInfo;

pub struct ViewOptions {
    pub show_inspector: bool,
    pub inspector_drawer: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActiveContentPage {
    /// Search/install page.
    #[default]
    Finding,
    /// Available updates page.
    Updates,
    /// Installed packages page.
    Installed,
    /// Package-manager health page.
    Health,
    /// Settings page.
    Settings,
}

#[derive(Debug, Clone, Default)]
pub struct Content {
    /// Currently visible content page.
    pub active_content: ActiveContentPage,
    /// Settings page state.
    pub settings: Settings,
    /// Installed page state.
    pub installed: Installed,
    /// Updates page state.
    pub updates: Updates,
    /// Finding page state.
    pub finding: Finding,
    /// Package-manager health page state.
    pub health: HealthCenter,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Settings page message.
    Settings(setting::Message),
    /// Installed page message.
    Installed(installed::Message),
    /// Updates page message.
    Updates(updates::Message),
    /// Finding page message.
    Finding(finding::Message),
    /// Package-manager health page message.
    Health(health::Message),
}

pub enum Action {
    /// No-op action.
    None,
    /// Asynchronous task action.
    Run(iced::Task<Message>),
    /// Cooperative package-operation task.
    CancellableRun(iced::Task<Message>, updater_core::CancellationToken),
    /// Reload package data and run an optional page follow-up task.
    ReloadPackageData {
        /// Why package data needs to be reloaded.
        reason: ReloadReason,
        /// Optional task to run after the reload starts.
        follow_up: iced::Task<Message>,
    },
    /// Record a completed package operation and optionally reload package data.
    PackageOperationFinished {
        outcome: OperationOutcome,
        reload: bool,
        follow_up: iced::Task<Message>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadReason {
    /// Initial package-data load after configuration startup.
    Startup,
    /// Configured package managers changed.
    ConfigurationChanged,
    /// A package install, update, or removal completed.
    PackageOperation,
}

impl ReloadReason {
    pub fn preserves_page_context(self) -> bool {
        !matches!(self, Self::Startup)
    }
}

impl Content {
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        message: Message,
        pm_config: &mut updater_core::Config,
        installed_info: &mut InstalledInfo,
        updates_info: &mut UpdatesInfo,
        finding_info: &mut FindingInfo,
        health_info: &mut ManagerHealthInfo,
        catalog: &crate::manager_catalog::ManagerCatalog,
    ) -> Action {
        let pm_config_ref: &updater_core::Config = pm_config;

        match message {
            Message::Settings(settings_msg) => {
                let action = self.settings.update(settings_msg, pm_config, catalog);
                match action {
                    setting::Action::Run(task) => Action::Run(task.map(Message::Settings)),
                    setting::Action::ApplySavedConfig(config) => {
                        let manager_config_changed = pm_config.managers != config.managers;
                        *pm_config = config;
                        if manager_config_changed {
                            Action::ReloadPackageData {
                                reason: ReloadReason::ConfigurationChanged,
                                follow_up: iced::Task::none(),
                            }
                        } else {
                            Action::None
                        }
                    }
                    setting::Action::ManagerConfigChanged => {
                        health_info.invalidate();
                        Action::None
                    }
                    setting::Action::None => Action::None,
                }
            }
            Message::Installed(installed_msg) => {
                let action =
                    self.installed
                        .update(installed_msg, pm_config_ref, installed_info, catalog);
                match action {
                    installed::Action::Run(task) => Action::Run(task.map(Message::Installed)),
                    installed::Action::CancellableRun(task, cancellation) => {
                        Action::CancellableRun(task.map(Message::Installed), cancellation)
                    }
                    installed::Action::None => Action::None,
                    installed::Action::PackageOperationFinished { outcome, reload } => {
                        Action::PackageOperationFinished {
                            outcome,
                            reload,
                            follow_up: iced::Task::none(),
                        }
                    }
                }
            }
            Message::Updates(updates_msg) => {
                let action = self
                    .updates
                    .update(updates_msg, pm_config_ref, updates_info, catalog);
                match action {
                    updates::Action::Run(task) => Action::Run(task.map(Message::Updates)),
                    updates::Action::CancellableRun(task, cancellation) => {
                        Action::CancellableRun(task.map(Message::Updates), cancellation)
                    }
                    updates::Action::PackageOperationFinished { outcome, reload } => {
                        Action::PackageOperationFinished {
                            outcome,
                            reload,
                            follow_up: iced::Task::none(),
                        }
                    }
                    updates::Action::None => Action::None,
                }
            }
            Message::Finding(finding_msg) => {
                let action = self
                    .finding
                    .update(finding_msg, pm_config_ref, finding_info, catalog);
                match action {
                    finding::Action::Run(task) => Action::Run(task.map(Message::Finding)),
                    finding::Action::CancellableRun(task, cancellation) => {
                        Action::CancellableRun(task.map(Message::Finding), cancellation)
                    }
                    finding::Action::PackageOperationFinished {
                        outcome,
                        reload,
                        follow_up,
                    } => Action::PackageOperationFinished {
                        outcome,
                        reload,
                        follow_up: follow_up.map(Message::Finding),
                    },
                    finding::Action::None => Action::None,
                }
            }
            Message::Health(health_msg) => {
                let draft_config = self.settings.draft_config();
                let action = self.health.update(
                    health_msg,
                    draft_config,
                    health_info,
                    catalog,
                    installed_info,
                    updates_info,
                );
                match action {
                    health::Action::Run(task) => Action::Run(task.map(Message::Health)),
                    health::Action::None => Action::None,
                }
            }
        }
    }

    pub fn dismiss_active_transient(&mut self, installed_info: &mut InstalledInfo) -> bool {
        match self.active_content {
            ActiveContentPage::Finding => self.finding.dismiss_transient(),
            ActiveContentPage::Updates => self.updates.dismiss_transient(),
            ActiveContentPage::Installed => self.installed.dismiss_transient(installed_info),
            ActiveContentPage::Health => false,
            ActiveContentPage::Settings => false,
        }
    }

    pub fn refresh_current_page(
        &self,
        installed_info: &InstalledInfo,
        updates_info: &UpdatesInfo,
        finding_info: &FindingInfo,
    ) -> Option<Message> {
        match self.active_content {
            ActiveContentPage::Finding => self.finding.refresh(finding_info).map(Message::Finding),
            ActiveContentPage::Updates => (!updates_info.is_updating
                && self.updates.can_select_packages())
            .then_some(Message::Updates(updates::Message::RefreshSelected)),
            ActiveContentPage::Installed => (!installed_info.is_removing)
                .then_some(Message::Installed(installed::Message::RefreshInfo)),
            ActiveContentPage::Health => Some(Message::Health(health::Message::StartScan)),
            ActiveContentPage::Settings => None,
        }
    }

    pub fn prepare_primary_action(
        &self,
        installed_info: &InstalledInfo,
        updates_info: &UpdatesInfo,
        finding_info: &FindingInfo,
    ) -> Option<Message> {
        match self.active_content {
            ActiveContentPage::Finding => self
                .finding
                .primary_action(finding_info)
                .map(Message::Finding),
            ActiveContentPage::Updates => self
                .updates
                .primary_action(updates_info)
                .map(Message::Updates),
            ActiveContentPage::Installed => self
                .installed
                .primary_action(installed_info)
                .map(Message::Installed),
            ActiveContentPage::Health => self
                .settings
                .is_dirty()
                .then_some(Message::Settings(setting::Message::SaveConfig)),
            ActiveContentPage::Settings => self
                .settings
                .is_dirty()
                .then_some(Message::Settings(setting::Message::SaveConfig)),
        }
    }

    pub fn select_all_visible(
        &self,
        installed_info: &InstalledInfo,
        updates_info: &UpdatesInfo,
        finding_info: &FindingInfo,
    ) -> Option<Message> {
        match self.active_content {
            ActiveContentPage::Finding => self
                .finding
                .can_select_packages(finding_info)
                .then_some(Message::Finding(finding::Message::ToggleSelectAll(true))),
            ActiveContentPage::Updates => (self.updates.can_select_packages()
                && !updates_info.is_updating)
                .then_some(Message::Updates(updates::Message::ToggleSelectAll(true))),
            ActiveContentPage::Installed => {
                (self.installed.can_select_packages() && !installed_info.is_removing).then_some(
                    Message::Installed(installed::Message::ToggleSelectAll(true)),
                )
            }
            ActiveContentPage::Health => None,
            ActiveContentPage::Settings => None,
        }
    }

    pub fn move_keyboard_selection(
        &self,
        direction: SelectionDirection,
        installed_info: &InstalledInfo,
        updates_info: &UpdatesInfo,
        finding_info: &FindingInfo,
        catalog: &crate::manager_catalog::ManagerCatalog,
    ) -> Option<Message> {
        match self.active_content {
            ActiveContentPage::Finding => self
                .finding
                .move_keyboard_selection(finding_info, catalog, direction)
                .map(Message::Finding),
            ActiveContentPage::Updates => self
                .updates
                .move_keyboard_selection(updates_info, catalog, direction)
                .map(Message::Updates),
            ActiveContentPage::Installed => self
                .installed
                .move_keyboard_selection(installed_info, catalog, direction)
                .map(Message::Installed),
            ActiveContentPage::Health => None,
            ActiveContentPage::Settings => None,
        }
    }

    pub fn toggle_keyboard_selection(
        &self,
        installed_info: &InstalledInfo,
        updates_info: &UpdatesInfo,
        finding_info: &FindingInfo,
    ) -> Option<Message> {
        match self.active_content {
            ActiveContentPage::Finding => self
                .finding
                .toggle_keyboard_selection(finding_info)
                .map(Message::Finding),
            ActiveContentPage::Updates => self
                .updates
                .toggle_keyboard_selection(updates_info)
                .map(Message::Updates),
            ActiveContentPage::Installed => self
                .installed
                .toggle_keyboard_selection(installed_info)
                .map(Message::Installed),
            ActiveContentPage::Health => None,
            ActiveContentPage::Settings => None,
        }
    }

    pub fn has_active_inspector(&self) -> bool {
        match self.active_content {
            ActiveContentPage::Finding => self.finding.has_inspector_selection(),
            ActiveContentPage::Updates => self.updates.has_inspector_selection(),
            ActiveContentPage::Installed => self.installed.has_inspector_selection(),
            ActiveContentPage::Health => false,
            ActiveContentPage::Settings => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn view<'a>(
        &'a self,
        pm_config: &'a updater_core::Config,
        installed_info: &'a InstalledInfo,
        updates_info: &'a UpdatesInfo,
        finding_info: &'a FindingInfo,
        health_info: &'a ManagerHealthInfo,
        catalog: &'a crate::manager_catalog::ManagerCatalog,
        options: ViewOptions,
    ) -> iced::Element<'a, Message> {
        let ViewOptions {
            show_inspector,
            inspector_drawer,
        } = options;
        match self.active_content {
            ActiveContentPage::Finding => self
                .finding
                .view(
                    finding_info,
                    pm_config,
                    catalog,
                    show_inspector,
                    inspector_drawer,
                )
                .map(Message::Finding),
            ActiveContentPage::Updates => self
                .updates
                .view(
                    updates_info,
                    pm_config,
                    catalog,
                    show_inspector,
                    inspector_drawer,
                )
                .map(Message::Updates),
            ActiveContentPage::Installed => self
                .installed
                .view(
                    installed_info,
                    pm_config,
                    catalog,
                    show_inspector,
                    inspector_drawer,
                )
                .map(Message::Installed),
            ActiveContentPage::Health => iced::widget::column![
                self.health
                    .view(
                        self.settings.draft_config(),
                        health_info,
                        catalog,
                        installed_info,
                        updates_info,
                    )
                    .map(Message::Health),
                self.settings
                    .view_managers(catalog, health_info)
                    .map(Message::Settings),
            ]
            .spacing(crate::theme::spacing::LG)
            .height(iced::Length::Fill)
            .into(),
            ActiveContentPage::Settings => self.settings.view().map(Message::Settings),
        }
    }
}

#[cfg(test)]
mod reload_reason_tests {
    use super::*;

    fn manager_id(value: &str) -> updater_manager_api::ManagerId {
        updater_manager_api::ManagerId::parse(value).unwrap()
    }

    #[test]
    fn only_startup_resets_page_context() {
        assert!(!ReloadReason::Startup.preserves_page_context());
        assert!(ReloadReason::ConfigurationChanged.preserves_page_context());
        assert!(ReloadReason::PackageOperation.preserves_page_context());
    }

    #[test]
    fn manager_draft_change_invalidates_an_active_health_scan() {
        let mut content = Content::default();
        let mut config = updater_core::Config::default();
        let mut installed = InstalledInfo::default();
        let mut updates = UpdatesInfo::default();
        let mut finding = FindingInfo::default();
        let mut health = ManagerHealthInfo::default();
        let catalog = crate::manager_catalog::ManagerCatalog::builtin();
        content.settings.sync_from_config(&config);

        let _ = content.update(
            Message::Health(health::Message::StartScan),
            &mut config,
            &mut installed,
            &mut updates,
            &mut finding,
            &mut health,
            &catalog,
        );
        assert!(health.is_checking());

        let action = content.update(
            Message::Settings(setting::Message::AddDetectedManager(manager_id(
                "builtin:cargo",
            ))),
            &mut config,
            &mut installed,
            &mut updates,
            &mut finding,
            &mut health,
            &catalog,
        );

        assert!(matches!(action, Action::None));
        assert!(!health.is_checking());
        assert!(!health.has_results());
    }

    #[test]
    fn application_preference_change_preserves_an_active_health_scan() {
        let mut content = Content::default();
        let mut config = updater_core::Config::default();
        let mut installed = InstalledInfo::default();
        let mut updates = UpdatesInfo::default();
        let mut finding = FindingInfo::default();
        let mut health = ManagerHealthInfo::default();
        let catalog = crate::manager_catalog::ManagerCatalog::builtin();
        content.settings.sync_from_config(&config);

        let _ = content.update(
            Message::Health(health::Message::StartScan),
            &mut config,
            &mut installed,
            &mut updates,
            &mut finding,
            &mut health,
            &catalog,
        );
        let action = content.update(
            Message::Settings(setting::Message::NotificationsChanged(true)),
            &mut config,
            &mut installed,
            &mut updates,
            &mut finding,
            &mut health,
            &catalog,
        );

        assert!(matches!(action, Action::None));
        assert!(health.is_checking());
    }

    #[test]
    fn saving_application_preferences_does_not_reload_package_data() {
        let mut content = Content::default();
        let mut active_config = updater_core::Config::default();
        let mut installed = InstalledInfo::default();
        let mut updates = UpdatesInfo::default();
        let mut finding = FindingInfo::default();
        let mut health = ManagerHealthInfo::default();
        let catalog = crate::manager_catalog::ManagerCatalog::builtin();
        content.settings.sync_from_config(&active_config);

        let _ = content.update(
            Message::Settings(setting::Message::NotificationsChanged(true)),
            &mut active_config,
            &mut installed,
            &mut updates,
            &mut finding,
            &mut health,
            &catalog,
        );
        let mut saved_config = active_config.clone();
        saved_config.notifications_enabled = true;

        let action = content.update(
            Message::Settings(setting::Message::SaveConfigResult {
                config: saved_config,
                validation: Vec::new(),
                result: Ok(()),
            }),
            &mut active_config,
            &mut installed,
            &mut updates,
            &mut finding,
            &mut health,
            &catalog,
        );

        assert!(matches!(action, Action::None));
        assert!(active_config.notifications_enabled);
    }
}
