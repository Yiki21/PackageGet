use iced::{
    Alignment, Length,
    border::Radius,
    widget::{Container, Svg, Text, button, column, row, svg},
};

use crate::{
    app::{self},
    content::ActiveContentPage,
    icon::{FIND_ICON, INSTALLED_ICON, SETTINGS_ICON, UPDATE_ICON},
};

#[derive(Debug, Clone, Default)]
pub struct SideBar {
    pub active_tab: Tab,
}

// Will move to separate files later
#[derive(Debug, Clone, PartialEq, Eq, Default, Copy)]
pub enum Tab {
    #[default]
    Finding,
    Updates,
    Installed,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Select(Tab),
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum Action {
    None,
    ChangeContent(ActiveContentPage),
    Run(iced::Task<Message>),
}

impl From<Message> for app::Message {
    fn from(msg: Message) -> Self {
        app::Message::SideBar(msg)
    }
}

impl From<Tab> for ActiveContentPage {
    fn from(sidebar: Tab) -> Self {
        match sidebar {
            Tab::Finding => ActiveContentPage::Finding,
            Tab::Updates => ActiveContentPage::Updates,
            Tab::Installed => ActiveContentPage::Installed,
            Tab::Settings => ActiveContentPage::Settings,
        }
    }
}

impl SideBar {
    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Select(sidebar) => {
                if self.active_tab == sidebar {
                    Action::None
                } else {
                    self.active_tab = sidebar;
                    log::debug!("Switched to {:?} tab", sidebar);
                    Action::ChangeContent(sidebar.into())
                }
            }
        }
    }

    pub fn view(&self) -> iced::Element<'_, Message> {
        column(
            Tab::ALL
                .iter()
                .map(|&tab| sidebar_button(tab, self.active_tab, tab.icon())),
        )
        .spacing(8)
        .padding(8)
        .into()
    }
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Finding, Tab::Updates, Tab::Installed, Tab::Settings];

    fn label(self) -> &'static str {
        match self {
            Tab::Finding => "Finding",
            Tab::Updates => "Updates",
            Tab::Installed => "Installed",
            Tab::Settings => "Settings",
        }
    }

    fn icon(self) -> svg::Handle {
        match self {
            Tab::Finding => FIND_ICON.clone(),
            Tab::Updates => UPDATE_ICON.clone(),
            Tab::Installed => INSTALLED_ICON.clone(),
            Tab::Settings => SETTINGS_ICON.clone(),
        }
    }
}

fn sidebar_button(tab: Tab, active: Tab, icon: svg::Handle) -> iced::Element<'static, Message> {
    let is_active = tab == active;

    let text = Text::new(tab.label()).size(16);

    let icon = Svg::new(icon).width(16).height(16);

    let content = Container::new(row![icon, text].spacing(12).align_y(Alignment::Center))
        .padding([14, 16])
        .width(Length::Fill)
        .align_y(Alignment::Center)
        .align_x(Alignment::Start);

    button(content)
        .on_press(Message::Select(tab))
        .width(Length::Fill)
        .style(move |_theme, status| {
            use iced::{Shadow, Vector};

            let (background, text_color, shadow) = match (is_active, status) {
                (true, button::Status::Hovered) => (
                    app::colors::PRIMARY_HOVER.into(),
                    app::colors::ON_PRIMARY,
                    Shadow {
                        color: iced::Color::from_rgba(0.3, 0.4, 0.9, 0.25),
                        offset: Vector::new(0.0, 1.0),
                        blur_radius: 4.0,
                    },
                ),
                (true, _) => (
                    app::colors::PRIMARY.into(),
                    app::colors::ON_PRIMARY,
                    Shadow {
                        color: iced::Color::from_rgba(0.4, 0.5, 0.95, 0.3),
                        offset: Vector::new(0.0, 2.0),
                        blur_radius: 8.0,
                    },
                ),
                (_, button::Status::Pressed) => (
                    app::colors::SURFACE_PRESSED.into(),
                    app::colors::ON_SURFACE,
                    Shadow {
                        color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.05),
                        offset: Vector::new(0.0, 1.0),
                        blur_radius: 2.0,
                    },
                ),
                (_, button::Status::Hovered) => (
                    app::colors::SURFACE_HOVER.into(),
                    app::colors::ON_SURFACE,
                    Shadow {
                        color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.08),
                        offset: Vector::new(0.0, 2.0),
                        blur_radius: 4.0,
                    },
                ),
                _ => (
                    app::colors::SURFACE.into(),
                    app::colors::ON_SURFACE_IDLE,
                    Shadow::default(),
                ),
            };

            button::Style {
                background: Some(background),
                text_color,
                border: iced::Border {
                    radius: Radius::new(10.0),
                    ..Default::default()
                },
                shadow,
                snap: false,
            }
        })
        .into()
}
