use iced::widget::{column, container, text, text_input};
use iced::{Border, Element};

use crate::app;

/// Shared UI helpers for Installed/Updates pages.
pub struct SharedUi;

impl SharedUi {
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
