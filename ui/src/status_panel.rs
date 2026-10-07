//! Status panel module.
//!
//! This module owns both the status panel UI rendering and its local animation state.
//! It exposes a local `Message`/`update`/`view` flow for animation ticks and state sync.

use std::time::{Duration, Instant};

use iced::{Animation, Border, Length, Subscription};
use updater_manager_api::ManagerId;

use crate::{
    content::{FindingInfo, InstalledInfo, OperationOutcome, UpdatesInfo},
    manager_catalog::ManagerCatalog,
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
    /// Current status text shown to user, without the elapsed-time suffix.
    status_label: String,
    /// Status text as rendered, including the current step's elapsed time.
    display_label: String,
    /// Start of the current progress step, used for the elapsed-time suffix.
    step_started_at: Instant,
    /// Whole seconds of the current step already reflected in `display_label`.
    step_elapsed_secs: u64,
    /// Current interpolated progress value in [0, 1].
    progress: f32,
    /// Merged command logs displayed in panel.
    command_logs: Vec<String>,
    /// Aggregated known progress as `(done, total)`.
    progress_counts: Option<(usize, usize)>,
    /// Phase of the indeterminate activity animation in [0, 1).
    activity_phase: f32,
    /// Whether any package-manager work is currently active.
    is_active: bool,
    /// Whether the active work belongs to a package operation this panel can stop.
    is_stoppable: bool,
    /// Whether the rendered label currently carries an elapsed-time suffix.
    display_label_active: bool,
    /// Whether the active write should stop before the next manager starts.
    cancellation_requested: bool,
    /// Whether command output is expanded.
    details_expanded: bool,
    /// Activity drawer expansion animation.
    drawer_animation: Animation<f32>,
    /// Most recent completed package operation.
    outcome: Option<OperationOutcome>,
    /// Command output captured when the most recent operation completed.
    outcome_logs: Vec<String>,
}

/// Whether the active operation's writer should stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Stop the active operation before its next manager starts.
    Stop,
    /// Toggle the live command-output drawer.
    ToggleOutput,
    /// Dismiss the completed-operation summary.
    DismissOutcome,
}

impl Action {
    /// Message this action sends when pressed.
    pub const fn message(self) -> Message {
        match self {
            Self::Stop => Message::StopOperation,
            Self::ToggleOutput => Message::ToggleDetails,
            Self::DismissOutcome => Message::DismissOutcome,
        }
    }

    /// Whether this action is the panel's primary control for the active operation.
    const fn is_primary(self) -> bool {
        matches!(self, Self::Stop)
    }
}

/// Label of the History button in the application footer.
pub const FOOTER_HISTORY_LABEL: &str = "History";
/// Label of the output drawer toggle while the drawer is collapsed.
pub const SHOW_OUTPUT_LABEL: &str = "Show output";
/// Label of the output drawer toggle while the drawer is expanded.
pub const HIDE_OUTPUT_LABEL: &str = "Hide output";
/// Label of the Stop action while the operation is still running.
const STOP_LABEL: &str = "Stop";
/// Label of the Stop action once cancellation is already requested.
const STOPPING_LABEL: &str = "Stopping...";
/// Label of the completed-operation dismiss action.
const DISMISS_LABEL: &str = "Dismiss";
/// Status label shown while the active manager command is terminating.
const STOPPING_STATUS: &str = "Stopping current manager...";

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
    /// Toggle command-output details.
    ToggleDetails,
    /// Stop the active operation before its next manager starts.
    StopOperation,
    /// Dismiss the completed-operation summary.
    DismissOutcome,
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
            Message::ToggleDetails | Message::StopOperation | Message::DismissOutcome => {
                Instant::now()
            }
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
            display_label: "Idle".to_string(),
            step_started_at: now,
            step_elapsed_secs: 0,
            progress: 1.0,
            command_logs: Vec::new(),
            progress_counts: None,
            activity_phase: 0.0,
            is_active: false,
            is_stoppable: false,
            display_label_active: false,
            cancellation_requested: false,
            details_expanded: false,
            drawer_animation: Animation::new(0.0).duration(Duration::from_millis(180)),
            outcome: None,
            outcome_logs: Vec::new(),
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
            || self.drawer_animation.is_animating(self.last_frame)
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
        catalog: &ManagerCatalog,
    ) {
        let at = message.at();
        let should_refresh_snapshot = matches!(message, Message::Sync(_));
        let should_toggle_details = matches!(message, Message::ToggleDetails);
        if matches!(message, Message::DismissOutcome) {
            self.outcome = None;
            self.outcome_logs.clear();
            if !has_active_work(installed_info, updates_info, finding_info) {
                self.details_expanded = false;
                self.drawer_animation.go_mut(0.0, at);
            }
        }
        let is_active = has_active_work(installed_info, updates_info, finding_info);
        let elapsed = at.saturating_duration_since(self.last_frame).as_secs_f32();

        if is_active {
            self.activity_phase = (self.activity_phase + elapsed * 0.9) % 1.0;
        } else {
            self.activity_phase = 0.0;
        }
        self.is_active = is_active;

        self.last_frame = at;
        let progress_target = progress_value(installed_info, updates_info, finding_info);
        if (self.progress_target - progress_target).abs() > 0.001 {
            self.progress_target = progress_target;
            self.progress_animation.go_mut(progress_target, at);
        }
        self.progress = self
            .progress_animation
            .interpolate_with(|value| value, self.last_frame)
            .clamp(0.0, 1.0);

        if should_toggle_details && !self.command_logs.is_empty() {
            self.details_expanded = !self.details_expanded;
            self.drawer_animation
                .go_mut(if self.details_expanded { 1.0 } else { 0.0 }, at);
        }

        let mut label_step_changed = false;
        if should_refresh_snapshot {
            let base_label = if self.cancellation_requested && is_active {
                STOPPING_STATUS.to_owned()
            } else {
                status_label(installed_info, updates_info, finding_info, catalog)
            };
            let counts = progress_counts(installed_info, updates_info, finding_info);
            if base_label != self.status_label || counts != self.progress_counts {
                self.step_started_at = at;
                self.step_elapsed_secs = 0;
                label_step_changed = true;
            }
            self.status_label = base_label;
            self.progress_counts = counts;
            if is_active {
                rebuild_command_logs(
                    &mut self.command_logs,
                    installed_info,
                    updates_info,
                    finding_info,
                );
            } else if self.outcome.is_some() {
                self.command_logs.clone_from(&self.outcome_logs);
            } else {
                self.command_logs.clear();
            }
            if self.command_logs.is_empty() && self.details_expanded {
                self.details_expanded = false;
                self.drawer_animation.go_mut(0.0, at);
            }
        }

        // The elapsed-time suffix is the panel's liveness signal while an
        // operation runs, so it must advance on frame ticks too, not only on
        // progress messages. Rebuild it only when the step changes or the whole
        // second ticks, so every other frame stays allocation-free.
        let step_elapsed_secs = at.saturating_duration_since(self.step_started_at).as_secs();
        if label_step_changed
            || step_elapsed_secs != self.step_elapsed_secs
            || is_active != self.display_label_active
        {
            self.step_elapsed_secs = step_elapsed_secs;
            self.display_label_active = is_active;
            self.display_label =
                compose_display_label(&self.status_label, step_elapsed_secs, is_active);
        }
    }

    /// Returns the actions the panel currently offers, in display order.
    ///
    /// Used by both `render` and its tests so the rendered action row cannot
    /// drift from the asserted one.
    pub fn actions(&self) -> impl Iterator<Item = Action> + '_ {
        let stop = (self.is_active && self.is_stoppable).then_some(Action::Stop);
        let toggle = (!self.command_logs.is_empty()).then_some(Action::ToggleOutput);
        let dismiss = (self.outcome.is_some() && !self.is_active).then_some(Action::DismissOutcome);

        stop.into_iter().chain(toggle).chain(dismiss)
    }

    /// Label rendered for one action in the panel's current state.
    pub fn action_label(&self, action: Action) -> &'static str {
        match action {
            Action::Stop if self.cancellation_requested => STOPPING_LABEL,
            Action::Stop => STOP_LABEL,
            Action::ToggleOutput if self.details_expanded => HIDE_OUTPUT_LABEL,
            Action::ToggleOutput => SHOW_OUTPUT_LABEL,
            Action::DismissOutcome => DISMISS_LABEL,
        }
    }

    /// Whether one action can be pressed in the panel's current state.
    pub fn action_enabled(&self, action: Action) -> bool {
        match action {
            Action::Stop => !self.cancellation_requested,
            Action::ToggleOutput | Action::DismissOutcome => true,
        }
    }

    /// Clears cancellation state when a new package operation starts.
    pub fn begin_package_operation(&mut self) {
        self.cancellation_requested = false;
        self.is_stoppable = true;
    }

    /// Marks the active operation as terminating its current manager command.
    pub fn request_cancellation(&mut self) {
        self.cancellation_requested = true;
    }

    /// Records a package-operation result until it is dismissed or superseded.
    pub fn record_outcome(&mut self, outcome: OperationOutcome) {
        self.cancellation_requested = false;
        self.is_stoppable = false;
        self.outcome_logs.clone_from(&self.command_logs);
        self.outcome = Some(outcome);
    }

    /// Dismisses the topmost completed-operation surface.
    pub fn dismiss_top_surface(&mut self) -> bool {
        let at = Instant::now();
        if self.details_expanded {
            self.details_expanded = false;
            self.drawer_animation.go_mut(0.0, at);
            true
        } else if self.outcome.take().is_some() {
            self.outcome_logs.clear();
            self.command_logs.clear();
            true
        } else {
            false
        }
    }

    /// Whether the status panel currently has useful activity to show.
    pub fn is_visible(&self) -> bool {
        self.is_active || self.details_expanded || self.outcome.is_some()
    }

    /// Renders the status panel view.
    pub fn view<'a>(&'a self) -> iced::Element<'a, Message> {
        render(self)
    }
}

/// Formats whole seconds as a compact elapsed-time suffix.
fn format_elapsed(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// Appends the current step's elapsed time while the operation is active.
fn compose_display_label(base: &str, elapsed_secs: u64, is_active: bool) -> String {
    if !is_active || elapsed_secs == 0 {
        return base.to_owned();
    }

    format!("{base} · {}", format_elapsed(elapsed_secs))
}

/// Whether there is user-visible work for the panel to show.
///
/// `UpdatesInfo::background_loading_count` is deliberately absent: a scheduled
/// read-only update check must not slide the panel open or subscribe to window
/// frames when the user did nothing.
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
        || installed_info.is_updating
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

    if installed_info.is_updating
        && let Some((completed, total, _, _)) = &installed_info.update_progress
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
    if installed_info.is_updating {
        out.extend(installed_info.update_logs.iter().cloned());
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
    catalog: &ManagerCatalog,
) -> String {
    if installed_info.is_loading_count || updates_info.is_loading_count {
        return "Initializing package manager data...".to_string();
    }

    if finding_info.is_installing {
        return operation_status_label(
            "Installing",
            finding_info.install_progress.as_ref(),
            catalog,
            "Installing selected packages...",
        );
    }

    if updates_info.is_updating {
        return operation_status_label(
            "Updating",
            updates_info.update_progress.as_ref(),
            catalog,
            "Updating selected packages...",
        );
    }

    if installed_info.is_removing {
        return operation_status_label(
            "Removing",
            installed_info.remove_progress.as_ref(),
            catalog,
            "Removing selected packages...",
        );
    }

    if installed_info.is_updating {
        return operation_status_label(
            "Updating",
            installed_info.update_progress.as_ref(),
            catalog,
            "Updating the inspected package...",
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
    progress: Option<&(usize, usize, ManagerId, String)>,
    catalog: &ManagerCatalog,
    fallback: &str,
) -> String {
    if let Some((completed, total, manager, package)) = progress {
        if package.is_empty() {
            return format!("{verb} packages ({completed}/{total})...");
        }

        return format!(
            "{verb} {completed}/{total}: {package} ({})",
            catalog.display_name(manager)
        );
    }

    fallback.to_string()
}

fn render(panel: &StatusPanel) -> iced::Element<'_, Message> {
    use iced::widget::{button, column, container, row, scrollable, text};

    let progress_widget = activity_capsule_bar(
        panel.progress,
        panel.is_active && panel.progress <= 0.001,
        panel.activity_phase,
    );

    let mut status_right = format!("{:.0}%", panel.progress * 100.0);
    if let Some(outcome) = panel.outcome.as_ref().filter(|_| !panel.is_active) {
        status_right = format!("{}/{}", outcome.completed_packages, outcome.total_packages);
    } else if let Some((done, total)) = panel.progress_counts {
        status_right = format!("{}/{}", done, total);
    }

    let mut status_actions = row![
        text(status_right)
            .size(12)
            .style(crate::theme::text_on_surface_muted)
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    for action in panel.actions() {
        let enabled = panel.action_enabled(action);
        status_actions = status_actions.push(
            button(
                text(panel.action_label(action)).size(if action.is_primary() { 13 } else { 12 }),
            )
            .padding(if action.is_primary() { [6, 12] } else { [5, 9] })
            .style(crate::theme::secondary_button(enabled))
            .on_press_maybe(enabled.then_some(action.message())),
        );
    }

    let status_text = match panel.outcome.as_ref().filter(|_| !panel.is_active) {
        Some(outcome) => outcome.summary(),
        None => panel.display_label.clone(),
    };

    let mut panel_content = column![
        row![
            text(status_text)
                .size(13)
                .font(crate::theme::FONT_SEMIBOLD)
                .style(move |theme| {
                    let semantic = crate::theme::semantic_colors(theme);
                    iced::widget::text::Style {
                        color: Some(match panel.outcome.as_ref().filter(|_| !panel.is_active) {
                            Some(outcome) if outcome.is_success() => semantic.success,
                            Some(_) => semantic.error,
                            None => semantic.on_surface,
                        }),
                    }
                })
                .width(Length::Fill),
            status_actions,
        ]
        .align_y(iced::Alignment::Center)
        .spacing(12)
    ]
    .spacing(8)
    .height(Length::Fill);

    if panel.is_active {
        panel_content = panel_content.push(progress_widget);

        // Show the newest command line without requiring the output drawer, so
        // a long package step has a second liveness signal beside the clock.
        if !panel.details_expanded
            && let Some(latest) = panel.command_logs.last()
        {
            panel_content = panel_content.push(
                text(latest.as_str())
                    .size(11)
                    .font(crate::theme::FONT_MONO)
                    .style(crate::theme::text_on_surface_alt)
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            );
        }
    }

    let drawer_progress = panel
        .drawer_animation
        .interpolate_with(|value| value, panel.last_frame)
        .clamp(0.0, 1.0);

    if !panel.command_logs.is_empty() && drawer_progress > 0.001 {
        let lines = panel.command_logs.iter().map(|line| {
            text(line)
                .size(12)
                .font(crate::theme::FONT_MONO)
                .style(crate::theme::text_on_surface_alt)
                .width(Length::Fill)
                .into()
        });

        let log_list = scrollable(column(lines).spacing(4))
            .height(Length::Fill)
            .width(Length::Fill);

        panel_content = panel_content.push(log_list);
    }

    let base_height = if panel.is_active { 58.0 } else { 42.0 };
    let hidden_drawer_preview =
        panel.is_active && !panel.details_expanded && !panel.command_logs.is_empty();
    let panel_height =
        base_height + drawer_progress * 174.0 + if hidden_drawer_preview { 18.0 } else { 0.0 };

    container(panel_content)
        .padding([7, 16])
        .height(Length::Fixed(panel_height))
        .width(Length::Fill)
        .clip(true)
        .style(crate::theme::status_container)
        .into()
}

fn activity_capsule_bar<Message: 'static>(
    progress: f32,
    indeterminate: bool,
    phase: f32,
) -> iced::Element<'static, Message> {
    use iced::widget::{Space, container, row};

    let travel_portion: u16 = 1000;
    let (left, filled, right) = if indeterminate {
        let capsule_width = 160;
        let ping_pong = 1.0 - (phase.clamp(0.0, 1.0) * 2.0 - 1.0).abs();
        let left = ((travel_portion - capsule_width) as f32 * ping_pong).round() as u16;
        (left, capsule_width, travel_portion - capsule_width - left)
    } else {
        let filled =
            ((progress.clamp(0.0, 1.0) * travel_portion as f32).round() as u16).min(travel_portion);
        (0, filled, travel_portion.saturating_sub(filled))
    };

    let left_spacer = Space::new().width(if left == 0 {
        Length::Shrink
    } else {
        Length::FillPortion(left)
    });

    let capsule = container("")
        .width(if filled == 0 {
            Length::Shrink
        } else {
            Length::FillPortion(filled)
        })
        .height(Length::Fixed(3.0))
        .style(|theme: &iced::Theme| container::Style {
            background: Some(crate::theme::semantic_colors(theme).accent.into()),
            border: Border {
                width: 0.0,
                radius: 999.0.into(),
                ..Default::default()
            },
            text_color: None,
            shadow: iced::Shadow::default(),
            snap: false,
        });

    let right_spacer = Space::new().width(if right == 0 {
        Length::Shrink
    } else {
        Length::FillPortion(right)
    });

    let bar = row![left_spacer, capsule, right_spacer]
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

    container(bar)
        .padding([1, 0])
        .width(Length::Fill)
        .style(|theme: &iced::Theme| container::Style {
            background: Some(crate::theme::semantic_colors(theme).surface_muted.into()),
            border: Border {
                width: 0.0,
                radius: 999.0.into(),
                ..Default::default()
            },
            text_color: None,
            shadow: Default::default(),
            snap: false,
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_updates_info(completed: usize, total: usize, package: &str) -> UpdatesInfo {
        UpdatesInfo {
            is_updating: true,
            update_progress: Some((
                completed,
                total,
                ManagerId::parse("builtin:cargo").unwrap(),
                package.to_owned(),
            )),
            ..UpdatesInfo::default()
        }
    }

    #[test]
    fn cancellation_status_describes_the_manager_boundary() {
        let now = Instant::now();
        let mut panel = StatusPanel::new(now);
        let installed = InstalledInfo::default();
        let updates = UpdatesInfo::default();
        let mut finding = FindingInfo {
            is_installing: true,
            ..FindingInfo::default()
        };
        finding.install_progress = Some((
            0,
            2,
            ManagerId::parse("builtin:cargo").unwrap(),
            "alpha".to_owned(),
        ));

        panel.begin_package_operation();
        panel.request_cancellation();
        panel.update(
            Message::Sync(now),
            &installed,
            &updates,
            &finding,
            &ManagerCatalog::builtin(),
        );

        assert_eq!(panel.status_label, "Stopping current manager...");
        assert!(panel.cancellation_requested);
    }

    #[test]
    fn active_partial_progress_keeps_liveness_signal() {
        let now = Instant::now();
        let mut panel = StatusPanel::new(now);
        let installed = InstalledInfo::default();
        let updates = active_updates_info(1, 3, "firefox");
        let finding = FindingInfo::default();
        let catalog = ManagerCatalog::builtin();

        panel.update(Message::Sync(now), &installed, &updates, &finding, &catalog);
        let synced_label = panel.display_label.clone();

        // A second and a second later the only input is the frame tick that a
        // frozen bar would otherwise not produce any visible change for.
        panel.update(
            Message::Tick(now + Duration::from_secs(1)),
            &installed,
            &updates,
            &finding,
            &catalog,
        );
        let after_one_second = panel.display_label.clone();
        panel.update(
            Message::Tick(now + Duration::from_secs(2)),
            &installed,
            &updates,
            &finding,
            &catalog,
        );
        let after_two_seconds = panel.display_label.clone();

        assert!(
            panel.progress > 0.0 && panel.progress < 1.0,
            "partial progress"
        );
        assert!(
            synced_label.contains("Updating 1/3: firefox"),
            "unexpected base label: {synced_label}"
        );
        assert_ne!(synced_label, after_one_second);
        assert_ne!(after_one_second, after_two_seconds);
        assert!(after_one_second.ends_with("· 1s"), "{after_one_second}");
        assert!(after_two_seconds.ends_with("· 2s"), "{after_two_seconds}");
    }

    #[test]
    fn elapsed_suffix_is_dropped_when_the_operation_ends() {
        let now = Instant::now();
        let mut panel = StatusPanel::new(now);
        let installed = InstalledInfo::default();
        let finding = FindingInfo::default();
        let catalog = ManagerCatalog::builtin();

        panel.update(
            Message::Sync(now),
            &installed,
            &active_updates_info(1, 3, "firefox"),
            &finding,
            &catalog,
        );
        panel.update(
            Message::Tick(now + Duration::from_secs(5)),
            &installed,
            &active_updates_info(1, 3, "firefox"),
            &finding,
            &catalog,
        );
        assert!(panel.display_label.ends_with("· 5s"));

        panel.update(
            Message::Sync(now + Duration::from_secs(6)),
            &installed,
            &UpdatesInfo::default(),
            &finding,
            &catalog,
        );

        assert_eq!(panel.display_label, "Idle");
    }

    #[test]
    fn stop_action_is_offered_while_active_and_absent_when_idle() {
        let now = Instant::now();
        let installed = InstalledInfo::default();
        let finding = FindingInfo::default();
        let catalog = ManagerCatalog::builtin();
        let updates = active_updates_info(1, 3, "firefox");

        let mut panel = StatusPanel::new(now);
        panel.update(Message::Sync(now), &installed, &updates, &finding, &catalog);

        assert!(
            !panel.actions().any(|action| action == Action::Stop),
            "a read-only scan is not stoppable"
        );

        panel.begin_package_operation();
        panel.update(Message::Sync(now), &installed, &updates, &finding, &catalog);
        let actions: Vec<_> = panel.actions().collect();

        assert!(actions.contains(&Action::Stop));
        assert_eq!(panel.action_label(Action::Stop), "Stop");
        assert!(matches!(Action::Stop.message(), Message::StopOperation));
        assert!(panel.action_enabled(Action::Stop));

        panel.request_cancellation();
        panel.update(Message::Sync(now), &installed, &updates, &finding, &catalog);

        assert_eq!(panel.action_label(Action::Stop), "Stopping...");
        assert!(!panel.action_enabled(Action::Stop));

        let mut idle = StatusPanel::new(now);
        idle.update(
            Message::Sync(now),
            &installed,
            &UpdatesInfo::default(),
            &finding,
            &catalog,
        );

        assert!(!idle.actions().any(|action| action == Action::Stop));
        assert_eq!(idle.actions().count(), 0);
    }

    #[test]
    fn visible_action_labels_are_unique_per_screen_state() {
        let now = Instant::now();
        let installed = InstalledInfo::default();
        let finding = FindingInfo::default();
        let catalog = ManagerCatalog::builtin();
        let updates = active_updates_info(1, 3, "firefox");

        let mut panel = StatusPanel::new(now);
        panel.begin_package_operation();
        panel.update(Message::Sync(now), &installed, &updates, &finding, &catalog);

        let labels: Vec<_> = panel
            .actions()
            .map(|action| panel.action_label(action))
            .collect();
        let unique: std::collections::HashSet<_> = labels.iter().collect();

        assert_eq!(labels.len(), unique.len(), "duplicate labels: {labels:?}");
        // The footer's history button must not share a label with any panel
        // control, since both are on screen during an operation.
        assert!(!labels.contains(&FOOTER_HISTORY_LABEL));
        assert_eq!(panel.action_label(Action::ToggleOutput), SHOW_OUTPUT_LABEL);

        panel.update(Message::Sync(now), &installed, &updates, &finding, &catalog);

        assert_eq!(panel.action_label(Action::Stop), "Stop");
    }

    #[test]
    fn background_update_check_keeps_the_panel_closed() {
        let now = Instant::now();
        let mut panel = StatusPanel::new(now);
        let installed = InstalledInfo::default();
        let finding = FindingInfo::default();
        let updates = UpdatesInfo {
            background_loading_count: true,
            init_progress: Some((0, 3)),
            ..UpdatesInfo::default()
        };

        panel.update(
            Message::Sync(now),
            &installed,
            &updates,
            &finding,
            &ManagerCatalog::builtin(),
        );

        assert!(!has_active_work(&installed, &updates, &finding));
        assert!(!panel.is_visible());
        assert_eq!(panel.status_label, "Idle");

        // The same scan is user-visible once the Updates page adopts it.
        let adopted = UpdatesInfo {
            is_loading_count: true,
            ..updates
        };
        panel.update(
            Message::Sync(now),
            &installed,
            &adopted,
            &finding,
            &ManagerCatalog::builtin(),
        );

        assert!(panel.is_visible());
    }

    #[test]
    fn elapsed_time_is_formatted_compactly() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(45), "45s");
        assert_eq!(format_elapsed(60), "1m 0s");
        assert_eq!(format_elapsed(134), "2m 14s");
        assert_eq!(format_elapsed(3600), "1h 0m");
        assert_eq!(format_elapsed(7380), "2h 3m");
    }
}
