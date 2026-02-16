mod finding;
#[allow(dead_code, unused)]
mod installed;
mod setting;
mod shared;
mod updates;

use crate::{
    app,
    content::{finding::Finding, installed::Installed, setting::Settings, updates::Updates},
};

// pub use installed::SortOption;
pub use finding::FindingInfo;
pub use installed::InstalledInfo;
pub use updates::UpdatesInfo;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActiveContentPage {
    #[default]
    Finding,
    Updates,
    Installed,
    Settings,
}

#[derive(Debug, Clone, Default)]
pub struct Content {
    pub actinve_content: ActiveContentPage,
    pub settings: Settings,
    pub installed: Installed,
    pub updates: Updates,
    pub finding: Finding,
}

#[derive(Debug, Clone)]
pub enum Message {
    Settings(setting::Message),
    Installed(installed::Message),
    Updates(updates::Message),
    Finding(finding::Message),
}

pub enum Action {
    None,
    Run(iced::Task<Message>),
    ClearCacheAndReload,
}

impl From<Message> for app::Message {
    fn from(msg: Message) -> Self {
        crate::app::Message::Content(msg)
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
        match message {
            Message::Settings(settings_msg) => {
                if let ActiveContentPage::Settings = &self.actinve_content {
                    let action = self.settings.update(settings_msg, pm_config);
                    match action {
                        setting::Action::Run(task) => Action::Run(task.map(Message::Settings)),
                        setting::Action::None => Action::None,
                    }
                } else {
                    Action::None
                }
            }
            Message::Installed(installed_msg) => {
                if let ActiveContentPage::Installed = &self.actinve_content {
                    let action = self.installed.update(installed_msg, installed_info);
                    match action {
                        installed::Action::Run(task) => Action::Run(task.map(Message::Installed)),
                        installed::Action::None => Action::None,
                        installed::Action::ClearCacheAndReload => Action::ClearCacheAndReload,
                    }
                } else {
                    Action::None
                }
            }
            Message::Updates(updates_msg) => {
                if let ActiveContentPage::Updates = &self.actinve_content {
                    let action = self.updates.update(updates_msg, updates_info);
                    match action {
                        updates::Action::Run(task) => Action::Run(task.map(Message::Updates)),
                        updates::Action::None => Action::None,
                    }
                } else {
                    Action::None
                }
            }
            Message::Finding(finding_msg) => {
                if let ActiveContentPage::Finding = &self.actinve_content {
                    let action = self.finding.update(finding_msg, finding_info);
                    match action {
                        finding::Action::Run(task) => Action::Run(task.map(Message::Finding)),
                        finding::Action::None => Action::None,
                    }
                } else {
                    Action::None
                }
            }
        }
    }

    pub fn view<'a>(
        &self,
        pm_config: &updater_core::Config,
        installed_info: &'a InstalledInfo,
        updates_info: &'a UpdatesInfo,
        finding_info: &'a FindingInfo,
    ) -> iced::Element<'a, Message> {
        match self.actinve_content {
            ActiveContentPage::Finding => self
                .finding
                .view(finding_info, pm_config)
                .map(Message::Finding),
            ActiveContentPage::Updates => self
                .updates
                .view(updates_info, pm_config)
                .map(Message::Updates),
            ActiveContentPage::Installed => self
                .installed
                .view(installed_info, pm_config)
                .map(Message::Installed),
            ActiveContentPage::Settings => self.settings.view(pm_config).map(Message::Settings),
        }
    }
}
