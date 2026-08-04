use std::{collections::HashMap, sync::LazyLock};

use iced::{
    Border, Color, Length,
    advanced::svg,
    widget::{container, text},
};
use simple_icons_pack::{
    Icon, SI_ARCHLINUX, SI_BUN, SI_COMPOSER, SI_DEBIAN, SI_DOTNET, SI_FEDORA, SI_FLATPAK, SI_GO,
    SI_HOMEBREW, SI_NIXOS, SI_NPM, SI_OPENSUSE, SI_PNPM, SI_PYTHON, SI_RUBYGEMS, SI_RUST,
    SI_SNAPCRAFT, SI_UV,
};
use updater_manager_api::ManagerId;

use crate::theme;

pub static SAVE_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/save.svg").to_vec())
});

pub static ADD_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/add.svg").to_vec())
});

pub static REFRESH_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/refresh.svg").to_vec())
});

pub static FIND_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/find.svg").to_vec())
});

pub static UPDATE_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/update.svg").to_vec())
});

pub static INSTALLED_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/installed.svg").to_vec())
});

pub static SETTINGS_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/settings.svg").to_vec())
});

pub static HEALTH_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/health.svg").to_vec())
});

#[derive(Debug, Clone)]
struct ManagerLogo {
    handle: svg::Handle,
    color: Color,
}

static MANAGER_LOGOS: LazyLock<HashMap<&'static str, ManagerLogo>> = LazyLock::new(|| {
    [
        ("builtin:apt", &SI_DEBIAN),
        ("builtin:dnf", &SI_FEDORA),
        ("builtin:pacman", &SI_ARCHLINUX),
        ("builtin:zypper", &SI_OPENSUSE),
        ("builtin:flatpak", &SI_FLATPAK),
        ("builtin:snap", &SI_SNAPCRAFT),
        ("builtin:homebrew", &SI_HOMEBREW),
        ("builtin:cargo", &SI_RUST),
        ("builtin:go", &SI_GO),
        ("builtin:npm", &SI_NPM),
        ("builtin:pnpm", &SI_PNPM),
        ("builtin:bun", &SI_BUN),
        ("builtin:pipx", &SI_PYTHON),
        ("builtin:uv", &SI_UV),
        ("builtin:dotnet-tool", &SI_DOTNET),
        ("builtin:rubygems", &SI_RUBYGEMS),
        ("builtin:composer-global", &SI_COMPOSER),
        ("builtin:nix-profile", &SI_NIXOS),
    ]
    .into_iter()
    .map(|(id, icon)| {
        (
            id,
            ManagerLogo {
                handle: svg::Handle::from_memory(icon.svg.as_bytes().to_vec()),
                color: brand_color(icon),
            },
        )
    })
    .collect()
});

/// Renders a fixed-size package-manager identity mark.
pub fn manager_logo<'a, Message: 'a>(
    manager: &ManagerId,
    display_name: &str,
    size: f32,
) -> iced::Element<'a, Message> {
    let content: iced::Element<'a, Message> =
        if let Some(logo) = MANAGER_LOGOS.get(manager.as_str()) {
            iced::widget::Svg::new(logo.handle.clone())
                .width(Length::Fixed(size - 8.0))
                .height(Length::Fixed(size - 8.0))
                .style(move |_theme, _status| iced::widget::svg::Style {
                    color: Some(logo.color),
                })
                .into()
        } else {
            text(manager_initials(display_name))
                .size(if size >= 28.0 { 10 } else { 9 })
                .font(theme::FONT_SEMIBOLD)
                .style(theme::text_accent)
                .into()
        };

    container(content)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .align_x(iced::Alignment::Center)
        .align_y(iced::Alignment::Center)
        .style(|theme| {
            let semantic = theme::semantic_colors(theme);
            container::Style {
                background: Some(Color::WHITE.into()),
                border: Border {
                    color: semantic.divider_light,
                    width: 1.0,
                    radius: 5.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

fn brand_color(icon: &Icon) -> Color {
    let [red, green, blue] = parse_hex_color(icon.hex).unwrap_or([33, 37, 41]);
    Color::from_rgb8(red, green, blue)
}

fn parse_hex_color(value: &str) -> Option<[u8; 3]> {
    let bytes = value.as_bytes();
    if bytes.len() != 6 {
        return None;
    }

    Some([
        hex_byte(bytes[0], bytes[1])?,
        hex_byte(bytes[2], bytes[3])?,
        hex_byte(bytes[4], bytes[5])?,
    ])
}

fn hex_byte(high: u8, low: u8) -> Option<u8> {
    Some(hex_digit(high)? * 16 + hex_digit(low)?)
}

const fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn manager_initials(display_name: &str) -> String {
    let mut words = display_name
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty());
    let Some(first) = words.next() else {
        return "?".to_owned();
    };
    let mut initials = first.chars().take(2).collect::<String>();
    if let Some(second) = words.next() {
        initials.truncate(1);
        initials.extend(second.chars().take(1));
    }
    initials.to_ascii_uppercase()
}

#[cfg(test)]
mod manager_logo_tests {
    use super::{manager_initials, parse_hex_color};

    #[test]
    fn initials_use_multiple_words_when_available() {
        assert_eq!(manager_initials("Nix Profile"), "NP");
    }

    #[test]
    fn initials_use_two_characters_for_single_word() {
        assert_eq!(manager_initials("Winget"), "WI");
    }

    #[test]
    fn hex_color_rejects_invalid_values() {
        assert_eq!(parse_hex_color("not-a-color"), None);
    }
}
