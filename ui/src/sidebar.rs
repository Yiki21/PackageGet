use iced::{
    Alignment, Length,
    widget::{Container, Space, Svg, Text, button, column, container, row, stack, svg},
};

use crate::{
    content::ActiveContentPage,
    icon::{FIND_ICON, HEALTH_ICON, INSTALLED_ICON, SETTINGS_ICON, UPDATE_ICON},
    theme,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct Summary {
    pub update_count: usize,
    pub updates_loading: bool,
    pub updates_failed: bool,
    pub health_checking: bool,
    pub health_has_issues: bool,
    pub settings_dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Badge {
    Count(usize),
    Loading,
    Warning,
    Editing,
}

impl Summary {
    fn updates_badge(self) -> Option<Badge> {
        if self.updates_failed {
            Some(Badge::Warning)
        } else if self.updates_loading {
            Some(Badge::Loading)
        } else if self.update_count > 0 {
            Some(Badge::Count(self.update_count))
        } else {
            None
        }
    }

    fn health_badge(self) -> Option<Badge> {
        if self.health_has_issues {
            Some(Badge::Warning)
        } else if self.health_checking {
            Some(Badge::Loading)
        } else {
            None
        }
    }
}

impl Badge {
    fn label(self) -> String {
        match self {
            Self::Count(count) if count > 99 => "99+".to_owned(),
            Self::Count(count) => count.to_string(),
            Self::Loading => "…".to_owned(),
            Self::Warning => "!".to_owned(),
            Self::Editing => "●".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SideBar {
    /// Currently selected sidebar tab.
    pub active_tab: Tab,
}

// TODO: Move sidebar types into dedicated files.
#[derive(Debug, Clone, PartialEq, Eq, Default, Copy)]
pub enum Tab {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Tab selection message.
    Select(Tab),
    /// Dismiss the narrow-width sidebar.
    Close,
}

#[derive(Debug)]
pub enum Action {
    /// No-op action.
    None,
    /// Content page switch action.
    ChangeContent(ActiveContentPage),
    /// Close the responsive sidebar.
    CloseRequested,
}

impl From<Tab> for ActiveContentPage {
    fn from(sidebar: Tab) -> Self {
        match sidebar {
            Tab::Finding => ActiveContentPage::Finding,
            Tab::Updates => ActiveContentPage::Updates,
            Tab::Installed => ActiveContentPage::Installed,
            Tab::Health => ActiveContentPage::Health,
            Tab::Settings => ActiveContentPage::Settings,
        }
    }
}

impl From<ActiveContentPage> for Tab {
    fn from(page: ActiveContentPage) -> Self {
        match page {
            ActiveContentPage::Finding => Tab::Finding,
            ActiveContentPage::Updates => Tab::Updates,
            ActiveContentPage::Installed => Tab::Installed,
            ActiveContentPage::Health => Tab::Health,
            ActiveContentPage::Settings => Tab::Settings,
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
            Message::Close => Action::CloseRequested,
        }
    }

    pub fn view(
        &self,
        summary: Summary,
        compact: bool,
        closeable: bool,
    ) -> iced::Element<'_, Message> {
        let brand_mark = row![
            brand_dot(theme::colors::DISCOVER),
            brand_dot(theme::colors::UPDATES),
            brand_dot(theme::colors::INSTALLED),
            brand_dot(theme::colors::HEALTH),
            brand_dot(theme::colors::SETTINGS),
        ]
        .spacing(3)
        .align_y(Alignment::Center);
        let brand_content: iced::Element<'_, Message> = if compact {
            brand_mark.into()
        } else {
            let mut brand_row = row![
                Text::new("Updater")
                    .size(21)
                    .font(theme::FONT_SEMIBOLD)
                    .style(theme::text_on_surface),
                brand_mark,
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            if closeable {
                brand_row = brand_row.push(Space::new().width(Length::Fill)).push(
                    button(Text::new("Close").size(12))
                        .padding([5, 8])
                        .style(theme::secondary_button(true))
                        .on_press(Message::Close),
                );
            }
            brand_row.into()
        };
        let brand = container(brand_content).padding([8, 10]);

        let primary_navigation = column(Tab::PRIMARY.iter().map(|&tab| {
            sidebar_button(
                tab,
                self.active_tab,
                tab.icon(),
                match tab {
                    Tab::Updates => summary.updates_badge(),
                    Tab::Health => summary.health_badge(),
                    _ => None,
                },
                compact,
            )
        }))
        .spacing(4);

        column![
            brand,
            primary_navigation,
            Space::new().height(Length::Fill),
            sidebar_button(
                Tab::Settings,
                self.active_tab,
                Tab::Settings.icon(),
                summary.settings_dirty.then_some(Badge::Editing),
                compact,
            ),
        ]
        .spacing(12)
        .height(Length::Fill)
        .into()
    }
}

impl Tab {
    const PRIMARY: [Tab; 4] = [Tab::Finding, Tab::Updates, Tab::Installed, Tab::Health];

    fn label(self) -> &'static str {
        match self {
            Tab::Finding => "Discover",
            Tab::Updates => "Updates",
            Tab::Installed => "Installed",
            Tab::Health => "Managers",
            Tab::Settings => "Settings",
        }
    }

    fn colors(self) -> (iced::Color, iced::Color) {
        match self {
            Tab::Finding => (theme::colors::DISCOVER, theme::colors::DISCOVER_SOFT),
            Tab::Updates => (theme::colors::UPDATES, theme::colors::UPDATES_SOFT),
            Tab::Installed => (theme::colors::INSTALLED, theme::colors::INSTALLED_SOFT),
            Tab::Health => (theme::colors::HEALTH, theme::colors::HEALTH_SOFT),
            Tab::Settings => (theme::colors::SETTINGS, theme::colors::SETTINGS_SOFT),
        }
    }

    fn icon(self) -> svg::Handle {
        match self {
            Tab::Finding => FIND_ICON.clone(),
            Tab::Updates => UPDATE_ICON.clone(),
            Tab::Installed => INSTALLED_ICON.clone(),
            Tab::Health => HEALTH_ICON.clone(),
            Tab::Settings => SETTINGS_ICON.clone(),
        }
    }
}

fn brand_dot(color: iced::Color) -> iced::widget::Container<'static, Message> {
    container("")
        .width(6)
        .height(6)
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(color.into()),
            border: iced::Border {
                radius: 999.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
}

fn sidebar_button(
    tab: Tab,
    active: Tab,
    icon: svg::Handle,
    badge: Option<Badge>,
    compact: bool,
) -> iced::Element<'static, Message> {
    let is_active = tab == active;
    let (accent, accent_soft) = tab.colors();

    let text = Text::new(tab.label()).size(14).font(if is_active {
        theme::FONT_SEMIBOLD
    } else {
        theme::FONT_REGULAR
    });

    let icon = Svg::new(icon)
        .width(16)
        .height(16)
        .style(move |iced_theme, _status| {
            let semantic = theme::semantic_colors(iced_theme);
            svg::Style {
                color: Some(if is_active {
                    semantic.accent
                } else {
                    iced::Color {
                        a: 0.72,
                        ..semantic.on_surface_idle
                    }
                }),
            }
        });

    let content: iced::Element<'static, Message> = if compact {
        let icon_layer = container(icon)
            .width(28)
            .height(28)
            .center_x(28)
            .center_y(28);
        let mut compact_icon = stack![icon_layer].width(28).height(28);
        if let Some(badge) = badge {
            compact_icon = compact_icon.push(
                container(
                    Text::new(badge.label())
                        .size(9)
                        .font(theme::FONT_SEMIBOLD)
                        .style(move |iced_theme| iced::widget::text::Style {
                            color: Some(match badge {
                                Badge::Warning => theme::semantic_colors(iced_theme).error,
                                _ => accent,
                            }),
                        }),
                )
                .width(28)
                .height(28)
                .align_x(Alignment::End)
                .align_y(Alignment::Start),
            );
        }
        Container::new(compact_icon)
            .padding([8, 0])
            .width(Length::Fill)
            .align_x(Alignment::Center)
            .into()
    } else {
        let mut row_content = row![icon, text]
            .spacing(10)
            .align_y(Alignment::Center)
            .width(Length::Fill);
        if let Some(badge) = badge {
            row_content = row_content.push(Space::new().width(Length::Fill)).push(
                container(
                    Text::new(badge.label())
                        .size(11)
                        .font(theme::FONT_SEMIBOLD)
                        .style(move |iced_theme| iced::widget::text::Style {
                            color: Some(match badge {
                                Badge::Warning => theme::semantic_colors(iced_theme).error,
                                _ => accent,
                            }),
                        }),
                )
                .padding([0, 2]),
            );
        }
        Container::new(row_content)
            .padding([8, 10])
            .width(Length::Fill)
            .align_y(Alignment::Center)
            .align_x(Alignment::Start)
            .into()
    };

    button(content)
        .on_press(Message::Select(tab))
        .width(Length::Fill)
        .style(theme::navigation_button(is_active, accent, accent_soft))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_badge_prioritizes_failures_then_loading_then_count() {
        assert_eq!(
            Summary {
                update_count: 12,
                updates_loading: true,
                updates_failed: true,
                health_checking: false,
                health_has_issues: false,
                settings_dirty: false,
            }
            .updates_badge(),
            Some(Badge::Warning)
        );
        assert_eq!(
            Summary {
                update_count: 12,
                updates_loading: true,
                updates_failed: false,
                health_checking: false,
                health_has_issues: false,
                settings_dirty: false,
            }
            .updates_badge(),
            Some(Badge::Loading)
        );
        assert_eq!(
            Summary {
                update_count: 12,
                updates_loading: false,
                updates_failed: false,
                health_checking: false,
                health_has_issues: false,
                settings_dirty: false,
            }
            .updates_badge(),
            Some(Badge::Count(12))
        );
    }

    #[test]
    fn health_badge_prioritizes_issues_over_checking() {
        assert_eq!(
            Summary {
                health_checking: true,
                health_has_issues: true,
                ..Summary::default()
            }
            .health_badge(),
            Some(Badge::Warning)
        );
        assert_eq!(
            Summary {
                health_checking: true,
                ..Summary::default()
            }
            .health_badge(),
            Some(Badge::Loading)
        );
    }

    #[test]
    fn large_badge_counts_are_capped() {
        assert_eq!(Badge::Count(99).label(), "99");
        assert_eq!(Badge::Count(100).label(), "99+");
    }
}
