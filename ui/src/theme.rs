//! Lightweight design system shared by the desktop UI.

use iced::theme::Base;
use iced::widget::{button, container, scrollable, text, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme, font};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
    HighContrast,
}

impl Appearance {
    pub const ALL: [Self; 4] = [Self::System, Self::Light, Self::Dark, Self::HighContrast];

    pub const fn name(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::HighContrast => "High Contrast",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedAppearance {
    Light,
    Dark,
    HighContrast,
}

impl Appearance {
    pub fn from_config(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" => Self::Dark,
            "high_contrast" => Self::HighContrast,
            _ => Self::System,
        }
    }

    pub const fn config_value(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
            Self::HighContrast => "high_contrast",
        }
    }

    pub fn resolve(self, system_mode: iced::theme::Mode) -> ResolvedAppearance {
        match self {
            Self::System if system_mode == iced::theme::Mode::Dark => ResolvedAppearance::Dark,
            Self::System | Self::Light => ResolvedAppearance::Light,
            Self::Dark => ResolvedAppearance::Dark,
            Self::HighContrast => ResolvedAppearance::HighContrast,
        }
    }
}

pub const GEIST_REGULAR_BYTES: &[u8] = include_bytes!("../assets/fonts/Geist-Regular.ttf");
pub const GEIST_SEMIBOLD_BYTES: &[u8] = include_bytes!("../assets/fonts/Geist-SemiBold.ttf");
pub const GEIST_MONO_REGULAR_BYTES: &[u8] = include_bytes!("../assets/fonts/GeistMono-Regular.ttf");

pub const FONT_REGULAR: Font = Font::with_name("Geist");
pub const FONT_SEMIBOLD: Font = Font {
    family: font::Family::Name("Geist"),
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};
pub const FONT_MONO: Font = Font::with_name("Geist Mono");

pub mod spacing {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
}

pub mod radius {
    pub const CONTROL: f32 = 8.0;
    pub const SURFACE: f32 = 12.0;
}

pub mod palette {
    //! Open Color 1.9.1 primitives and light application surface tints.
    //!
    //! Keep raw palette values here. Components should use semantic tokens from
    //! [`super::colors`] instead of depending on a particular shade directly.

    use iced::Color;

    pub const GRAY_2: Color = Color::from_rgb8(233, 236, 239);
    pub const GRAY_4: Color = Color::from_rgb8(206, 212, 218);
    pub const GRAY_6: Color = Color::from_rgb8(134, 142, 150);
    pub const GRAY_7: Color = Color::from_rgb8(73, 80, 87);
    pub const GRAY_9: Color = Color::from_rgb8(33, 37, 41);

    pub const BLUE_0: Color = Color::from_rgb8(231, 245, 255);
    pub const BLUE_1: Color = Color::from_rgb8(208, 235, 255);
    pub const BLUE_8: Color = Color::from_rgb8(25, 113, 194);
    pub const BLUE_9: Color = Color::from_rgb8(24, 100, 171);

    pub const VIOLET_0: Color = Color::from_rgb8(243, 240, 255);
    pub const VIOLET_7: Color = Color::from_rgb8(112, 72, 232);
    pub const TEAL_0: Color = Color::from_rgb8(230, 252, 245);
    pub const TEAL_9: Color = Color::from_rgb8(8, 127, 91);
    pub const GRAPE_0: Color = Color::from_rgb8(248, 240, 252);
    pub const GRAPE_8: Color = Color::from_rgb8(156, 54, 181);

    pub const APP_SIDEBAR: Color = Color::from_rgb8(247, 249, 255);
    pub const APP_GROUPED: Color = Color::from_rgb8(250, 251, 255);

    pub const ORANGE_9: Color = Color::from_rgb8(217, 72, 15);

    pub const RED_0: Color = Color::from_rgb8(255, 245, 245);
    pub const RED_8: Color = Color::from_rgb8(224, 49, 49);
    pub const RED_9: Color = Color::from_rgb8(201, 42, 42);
}

pub mod colors {
    //! Semantic color tokens for UI components.

    use iced::Color;

    use super::palette;

    pub const ACCENT: Color = palette::BLUE_8;
    pub const ACCENT_HOVER: Color = palette::BLUE_9;
    pub const ACCENT_ACTIVE: Color = palette::BLUE_9;
    pub const ACCENT_SOFT: Color = palette::BLUE_0;

    pub const BACKGROUND: Color = Color::WHITE;
    pub const SIDEBAR: Color = palette::APP_SIDEBAR;
    pub const SURFACE: Color = Color::WHITE;
    pub const SURFACE_HOVER: Color = palette::BLUE_0;
    pub const SURFACE_PRESSED: Color = palette::BLUE_1;
    pub const SURFACE_MUTED: Color = palette::APP_GROUPED;

    pub const ON_PRIMARY: Color = Color::WHITE;
    pub const ON_SURFACE: Color = palette::GRAY_9;
    pub const ON_SURFACE_IDLE: Color = palette::GRAY_7;
    pub const ON_SURFACE_MUTED: Color = palette::GRAY_7;
    pub const ON_SURFACE_ALT: Color = palette::GRAY_6;

    pub const SUCCESS: Color = palette::TEAL_9;
    pub const WARNING: Color = palette::ORANGE_9;
    pub const ERROR: Color = palette::RED_8;
    pub const ERROR_SOFT: Color = palette::RED_0;

    pub const DISCOVER: Color = palette::VIOLET_7;
    pub const DISCOVER_SOFT: Color = palette::VIOLET_0;
    pub const UPDATES: Color = palette::TEAL_9;
    pub const UPDATES_SOFT: Color = palette::TEAL_0;
    pub const INSTALLED: Color = palette::BLUE_8;
    pub const INSTALLED_SOFT: Color = palette::BLUE_0;
    pub const HEALTH: Color = palette::ORANGE_9;
    pub const HEALTH_SOFT: Color = Color::from_rgb8(255, 249, 219);
    pub const SETTINGS: Color = palette::GRAPE_8;
    pub const SETTINGS_SOFT: Color = palette::GRAPE_0;

    pub const DIVIDER: Color = palette::GRAY_4;
    pub const DIVIDER_LIGHT: Color = palette::GRAY_2;
    pub const SEPARATOR: Color = Color::from_rgba(0.13, 0.15, 0.16, 0.10);

    pub const INSTALL_ACTION: Color = ACCENT;
    pub const INSTALL_ACTION_HOVER: Color = ACCENT_HOVER;
    pub const INSTALL_ACTION_ACTIVE: Color = ACCENT_ACTIVE;
    pub const UPDATE_ACTION: Color = ACCENT;
    pub const UPDATE_ACTION_HOVER: Color = ACCENT_HOVER;
    pub const UPDATE_ACTION_ACTIVE: Color = ACCENT_ACTIVE;
    pub const REMOVE_ACTION: Color = palette::RED_8;
    pub const REMOVE_ACTION_HOVER: Color = palette::RED_9;
    pub const REMOVE_ACTION_ACTIVE: Color = palette::RED_9;
}

#[derive(Debug, Clone, Copy)]
pub struct SemanticColors {
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_soft: Color,
    pub surface: Color,
    pub surface_hover: Color,
    pub surface_pressed: Color,
    pub surface_muted: Color,
    pub on_primary: Color,
    pub on_surface: Color,
    pub on_surface_idle: Color,
    pub on_surface_muted: Color,
    pub on_surface_alt: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub error_soft: Color,
    pub divider: Color,
    pub divider_light: Color,
}

pub fn semantic_colors(theme: &Theme) -> SemanticColors {
    let base = palette(theme);
    if is_dark(theme) {
        let high_contrast = base.background == Color::BLACK && base.text == Color::WHITE;
        SemanticColors {
            accent: base.primary,
            accent_hover: if high_contrast {
                Color::WHITE
            } else {
                Color::from_rgb8(116, 192, 252)
            },
            accent_soft: if high_contrast {
                Color::from_rgb8(0, 63, 70)
            } else {
                Color::from_rgb8(24, 54, 78)
            },
            surface: surface(theme),
            surface_hover: if high_contrast {
                Color::from_rgb8(0, 54, 60)
            } else {
                Color::from_rgb8(44, 51, 62)
            },
            surface_pressed: if high_contrast {
                Color::from_rgb8(0, 78, 87)
            } else {
                Color::from_rgb8(54, 62, 76)
            },
            surface_muted: surface_muted(theme),
            on_primary: Color::from_rgb8(8, 12, 16),
            on_surface: base.text,
            on_surface_idle: if high_contrast {
                Color::WHITE
            } else {
                Color::from_rgb8(206, 212, 218)
            },
            on_surface_muted: foreground_muted(theme),
            on_surface_alt: if high_contrast {
                Color::from_rgb8(230, 230, 230)
            } else {
                Color::from_rgb8(173, 181, 189)
            },
            success: base.success,
            warning: base.warning,
            error: base.danger,
            error_soft: if high_contrast {
                Color::from_rgb8(74, 0, 0)
            } else {
                Color::from_rgb8(78, 31, 31)
            },
            divider: if high_contrast {
                Color::WHITE
            } else {
                Color::from_rgb8(73, 80, 87)
            },
            divider_light: if high_contrast {
                Color::from_rgb8(210, 210, 210)
            } else {
                Color::from_rgb8(54, 62, 76)
            },
        }
    } else {
        SemanticColors {
            accent: colors::ACCENT,
            accent_hover: colors::ACCENT_HOVER,
            accent_soft: colors::ACCENT_SOFT,
            surface: colors::SURFACE,
            surface_hover: colors::SURFACE_HOVER,
            surface_pressed: colors::SURFACE_PRESSED,
            surface_muted: colors::SURFACE_MUTED,
            on_primary: colors::ON_PRIMARY,
            on_surface: colors::ON_SURFACE,
            on_surface_idle: colors::ON_SURFACE_IDLE,
            on_surface_muted: colors::ON_SURFACE_MUTED,
            on_surface_alt: colors::ON_SURFACE_ALT,
            success: colors::SUCCESS,
            warning: colors::WARNING,
            error: colors::ERROR,
            error_soft: colors::ERROR_SOFT,
            divider: colors::DIVIDER,
            divider_light: colors::DIVIDER_LIGHT,
        }
    }
}

pub fn text_on_surface(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(semantic_colors(theme).on_surface),
    }
}

pub fn text_on_surface_muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(semantic_colors(theme).on_surface_muted),
    }
}

pub fn text_on_surface_alt(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(semantic_colors(theme).on_surface_alt),
    }
}

pub fn text_on_primary(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(semantic_colors(theme).on_primary),
    }
}

pub fn text_accent(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(semantic_colors(theme).accent),
    }
}

pub fn text_success(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(semantic_colors(theme).success),
    }
}

pub fn text_warning(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(semantic_colors(theme).warning),
    }
}

pub fn text_error(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(semantic_colors(theme).error),
    }
}

/// Builds the Iced base theme from the selected appearance.
pub fn application_theme(appearance: ResolvedAppearance) -> Theme {
    match appearance {
        ResolvedAppearance::Light => Theme::custom(
            "Updater Light",
            iced::theme::Palette {
                background: colors::BACKGROUND,
                text: colors::ON_SURFACE,
                primary: colors::ACCENT,
                success: colors::SUCCESS,
                warning: colors::WARNING,
                danger: colors::ERROR,
            },
        ),
        ResolvedAppearance::Dark => Theme::custom(
            "Updater Dark",
            iced::theme::Palette {
                background: Color::from_rgb8(18, 20, 24),
                text: Color::from_rgb8(241, 243, 245),
                primary: Color::from_rgb8(77, 171, 247),
                success: Color::from_rgb8(56, 217, 169),
                warning: Color::from_rgb8(255, 169, 77),
                danger: Color::from_rgb8(255, 107, 107),
            },
        ),
        ResolvedAppearance::HighContrast => Theme::custom(
            "Updater High Contrast",
            iced::theme::Palette {
                background: Color::BLACK,
                text: Color::WHITE,
                primary: Color::from_rgb8(0, 229, 255),
                success: Color::from_rgb8(0, 255, 128),
                warning: Color::from_rgb8(255, 215, 0),
                danger: Color::from_rgb8(255, 64, 64),
            },
        ),
    }
}

fn palette(theme: &Theme) -> iced::theme::Palette {
    theme.palette()
}

fn is_dark(theme: &Theme) -> bool {
    theme.base().background_color.relative_luminance() < 0.35
}

fn surface(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb8(30, 33, 40)
    } else {
        colors::SURFACE
    }
}

fn surface_muted(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb8(39, 43, 52)
    } else {
        colors::SURFACE_MUTED
    }
}

fn foreground(theme: &Theme) -> Color {
    palette(theme).text
}

fn foreground_muted(theme: &Theme) -> Color {
    if is_dark(theme) {
        Color::from_rgb8(206, 212, 218)
    } else {
        colors::ON_SURFACE_MUTED
    }
}

/// Styles the full-height navigation region without drawing edge borders.
pub fn sidebar_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(
            (if is_dark(theme) {
                Color::from_rgb8(24, 27, 33)
            } else {
                colors::SIDEBAR
            })
            .into(),
        ),
        text_color: Some(foreground(theme)),
        ..Default::default()
    }
}

/// Styles the main page region without drawing edge borders.
pub fn content_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(palette(theme).background.into()),
        text_color: Some(foreground(theme)),
        ..Default::default()
    }
}

/// Styles a one-axis divider inserted between adjacent layout regions.
pub fn separator(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(
            (if is_dark(theme) {
                Color::from_rgba8(255, 255, 255, 0.22)
            } else {
                colors::SEPARATOR
            })
            .into(),
        ),
        ..Default::default()
    }
}

/// Styles the bottom activity region; its top edge is provided by a separator.
pub fn status_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(surface(theme).into()),
        text_color: Some(foreground(theme)),
        ..Default::default()
    }
}

pub fn scrollable_style(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let semantic = semantic_colors(theme);
    let mut style = scrollable::default(theme, status);
    style.vertical_rail.background = None;
    style.vertical_rail.border = Border::default();
    style.vertical_rail.scroller.background = match status {
        scrollable::Status::Dragged { .. } => semantic.accent.into(),
        scrollable::Status::Hovered {
            is_vertical_scrollbar_hovered: true,
            ..
        } => semantic.on_surface_alt.into(),
        _ => semantic.divider.into(),
    };
    style.vertical_rail.scroller.border = Border {
        radius: 999.0.into(),
        ..Default::default()
    };
    style
}

pub fn surface_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(surface_muted(theme).into()),
        text_color: Some(foreground(theme)),
        border: Border {
            radius: radius::SURFACE.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn toolbar_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(surface(theme).into()),
        text_color: Some(foreground_muted(theme)),
        ..Default::default()
    }
}

pub fn source_picker_panel(theme: &Theme) -> container::Style {
    let semantic = semantic_colors(theme);
    container::Style {
        background: Some(semantic.surface_muted.into()),
        text_color: Some(semantic.on_surface),
        border: Border {
            color: semantic.divider_light,
            width: 1.0,
            radius: radius::CONTROL.into(),
        },
        ..Default::default()
    }
}

pub fn source_picker_button(expanded: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let semantic = semantic_colors(theme);
        let hovered = matches!(status, button::Status::Hovered);
        button::Style {
            background: Some(
                if matches!(status, button::Status::Pressed) {
                    semantic.surface_pressed
                } else if hovered {
                    semantic.surface_hover
                } else {
                    semantic.surface
                }
                .into(),
            ),
            text_color: semantic.on_surface,
            border: Border {
                color: if expanded || hovered {
                    semantic.accent
                } else {
                    semantic.divider
                },
                width: if expanded || hovered { 2.0 } else { 1.0 },
                radius: radius::CONTROL.into(),
            },
            ..Default::default()
        }
    }
}

pub fn text_input_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let semantic = semantic_colors(theme);
    let focused = matches!(status, text_input::Status::Focused { .. });
    let hovered = matches!(status, text_input::Status::Hovered);

    text_input::Style {
        background: semantic.surface.into(),
        border: Border {
            color: if focused {
                semantic.accent
            } else if hovered {
                semantic.on_surface_alt
            } else {
                semantic.divider
            },
            width: if focused { 2.0 } else { 1.0 },
            radius: radius::CONTROL.into(),
        },
        icon: semantic.on_surface_muted,
        placeholder: semantic.on_surface_alt,
        value: semantic.on_surface,
        selection: semantic.accent_soft,
    }
}

pub fn segmented_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let semantic = semantic_colors(theme);
        let background = if selected {
            Some(Background::Color(semantic.surface))
        } else if matches!(status, button::Status::Pressed) {
            Some(Background::Color(semantic.surface_pressed))
        } else if matches!(status, button::Status::Hovered) {
            Some(Background::Color(semantic.surface_hover))
        } else {
            None
        };

        button::Style {
            background,
            text_color: if selected {
                semantic.on_surface
            } else {
                semantic.on_surface_muted
            },
            border: Border {
                color: if matches!(status, button::Status::Hovered) {
                    semantic.accent
                } else {
                    Color::TRANSPARENT
                },
                width: if matches!(status, button::Status::Hovered) {
                    2.0
                } else {
                    0.0
                },
                radius: radius::CONTROL.into(),
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

pub fn navigation_button(
    selected: bool,
    _accent: Color,
    _accent_soft: Color,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let semantic = semantic_colors(theme);
        let background = if selected {
            Some(Background::Color(semantic.accent_soft))
        } else if matches!(status, button::Status::Pressed) {
            Some(Background::Color(semantic.surface_pressed))
        } else if matches!(status, button::Status::Hovered) {
            Some(Background::Color(semantic.surface_hover))
        } else {
            None
        };

        button::Style {
            background,
            text_color: if selected {
                semantic.accent
            } else {
                semantic.on_surface_idle
            },
            border: Border {
                color: if matches!(status, button::Status::Hovered) {
                    semantic.accent
                } else {
                    Color::TRANSPARENT
                },
                width: if matches!(status, button::Status::Hovered) {
                    2.0
                } else {
                    0.0
                },
                radius: radius::CONTROL.into(),
            },
            ..Default::default()
        }
    }
}

pub fn list_row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let semantic = semantic_colors(theme);
        let background = if selected {
            Some(Background::Color(semantic.accent_soft))
        } else if matches!(status, button::Status::Pressed) {
            Some(Background::Color(semantic.surface_pressed))
        } else if matches!(status, button::Status::Hovered) {
            Some(Background::Color(semantic.surface_hover))
        } else {
            None
        };

        button::Style {
            background,
            text_color: semantic.on_surface,
            border: Border {
                color: if matches!(status, button::Status::Hovered) {
                    semantic.accent
                } else {
                    Color::TRANSPARENT
                },
                width: if matches!(status, button::Status::Hovered) {
                    2.0
                } else {
                    0.0
                },
                radius: radius::CONTROL.into(),
            },
            ..Default::default()
        }
    }
}

pub fn link_button(theme: &Theme, status: button::Status) -> button::Style {
    let semantic = semantic_colors(theme);
    button::Style {
        background: None,
        text_color: if matches!(status, button::Status::Pressed) {
            semantic.accent
        } else if matches!(status, button::Status::Hovered) {
            semantic.accent_hover
        } else {
            semantic.accent
        },
        border: Border {
            color: if matches!(status, button::Status::Hovered) {
                semantic.accent
            } else {
                Color::TRANSPARENT
            },
            width: if matches!(status, button::Status::Hovered) {
                2.0
            } else {
                0.0
            },
            radius: radius::CONTROL.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn secondary_button(enabled: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let semantic = semantic_colors(theme);
        let disabled = !enabled || matches!(status, button::Status::Disabled);
        let background = if disabled {
            semantic.surface_muted
        } else if matches!(status, button::Status::Pressed) {
            semantic.surface_pressed
        } else if matches!(status, button::Status::Hovered) {
            semantic.surface_hover
        } else {
            semantic.surface
        };

        button::Style {
            background: Some(Background::Color(background)),
            text_color: if disabled {
                semantic.on_surface_alt
            } else {
                semantic.on_surface
            },
            border: Border {
                color: if disabled {
                    semantic.divider_light
                } else if matches!(status, button::Status::Hovered) {
                    semantic.accent
                } else {
                    semantic.divider
                },
                width: if !disabled && matches!(status, button::Status::Hovered) {
                    2.0
                } else {
                    1.0
                },
                radius: radius::CONTROL.into(),
            },
            ..Default::default()
        }
    }
}

pub fn action_button(
    enabled: bool,
    base: Color,
    hovered: Color,
    pressed: Color,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let semantic = semantic_colors(theme);
        if !enabled {
            return button::Style {
                background: Some(Background::Color(semantic.surface_muted)),
                text_color: semantic.on_surface_alt,
                border: Border {
                    color: semantic.divider_light,
                    width: 1.0,
                    radius: radius::CONTROL.into(),
                },
                ..Default::default()
            };
        }

        let (base, hovered, pressed) = if base == colors::ACCENT {
            (semantic.accent, semantic.accent_hover, semantic.accent)
        } else if base == colors::REMOVE_ACTION {
            (semantic.error, semantic.error, semantic.error)
        } else {
            (base, hovered, pressed)
        };

        button::Style {
            background: Some(Background::Color(
                if matches!(status, button::Status::Pressed) {
                    pressed
                } else if matches!(status, button::Status::Hovered) {
                    hovered
                } else {
                    base
                },
            )),
            text_color: semantic.on_primary,
            border: Border {
                color: if matches!(status, button::Status::Hovered) {
                    semantic.on_primary
                } else {
                    Color::TRANSPARENT
                },
                width: if matches!(status, button::Status::Hovered) {
                    2.0
                } else {
                    0.0
                },
                radius: radius::CONTROL.into(),
            },
            ..Default::default()
        }
    }
}
