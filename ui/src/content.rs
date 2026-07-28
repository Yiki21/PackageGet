mod errors;
mod finding;
mod installed;
mod setting;
mod shared;
mod updates;
mod workflows;

use crate::{
    content::{finding::Finding, installed::Installed, setting::Settings, updates::Updates},
    shortcut::SelectionDirection,
};

pub use finding::FindingInfo;
pub use installed::{InstalledInfo, Message as InstalledMessage};
pub use setting::Message as SettingsMessage;
pub(crate) use shared::search_input_id;
pub use updates::UpdatesInfo;
pub(crate) use workflows::CancellationToken;
pub use workflows::OperationOutcome;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActiveContentPage {
    /// Search/install page.
    #[default]
    Finding,
    /// Available updates page.
    Updates,
    /// Installed packages page.
    Installed,
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
}

pub enum Action {
    /// No-op action.
    None,
    /// Asynchronous task action.
    Run(iced::Task<Message>),
    /// Cooperative package-operation task.
    CancellableRun(iced::Task<Message>, workflows::CancellationToken),
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
    pub fn update(
        &mut self,
        message: Message,
        pm_config: &mut updater_core::Config,
        installed_info: &mut InstalledInfo,
        updates_info: &mut UpdatesInfo,
        finding_info: &mut FindingInfo,
    ) -> Action {
        let pm_config_ref: &updater_core::Config = pm_config;

        match message {
            Message::Settings(settings_msg) => {
                let action = self.settings.update(settings_msg, pm_config);
                match action {
                    setting::Action::Run(task) => Action::Run(task.map(Message::Settings)),
                    setting::Action::ApplySavedConfig(config) => {
                        *pm_config = config;
                        Action::ReloadPackageData {
                            reason: ReloadReason::ConfigurationChanged,
                            follow_up: iced::Task::none(),
                        }
                    }
                    setting::Action::None => Action::None,
                }
            }
            Message::Installed(installed_msg) => {
                let action = self
                    .installed
                    .update(installed_msg, pm_config_ref, installed_info);
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
                    .update(updates_msg, pm_config_ref, updates_info);
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
                    .update(finding_msg, pm_config_ref, finding_info);
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
        }
    }

    pub fn dismiss_active_transient(&mut self, installed_info: &mut InstalledInfo) -> bool {
        match self.active_content {
            ActiveContentPage::Finding => self.finding.dismiss_transient(),
            ActiveContentPage::Updates => self.updates.dismiss_transient(),
            ActiveContentPage::Installed => self.installed.dismiss_transient(installed_info),
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
            ActiveContentPage::Settings => None,
        }
    }

    pub fn move_keyboard_selection(
        &self,
        direction: SelectionDirection,
        installed_info: &InstalledInfo,
        updates_info: &UpdatesInfo,
        finding_info: &FindingInfo,
    ) -> Option<Message> {
        match self.active_content {
            ActiveContentPage::Finding => self
                .finding
                .move_keyboard_selection(finding_info, direction)
                .map(Message::Finding),
            ActiveContentPage::Updates => self
                .updates
                .move_keyboard_selection(updates_info, direction)
                .map(Message::Updates),
            ActiveContentPage::Installed => self
                .installed
                .move_keyboard_selection(installed_info, direction)
                .map(Message::Installed),
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
            ActiveContentPage::Settings => None,
        }
    }

    pub fn has_active_inspector(&self) -> bool {
        match self.active_content {
            ActiveContentPage::Finding => self.finding.has_inspector_selection(),
            ActiveContentPage::Updates => self.updates.has_inspector_selection(),
            ActiveContentPage::Installed => self.installed.has_inspector_selection(),
            ActiveContentPage::Settings => false,
        }
    }

    pub fn view<'a>(
        &'a self,
        pm_config: &'a updater_core::Config,
        installed_info: &'a InstalledInfo,
        updates_info: &'a UpdatesInfo,
        finding_info: &'a FindingInfo,
        show_inspector: bool,
        inspector_drawer: bool,
    ) -> iced::Element<'a, Message> {
        match self.active_content {
            ActiveContentPage::Finding => self
                .finding
                .view(finding_info, pm_config, show_inspector, inspector_drawer)
                .map(Message::Finding),
            ActiveContentPage::Updates => self
                .updates
                .view(updates_info, pm_config, show_inspector, inspector_drawer)
                .map(Message::Updates),
            ActiveContentPage::Installed => self
                .installed
                .view(installed_info, pm_config, show_inspector, inspector_drawer)
                .map(Message::Installed),
            ActiveContentPage::Settings => self.settings.view(pm_config).map(Message::Settings),
        }
    }
}

#[cfg(test)]
mod reload_reason_tests {
    use super::ReloadReason;

    #[test]
    fn only_startup_resets_page_context() {
        assert!(!ReloadReason::Startup.preserves_page_context());
        assert!(ReloadReason::ConfigurationChanged.preserves_page_context());
        assert!(ReloadReason::PackageOperation.preserves_page_context());
    }
}
