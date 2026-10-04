use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    path::PathBuf,
    sync::Arc,
};

use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Border, Element, Length};
use updater_core::{Config, ManagerRegistry, OperationOutcome};
use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerCategory, ManagerConfig, ManagerDescriptor,
    ManagerError, ManagerErrorKind, ManagerId, PackageAction, PackageInfo, PackageOrigin,
    PackageScope, PackageTarget, Platform,
};

use crate::{icon, manager_catalog::ManagerCatalog, theme};

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
    open_desktop_target(DesktopTargetKind::Url, OsStr::new(url.as_str())).await
}

pub async fn open_directory(path: PathBuf) -> Result<(), String> {
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| format!("Could not access {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!("Not a directory: {}", path.display()));
    }

    open_desktop_target(DesktopTargetKind::Directory, path.as_os_str()).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopTargetKind {
    Url,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopOpenCommand {
    program: &'static str,
    arguments: Vec<OsString>,
}

fn desktop_open_commands(
    platform: Platform,
    kind: DesktopTargetKind,
    target: &OsStr,
) -> Vec<DesktopOpenCommand> {
    let target = target.to_os_string();
    match (platform, kind) {
        (Platform::Linux, _) => vec![
            DesktopOpenCommand {
                program: "gio",
                arguments: vec![OsString::from("open"), target.clone()],
            },
            DesktopOpenCommand {
                program: "xdg-open",
                arguments: vec![target],
            },
        ],
        (Platform::MacOs, _) => vec![DesktopOpenCommand {
            program: "open",
            arguments: vec![target],
        }],
        (Platform::Windows, DesktopTargetKind::Url) => vec![DesktopOpenCommand {
            program: "rundll32.exe",
            arguments: vec![OsString::from("url.dll,FileProtocolHandler"), target],
        }],
        (Platform::Windows, DesktopTargetKind::Directory) => vec![DesktopOpenCommand {
            program: "explorer.exe",
            arguments: vec![target],
        }],
        _ => Vec::new(),
    }
}

async fn open_desktop_target(kind: DesktopTargetKind, target: &OsStr) -> Result<(), String> {
    let platform = Platform::current()
        .ok_or_else(|| "Desktop opener is unsupported on this platform".to_owned())?;
    let attempts = desktop_open_commands(platform, kind, target);
    let mut last_error = None;
    for attempt in attempts {
        match tokio::process::Command::new(attempt.program)
            .args(&attempt.arguments)
            .status()
            .await
        {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                last_error = Some(format!("{} exited with status {status}", attempt.program));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => last_error = Some(format!("{} failed: {error}", attempt.program)),
        }
    }

    Err(last_error.unwrap_or_else(|| format!("No desktop opener was found for {platform:?}")))
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
        crate::content::ActiveContentPage::Health => iced::widget::Id::new("health-search"),
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
    pub scope: PackageScope,
    pub origin: Option<&'a PackageOrigin>,
    pub is_loading: bool,
    pub detail_error: Option<&'a str>,
}

#[derive(Debug, Clone, Default)]
pub struct PackageDetailState {
    generation: u64,
    key: Option<PackageSelectionKey>,
    request: Option<(u64, PackageSelectionKey)>,
    package: Option<PackageInfo>,
    error: Option<String>,
}

impl PackageDetailState {
    pub fn begin(&mut self, key: PackageSelectionKey) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.key = Some(key.clone());
        self.request = Some((self.generation, key));
        self.package = None;
        self.error = None;
        self.generation
    }

    pub fn finish(
        &mut self,
        generation: u64,
        key: PackageSelectionKey,
        result: Result<Option<PackageInfo>, String>,
    ) -> bool {
        if self.request.as_ref() != Some(&(generation, key.clone())) {
            return false;
        }
        self.request = None;
        match result {
            Ok(package) => self.package = package,
            Err(error) => self.error = Some(error),
        }
        true
    }

    pub fn package(&self, key: &PackageSelectionKey) -> Option<&PackageInfo> {
        (self.key.as_ref() == Some(key))
            .then_some(self.package.as_ref())
            .flatten()
    }

    pub fn is_loading(&self, key: &PackageSelectionKey) -> bool {
        self.request
            .as_ref()
            .is_some_and(|(_, request_key)| request_key == key)
    }

    pub fn error(&self, key: &PackageSelectionKey) -> Option<&str> {
        (self.key.as_ref() == Some(key))
            .then_some(self.error.as_deref())
            .flatten()
    }
}

pub struct ManagerSectionStyle {
    pub accent: iced::Color,
    pub error_prefix: &'static str,
}

pub fn selection_key(manager: &ManagerId, package_name: &str) -> PackageSelectionKey {
    (manager.clone(), package_name.to_owned())
}

pub async fn load_package_info(
    registry: Arc<ManagerRegistry>,
    config: ManagerConfig,
    target: PackageTarget,
) -> Result<Option<PackageInfo>, String> {
    let manager = registry
        .get(&target.manager_id)
        .ok_or_else(|| format!("manager is not registered: {}", target.manager_id))?;
    manager
        .package_info(&config, &target)
        .await
        .map_err(|error| error.to_string())
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
    manager_groups: &'a [(ManagerId, Vec<PackageTarget>)],
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
                text(
                    packages
                        .iter()
                        .map(|package| package.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
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

pub fn configured_managers_with_capability(
    pm_config: &Config,
    catalog: &ManagerCatalog,
    capability: ManagerCapability,
) -> Vec<ManagerId> {
    configured_managers(pm_config)
        .into_iter()
        .filter(|manager| {
            catalog
                .descriptor(manager)
                .is_some_and(|descriptor| descriptor.capabilities().contains(capability))
        })
        .collect()
}

/// Returns whether manager metadata matches a case-insensitive UI filter.
pub fn manager_matches_query(manager: &ManagerId, catalog: &ManagerCatalog, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }

    let display_name = catalog.display_name(manager);
    let description = catalog
        .descriptor(manager)
        .map_or("", |descriptor| descriptor.description());
    display_name.to_lowercase().contains(&query)
        || description.to_lowercase().contains(&query)
        || manager.as_str().to_lowercase().contains(&query)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerSourceStatus {
    Ready,
    Loading,
    Initializing,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ManagerSourceEntry {
    pub manager: ManagerId,
    pub count: Option<usize>,
    pub status: ManagerSourceStatus,
}

pub struct ManagerSourcePickerState<'a> {
    pub selected_managers: &'a HashSet<ManagerId>,
    pub expanded: bool,
    pub query: &'a str,
    pub count_label: &'static str,
    pub disabled: bool,
}

pub struct ManagerSourcePickerMessages<Message> {
    pub toggle_picker: Message,
    pub query_changed: fn(String) -> Message,
    pub set_visible_selection: fn(Vec<ManagerId>, bool) -> Message,
    pub toggle_manager: fn(ManagerId, bool) -> Message,
}

pub fn manager_source_picker<'a, Message>(
    entries: Vec<ManagerSourceEntry>,
    catalog: &'a ManagerCatalog,
    state: ManagerSourcePickerState<'a>,
    messages: ManagerSourcePickerMessages<Message>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let total_count = entries.len();
    let selected_count = entries
        .iter()
        .filter(|entry| state.selected_managers.contains(&entry.manager))
        .count();
    let preview = row(entries
        .iter()
        .filter(|entry| state.selected_managers.contains(&entry.manager))
        .take(4)
        .map(|entry| {
            icon::manager_logo(&entry.manager, catalog.display_name(&entry.manager), 24.0)
        }))
    .spacing(4)
    .align_y(Alignment::Center);

    let summary = if selected_count == 0 {
        "No sources selected".to_owned()
    } else {
        format!("{selected_count} of {total_count} sources")
    };
    let trigger = button(
        row![
            preview,
            text(summary)
                .size(13)
                .font(theme::FONT_SEMIBOLD)
                .width(Length::Fill),
            text(if state.expanded { "▴" } else { "▾" })
                .size(14)
                .font(theme::FONT_SEMIBOLD)
                .style(theme::text_on_surface_muted),
        ]
        .spacing(theme::spacing::SM)
        .align_y(Alignment::Center),
    )
    .padding([7, 10])
    .width(Length::Fill)
    .style(theme::source_picker_button(state.expanded))
    .on_press(messages.toggle_picker);

    if !state.expanded {
        return trigger.into();
    }

    let mut groups = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut visible_selectable = Vec::new();
    for entry in entries {
        if !manager_matches_query(&entry.manager, catalog, state.query) {
            continue;
        }
        if matches!(
            entry.status,
            ManagerSourceStatus::Ready | ManagerSourceStatus::Failed
        ) {
            visible_selectable.push(entry.manager.clone());
        }
        let category = catalog
            .descriptor(&entry.manager)
            .map_or(ManagerCategory::Other, |descriptor| descriptor.category());
        groups[manager_category_rank(category)].push(entry);
    }
    let visible_count = groups.iter().map(Vec::len).sum::<usize>();
    let visible_selected_count = visible_selectable
        .iter()
        .filter(|manager| state.selected_managers.contains(*manager))
        .count();

    let search = text_input("Filter sources...", state.query)
        .on_input(messages.query_changed)
        .padding([8, 10])
        .size(13)
        .style(theme::text_input_style);
    let select_all_button = picker_action_button(
        "Select shown",
        (!state.disabled && visible_selected_count < visible_selectable.len())
            .then(|| (messages.set_visible_selection)(visible_selectable.clone(), true)),
    );
    let clear_button = picker_action_button(
        "Clear shown",
        (!state.disabled && visible_selected_count > 0)
            .then(|| (messages.set_visible_selection)(visible_selectable, false)),
    );
    let controls = row![
        select_all_button,
        clear_button,
        text(format!("{visible_count} shown"))
            .size(12)
            .style(theme::text_on_surface_muted)
            .width(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(theme::spacing::SM)
    .align_y(Alignment::Center);

    let mut group_elements = Vec::new();
    for (index, group) in groups.into_iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        let category = match index {
            0 => ManagerCategory::System,
            1 => ManagerCategory::Application,
            2 => ManagerCategory::Development,
            _ => ManagerCategory::Other,
        };
        let rows = column(group.into_iter().map(|entry| {
            manager_source_row(
                entry,
                state.selected_managers,
                catalog,
                state.count_label,
                state.disabled,
                messages.toggle_manager,
            )
        }))
        .spacing(2);
        group_elements.push(
            column![
                text(manager_category_label(category))
                    .size(11)
                    .font(theme::FONT_SEMIBOLD)
                    .style(theme::text_on_surface_muted),
                rows,
            ]
            .spacing(4)
            .into(),
        );
    }
    if group_elements.is_empty() {
        group_elements.push(
            container(
                text("No package managers match this filter")
                    .size(13)
                    .style(theme::text_on_surface_muted),
            )
            .padding([16, 10])
            .width(Length::Fill)
            .into(),
        );
    }

    let list = scrollable(column(group_elements).spacing(theme::spacing::MD))
        .height(Length::Fixed(280.0))
        .spacing(theme::spacing::XS)
        .style(theme::scrollable_style);
    let panel = container(column![search, controls, list].spacing(theme::spacing::SM))
        .padding(theme::spacing::SM)
        .width(Length::Fill)
        .style(theme::source_picker_panel);

    column![trigger, panel]
        .spacing(theme::spacing::XS)
        .width(Length::Fill)
        .into()
}

fn manager_source_row<'a, Message>(
    entry: ManagerSourceEntry,
    selected_managers: &'a HashSet<ManagerId>,
    catalog: &'a ManagerCatalog,
    count_label: &'static str,
    globally_disabled: bool,
    on_toggle: fn(ManagerId, bool) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let manager = entry.manager;
    let display_name = catalog.display_name(&manager).to_owned();
    let selected = selected_managers.contains(&manager);
    let row_disabled = globally_disabled
        || matches!(
            entry.status,
            ManagerSourceStatus::Loading | ManagerSourceStatus::Initializing
        );
    let detail_style: fn(&iced::Theme) -> iced::widget::text::Style = match entry.status {
        ManagerSourceStatus::Ready => theme::text_on_surface_muted,
        ManagerSourceStatus::Loading => theme::text_accent,
        ManagerSourceStatus::Initializing => theme::text_on_surface_alt,
        ManagerSourceStatus::Failed => theme::text_error,
    };
    // The descriptor decides this, never a manager-ID list in the UI.
    let exact_lookup = manager_exact_lookup(&manager, catalog);
    let detail = source_detail_with_lookup(
        manager_source_detail(entry.status, entry.count, count_label),
        exact_lookup,
    );
    let checkbox: Element<'_, Message> = iced::widget::checkbox(selected)
        .size(18)
        .style(checkbox_style(row_disabled))
        .into();

    let content = row![
        icon::manager_logo(&manager, &display_name, 28.0),
        column![
            text(display_name)
                .size(13)
                .font(theme::FONT_SEMIBOLD)
                .style(theme::text_on_surface),
            text(detail).size(12).style(detail_style),
        ]
        .spacing(1)
        .width(Length::Fill),
        checkbox,
    ]
    .spacing(theme::spacing::SM)
    .align_y(Alignment::Center);

    if row_disabled {
        return container(content)
            .padding([6, 8])
            .width(Length::Fill)
            .into();
    }

    button(content)
        .padding([6, 8])
        .width(Length::Fill)
        .style(theme::list_row(selected))
        .on_press(on_toggle(manager, !selected))
        .into()
}

fn picker_action_button<'a, Message>(
    label: &'static str,
    message: Option<Message>,
) -> iced::widget::Button<'a, Message>
where
    Message: Clone + 'a,
{
    let enabled = message.is_some();
    let action = button(text(label).size(12))
        .padding([5, 8])
        .style(theme::secondary_button(enabled));
    if let Some(message) = message {
        action.on_press(message)
    } else {
        action
    }
}

pub const fn manager_category_label(category: ManagerCategory) -> &'static str {
    match category {
        ManagerCategory::System => "System",
        ManagerCategory::Application => "Applications",
        ManagerCategory::Development => "Development",
        ManagerCategory::Other => "Other",
        _ => "Other",
    }
}

const fn manager_category_rank(category: ManagerCategory) -> usize {
    match category {
        ManagerCategory::System => 0,
        ManagerCategory::Application => 1,
        ManagerCategory::Development => 2,
        ManagerCategory::Other => 3,
        _ => 3,
    }
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
    on_retry_info: Option<Message>,
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
    if package.is_loading {
        details = details.push(
            text("Loading package information...")
                .size(12)
                .style(theme::text_accent),
        );
    }
    if let Some(error) = package.detail_error {
        let mut error_details = column![
            text(format!("Package information could not be loaded: {error}"))
                .size(12)
                .style(theme::text_error)
                .wrapping(text::Wrapping::WordOrGlyph)
        ]
        .spacing(theme::spacing::XS);
        if let Some(message) = on_retry_info {
            error_details = error_details.push(
                button(text("Retry").size(12))
                    .padding([6, 10])
                    .style(theme::secondary_button(true))
                    .on_press(message),
            );
        }
        details = details.push(error_details);
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
    details = details.push(inspector_field(
        "Scope",
        package_scope_label(package.scope),
        false,
    ));
    details = details.push(inspector_field(
        "Manager ID",
        package.manager.as_str(),
        true,
    ));
    if let Some(origin) = package.origin {
        if !origin.name.trim().is_empty() {
            details = details.push(inspector_field("Source", origin.name.as_str(), false));
        }
        if let Some(reference) = origin
            .reference
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            details = details.push(inspector_field("Source reference", reference, true));
        }
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

fn package_scope_label(scope: PackageScope) -> &'static str {
    match scope {
        PackageScope::System => "System",
        PackageScope::User => "User",
        PackageScope::Project => "Project",
        PackageScope::Unknown => "Unknown",
        _ => "Unknown",
    }
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

/// Headline for a page with no configured package managers.
pub const NO_MANAGERS_CONFIGURED: &str = "No package managers are configured";

/// Hint pointing at where package managers are configured.
pub const NO_MANAGERS_HINT: &str =
    "Open the Package Managers page and click \"Scan $PATH\" to discover available managers.";

/// Hint for a page whose sources exist but none of them is selected.
pub const NO_SOURCE_SELECTED_HINT: &str = "Select at least one source above";

/// Why a package page has no list to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyState {
    /// No package manager is configured, so there is nothing to select.
    NoManagersConfigured,
    /// Managers are configured but none is ticked as a source.
    NoSourceSelected,
}

/// Resolves the empty state a package page should show.
///
/// `configured` is the number of configured package managers and `selected`
/// the number of those ticked as sources. Nothing configured is reported as
/// such rather than asking the user to select something that does not exist.
#[must_use]
pub fn empty_state(configured: usize, selected: usize) -> Option<EmptyState> {
    if configured == 0 {
        Some(EmptyState::NoManagersConfigured)
    } else if selected == 0 {
        Some(EmptyState::NoSourceSelected)
    } else {
        None
    }
}

/// Renders the empty state for a package page.
///
/// Returns `None` when the page has configured and selected sources, so the
/// caller keeps rendering its normal content. `open_managers` navigates to
/// the Package Managers page, which is where the empty configuration is fixed.
pub fn empty_state_view<'a, Message>(
    state: EmptyState,
    open_managers: Message,
) -> Option<Element<'a, Message>>
where
    Message: Clone + 'a,
{
    match state {
        EmptyState::NoSourceSelected => Some(centered_message(NO_SOURCE_SELECTED_HINT)),
        EmptyState::NoManagersConfigured => Some(
            container(
                column![
                    text(NO_MANAGERS_CONFIGURED)
                        .size(16)
                        .font(theme::FONT_SEMIBOLD)
                        .style(theme::text_on_surface),
                    text(NO_MANAGERS_HINT)
                        .size(13)
                        .style(theme::text_on_surface_muted),
                    button(text("Open Managers").size(13).font(theme::FONT_SEMIBOLD))
                        .padding([7, 12])
                        .style(theme::secondary_button(true))
                        .on_press(open_managers),
                ]
                .spacing(theme::spacing::MD)
                .align_x(iced::Alignment::Center)
                .width(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into(),
        ),
    }
}

/// Qualifier shown for a source that resolves an exact identifier.
pub const EXACT_LOOKUP_LABEL: &str = "exact identifier lookup";

/// Label describing an exact-identifier source, or `None` for a catalog source.
#[must_use]
pub fn exact_lookup_label(exact_lookup: bool) -> Option<&'static str> {
    exact_lookup.then_some(EXACT_LOOKUP_LABEL)
}

/// Returns whether a manager's descriptor declares an exact identifier lookup.
#[must_use]
pub fn manager_exact_lookup(manager: &ManagerId, catalog: &ManagerCatalog) -> bool {
    catalog
        .descriptor(manager)
        .is_some_and(ManagerDescriptor::exact_lookup)
}

/// Base detail line for one source row in the source picker.
#[must_use]
pub fn manager_source_detail(
    status: ManagerSourceStatus,
    count: Option<usize>,
    count_label: &str,
) -> String {
    match status {
        ManagerSourceStatus::Ready => count.map_or_else(
            || "Ready".to_owned(),
            |count| format!("{count} {count_label}"),
        ),
        ManagerSourceStatus::Loading => "Loading...".to_owned(),
        ManagerSourceStatus::Initializing => "Initializing...".to_owned(),
        ManagerSourceStatus::Failed => "Failed".to_owned(),
    }
}

/// Appends the exact-identifier qualifier when the source needs one.
#[must_use]
pub fn source_detail_with_lookup(detail: String, exact_lookup: bool) -> String {
    exact_lookup_label(exact_lookup).map_or(detail.clone(), |label| format!("{detail} · {label}"))
}

/// Hint for the Discover empty state naming the selected exact-identifier
/// sources, or `None` when every selected source is an ordinary catalog search.
pub fn exact_lookup_search_hint(
    sources: impl IntoIterator<Item = (String, bool)>,
) -> Option<String> {
    let names: Vec<String> = sources
        .into_iter()
        .filter(|(_, exact_lookup)| *exact_lookup)
        .map(|(name, _)| name)
        .collect();
    (!names.is_empty()).then(|| {
        format!(
            "{} only accept{} an exact identifier, not a catalog search.",
            names.join(", "),
            if names.len() == 1 { "s" } else { "" },
        )
    })
}

/// Separates the readable summary of a described error from its diagnostic detail.
const ERROR_DETAIL_SEPARATOR: &str = "\n\n";

/// Height cap for diagnostic detail inside an error card.
const ERROR_DETAIL_MAX_HEIGHT: f32 = 140.0;

/// Describes a read-path [`ManagerError`] for the user.
///
/// The text leads with an actionable sentence for the error kind, follows with the
/// manager message, and ends with the bounded detail (command and stderr tail)
/// after [`ERROR_DETAIL_SEPARATOR`], which [`error_card`] renders as monospace.
pub fn describe_manager_error(error: &ManagerError) -> String {
    let lead = match error.kind() {
        ManagerErrorKind::Cancelled => "Authorization was cancelled.",
        ManagerErrorKind::Permission => "Administrator permission was denied.",
        ManagerErrorKind::Network => "The network is unavailable; check the connection and retry.",
        ManagerErrorKind::Busy => {
            "Another program holds the package database lock; try again when it finishes."
        }
        ManagerErrorKind::Timeout => "The package manager did not finish in time; try again.",
        ManagerErrorKind::CommandMissing => {
            "Command not found; check the path on the Package Managers page."
        }
        _ => "The package manager reported an error.",
    };
    let mut description = format!("{lead}\n{}", error.message());
    if let Some(detail) = error
        .detail()
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
    {
        description.push_str(ERROR_DETAIL_SEPARATOR);
        description.push_str(detail);
    }
    description
}

fn split_error_detail(error: &str) -> (&str, Option<&str>) {
    match error.split_once(ERROR_DETAIL_SEPARATOR) {
        Some((summary, detail)) if !detail.trim().is_empty() => (summary, Some(detail)),
        _ => (error, None),
    }
}

pub fn error_card<'a, Message>(
    title: impl Into<String>,
    detail: &'a str,
    retry: Message,
    copy: impl FnOnce(String) -> Message,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    let title = title.into();
    let copy = copy(format!("{title}\n{detail}"));
    let (summary, diagnostics) = split_error_detail(detail);
    let mut body = column![
        text(title)
            .size(14)
            .font(theme::FONT_SEMIBOLD)
            .style(theme::text_on_surface),
        text(summary)
            .size(13)
            .style(theme::text_on_surface_muted)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(theme::spacing::XS)
    .width(iced::Length::Fill);
    if let Some(diagnostics) = diagnostics {
        body = body.push(
            container(
                scrollable(
                    text(diagnostics)
                        .size(12)
                        .font(theme::FONT_MONO)
                        .style(theme::text_on_surface_alt)
                        .width(iced::Length::Fill)
                        .wrapping(text::Wrapping::WordOrGlyph),
                )
                .width(iced::Length::Fill),
            )
            .padding(theme::spacing::SM)
            .width(iced::Length::Fill)
            .max_height(ERROR_DETAIL_MAX_HEIGHT)
            .style(theme::surface_container),
        );
    }

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
        body,
        column![
            button(text("Retry").size(13).font(theme::FONT_SEMIBOLD))
                .padding([7, 12])
                .style(theme::secondary_button(true))
                .on_press(retry),
            button(text("Copy").size(13))
                .padding([7, 12])
                .style(theme::secondary_button(true))
                .on_press(copy),
        ]
        .spacing(theme::spacing::XS),
    ]
    .spacing(theme::spacing::MD)
    .align_y(iced::Alignment::Start);

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

/// Height cap for a package-operation failure's diagnostic block.
const OPERATION_DETAIL_MAX_HEIGHT: f32 = 96.0;

/// User-facing result of a finished package operation.
///
/// A cancellation the user asked for is not a failure, so it is reported as
/// [`OperationNotice::Stopped`] with neutral wording rather than an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationNotice {
    /// The user stopped the operation before it finished.
    Stopped {
        /// Action the operation was performing.
        action: PackageAction,
        /// Packages finished before the stop.
        completed: usize,
        /// Packages the operation covered.
        total: usize,
    },
    /// The operation failed.
    Failed {
        /// Action the operation was performing.
        action: PackageAction,
        /// Manager that failed, when the failure came from one manager.
        manager: Option<ManagerId>,
        /// Failure message reported by the execution layer.
        message: String,
    },
}

impl OperationNotice {
    /// Classifies a finished operation, or `None` when it succeeded.
    #[must_use]
    pub fn from_outcome(outcome: &OperationOutcome) -> Option<Self> {
        if outcome.is_cancelled() {
            return Some(Self::Stopped {
                action: outcome.action,
                completed: outcome.completed_packages,
                total: outcome.total_packages,
            });
        }
        if outcome.is_success() {
            return None;
        }
        Some(Self::Failed {
            action: outcome.action,
            manager: outcome.failed_manager.clone(),
            message: outcome.error.clone().unwrap_or_else(|| outcome.summary()),
        })
    }

    /// Builds a failure that never reached a manager, such as a stale plan.
    #[must_use]
    pub fn failed(action: PackageAction, message: String) -> Self {
        Self::Failed {
            action,
            manager: None,
            message,
        }
    }

    /// Returns whether the user stopped this operation.
    #[must_use]
    pub const fn is_stopped(&self) -> bool {
        matches!(self, Self::Stopped { .. })
    }
}

/// Verb for the action the operation was performing.
const fn operation_verb(action: PackageAction) -> &'static str {
    match action {
        PackageAction::Install => "install",
        PackageAction::Update => "update",
        PackageAction::Uninstall => "remove",
        _ => "process",
    }
}

/// Noun for the action the operation was performing.
const fn operation_noun(action: PackageAction) -> &'static str {
    match action {
        PackageAction::Install => "Install",
        PackageAction::Update => "Update",
        PackageAction::Uninstall => "Removal",
        _ => "Operation",
    }
}

/// First line of an operation notice, naming the manager when one is known.
#[must_use]
pub fn operation_notice_headline(notice: &OperationNotice, catalog: &ManagerCatalog) -> String {
    match notice {
        OperationNotice::Stopped {
            action,
            completed,
            total,
        } => format!(
            "{} stopped after {completed} of {total} packages",
            operation_noun(*action)
        ),
        OperationNotice::Failed {
            action,
            manager: Some(manager),
            ..
        } => format!(
            "{}: {} failed",
            catalog.display_name(manager),
            operation_verb(*action)
        ),
        OperationNotice::Failed { action, .. } => {
            format!("{} failed", operation_noun(*action))
        }
    }
}

/// Failure text with the execution layer's manager-ID envelope removed.
fn strip_manager_envelope(message: &str, action: PackageAction, manager: &ManagerId) -> String {
    let prefix = format!(
        "Failed to {} packages from {manager}: ",
        operation_verb(action)
    );
    message
        .strip_prefix(&prefix)
        .map_or_else(|| message.to_owned(), std::borrow::ToOwned::to_owned)
}

/// Message body of an operation notice, bounded to the failure text itself.
#[must_use]
pub fn operation_notice_message(notice: &OperationNotice) -> Option<String> {
    let OperationNotice::Failed {
        action,
        manager,
        message,
    } = notice
    else {
        return None;
    };
    Some(manager.as_ref().map_or_else(
        || message.clone(),
        |manager| strip_manager_envelope(message, *action, manager),
    ))
}

/// Splits an operation failure into a leading line and bounded diagnostics.
///
/// Execution failures put the summary and the stderr tail on adjacent lines
/// rather than after [`ERROR_DETAIL_SEPARATOR`], so a single newline is also a
/// split point. Without one the whole message is the summary.
fn split_operation_detail(message: &str) -> (&str, Option<&str>) {
    let (summary, detail) = split_error_detail(message);
    if let Some(detail) = detail {
        return (summary, Some(detail));
    }
    match message.split_once('\n') {
        Some((summary, detail)) if !detail.trim().is_empty() => (summary, Some(detail)),
        _ => (message, None),
    }
}

/// Renders a finished package operation for the page's action area.
///
/// A user-stopped operation gets a single neutral line and only a dismiss
/// control. A real failure names the manager by display name and keeps its
/// stderr in a bounded, scrollable monospace block with copy and dismiss,
/// instead of dumping an unbounded error string inline.
pub fn operation_notice_card<'a, Message>(
    notice: &'a OperationNotice,
    catalog: &ManagerCatalog,
    copy: impl FnOnce(String) -> Message,
    dismiss: Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let headline = operation_notice_headline(notice, catalog);
    let dismiss_button = button(text("Dismiss").size(13))
        .padding([7, 12])
        .style(theme::secondary_button(true))
        .on_press(dismiss);

    if notice.is_stopped() {
        return container(
            row![
                text(headline)
                    .size(13)
                    .style(theme::text_warning)
                    .width(iced::Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
                dismiss_button,
            ]
            .spacing(theme::spacing::MD)
            .align_y(iced::Alignment::Center),
        )
        .padding(theme::spacing::SM)
        .width(iced::Length::Fill)
        .style(theme::surface_container)
        .into();
    }

    let message = operation_notice_message(notice).unwrap_or_default();
    let copy = copy(format!("{headline}\n{message}"));
    // The text widgets own their strings so the card can borrow no local state.
    let (summary, diagnostics) = split_operation_detail(&message);
    let summary = summary.to_owned();
    let diagnostics = diagnostics.map(str::to_owned);
    let mut body = column![
        text(headline)
            .size(14)
            .font(theme::FONT_SEMIBOLD)
            .style(theme::text_on_surface),
        text(summary)
            .size(13)
            .style(theme::text_on_surface_muted)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(theme::spacing::XS)
    .width(iced::Length::Fill);
    if let Some(diagnostics) = diagnostics {
        body = body.push(
            container(
                scrollable(
                    text(diagnostics)
                        .size(12)
                        .font(theme::FONT_MONO)
                        .style(theme::text_on_surface_alt)
                        .width(iced::Length::Fill)
                        .wrapping(text::Wrapping::WordOrGlyph),
                )
                .width(iced::Length::Fill),
            )
            .padding(theme::spacing::SM)
            .width(iced::Length::Fill)
            .height(iced::Length::Fixed(OPERATION_DETAIL_MAX_HEIGHT))
            .style(theme::surface_container),
        );
    }

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
        body,
        column![
            button(text("Copy").size(13))
                .padding([7, 12])
                .style(theme::secondary_button(true))
                .on_press(copy),
            dismiss_button,
        ]
        .spacing(theme::spacing::XS),
    ]
    .spacing(theme::spacing::MD)
    .align_y(iced::Alignment::Start);

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

// Retry and copy are per-section message constructors for the error card.
#[allow(clippy::too_many_arguments)]
pub fn manager_section<'a, Message>(
    manager: ManagerId,
    catalog: &'a ManagerCatalog,
    subtitle: String,
    style: ManagerSectionStyle,
    error: Option<&'a str>,
    retry: impl FnOnce() -> Message,
    copy_error: impl FnOnce(String) -> Message,
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
                retry(),
                copy_error,
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

pub fn empty_filter_view<'a, Message>(message: &'static str) -> Element<'a, Message>
where
    Message: 'a,
{
    column![text(message).size(14).style(theme::text_on_surface_muted)]
        .spacing(8)
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

/// Search-contract version reported for packages that are not installed.
pub const NOT_INSTALLED: &str = "Not Installed";

/// Returns whether a search result may be selected for installation.
pub fn is_installable_search_result(package: &PackageInfo) -> bool {
    package.version.trim() == NOT_INSTALLED
}

/// Error recorded for a source whose pending read was abandoned by the user.
pub fn stopped_waiting_error(source: &str) -> String {
    format!("{STOPPED_WAITING_PREFIX}{source}; a late response will be ignored.")
}

/// Prefix shared by every abandoned-source error.
pub const STOPPED_WAITING_PREFIX: &str = "Stopped waiting for ";

/// Returns whether `error` records a source the user stopped waiting for.
pub fn is_stopped_waiting_error(error: &str) -> bool {
    error.starts_with(STOPPED_WAITING_PREFIX)
}

/// Status line for sources still loading, with a control to stop waiting
/// when at least one of them can be abandoned.
pub fn pending_sources_notice<'a, Message>(
    label: String,
    stop_waiting: Option<Message>,
) -> Element<'a, Message>
where
    Message: 'a + Clone,
{
    let mut notice = row![
        text(label)
            .size(13)
            .style(theme::text_accent)
            .width(Length::Fill)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(theme::spacing::MD)
    .align_y(Alignment::Center);
    if let Some(message) = stop_waiting {
        notice = notice.push(
            button(text("Stop waiting").size(12).font(theme::FONT_SEMIBOLD))
                .padding([6, 10])
                .style(theme::secondary_button(true))
                .on_press(message),
        );
    }
    notice.into()
}

/// Segmented button whose label is owned by the caller.
pub fn segmented_button_owned<'a, Message>(
    label: String,
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
            .align_x(Alignment::Center),
    )
    .padding([7, 10])
    .width(Length::Fill)
    .style(theme::segmented_button(selected))
    .on_press(message)
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};

    use super::ManagerCatalog;
    use super::{
        DesktopOpenCommand, DesktopTargetKind, EmptyState, ManagerSourceStatus, OperationNotice,
        PackageDetailState, desktop_open_commands, empty_state, exact_lookup_label,
        exact_lookup_search_hint, is_installable_search_result, is_stopped_waiting_error,
        manager_source_detail, operation_notice_headline, operation_notice_message, selection_key,
        source_detail_with_lookup, stopped_waiting_error, validate_http_url,
    };
    use updater_manager_api::{
        ManagerCapability, ManagerConfig, ManagerError, ManagerErrorKind, ManagerId, PackageAction,
        PackageInfo, Platform,
    };

    fn manager_id(value: &str) -> ManagerId {
        ManagerId::parse(value).unwrap()
    }

    #[test]
    fn describe_manager_error_includes_kind_lead_and_detail() {
        let busy = ManagerError::new(ManagerErrorKind::Busy, "package manager command failed")
            .with_detail(
                "apt-get install failed:\nE: Could not get lock /var/lib/dpkg/lock - open\nE: Unable to lock",
            );
        let described = super::describe_manager_error(&busy);
        assert!(
            described.contains("Another program holds the package database lock"),
            "expected the lock-specific lead sentence, got: {described}"
        );
        assert!(described.contains("E: Could not get lock /var/lib/dpkg/lock"));
        assert!(described.contains("package manager command failed"));

        let (summary, detail) = super::split_error_detail(&described);
        assert!(summary.contains("Another program holds the package database lock"));
        assert_eq!(
            detail,
            Some(
                "apt-get install failed:\nE: Could not get lock /var/lib/dpkg/lock - open\nE: Unable to lock"
            )
        );

        let command_missing = ManagerError::new(
            ManagerErrorKind::CommandMissing,
            "package manager command failed",
        );
        assert!(
            super::describe_manager_error(&command_missing)
                .contains("check the path on the Package Managers page")
        );
    }

    #[test]
    fn describe_manager_error_leads_each_classification() {
        let cases = [
            (ManagerErrorKind::Cancelled, "Authorization was cancelled"),
            (
                ManagerErrorKind::Permission,
                "Administrator permission was denied",
            ),
            (ManagerErrorKind::Network, "The network is unavailable"),
            (
                ManagerErrorKind::Busy,
                "Another program holds the package database lock",
            ),
            (
                ManagerErrorKind::Timeout,
                "The package manager did not finish in time",
            ),
            (
                ManagerErrorKind::CommandMissing,
                "Command not found; check the path on the Package Managers page",
            ),
        ];

        for (kind, lead) in cases {
            let error = ManagerError::new(kind, "boom");
            let described = super::describe_manager_error(&error);
            assert!(
                described.starts_with(lead),
                "{kind:?} should lead with {lead:?}, got {described:?}"
            );
            assert!(described.contains("boom"));
            assert_eq!(super::split_error_detail(&described).1, None);
        }
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
    fn package_detail_state_rejects_stale_async_results() {
        let manager = manager_id("builtin:dnf");
        let first_key = selection_key(&manager, "first");
        let second_key = selection_key(&manager, "second");
        let mut state = PackageDetailState::default();

        let first_generation = state.begin(first_key.clone());
        let second_generation = state.begin(second_key.clone());
        let stale_package = PackageInfo::new(manager.clone(), "first", "1.0");
        let current_package = PackageInfo::new(manager, "second", "2.0");

        assert!(!state.finish(first_generation, first_key, Ok(Some(stale_package))));
        assert!(state.is_loading(&second_key));
        assert!(state.finish(
            second_generation,
            second_key.clone(),
            Ok(Some(current_package.clone())),
        ));
        assert_eq!(state.package(&second_key), Some(&current_package));
        assert!(!state.is_loading(&second_key));
    }

    #[test]
    fn inspector_rejects_unsafe_url_schemes() {
        assert!(validate_http_url("https://example.com/package").is_ok());
        assert!(validate_http_url("http://example.com/package").is_ok());
        assert!(validate_http_url("file:///etc/passwd").is_err());
        assert!(validate_http_url("javascript:alert(1)").is_err());
        assert!(validate_http_url("https://user:secret@example.com/").is_err());
    }

    #[test]
    fn manager_sources_are_filtered_by_advertised_capability() {
        let config = updater_core::Config {
            managers: ["builtin:cargo", "builtin:uv", "builtin:nix-profile"]
                .into_iter()
                .map(|id| ManagerConfig::new(manager_id(id)))
                .collect(),
            ..updater_core::Config::default()
        };
        let catalog = crate::manager_catalog::ManagerCatalog::builtin();

        assert_eq!(
            super::configured_managers_with_capability(
                &config,
                &catalog,
                ManagerCapability::Search,
            ),
            [manager_id("builtin:cargo")]
        );
        assert_eq!(
            super::configured_managers_with_capability(
                &config,
                &catalog,
                ManagerCapability::Updates,
            ),
            [manager_id("builtin:cargo"), manager_id("builtin:uv")]
        );
    }

    #[test]
    fn manager_query_matches_name_description_and_stable_id() {
        let catalog = crate::manager_catalog::ManagerCatalog::builtin();
        let cargo = manager_id("builtin:cargo");

        assert!(super::manager_matches_query(&cargo, &catalog, "cargo"));
        assert!(super::manager_matches_query(&cargo, &catalog, "rust"));
        assert!(super::manager_matches_query(
            &cargo,
            &catalog,
            "BUILTIN:CARGO"
        ));
        assert!(!super::manager_matches_query(&cargo, &catalog, "python"));
    }

    #[test]
    fn desktop_open_commands_are_platform_native_and_shell_free() {
        let url = OsStr::new("https://example.com/a?x=1&y=2");
        let directory = OsStr::new("C:\\Users\\A Y\\Downloads");

        assert_eq!(
            desktop_open_commands(Platform::Linux, DesktopTargetKind::Url, url),
            vec![
                DesktopOpenCommand {
                    program: "gio",
                    arguments: vec![OsString::from("open"), url.to_os_string()],
                },
                DesktopOpenCommand {
                    program: "xdg-open",
                    arguments: vec![url.to_os_string()],
                },
            ]
        );
        assert_eq!(
            desktop_open_commands(Platform::MacOs, DesktopTargetKind::Directory, directory),
            vec![DesktopOpenCommand {
                program: "open",
                arguments: vec![directory.to_os_string()],
            }]
        );
        assert_eq!(
            desktop_open_commands(Platform::Windows, DesktopTargetKind::Url, url),
            vec![DesktopOpenCommand {
                program: "rundll32.exe",
                arguments: vec![
                    OsString::from("url.dll,FileProtocolHandler"),
                    url.to_os_string(),
                ],
            }]
        );
        assert_eq!(
            desktop_open_commands(Platform::Windows, DesktopTargetKind::Directory, directory,),
            vec![DesktopOpenCommand {
                program: "explorer.exe",
                arguments: vec![directory.to_os_string()],
            }]
        );
    }

    #[test]
    fn installability_follows_the_sentinel_contract() {
        let manager = manager_id("builtin:snap");
        let mut not_installed = PackageInfo::new(manager.clone(), "code", "  Not Installed  ");
        assert!(is_installable_search_result(&not_installed));

        not_installed.version = "1.99".to_owned();
        assert!(!is_installable_search_result(&not_installed));

        not_installed.version = "unknown".to_owned();
        assert!(!is_installable_search_result(&not_installed));
    }

    #[test]
    fn only_abandoned_source_errors_are_recognized() {
        assert!(is_stopped_waiting_error(&stopped_waiting_error("Snap")));
        assert!(!is_stopped_waiting_error("network unreachable"));
        assert!(!is_stopped_waiting_error("Failed to search in Snap"));
    }

    #[test]
    fn empty_state_separates_missing_managers_from_missing_selection() {
        assert_eq!(empty_state(0, 0), Some(EmptyState::NoManagersConfigured));
        assert_eq!(empty_state(2, 0), Some(EmptyState::NoSourceSelected));
        assert_eq!(empty_state(2, 1), None);
        assert_eq!(empty_state(2, 2), None);
    }

    #[test]
    fn exact_lookup_label_follows_the_descriptor_flag() {
        assert_eq!(exact_lookup_label(true), Some(super::EXACT_LOOKUP_LABEL));
        assert_eq!(exact_lookup_label(false), None);

        let exact = source_detail_with_lookup(
            manager_source_detail(ManagerSourceStatus::Ready, Some(3), "results"),
            true,
        );
        assert!(exact.contains("3 results"));
        assert!(exact.contains(super::EXACT_LOOKUP_LABEL));

        let catalog = source_detail_with_lookup(
            manager_source_detail(ManagerSourceStatus::Ready, Some(3), "results"),
            false,
        );
        assert_eq!(catalog, "3 results");
    }

    #[test]
    fn exact_lookup_search_hint_names_only_exact_sources() {
        assert_eq!(exact_lookup_search_hint(Vec::new()), None);
        assert_eq!(
            exact_lookup_search_hint(vec![("Cargo".to_owned(), false)]),
            None
        );
        let single = exact_lookup_search_hint(vec![("Go".to_owned(), true)]).expect("hint");
        assert!(single.starts_with("Go only accepts an exact identifier"));
        let multiple = exact_lookup_search_hint(vec![
            ("Go".to_owned(), true),
            ("pipx".to_owned(), true),
            ("Cargo".to_owned(), false),
        ])
        .expect("hint");
        assert!(multiple.contains("Go, pipx"));
        assert!(!multiple.contains("Cargo"));
    }

    #[test]
    fn cancelled_operation_is_reported_as_stopped_not_failed() {
        let outcome = updater_core::OperationOutcome {
            action: PackageAction::Update,
            completed_packages: 2,
            total_packages: 5,
            completed_managers: 1,
            total_managers: 2,
            failed_manager: None,
            error: Some("pkexec authentication failed".to_owned()),
            cancelled: true,
            manager_outcomes: Vec::new(),
            scope: updater_manager_api::PackageScope::System,
        };

        let notice = OperationNotice::from_outcome(&outcome).expect("a stopped notice");
        assert!(notice.is_stopped());
        assert_eq!(operation_notice_message(&notice), None);
        let headline = operation_notice_headline(&notice, &ManagerCatalog::builtin());
        assert_eq!(headline, "Update stopped after 2 of 5 packages");
        assert!(!headline.to_lowercase().contains("fail"));
    }

    #[test]
    fn failed_operation_uses_the_manager_display_name_and_hides_the_id() {
        let manager = manager_id("builtin:apt");
        let outcome = updater_core::OperationOutcome {
            action: PackageAction::Update,
            completed_packages: 0,
            total_packages: 2,
            completed_managers: 0,
            total_managers: 1,
            failed_manager: Some(manager.clone()),
            error: Some(
                "Failed to update packages from builtin:apt: package manager command failed: /usr/bin/pkexec failed:\nE: boom\nE: bang"
                    .to_owned(),
            ),
            cancelled: false,
            manager_outcomes: Vec::new(),
            scope: updater_manager_api::PackageScope::System,
        };

        let notice = OperationNotice::from_outcome(&outcome).expect("a failure notice");
        assert!(!notice.is_stopped());
        let headline = operation_notice_headline(&notice, &ManagerCatalog::builtin());
        assert_eq!(headline, "APT: update failed");
        assert!(!headline.contains("builtin:apt"));

        let message = operation_notice_message(&notice).expect("failure detail");
        assert!(!message.contains("builtin:apt"));
        assert!(message.starts_with("package manager command failed"));
    }

    #[test]
    fn successful_operation_has_no_notice() {
        let outcome = updater_core::OperationOutcome {
            action: PackageAction::Install,
            completed_packages: 1,
            total_packages: 1,
            completed_managers: 1,
            total_managers: 1,
            failed_manager: None,
            error: None,
            cancelled: false,
            manager_outcomes: Vec::new(),
            scope: updater_manager_api::PackageScope::User,
        };

        assert_eq!(OperationNotice::from_outcome(&outcome), None);
    }

    #[test]
    fn operation_failure_detail_splits_on_a_single_newline() {
        let notice = OperationNotice::failed(
            PackageAction::Uninstall,
            "package manager command failed:\nE: boom\nE: bang".to_owned(),
        );
        let headline = operation_notice_headline(&notice, &ManagerCatalog::builtin());
        assert_eq!(headline, "Removal failed");
        let message = operation_notice_message(&notice).expect("failure detail");
        let (summary, detail) = super::split_operation_detail(&message);
        assert_eq!(summary, "package manager command failed:");
        assert_eq!(detail, Some("E: boom\nE: bang"));
    }
}
