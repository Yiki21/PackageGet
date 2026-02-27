//! Status panel module.
//!
//! This module owns both the status panel UI rendering and its local animation state.
//! It follows the same `Message`/`Action`/`update`/`view` pattern used by other UI modules.

use std::time::{Duration, Instant};

use iced::{Animation, Border, Length, Subscription, Task};

use crate::{
    app::{self, colors},
    content::{FindingInfo, InstalledInfo, PackageTaskState, UpdatesInfo},
};

/// Stateful bottom panel that presents overall and per-package progress.
#[derive(Debug, Clone)]
pub struct StatusPanel {
    progress_animation: Animation<f32>,
    progress_target: f32,
    is_indeterminate_progress: bool,
    animation_origin: Instant,
    last_frame: Instant,
    status_label: String,
    progress: ProgressDisplay,
    activity_phase: f32,
    package_tasks: Vec<PackageTaskView>,
}

/// Messages handled by the status panel.
///
/// `Tick` comes from frame subscription. `Sync` is sent by `App` after
/// non-panel updates so the panel can recalculate animation targets.
#[derive(Debug, Clone, Copy)]
pub enum Message {
    Tick(Instant),
    Sync(Instant),
}

/// Side effects produced by [`StatusPanel::update`].
#[derive(Debug)]
#[allow(dead_code)]
pub enum Action {
    None,
    Run(Task<Message>),
}

impl From<Message> for app::Message {
    fn from(msg: Message) -> Self {
        app::Message::StatusPanel(msg)
    }
}

#[derive(Debug, Clone, Copy)]
enum ProgressDisplay {
    Determinate(f32),
    Indeterminate,
}

#[derive(Debug, Clone)]
struct PackageTaskView {
    operation_label: &'static str,
    manager_name: String,
    package_name: String,
    state: PackageTaskState,
    progress: f32,
}

#[derive(Debug, Default)]
struct ProgressCounter {
    total: usize,
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
            is_indeterminate_progress: false,
            animation_origin: now,
            last_frame: now,
            status_label: "Idle".to_string(),
            progress: ProgressDisplay::Determinate(1.0),
            activity_phase: 0.0,
            package_tasks: Vec::new(),
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
            || self.is_indeterminate_progress
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
        self.progress = if self.is_indeterminate_progress {
            ProgressDisplay::Indeterminate
        } else {
            ProgressDisplay::Determinate(self.current_progress_value())
        };
        self.activity_phase = self.moving_phase(Duration::from_millis(2200));

        if should_refresh_snapshot {
            self.status_label = status_label(installed_info, updates_info, finding_info);
            rebuild_package_tasks(
                &mut self.package_tasks,
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
        match progress_mode(installed_info, updates_info, finding_info) {
            ProgressDisplay::Determinate(target) => {
                if self.is_indeterminate_progress {
                    self.is_indeterminate_progress = false;
                }

                if (self.progress_target - target).abs() > 0.001 {
                    self.progress_target = target;
                    self.progress_animation.go_mut(target, at);
                }
            }
            ProgressDisplay::Indeterminate => {
                if !self.is_indeterminate_progress {
                    self.is_indeterminate_progress = true;
                    self.animation_origin = at;
                }
            }
        }
    }

    fn current_progress_value(&self) -> f32 {
        self.progress_animation
            .interpolate_with(|value| value, self.last_frame)
            .clamp(0.0, 1.0)
    }

    fn moving_phase(&self, cycle: Duration) -> f32 {
        let cycle_ms = cycle.as_millis().max(1);
        let elapsed_ms = self
            .last_frame
            .saturating_duration_since(self.animation_origin)
            .as_millis();

        (elapsed_ms % cycle_ms) as f32 / cycle_ms as f32
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

fn progress_mode(
    installed_info: &InstalledInfo,
    updates_info: &UpdatesInfo,
    finding_info: &FindingInfo,
) -> ProgressDisplay {
    let mut known = ProgressCounter::default();

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

    if known.total > 0 {
        ProgressDisplay::Determinate((known.done as f32 / known.total as f32).clamp(0.0, 1.0))
    } else if has_active_work(installed_info, updates_info, finding_info) {
        ProgressDisplay::Indeterminate
    } else {
        ProgressDisplay::Determinate(1.0)
    }
}

fn rebuild_package_tasks(
    out: &mut Vec<PackageTaskView>,
    installed_info: &InstalledInfo,
    updates_info: &UpdatesInfo,
    finding_info: &FindingInfo,
) {
    out.clear();

    let target_capacity = if finding_info.is_installing {
        finding_info.install_items.len()
    } else {
        0
    } + if updates_info.is_updating {
        updates_info.update_items.len()
    } else {
        0
    } + if installed_info.is_removing {
        installed_info.remove_items.len()
    } else {
        0
    };
    out.reserve(target_capacity.saturating_sub(out.capacity()));

    if finding_info.is_installing {
        out.extend(
            finding_info
                .install_items
                .iter()
                .map(|item| PackageTaskView {
                    operation_label: "Install",
                    manager_name: item.manager.name().to_string(),
                    package_name: item.package_name.clone(),
                    state: item.state,
                    progress: item.progress,
                }),
        );
    }

    if updates_info.is_updating {
        out.extend(
            updates_info
                .update_items
                .iter()
                .map(|item| PackageTaskView {
                    operation_label: "Update",
                    manager_name: item.manager.name().to_string(),
                    package_name: item.package_name.clone(),
                    state: item.state,
                    progress: item.progress,
                }),
        );
    }

    if installed_info.is_removing {
        out.extend(
            installed_info
                .remove_items
                .iter()
                .map(|item| PackageTaskView {
                    operation_label: "Remove",
                    manager_name: item.manager.name().to_string(),
                    package_name: item.package_name.clone(),
                    state: item.state,
                    progress: item.progress,
                }),
        );
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

    let (progress_widget, status_right): (iced::Element<'a, Message>, String) = match panel.progress
    {
        ProgressDisplay::Determinate(value) => (
            determinate_capsule_bar(value),
            format!("{:.0}%", value * 100.0),
        ),
        ProgressDisplay::Indeterminate => (
            indeterminate_activity_strip(panel.activity_phase),
            "Working".to_string(),
        ),
    };

    let task_count = panel.package_tasks.len();

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

    if task_count > 0 {
        let rows: Vec<iced::Element<'a, Message>> = panel
            .package_tasks
            .iter()
            .map(|task| package_task_row(task, panel.activity_phase))
            .collect();

        let list = scrollable(column(rows).spacing(8))
            .height(Length::Fill)
            .width(Length::Fill);

        panel_content = panel_content
            .push(
                text(format!("Package Progress ({})", task_count))
                    .size(12)
                    .color(colors::ON_SURFACE_MUTED),
            )
            .push(list);
    }

    let panel_height = if task_count == 0 { 86.0 } else { 280.0 };

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

fn package_task_row<Message: 'static>(
    task: &PackageTaskView,
    indeterminate_phase: f32,
) -> iced::Element<'_, Message> {
    use iced::widget::{column, container, row, text};

    let (state_text, state_color) = match task.state {
        PackageTaskState::Pending => ("Pending".to_string(), colors::ON_SURFACE_MUTED),
        PackageTaskState::InProgress => {
            if task.progress > 0.0 {
                (
                    format!("Running {:.0}%", (task.progress * 100.0).clamp(0.0, 100.0)),
                    colors::SECONDARY,
                )
            } else {
                ("Running".to_string(), colors::SECONDARY)
            }
        }
        PackageTaskState::Done => ("Done".to_string(), colors::SUCCESS),
    };

    let progress_widget = match task.state {
        PackageTaskState::Pending => determinate_capsule_bar(0.0),
        PackageTaskState::InProgress => {
            if task.progress > 0.0 {
                determinate_capsule_bar(task.progress)
            } else {
                indeterminate_activity_strip(indeterminate_phase)
            }
        }
        PackageTaskState::Done => determinate_capsule_bar(1.0),
    };

    container(
        column![
            row![
                text(format!(
                    "{} · {} ({})",
                    task.operation_label, task.package_name, task.manager_name
                ))
                .size(13)
                .color(colors::ON_SURFACE)
                .width(Length::Fill),
                text(state_text).size(12).color(state_color)
            ]
            .align_y(iced::Alignment::Center)
            .spacing(10),
            progress_widget
        ]
        .spacing(6),
    )
    .padding([8, 10])
    .width(Length::Fill)
    .style(|_theme: &iced::Theme| container::Style {
        background: Some(colors::SURFACE.into()),
        border: Border {
            color: colors::DIVIDER_LIGHT,
            width: 1.0,
            radius: 8.0.into(),
        },
        text_color: None,
        shadow: Default::default(),
        snap: false,
    })
    .into()
}

fn indeterminate_activity_strip<Message: 'static>(phase: f32) -> iced::Element<'static, Message> {
    use iced::widget::{container, row};

    let segment_count = 36usize;
    let highlight_width = 10usize;
    let head = ((phase.clamp(0.0, 1.0) * segment_count as f32).floor() as usize) % segment_count;

    let strip = row((0..segment_count).map(|index| {
        let distance = (index + segment_count - head) % segment_count;
        let alpha = if distance < highlight_width {
            let t = 1.0 - (distance as f32 / highlight_width as f32);
            0.20 + (t * 0.80)
        } else {
            0.0
        };

        let color = if alpha > 0.0 {
            iced::Color::from_rgba(
                colors::SECONDARY_SOFT.r,
                colors::SECONDARY_SOFT.g,
                colors::SECONDARY_SOFT.b,
                alpha,
            )
        } else {
            colors::DIVIDER_LIGHT
        };

        container("")
            .width(Length::FillPortion(1))
            .height(Length::Fixed(8.0))
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(color.into()),
                border: Border {
                    color: iced::Color::TRANSPARENT,
                    width: 0.0,
                    radius: 999.0.into(),
                },
                text_color: None,
                shadow: Default::default(),
                snap: false,
            })
            .into()
    }))
    .spacing(2)
    .width(Length::Fill)
    .align_y(iced::Alignment::Center);

    container(strip)
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
