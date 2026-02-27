use std::collections::HashSet;

use iced::widget::{column, container, text, text_input};
use iced::{Border, Element};
use updater_core::{Config, PackageManagerType};

use crate::app;

pub type PackageSelectionKey = (PackageManagerType, String);

/// Shared UI helpers for Installed/Updates pages.
pub struct SharedUi;

impl SharedUi {
    pub fn selection_key(pm_type: PackageManagerType, package_name: &str) -> PackageSelectionKey {
        (pm_type, package_name.to_owned())
    }

    pub fn configured_managers(pm_config: &Config) -> Vec<PackageManagerType> {
        pm_config
            .system_manager
            .iter()
            .map(|pm| pm.manager_type)
            .chain(pm_config.app_managers.iter().map(|pm| pm.manager_type))
            .collect()
    }

    pub fn section_title(text: &'static str) -> iced::widget::Text<'static> {
        iced::widget::text(text)
            .size(16)
            .color(app::colors::ON_SURFACE)
    }

    pub fn styled_container<'a, Message>(
        content: impl Into<Element<'a, Message>>,
    ) -> iced::widget::Container<'a, Message> {
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

    pub fn checkbox_style(
        is_loading: bool,
    ) -> impl Fn(&iced::Theme, iced::widget::checkbox::Status) -> iced::widget::checkbox::Style
    {
        move |_theme, status| {
            use iced::widget::checkbox::Style;

            match status {
                iced::widget::checkbox::Status::Active { is_checked } => {
                    let (icon_color, border_color) = if is_checked {
                        (app::colors::ON_PRIMARY, app::colors::SECONDARY)
                    } else {
                        (app::colors::ON_SURFACE_MUTED, app::colors::DIVIDER)
                    };

                    Style {
                        background: if is_checked {
                            app::colors::SECONDARY.into()
                        } else {
                            app::colors::SURFACE.into()
                        },
                        icon_color,
                        border: Border {
                            color: border_color,
                            width: 2.0,
                            radius: 4.0.into(),
                        },
                        text_color: if is_loading {
                            Some(app::colors::ON_SURFACE_MUTED)
                        } else {
                            Some(app::colors::ON_SURFACE)
                        },
                    }
                }
                iced::widget::checkbox::Status::Hovered { is_checked } => {
                    if is_loading {
                        Style {
                            background: app::colors::SURFACE.into(),
                            icon_color: app::colors::ON_SURFACE_MUTED,
                            border: Border {
                                color: app::colors::DIVIDER,
                                width: 2.0,
                                radius: 4.0.into(),
                            },
                            text_color: Some(app::colors::ON_SURFACE_MUTED),
                        }
                    } else {
                        let (icon_color, border_color, bg_color) = if is_checked {
                            (
                                app::colors::ON_PRIMARY,
                                app::colors::SECONDARY_HOVER,
                                app::colors::SECONDARY_HOVER,
                            )
                        } else {
                            (
                                app::colors::ON_SURFACE_MUTED,
                                app::colors::SECONDARY,
                                app::colors::SURFACE,
                            )
                        };

                        Style {
                            background: bg_color.into(),
                            icon_color,
                            border: Border {
                                color: border_color,
                                width: 2.0,
                                radius: 4.0.into(),
                            },
                            text_color: Some(app::colors::ON_SURFACE),
                        }
                    }
                }
                iced::widget::checkbox::Status::Disabled { .. } => Style {
                    background: app::colors::SURFACE.into(),
                    icon_color: app::colors::ON_SURFACE_MUTED,
                    border: Border {
                        color: app::colors::DIVIDER,
                        width: 2.0,
                        radius: 4.0.into(),
                    },
                    text_color: Some(app::colors::ON_SURFACE_MUTED),
                },
            }
        }
    }

    pub fn radio_style(
        _theme: &iced::Theme,
        status: iced::widget::radio::Status,
    ) -> iced::widget::radio::Style {
        use iced::widget::radio::Style;

        match status {
            iced::widget::radio::Status::Active { is_selected } => {
                let (dot_color, border_color) = if is_selected {
                    (app::colors::SECONDARY, app::colors::SECONDARY)
                } else {
                    (app::colors::ON_SURFACE_MUTED, app::colors::DIVIDER)
                };

                Style {
                    background: app::colors::SURFACE.into(),
                    dot_color,
                    border_width: 2.0,
                    border_color,
                    text_color: Some(app::colors::ON_SURFACE),
                }
            }
            iced::widget::radio::Status::Hovered { is_selected } => {
                let (dot_color, border_color) = if is_selected {
                    (app::colors::SECONDARY_HOVER, app::colors::SECONDARY_HOVER)
                } else {
                    (app::colors::ON_SURFACE_MUTED, app::colors::SECONDARY)
                };

                Style {
                    background: app::colors::SURFACE.into(),
                    dot_color,
                    border_width: 2.0,
                    border_color,
                    text_color: Some(app::colors::ON_SURFACE),
                }
            }
        }
    }

    pub fn centered_message<'a, Message>(message: &'a str) -> Element<'a, Message>
    where
        Message: 'a,
    {
        container(text(message).size(16).color(app::colors::ON_SURFACE_MUTED))
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .center_x(iced::Length::Fill)
            .center_y(iced::Length::Fill)
            .into()
    }

    pub fn filter_section<'a, Message>(
        title: &'static str,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message>
    where
        Message: 'a,
    {
        column![
            SharedUi::section_title(title),
            SharedUi::styled_container(content)
        ]
        .spacing(12)
        .into()
    }

    pub fn loading_manager_filter_view<'a, Message>(
        pm_config: &Config,
        loading_text: &'static str,
    ) -> iced::widget::Column<'a, Message>
    where
        Message: 'a,
    {
        let all_managers = Self::configured_managers(pm_config);

        if all_managers.is_empty() {
            return Self::empty_filter_view("No package managers detected");
        }

        let mut col_items: Vec<iced::Element<'a, Message>> = vec![
            text(loading_text)
                .size(13)
                .color(app::colors::ON_SURFACE_MUTED)
                .into(),
        ];

        let checkboxes = all_managers.iter().map(|pm_type| {
            iced::widget::checkbox(false)
                .label(pm_type.name())
                .spacing(10)
                .text_size(15)
                .style(move |_theme, _status| {
                    use iced::widget::checkbox::Style;
                    Style {
                        background: app::colors::SURFACE.into(),
                        icon_color: app::colors::ON_SURFACE_MUTED,
                        border: Border {
                            color: app::colors::DIVIDER,
                            width: 2.0,
                            radius: 4.0.into(),
                        },
                        text_color: Some(app::colors::ON_SURFACE_MUTED),
                    }
                })
                .into()
        });

        col_items.extend(checkboxes);
        column(col_items).spacing(8)
    }

    pub fn empty_filter_view<'a, Message>(
        message: &'static str,
    ) -> iced::widget::Column<'a, Message>
    where
        Message: 'a,
    {
        column![text(message).size(14).color(app::colors::ON_SURFACE_MUTED)].spacing(8)
    }

    pub fn active_manager_filter_view<'a, Message>(
        entries: Vec<(PackageManagerType, usize)>,
        selected_managers: &'a HashSet<PackageManagerType>,
        loading_managers: &'a HashSet<PackageManagerType>,
        on_toggle: impl Fn(PackageManagerType, bool) -> Message + Copy + 'a,
    ) -> iced::widget::Column<'a, Message>
    where
        Message: 'a,
    {
        column(entries.into_iter().map(move |(pm_type, count)| {
            let is_selected = selected_managers.contains(&pm_type);
            let is_loading = loading_managers.contains(&pm_type);

            let label = if is_loading {
                format!("{} (Loading...)", pm_type.name())
            } else {
                format!("{} ({})", pm_type.name(), count)
            };

            let checkbox = iced::widget::checkbox(is_selected)
                .label(label)
                .spacing(10)
                .text_size(15)
                .style(SharedUi::checkbox_style(is_loading));

            if is_loading {
                checkbox.into()
            } else {
                checkbox
                    .on_toggle(move |selected| on_toggle(pm_type, selected))
                    .into()
            }
        }))
        .spacing(12)
    }

    pub fn refresh_button<'a, Message>(message: Message) -> Element<'a, Message>
    where
        Message: 'a + Clone,
    {
        use iced::widget::button;

        button(text("Refresh").size(14).color(iced::Color::WHITE))
            .padding([8, 16])
            .style(|_theme, status| {
                use iced::widget::button::Style;
                let base_color = app::colors::SECONDARY;
                match status {
                    button::Status::Hovered => Style {
                        background: Some(iced::Background::Color(app::colors::SECONDARY_HOVER)),
                        text_color: iced::Color::WHITE,
                        border: Border {
                            color: iced::Color::TRANSPARENT,
                            width: 0.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    },
                    _ => Style {
                        background: Some(iced::Background::Color(base_color)),
                        text_color: iced::Color::WHITE,
                        border: Border {
                            color: iced::Color::TRANSPARENT,
                            width: 0.0,
                            radius: 6.0.into(),
                        },
                        ..Default::default()
                    },
                }
            })
            .on_press(message)
            .into()
    }

    pub fn search_input_view<'a, Message>(
        label: &'static str,
        placeholder: &'static str,
        value: &str,
        on_input: impl Fn(String) -> Message + 'a,
    ) -> Element<'a, Message>
    where
        Message: 'a + Clone,
    {
        let input = text_input(placeholder, value)
            .on_input(on_input)
            .padding(10)
            .size(15);

        column![Self::section_title(label), Self::styled_container(input)]
            .spacing(12)
            .into()
    }

    pub fn search_input_view_with_submit<'a, Message>(
        label: &'static str,
        placeholder: &'static str,
        value: &str,
        on_input: impl Fn(String) -> Message + 'a,
        on_submit: Message,
    ) -> Element<'a, Message>
    where
        Message: 'a + Clone,
    {
        let input = text_input(placeholder, value)
            .on_input(on_input)
            .on_submit(on_submit)
            .padding(10)
            .size(15);

        column![Self::section_title(label), Self::styled_container(input)]
            .spacing(12)
            .into()
    }
}
