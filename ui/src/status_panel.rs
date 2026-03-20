//! Status panel module.
//!
//! This module owns both the status panel UI rendering and its local animation state.
//! It follows the same `Message`/`Action`/`update`/`view` pattern used by other UI modules.

use std::time::{Duration, Instant};

use iced::{Animation, Border, Length, Subscription, Task};

use crate::{
    app::{self, colors},
    content::{FindingInfo, InstalledInfo, UpdatesInfo},
};

/// Stateful bottom panel that presents overall progress and command output.
#[derive(Debug, Clone)]
pub struct StatusPanel {
    /// Progress bar animation state.
    progress_animation: Animation<f32>,
    /// Target progress value for animation.
    progress_target: f32,
    /// Last frame/update timestamp.
    last_frame: Instant,
    /// Current status text shown to user.
    status_label: String,
    /// Current interpolated progress value in [0, 1].
    progress: f32,
    /// Merged command logs displayed in panel.
    command_logs: Vec<String>,
    /// Aggregated known progress as `(done, total)`.
    progress_counts: Option<(usize, usize)>,
}

/// Messages handled by the status panel.
///
/// `Tick` comes from frame subscription. `Sync` is sent by `App` after
/// non-panel updates so the panel can recalculate animation targets.
#[derive(Debug, Clone, Copy)]
pub enum Message {
    /// Frame tick message from the window subscription.
    Tick(Instant),
    /// Sync message after non-panel state updates.
    Sync(Instant),
}

/// Side effects produced by [`StatusPanel::update`].
#[derive(Debug)]
#[allow(dead_code)]
pub enum Action {
    /// No-op action.
    None,
    /// Asynchronous task action.
    Run(Task<Message>),
}

impl From<Message> for app::Message {
    fn from(msg: Message) -> Self {
        app::Message::StatusPanel(msg)
    }
}

#[derive(Debug, Default)]
struct ProgressCounter {
    /// Aggregated total units of work.
    total: usize,
    /// Aggregated completed units of work.
    done: usize,
}

impl ProgressCounter {
    fn add(&mut self, total: usize, done: usize) {
        self.total += total;
        self.done += done.min(total);
    }
}

impl Message {
    fn at(self) -> Instant {
        match self {
            Message::Tick(at) | Message::Sync(at) => at,
        }
    }
}

impl StatusPanel {
    /// Creates a new status panel state at the given time anchor.
    pub fn new(now: Instant) -> Self {
        Self {
            progress_animation: Animation::new(0.0).duration(Duration::from_millis(280)),
            progress_target: 0.0,
            last_frame: now,
            status_label: "Idle".to_string(),
            progress: 1.0,
            command_logs: Vec::new(),
            progress_counts: None,
        }
    }

    /// Returns frame subscription while work or animation is active.
    pub fn subscription(
        &self,
        installed_info: &InstalledInfo,
        updates_info: &UpdatesInfo,
        finding_info: &FindingInfo,
    ) -> Subscription<Message> {
        if has_active_work(installed_info, updates_info, finding_info)
            || self.progress_animation.is_animating(self.last_frame)
        {
            iced::window::frames().map(Message::Tick)
        } else {
            Subscription::none()
        }
    }

    /// Updates internal animation state using the latest app data.
    pub fn update(
        &mut self,
        message: Message,
        installed_info: &InstalledInfo,
        updates_info: &UpdatesInfo,
        finding_info: &FindingInfo,
    ) -> Action {
        let at = message.at();
        let should_refresh_snapshot = matches!(message, Message::Sync(_));

        self.last_frame = at;
        self.sync_progress_animation(at, installed_info, updates_info, finding_info);
        self.progress = self.current_progress_value();

        if should_refresh_snapshot {
            self.status_label = status_label(installed_info, updates_info, finding_info);
            self.progress_counts = progress_counts(installed_info, updates_info, finding_info);
            rebuild_command_logs(
                &mut self.command_logs,
                installed_info,
                updates_info,
                finding_info,
            );
        }

        Action::None
    }

    /// Renders the status panel view.
    pub fn view<'a>(&'a self) -> iced::Element<'a, Message> {
        render(self)
    }

    fn sync_progress_animation(
        &mut self,
        at: Instant,
        installed_info: &InstalledInfo,
        updates_info: &UpdatesInfo,
        finding_info: &FindingInfo,
    ) {
        let target = progress_value(installed_info, updates_info, finding_info);
        if (self.progress_target - target).abs() > 0.001 {
            self.progress_target = target;
            self.progress_animation.go_mut(target, at);
        }
    }

    fn current_progress_value(&self) -> f32 {
        self.progress_animation
            .interpolate_with(|value| value, self.last_frame)
            .clamp(0.0, 1.0)
    }
}

fn has_active_work(
    installed_info: &InstalledInfo,
    updates_info: &UpdatesInfo,
    finding_info: &FindingInfo,
) -> bool {
    installed_info.is_loading_count
        || updates_info.is_loading_count
        || !installed_info.loading_installed.is_empty()
        || !updates_info.loading_updates.is_empty()
        || !finding_info.searching_managers.is_empty()
        || finding_info.is_installing
        || updates_info.is_updating
        || installed_info.is_removing
}

fn collect_known_progress(
    installed_info: &InstalledInfo,
    updates_info: &UpdatesInfo,
    finding_info: &FindingInfo,
) -> ProgressCounter {
    let mut known = ProgressCounter::default();

    if installed_info.is_loading_count
        && let Some((completed, total)) = installed_info.init_progress
        && total > 0
    {
        known.add(total, completed);
    }

    if updates_info.is_loading_count
        && let Some((completed, total)) = updates_info.init_progress
        && total > 0
    {
        known.add(total, completed);
    }

    if !finding_info.searching_managers.is_empty() {
        let total = finding_info.selected_managers.len();
        let searching = finding_info.searching_managers.len();
        known.add(total, total.saturating_sub(searching));
    }

    if !installed_info.loading_installed.is_empty() {
        let total = installed_info.selected_managers.len();
        let loading = installed_info.loading_installed.len();
        known.add(total, total.saturating_sub(loading));
    }

    if !updates_info.loading_updates.is_empty() {
        let total = updates_info.selected_managers.len();
        let loading = updates_info.loading_updates.len();
        known.add(total, total.saturating_sub(loading));
    }

    if finding_info.is_installing
        && let Some((completed, total, _, _)) = &finding_info.install_progress
        && *total > 0
    {
        known.add(*total, *completed);
    }

    if updates_info.is_updating
        && let Some((completed, total, _, _)) = &updates_info.update_progress
        && *total > 0
    {
        known.add(*total, *completed);
    }

    if installed_info.is_removing
        && let Some((completed, total, _, _)) = &installed_info.remove_progress
        && *total > 0
    {
        known.add(*total, *completed);
    }

    known
}

fn progress_value(
    installed_info: &InstalledInfo,
    updates_info: &UpdatesInfo,
    finding_info: &FindingInfo,
) -> f32 {
    let known = collect_known_progress(installed_info, updates_info, finding_info);

    if known.total > 0 {
        (known.done as f32 / known.total as f32).clamp(0.0, 1.0)
    } else if has_active_work(installed_info, updates_info, finding_info) {
        0.0
    } else {
        1.0
    }
}

fn progress_counts(
    installed_info: &InstalledInfo,
    updates_info: &UpdatesInfo,
    finding_info: &FindingInfo,
) -> Option<(usize, usize)> {
    let known = collect_known_progress(installed_info, updates_info, finding_info);
    if known.total > 0 {
        Some((known.done.min(known.total), known.total))
    } else {
        None
    }
}

fn rebuild_command_logs(
    out: &mut Vec<String>,
    installed_info: &InstalledInfo,
    updates_info: &UpdatesInfo,
    finding_info: &FindingInfo,
) {
    out.clear();
    if installed_info.is_loading_count {
        out.extend(installed_info.init_logs.iter().cloned());
    }
    if updates_info.is_loading_count {
        out.extend(updates_info.init_logs.iter().cloned());
    }
    if finding_info.is_installing {
        out.extend(finding_info.install_logs.iter().cloned());
    }
    if updates_info.is_updating {
        out.extend(updates_info.update_logs.iter().cloned());
    }
    if installed_info.is_removing {
        out.extend(installed_info.remove_logs.iter().cloned());
    }

    const MAX_PANEL_LOGS: usize = 120;
    if out.len() > MAX_PANEL_LOGS {
        let overflow = out.len() - MAX_PANEL_LOGS;
        out.drain(0..overflow);
    }
}

fn status_label(
    installed_info: &InstalledInfo,
    updates_info: &UpdatesInfo,
    finding_info: &FindingInfo,
) -> String {
    if installed_info.is_loading_count || updates_info.is_loading_count {
        return "Initializing package manager data...".to_string();
    }

    if finding_info.is_installing {
        return operation_status_label(
            "Installing",
            finding_info.install_progress.as_ref(),
            "Installing selected packages...",
        );
    }

    if updates_info.is_updating {
        return operation_status_label(
            "Updating",
            updates_info.update_progress.as_ref(),
            "Updating selected packages...",
        );
    }

    if installed_info.is_removing {
        return operation_status_label(
            "Removing",
            installed_info.remove_progress.as_ref(),
            "Removing selected packages...",
        );
    }

    if !finding_info.searching_managers.is_empty() {
        let total = finding_info.selected_managers.len();
        let searching = finding_info.searching_managers.len();
        let done = total.saturating_sub(searching);
        return format!("Searching packages ({}/{})...", done, total);
    }

    if !installed_info.loading_installed.is_empty() {
        let total = installed_info.selected_managers.len();
        let loading = installed_info.loading_installed.len();
        let done = total.saturating_sub(loading);
        return format!("Loading installed packages ({}/{})...", done, total);
    }

    if !updates_info.loading_updates.is_empty() {
        let total = updates_info.selected_managers.len();
        let loading = updates_info.loading_updates.len();
        let done = total.saturating_sub(loading);
        return format!("Loading updates ({}/{})...", done, total);
    }

    "Idle".to_string()
}

fn operation_status_label(
    verb: &str,
    progress: Option<&(usize, usize, updater_core::PackageManagerType, String)>,
    fallback: &str,
) -> String {
    if let Some((completed, total, manager, package)) = progress {
        if package.is_empty() {
            return format!("{verb} packages ({completed}/{total})...");
        }

        return format!("{verb} {completed}/{total}: {package} ({})", manager.name());
    }

    fallback.to_string()
}

fn render<'a, Message: 'a + 'static>(panel: &'a StatusPanel) -> iced::Element<'a, Message> {
    use iced::widget::{column, container, row, scrollable, text};

    let progress_widget = determinate_capsule_bar(panel.progress);

    let mut status_right = format!("{:.0}%", panel.progress * 100.0);
    if let Some((done, total)) = panel.progress_counts {
        status_right = format!("{}/{}", done, total);
    }

    let mut panel_content = column![
        row![
            text(&panel.status_label).size(14).color(colors::ON_SURFACE),
            text(status_right).size(13).color(colors::ON_SURFACE_MUTED)
        ]
        .align_y(iced::Alignment::Center)
        .spacing(12),
        progress_widget
    ]
    .spacing(10)
    .height(Length::Fill);

    if !panel.command_logs.is_empty() {
        let lines = panel.command_logs.iter().map(|line| {
            text(line)
                .size(12)
                .color(colors::ON_SURFACE_ALT)
                .width(Length::Fill)
                .into()
        });

        let log_list = scrollable(column(lines).spacing(4))
            .height(Length::Fill)
            .width(Length::Fill);

        panel_content = panel_content
            .push(
                text("Command Output")
                    .size(12)
                    .color(colors::ON_SURFACE_MUTED),
            )
            .push(log_list);
    }

    let panel_height = if panel.command_logs.is_empty() {
        86.0
    } else {
        250.0
    };

    container(panel_content)
        .padding([10, 16])
        .height(Length::Fixed(panel_height))
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(colors::SURFACE_ALT.into()),
            border: Border {
                color: colors::DIVIDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: None,
            shadow: Default::default(),
            snap: false,
        })
        .into()
}

fn determinate_capsule_bar<Message: 'static>(progress: f32) -> iced::Element<'static, Message> {
    use iced::widget::{Space, container, row};

    let travel_portion: u16 = 1000;
    let filled =
        ((progress.clamp(0.0, 1.0) * travel_portion as f32).round() as u16).min(travel_portion);
    let remaining = travel_portion.saturating_sub(filled);

    let capsule = container("")
        .width(if filled == 0 {
            Length::Shrink
        } else {
            Length::FillPortion(filled)
        })
        .height(Length::Fixed(8.0))
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(colors::SECONDARY_SOFT.into()),
            border: Border {
                color: iced::Color::from_rgba(
                    colors::SECONDARY_SOFT.r,
                    colors::SECONDARY_SOFT.g,
                    colors::SECONDARY_SOFT.b,
                    0.6,
                ),
                width: 1.0,
                radius: 999.0.into(),
            },
            text_color: None,
            shadow: iced::Shadow {
                color: iced::Color::from_rgba(
                    colors::SECONDARY_SOFT.r,
                    colors::SECONDARY_SOFT.g,
                    colors::SECONDARY_SOFT.b,
                    0.28,
                ),
                offset: iced::Vector::new(0.0, 0.0),
                blur_radius: 8.0,
            },
            snap: false,
        });

    let right_spacer = Space::new().width(if remaining == 0 {
        Length::Shrink
    } else {
        Length::FillPortion(remaining)
    });

    let bar = row![capsule, right_spacer]
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

    container(bar)
        .padding([4, 6])
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(colors::SURFACE_HOVER.into()),
            border: Border {
                color: colors::DIVIDER_LIGHT,
                width: 1.0,
                radius: 999.0.into(),
            },
            text_color: None,
            shadow: Default::default(),
            snap: false,
        })
        .into()
}
