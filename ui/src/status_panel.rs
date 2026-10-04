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
    /// Current status text shown to user.
    status_label: String,
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
            progress: 1.0,
            command_logs: Vec::new(),
            progress_counts: None,
            activity_phase: 0.0,
            is_active: false,
            is_stoppable: false,
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

        if should_refresh_snapshot {
            self.status_label = if self.cancellation_requested && is_active {
                "Stopping current manager...".to_owned()
            } else {
                status_label(installed_info, updates_info, finding_info, catalog)
            };
            self.progress_counts = progress_counts(installed_info, updates_info, finding_info);
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

    let status_text = panel
        .outcome
        .as_ref()
        .filter(|_| !panel.is_active)
        .map_or_else(|| panel.status_label.clone(), OperationOutcome::summary);

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
    let panel_height = base_height + drawer_progress * 174.0;

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
}
