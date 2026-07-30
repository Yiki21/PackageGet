use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    path::PathBuf,
};

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Border, Element};
use updater_core::Config;
use updater_manager_api::{AuthorizationHint, ManagerId, PackageInfo};

use crate::{manager_catalog::ManagerCatalog, theme};

fn validate_http_url(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value).map_err(|error| format!("Invalid URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http and https URLs can be opened".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Homepage URLs containing credentials cannot be opened".to_owned());
    }
    Ok(url)
}

pub async fn open_http_url(value: String) -> Result<(), String> {
    let url = validate_http_url(&value)?;
    open_desktop_target(OsStr::new(url.as_str())).await
}

pub async fn open_directory(path: PathBuf) -> Result<(), String> {
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| format!("Could not access {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!("Not a directory: {}", path.display()));
    }

    open_desktop_target(path.as_os_str()).await
}

async fn open_desktop_target(target: &OsStr) -> Result<(), String> {
    let attempts = [("gio", Some("open")), ("xdg-open", None)];
    let mut last_error = None;
    for (program, prefix) in attempts {
        let mut command = tokio::process::Command::new(program);
        if let Some(prefix) = prefix {
            command.arg(prefix);
        }
        match command.arg(target).status().await {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => last_error = Some(format!("{program} exited with status {status}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => last_error = Some(format!("{program} failed: {error}")),
        }
    }

    Err(last_error
        .unwrap_or_else(|| "No desktop opener was found (tried gio and xdg-open)".to_owned()))
}

pub type PackageSelectionKey = (ManagerId, String);

pub fn next_keyboard_package(
    packages: &[(ManagerId, String)],
    current: Option<&PackageSelectionKey>,
    direction: crate::shortcut::SelectionDirection,
) -> Option<PackageSelectionKey> {
    if packages.is_empty() {
        return None;
    }

    let current_index =
        current.and_then(|current| packages.iter().position(|package| package == current));
    let next_index = match (current_index, direction) {
        (Some(index), crate::shortcut::SelectionDirection::Previous) => index.saturating_sub(1),
        (Some(index), crate::shortcut::SelectionDirection::Next) => {
            (index + 1).min(packages.len() - 1)
        }
        (None, crate::shortcut::SelectionDirection::Previous) => packages.len() - 1,
        (None, crate::shortcut::SelectionDirection::Next) => 0,
    };

    packages.get(next_index).cloned()
}

pub fn search_input_id(page: crate::content::ActiveContentPage) -> iced::widget::Id {
    match page {
        crate::content::ActiveContentPage::Finding => iced::widget::Id::new("finding-search"),
        crate::content::ActiveContentPage::Updates => iced::widget::Id::new("updates-search"),
        crate::content::ActiveContentPage::Installed => iced::widget::Id::new("installed-search"),
        crate::content::ActiveContentPage::Settings => iced::widget::Id::new("settings-search"),
    }
}

pub struct PackageInspector<'a> {
    pub manager: ManagerId,
    pub name: &'a str,
    pub version: &'a str,
    pub available_version: Option<&'a str>,
    pub description: Option<&'a str>,
    pub size: Option<u64>,
    pub install_date: Option<&'a str>,
    pub homepage: Option<&'a str>,
}

pub struct ManagerSectionStyle {
    pub accent: iced::Color,
    pub error_prefix: &'static str,
}

pub fn selection_key(manager: &ManagerId, package_name: &str) -> PackageSelectionKey {
    (manager.clone(), package_name.to_owned())
}

pub fn package_summary<'a, Message>(package: &'a PackageInfo) -> iced::widget::Column<'a, Message>
where
    Message: 'a,
{
    let mut summary = column![
        text(&package.name)
            .size(14)
            .font(theme::FONT_SEMIBOLD)
            .style(theme::text_on_surface)
            .width(iced::Length::Fill)
            .wrapping(text::Wrapping::WordOrGlyph)
    ]
    .spacing(theme::spacing::XS)
    .width(iced::Length::Fill);

    if let Some(description) = package
        .description
        .as_deref()
        .filter(|description| !description.trim().is_empty())
    {
        summary = summary.push(
            text(description)
                .size(13)
                .style(theme::text_on_surface_muted)
                .width(iced::Length::Fill)
                .height(iced::Length::Fixed(36.0))
                .wrapping(text::Wrapping::WordOrGlyph),
        );
    }

    let mut metadata = Vec::with_capacity(2);
    if let Some(bytes) = package.size {
        metadata.push(format!("Size: {}", format_size(bytes)));
    }
    if let Some(install_date) = package
        .install_date
        .as_deref()
        .filter(|date| !date.trim().is_empty())
    {
        metadata.push(format!("Installed: {install_date}"));
    }
    if !metadata.is_empty() {
        summary = summary.push(
            text(metadata.join("  |  "))
                .size(12)
                .style(theme::text_on_surface_alt)
                .width(iced::Length::Fill)
                .wrapping(text::Wrapping::WordOrGlyph),
        );
    }

    summary
}

pub fn format_size(bytes: u64) -> String {
    let (value, unit) = if bytes >= 1024 * 1024 * 1024 {
        (bytes as f64 / (1024_u64.pow(3)) as f64, "GiB")
    } else if bytes >= 1024 * 1024 {
        (bytes as f64 / (1024_u64.pow(2)) as f64, "MiB")
    } else if bytes >= 1024 {
        (bytes as f64 / 1024.0, "KiB")
    } else {
        return format!("{bytes} B");
    };

    format!("{value:.1} {unit}")
}

pub fn muted_badge<'a, Message>(label: &'a str) -> iced::widget::Container<'a, Message> {
    container(
        text(label)
            .size(12)
            .font(theme::FONT_MONO)
            .style(theme::text_on_surface_muted),
    )
    .padding([2, 0])
}

pub fn package_action_plan_view<'a, Message>(
    manager_groups: &'a [(ManagerId, Vec<String>)],
    catalog: &'a ManagerCatalog,
) -> Element<'a, Message>
where
    Message: 'a,
{
    iced::widget::scrollable(
        column(manager_groups.iter().map(|(manager, packages)| {
            let mut header = row![
                text(catalog.display_name(manager))
                    .size(13)
                    .font(theme::FONT_SEMIBOLD)
                    .style(theme::text_on_surface),
                text(format!(
                    "{} package{}",
                    packages.len(),
                    if packages.len() == 1 { "" } else { "s" }
                ))
                .size(12)
                .style(theme::text_on_surface_muted),
            ]
            .spacing(theme::spacing::SM)
            .align_y(iced::Alignment::Center);

            let authorization = catalog
                .descriptor(manager)
                .and_then(|descriptor| match descriptor.authorization() {
                    AuthorizationHint::None => None,
                    AuthorizationHint::MayRequireElevation { .. } => {
                        Some("May request authorization")
                    }
                    AuthorizationHint::RequiresElevation { .. } => Some("Authorization required"),
                    _ => Some("Authorization behavior may vary"),
                });
            if let Some(authorization) = authorization {
                header = header.push(
                    text(authorization)
                        .size(12)
                        .font(theme::FONT_SEMIBOLD)
                        .style(theme::text_warning),
                );
            }
            let header = header.wrap();

            column![
                header,
                text(packages.join(", "))
                    .size(12)
                    .font(theme::FONT_MONO)
                    .style(theme::text_on_surface_alt)
                    .width(iced::Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
            ]
            .spacing(theme::spacing::XS)
            .into()
        }))
        .spacing(theme::spacing::SM),
    )
    .height(iced::Length::Fixed(120.0))
    .into()
}

pub fn configured_managers(pm_config: &Config) -> Vec<ManagerId> {
    pm_config
        .managers
        .iter()
        .map(|manager| manager.id.clone())
        .collect()
}

pub fn section_title(text: &'static str) -> iced::widget::Text<'static> {
    iced::widget::text(text)
        .size(12)
        .font(theme::FONT_SEMIBOLD)
        .style(theme::text_on_surface_muted)
}

pub fn page_header<'a, Message>(
    title: &'a str,
    subtitle: impl Into<String>,
    accent: iced::Color,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let marker = container("")
        .width(iced::Length::Fixed(10.0))
        .height(iced::Length::Fixed(10.0))
        .style(move |_theme: &iced::Theme| container::Style {
            background: Some(accent.into()),
            border: Border {
                radius: 999.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    column![
        row![
            marker,
            text(title)
                .size(30)
                .font(theme::FONT_SEMIBOLD)
                .style(theme::text_on_surface),
        ]
        .spacing(theme::spacing::SM)
        .align_y(iced::Alignment::Center),
        text(subtitle.into())
            .size(13)
            .style(theme::text_on_surface_muted),
    ]
    .spacing(theme::spacing::XS)
    .into()
}

pub fn summary_row<'a, Message>(
    items: impl IntoIterator<Item = (String, iced::Color)>,
) -> Element<'a, Message>
where
    Message: 'a,
{
    row(items.into_iter().map(|(label, accent)| {
        row![
            container("")
                .width(6)
                .height(6)
                .style(move |_theme: &iced::Theme| container::Style {
                    background: Some(accent.into()),
                    border: Border {
                        radius: 999.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            text(label)
                .size(12)
                .font(theme::FONT_SEMIBOLD)
                .style(theme::text_on_surface_muted),
        ]
        .spacing(theme::spacing::XS)
        .align_y(iced::Alignment::Center)
        .into()
    }))
    .spacing(theme::spacing::LG)
    .wrap()
    .vertical_spacing(theme::spacing::SM)
    .into()
}

pub fn toolbar<'a, Message>(
    content: impl Into<Element<'a, Message>>,
) -> iced::widget::Container<'a, Message>
where
    Message: 'a,
{
    container(content)
        .width(iced::Length::Fill)
        .style(theme::toolbar_container)
}

pub fn segmented_button<'a, Message>(
    label: &'a str,
    selected: bool,
    message: Message,
) -> iced::widget::Button<'a, Message>
where
    Message: 'a + Clone,
{
    button(
        text(label)
            .size(12)
            .font(if selected {
                theme::FONT_SEMIBOLD
            } else {
                theme::FONT_REGULAR
            })
            .align_x(iced::Alignment::Center),
    )
    .padding([7, 10])
    .width(iced::Length::Fill)
    .style(theme::segmented_button(selected))
    .on_press(message)
}

pub fn segmented_group<'a, Message>(
    content: impl Into<Element<'a, Message>>,
) -> iced::widget::Container<'a, Message>
where
    Message: 'a,
{
    container(content)
        .padding(3)
        .width(iced::Length::Fill)
        .style(|iced_theme: &iced::Theme| {
            let semantic = theme::semantic_colors(iced_theme);
            container::Style {
                background: Some(semantic.surface_muted.into()),
                border: Border {
                    color: semantic.divider_light,
                    width: 1.0,
                    radius: theme::radius::SURFACE.into(),
                },
                ..Default::default()
            }
        })
}

pub fn styled_container<'a, Message>(
    content: impl Into<Element<'a, Message>>,
) -> iced::widget::Container<'a, Message> {
    container(content)
        .padding(theme::spacing::LG)
        .width(iced::Length::Fill)
        .style(theme::surface_container)
}

pub fn checkbox_style(
    is_loading: bool,
) -> impl Fn(&iced::Theme, iced::widget::checkbox::Status) -> iced::widget::checkbox::Style {
    move |iced_theme, status| {
        use iced::widget::checkbox::Style;
        let semantic = theme::semantic_colors(iced_theme);

        match status {
            iced::widget::checkbox::Status::Active { is_checked } => {
                let (icon_color, border_color) = if is_checked {
                    (semantic.on_primary, semantic.accent)
                } else {
                    (semantic.on_surface_muted, semantic.divider)
                };

                Style {
                    background: if is_checked {
                        semantic.accent.into()
                    } else {
                        semantic.surface.into()
                    },
                    icon_color,
                    border: Border {
                        color: border_color,
                        width: 2.0,
                        radius: 4.0.into(),
                    },
                    text_color: if is_loading {
                        Some(semantic.on_surface_muted)
                    } else {
                        Some(semantic.on_surface)
                    },
                }
            }
            iced::widget::checkbox::Status::Hovered { is_checked } => {
                if is_loading {
                    Style {
                        background: semantic.surface.into(),
                        icon_color: semantic.on_surface_muted,
                        border: Border {
                            color: semantic.divider,
                            width: 2.0,
                            radius: 4.0.into(),
                        },
                        text_color: Some(semantic.on_surface_muted),
                    }
                } else {
                    let (icon_color, border_color, bg_color) = if is_checked {
                        (
                            semantic.on_primary,
                            semantic.accent_hover,
                            semantic.accent_hover,
                        )
                    } else {
                        (semantic.on_surface_muted, semantic.accent, semantic.surface)
                    };

                    Style {
                        background: bg_color.into(),
                        icon_color,
                        border: Border {
                            color: border_color,
                            width: 2.0,
                            radius: 4.0.into(),
                        },
                        text_color: Some(semantic.on_surface),
                    }
                }
            }
            iced::widget::checkbox::Status::Disabled { .. } => Style {
                background: semantic.surface.into(),
                icon_color: semantic.on_surface_muted,
                border: Border {
                    color: semantic.divider,
                    width: 2.0,
                    radius: 4.0.into(),
                },
                text_color: Some(semantic.on_surface_muted),
            },
        }
    }
}

pub fn package_inspector<'a, Message>(
    package: Option<PackageInspector<'a>>,
    catalog: &'a ManagerCatalog,
    on_copy_name: impl FnOnce(String) -> Message,
    on_copy_homepage: impl FnOnce(String) -> Message,
    on_open_homepage: impl Fn(String) -> Message + Copy + 'a,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    let Some(package) = package else {
        return column![
            text("Package details")
                .size(18)
                .font(theme::FONT_SEMIBOLD)
                .style(theme::text_on_surface),
            text("No package selected")
                .size(13)
                .style(theme::text_on_surface_muted),
        ]
        .spacing(theme::spacing::SM)
        .into();
    };

    let mut details = column![
        text(format!(
            "Source · {}",
            catalog.display_name(&package.manager)
        ))
        .size(12)
        .font(theme::FONT_SEMIBOLD)
        .style(theme::text_on_surface_muted),
        text(package.name)
            .size(22)
            .font(theme::FONT_SEMIBOLD)
            .style(theme::text_on_surface)
            .width(iced::Length::Fill)
            .wrapping(text::Wrapping::WordOrGlyph),
        button(text("Copy Package Name").size(12))
            .padding([6, 10])
            .style(theme::secondary_button(true))
            .on_press(on_copy_name(package.name.to_owned())),
    ]
    .spacing(theme::spacing::MD);

    if let Some(description) = package.description.filter(|value| !value.trim().is_empty()) {
        details = details.push(
            text(description)
                .size(13)
                .style(theme::text_on_surface_muted)
                .width(iced::Length::Fill)
                .wrapping(text::Wrapping::WordOrGlyph),
        );
    }

    details = details
        .push(divider())
        .push(inspector_field("Version", package.version, true));
    if let Some(available) = package.available_version {
        details = details.push(inspector_field("Available", available, true));
    }
    if let Some(size) = package.size {
        details = details.push(inspector_field("Installed size", format_size(size), false));
    }
    if let Some(date) = package
        .install_date
        .filter(|value| !value.trim().is_empty())
    {
        details = details.push(inspector_field("Installed", date, false));
    }
    if let Some(homepage) = package.homepage.filter(|value| !value.trim().is_empty()) {
        details = details.push(
            column![
                section_title("Homepage"),
                button(
                    text(homepage)
                        .size(12)
                        .font(theme::FONT_MONO)
                        .style(theme::text_accent)
                        .width(iced::Length::Fill)
                        .wrapping(text::Wrapping::WordOrGlyph)
                )
                .padding(0)
                .width(iced::Length::Fill)
                .style(theme::link_button)
                .on_press(on_open_homepage(homepage.to_owned())),
                row![
                    button(text("Open Homepage").size(12).font(theme::FONT_SEMIBOLD))
                        .padding([6, 10])
                        .style(theme::secondary_button(true))
                        .on_press(on_open_homepage(homepage.to_owned())),
                    button(text("Copy URL").size(12))
                        .padding([6, 10])
                        .style(theme::secondary_button(true))
                        .on_press(on_copy_homepage(homepage.to_owned())),
                ]
                .spacing(theme::spacing::SM),
            ]
            .spacing(theme::spacing::XS),
        );
    }

    iced::widget::scrollable(details)
        .height(iced::Length::Fill)
        .style(theme::scrollable_style)
        .into()
}

fn divider<'a, Message>() -> iced::widget::Container<'a, Message> {
    container("")
        .height(iced::Length::Fixed(1.0))
        .width(iced::Length::Fill)
        .style(|iced_theme: &iced::Theme| container::Style {
            background: Some(theme::semantic_colors(iced_theme).divider_light.into()),
            ..Default::default()
        })
}

fn inspector_field<'a, Message>(
    label: &'static str,
    value: impl Into<String>,
    mono: bool,
) -> iced::widget::Column<'a, Message> {
    column![
        section_title(label),
        text(value.into())
            .size(13)
            .font(if mono {
                theme::FONT_MONO
            } else {
                theme::FONT_REGULAR
            })
            .style(theme::text_on_surface)
            .width(iced::Length::Fill)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(theme::spacing::XS)
}

pub fn centered_message<'a, Message>(message: &'a str) -> Element<'a, Message>
where
    Message: 'a,
{
    container(text(message).size(16).style(theme::text_on_surface_muted))
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .into()
}

pub fn error_card<'a, Message>(
    title: impl Into<String>,
    detail: &'a str,
    retry: Message,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    let content = row![
        container(
            text("!")
                .size(14)
                .font(theme::FONT_SEMIBOLD)
                .style(theme::text_on_primary)
        )
        .width(24)
        .height(24)
        .center_x(24)
        .center_y(24)
        .style(|iced_theme: &iced::Theme| container::Style {
            background: Some(theme::semantic_colors(iced_theme).error.into()),
            border: Border {
                radius: 999.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }),
        column![
            text(title.into())
                .size(14)
                .font(theme::FONT_SEMIBOLD)
                .style(theme::text_on_surface),
            text(detail)
                .size(13)
                .style(theme::text_on_surface_muted)
                .wrapping(text::Wrapping::WordOrGlyph),
        ]
        .spacing(theme::spacing::XS)
        .width(iced::Length::Fill),
        button(text("Retry").size(13).font(theme::FONT_SEMIBOLD))
            .padding([7, 12])
            .style(theme::secondary_button(true))
            .on_press(retry),
    ]
    .spacing(theme::spacing::MD)
    .align_y(iced::Alignment::Center);

    container(content)
        .padding(theme::spacing::MD)
        .width(iced::Length::Fill)
        .style(|iced_theme: &iced::Theme| container::Style {
            background: Some(theme::semantic_colors(iced_theme).error_soft.into()),
            border: Border {
                radius: theme::radius::SURFACE.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

pub fn manager_section<'a, Message>(
    manager: ManagerId,
    catalog: &'a ManagerCatalog,
    subtitle: String,
    style: ManagerSectionStyle,
    error: Option<&'a str>,
    retry: impl FnOnce() -> Message,
    body: Option<Element<'a, Message>>,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    use iced::widget::{column, row};

    let ManagerSectionStyle {
        accent,
        error_prefix,
    } = style;
    let manager_name = catalog.display_name(&manager).to_owned();
    let header = row![
        text(manager_name.clone())
            .size(15)
            .font(theme::FONT_SEMIBOLD)
            .color(accent),
        text(subtitle).size(13).style(theme::text_on_surface_muted)
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    if let Some(error) = error {
        return column![
            header,
            error_card(
                format!("{}: {}", error_prefix, manager_name),
                error,
                retry()
            ),
        ]
        .spacing(12)
        .into();
    }

    let Some(body) = body else {
        return column![].into();
    };

    column![header, styled_container(body)].spacing(12).into()
}

pub fn loading_manager_filter_view<'a, Message>(
    pm_config: &Config,
    catalog: &'a ManagerCatalog,
    loading_text: &'static str,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let all_managers = configured_managers(pm_config);

    if all_managers.is_empty() {
        return empty_filter_view("No package managers detected");
    }

    let mut col_items: Vec<iced::Element<'a, Message>> = vec![
        text(loading_text)
            .size(13)
            .style(theme::text_on_surface_muted)
            .into(),
    ];

    let checkboxes = all_managers.iter().map(|manager| {
        iced::widget::checkbox(false)
            .label(catalog.display_name(manager).to_owned())
            .spacing(10)
            .text_size(13)
            .style(move |iced_theme, _status| {
                use iced::widget::checkbox::Style;
                let semantic = theme::semantic_colors(iced_theme);
                Style {
                    background: semantic.surface.into(),
                    icon_color: semantic.on_surface_muted,
                    border: Border {
                        color: semantic.divider,
                        width: 2.0,
                        radius: 4.0.into(),
                    },
                    text_color: Some(semantic.on_surface_muted),
                }
            })
            .into()
    });

    col_items.extend(checkboxes);
    column(col_items).spacing(8).into()
}

pub fn empty_filter_view<'a, Message>(message: &'static str) -> Element<'a, Message>
where
    Message: 'a,
{
    column![text(message).size(14).style(theme::text_on_surface_muted)]
        .spacing(8)
        .into()
}

pub fn active_manager_filter_view<'a, Message>(
    entries: Vec<(ManagerId, usize)>,
    selected_managers: &'a HashSet<ManagerId>,
    loading_managers: &'a HashMap<ManagerId, u64>,
    catalog: &'a ManagerCatalog,
    disabled: bool,
    is_initializing: impl Fn(&ManagerId) -> bool + Copy + 'a,
    on_toggle: impl Fn(ManagerId, bool) -> Message + Copy + 'a,
) -> Element<'a, Message>
where
    Message: 'a,
{
    row(entries.into_iter().map(move |(manager, count)| {
        let is_selected = selected_managers.contains(&manager);
        let is_loading = loading_managers.contains_key(&manager);
        let is_initializing = is_initializing(&manager);
        let is_disabled = disabled || is_loading || is_initializing;
        let manager_name = catalog.display_name(&manager);

        let label = if is_loading {
            format!("{manager_name} (Loading...)")
        } else if is_initializing {
            format!("{manager_name} (Initializing...)")
        } else {
            format!("{manager_name} ({count})")
        };

        let checkbox = iced::widget::checkbox(is_selected)
            .label(label)
            .spacing(8)
            .text_size(13)
            .style(checkbox_style(is_disabled));

        if is_disabled {
            checkbox.into()
        } else {
            checkbox
                .on_toggle(move |selected| on_toggle(manager.clone(), selected))
                .into()
        }
    }))
    .spacing(18)
    .width(iced::Length::Fill)
    .wrap()
    .vertical_spacing(10)
    .into()
}

pub fn refresh_button_with_label<'a, Message>(
    label: &'static str,
    enabled: bool,
    message: Message,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    use iced::widget::button;

    let button = button(text(label).size(13).font(theme::FONT_SEMIBOLD))
        .padding([8, 12])
        .style(theme::secondary_button(enabled));
    if enabled {
        button.on_press(message).into()
    } else {
        button.into()
    }
}

pub fn search_input_view<'a, Message>(
    id: iced::widget::Id,
    label: &'static str,
    placeholder: &'static str,
    value: &str,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    let input = text_input(placeholder, value)
        .id(id)
        .on_input(on_input)
        .padding([9, 11])
        .size(14)
        .style(theme::text_input_style);

    column![section_title(label), input]
        .spacing(theme::spacing::SM)
        .into()
}

#[cfg(test)]
mod tests {
    use super::validate_http_url;
    use updater_manager_api::ManagerId;

    fn manager_id(value: &str) -> ManagerId {
        ManagerId::parse(value).unwrap()
    }

    #[test]
    fn keyboard_package_navigation_is_bounded() {
        let packages = vec![
            (manager_id("builtin:dnf"), "alpha".to_owned()),
            (manager_id("builtin:flatpak"), "beta".to_owned()),
        ];

        assert_eq!(
            super::next_keyboard_package(
                &packages,
                None,
                crate::shortcut::SelectionDirection::Next,
            ),
            Some(packages[0].clone())
        );
        assert_eq!(
            super::next_keyboard_package(
                &packages,
                None,
                crate::shortcut::SelectionDirection::Previous,
            ),
            Some(packages[1].clone())
        );
        assert_eq!(
            super::next_keyboard_package(
                &packages,
                Some(&packages[1]),
                crate::shortcut::SelectionDirection::Next,
            ),
            Some(packages[1].clone())
        );
        assert_eq!(
            super::next_keyboard_package(
                &packages,
                Some(&packages[0]),
                crate::shortcut::SelectionDirection::Previous,
            ),
            Some(packages[0].clone())
        );
    }

    #[test]
    fn inspector_rejects_unsafe_url_schemes() {
        assert!(validate_http_url("https://example.com/package").is_ok());
        assert!(validate_http_url("http://example.com/package").is_ok());
        assert!(validate_http_url("file:///etc/passwd").is_err());
        assert!(validate_http_url("javascript:alert(1)").is_err());
        assert!(validate_http_url("https://user:secret@example.com/").is_err());
    }
}
