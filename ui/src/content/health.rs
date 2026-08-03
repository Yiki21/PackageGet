//! Read-only health checks for configured package managers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::channel::mpsc;
use iced::{Element, Length, Task};
use updater_core::{CancellationToken, ManagerRegistry};
use updater_manager_api::{AvailabilityReason, ManagerAvailability, ManagerConfig, ManagerId};

use crate::{
    activity,
    content::{InstalledInfo, UpdatesInfo, shared},
    icon,
    manager_catalog::ManagerCatalog,
    theme,
};

#[derive(Debug, Clone)]
pub struct ManagerHealthRecord {
    checked_at: String,
    result: Result<ManagerAvailability, String>,
}

#[derive(Debug, Clone, Default)]
pub struct ManagerHealthInfo {
    generation: u64,
    records: HashMap<ManagerId, ManagerHealthRecord>,
    scan_scope: HashSet<ManagerId>,
    cancellation: Option<CancellationToken>,
    current_manager: Option<ManagerId>,
    completed: usize,
    total: usize,
    finished_at: Option<String>,
    is_checking: bool,
    cancellation_requested: bool,
    last_scan_cancelled: bool,
}

impl ManagerHealthInfo {
    pub fn is_checking(&self) -> bool {
        self.is_checking
    }

    pub fn has_results(&self) -> bool {
        !self.records.is_empty()
    }

    pub fn should_scan_on_open(&self) -> bool {
        !self.is_checking && self.finished_at.is_none() && self.records.is_empty()
    }

    pub fn result(&self, manager: &ManagerId) -> Option<&Result<ManagerAvailability, String>> {
        self.records.get(manager).map(|record| &record.result)
    }

    pub fn invalidate(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
        self.generation = self.generation.saturating_add(1);
        self.records.clear();
        self.scan_scope.clear();
        self.cancellation = None;
        self.current_manager = None;
        self.completed = 0;
        self.total = 0;
        self.finished_at = None;
        self.is_checking = false;
        self.cancellation_requested = false;
        self.last_scan_cancelled = false;
    }

    pub fn has_issues(
        &self,
        config: &updater_core::Config,
        installed: &InstalledInfo,
        updates: &UpdatesInfo,
    ) -> bool {
        config.managers.iter().any(|manager| {
            matches!(
                self.status_for(&manager.id, installed, updates),
                HealthStatus::Degraded | HealthStatus::Unavailable | HealthStatus::Error
            )
        })
    }

    fn begin_scan(&mut self, managers: &[ManagerConfig]) -> (u64, CancellationToken) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
        self.generation = self.generation.saturating_add(1);
        self.scan_scope = managers.iter().map(|manager| manager.id.clone()).collect();
        self.records
            .retain(|manager, _| self.scan_scope.contains(manager));
        self.current_manager = None;
        self.completed = 0;
        self.total = managers.len();
        self.finished_at = None;
        self.is_checking = true;
        self.cancellation_requested = false;
        self.last_scan_cancelled = false;
        let cancellation = CancellationToken::default();
        self.cancellation = Some(cancellation.clone());
        (self.generation, cancellation)
    }

    fn request_cancellation(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
            self.cancellation_requested = true;
        }
    }

    fn apply_started(&mut self, generation: u64, manager: ManagerId) {
        if generation == self.generation && self.is_checking {
            self.current_manager = Some(manager);
        }
    }

    fn apply_result(
        &mut self,
        generation: u64,
        manager: ManagerId,
        checked_at: String,
        result: Result<ManagerAvailability, String>,
    ) {
        if generation != self.generation || !self.scan_scope.contains(&manager) {
            return;
        }
        self.records
            .insert(manager, ManagerHealthRecord { checked_at, result });
        self.completed = self.completed.saturating_add(1).min(self.total);
    }

    fn finish_scan(&mut self, generation: u64, finished_at: String, cancelled: bool) {
        if generation != self.generation {
            return;
        }
        self.current_manager = None;
        self.cancellation = None;
        self.finished_at = Some(finished_at);
        self.is_checking = false;
        self.cancellation_requested = false;
        self.last_scan_cancelled = cancelled;
    }

    fn status_for(
        &self,
        manager: &ManagerId,
        installed: &InstalledInfo,
        updates: &UpdatesInfo,
    ) -> HealthStatus {
        let runtime_issue = runtime_issue_detail(manager, installed, updates).is_some();
        match self.records.get(manager).map(|record| &record.result) {
            Some(Err(_)) => HealthStatus::Error,
            Some(Ok(ManagerAvailability::Unavailable { .. })) => HealthStatus::Unavailable,
            Some(Ok(ManagerAvailability::Available { .. })) if runtime_issue => {
                HealthStatus::Degraded
            }
            Some(Ok(ManagerAvailability::Available { .. })) => HealthStatus::Healthy,
            Some(Ok(_)) => HealthStatus::Unavailable,
            None if runtime_issue => HealthStatus::Degraded,
            None => HealthStatus::Unchecked,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum HealthFilter {
    #[default]
    All,
    Healthy,
    Issues,
    Unchecked,
}

impl HealthFilter {
    const ALL: [Self; 4] = [Self::All, Self::Healthy, Self::Issues, Self::Unchecked];

    const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Healthy => "Healthy",
            Self::Issues => "Issues",
            Self::Unchecked => "Unchecked",
        }
    }

    fn includes(self, status: HealthStatus) -> bool {
        match self {
            Self::All => true,
            Self::Healthy => status == HealthStatus::Healthy,
            Self::Issues => matches!(
                status,
                HealthStatus::Degraded | HealthStatus::Unavailable | HealthStatus::Error
            ),
            Self::Unchecked => status == HealthStatus::Unchecked,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealthStatus {
    Healthy,
    Degraded,
    Unavailable,
    Error,
    Unchecked,
}

impl HealthStatus {
    const fn label(self) -> &'static str {
        match self {
            Self::Healthy => "Healthy",
            Self::Degraded => "Degraded",
            Self::Unavailable => "Unavailable",
            Self::Error => "Error",
            Self::Unchecked => "Unchecked",
        }
    }

    const fn color(self) -> iced::Color {
        match self {
            Self::Healthy => theme::colors::SUCCESS,
            Self::Degraded | Self::Unavailable => theme::colors::WARNING,
            Self::Error => theme::colors::ERROR,
            Self::Unchecked => theme::colors::ON_SURFACE_ALT,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HealthCenter {
    query: String,
    filter: HealthFilter,
}

#[derive(Debug, Clone)]
pub enum Message {
    QueryChanged(String),
    FilterChanged(HealthFilter),
    StartScan,
    CancelScan,
    CheckStarted {
        generation: u64,
        manager: ManagerId,
    },
    CheckFinished {
        generation: u64,
        manager: ManagerId,
        checked_at: String,
        result: Result<ManagerAvailability, String>,
    },
    ScanFinished {
        generation: u64,
        finished_at: String,
        cancelled: bool,
    },
    CopyDiagnostics,
    OpenSettings(ManagerId),
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
    OpenSettings(ManagerId),
}

impl HealthCenter {
    pub fn update(
        &mut self,
        message: Message,
        config: &updater_core::Config,
        info: &mut ManagerHealthInfo,
        catalog: &ManagerCatalog,
        installed: &InstalledInfo,
        updates: &UpdatesInfo,
    ) -> Action {
        match message {
            Message::QueryChanged(query) => {
                self.query = query;
                Action::None
            }
            Message::FilterChanged(filter) => {
                self.filter = filter;
                Action::None
            }
            Message::StartScan => {
                if info.is_checking {
                    return Action::None;
                }
                let managers = config.managers.clone();
                let (generation, cancellation) = info.begin_scan(&managers);
                Action::Run(run_health_scan(
                    generation,
                    managers,
                    catalog.registry(),
                    cancellation,
                ))
            }
            Message::CancelScan => {
                info.request_cancellation();
                Action::None
            }
            Message::CheckStarted {
                generation,
                manager,
            } => {
                info.apply_started(generation, manager);
                Action::None
            }
            Message::CheckFinished {
                generation,
                manager,
                checked_at,
                result,
            } => {
                info.apply_result(generation, manager, checked_at, result);
                Action::None
            }
            Message::ScanFinished {
                generation,
                finished_at,
                cancelled,
            } => {
                info.finish_scan(generation, finished_at, cancelled);
                Action::None
            }
            Message::CopyDiagnostics => {
                if !info.has_results() {
                    return Action::None;
                }
                Action::Run(iced::clipboard::write(diagnostics_report(
                    config, info, catalog, installed, updates,
                )))
            }
            Message::OpenSettings(manager) => Action::OpenSettings(manager),
        }
    }

    pub fn view<'a>(
        &'a self,
        config: &'a updater_core::Config,
        info: &'a ManagerHealthInfo,
        catalog: &'a ManagerCatalog,
        installed: &'a InstalledInfo,
        updates: &'a UpdatesInfo,
    ) -> Element<'a, Message> {
        use iced::widget::{button, column, container, row, scrollable, text};

        let summary = HealthSummary::new(config, info, installed, updates);
        let search = shared::search_input_view(
            shared::search_input_id(crate::content::ActiveContentPage::Health),
            "Managers",
            "Filter managers...",
            &self.query,
            Message::QueryChanged,
        );
        let filters = row(HealthFilter::ALL.into_iter().map(|filter| {
            shared::segmented_button(
                filter.label(),
                self.filter == filter,
                Message::FilterChanged(filter),
            )
            .into()
        }))
        .spacing(2)
        .width(Length::Fill);

        let check = shared::refresh_button_with_label(
            if info.has_results() {
                "Recheck"
            } else {
                "Check all"
            },
            !info.is_checking,
            Message::StartScan,
        );
        let cancel = button(text(if info.cancellation_requested {
            "Stopping..."
        } else {
            "Cancel"
        }))
        .padding([8, 12])
        .style(theme::secondary_button(
            info.is_checking && !info.cancellation_requested,
        ))
        .on_press_maybe(
            (info.is_checking && !info.cancellation_requested).then_some(Message::CancelScan),
        );
        let copy = button(text("Copy report"))
            .padding([8, 12])
            .style(theme::secondary_button(info.has_results()))
            .on_press_maybe(info.has_results().then_some(Message::CopyDiagnostics));

        let toolbar = shared::toolbar(
            column![
                row![
                    container(search).width(Length::FillPortion(2)),
                    container(
                        column![
                            shared::section_title("Status"),
                            shared::segmented_group(filters)
                        ]
                        .spacing(theme::spacing::SM)
                    )
                    .width(Length::FillPortion(2)),
                ]
                .spacing(theme::spacing::LG)
                .align_y(iced::Alignment::End),
                row![check, cancel, copy].spacing(theme::spacing::SM).wrap(),
            ]
            .spacing(theme::spacing::MD),
        );

        let scan_state = if info.is_checking {
            let current = info
                .current_manager
                .as_ref()
                .map(|manager| catalog.display_name(manager))
                .unwrap_or("Preparing checks");
            format!("{current} · {}/{} checked", info.completed, info.total)
        } else if let Some(finished_at) = &info.finished_at {
            if info.last_scan_cancelled {
                format!(
                    "Scan stopped at {finished_at} · {}/{} checked",
                    info.completed, info.total
                )
            } else {
                format!("Last checked {finished_at}")
            }
        } else {
            "Not checked in this session".to_owned()
        };

        let manager_rows = config
            .managers
            .iter()
            .filter(|manager| shared::manager_matches_query(&manager.id, catalog, &self.query))
            .filter_map(|manager| {
                let status = info.status_for(&manager.id, installed, updates);
                self.filter
                    .includes(status)
                    .then(|| manager_health_row(manager, status, info, catalog, installed, updates))
            })
            .collect::<Vec<_>>();

        let list: Element<'_, Message> = if manager_rows.is_empty() {
            shared::centered_message(if config.managers.is_empty() {
                "No package managers configured"
            } else {
                "No managers match this filter"
            })
        } else {
            scrollable(column(manager_rows).spacing(theme::spacing::SM))
                .height(Length::Fill)
                .style(theme::scrollable_style)
                .into()
        };

        column![
            shared::page_header(
                "Package Managers",
                format!(
                    "{} configured managers · availability checks are read-only",
                    config.managers.len()
                ),
                theme::colors::HEALTH,
            ),
            shared::summary_row([
                (
                    format!("{} healthy", summary.healthy),
                    theme::colors::SUCCESS
                ),
                (
                    format!("{} degraded", summary.degraded),
                    theme::colors::WARNING
                ),
                (
                    format!("{} unavailable", summary.unavailable),
                    theme::colors::WARNING
                ),
                (format!("{} errors", summary.errors), theme::colors::ERROR),
                (
                    format!("{} unchecked", summary.unchecked),
                    theme::colors::ON_SURFACE_ALT
                ),
            ]),
            toolbar,
            text(scan_state)
                .size(14)
                .style(if info.cancellation_requested {
                    theme::text_warning
                } else {
                    theme::text_on_surface_muted
                }),
            list,
        ]
        .spacing(theme::spacing::LG)
        .height(Length::Fill)
        .into()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct HealthSummary {
    healthy: usize,
    degraded: usize,
    unavailable: usize,
    errors: usize,
    unchecked: usize,
}

impl HealthSummary {
    fn new(
        config: &updater_core::Config,
        info: &ManagerHealthInfo,
        installed: &InstalledInfo,
        updates: &UpdatesInfo,
    ) -> Self {
        let mut summary = Self::default();
        for manager in &config.managers {
            match info.status_for(&manager.id, installed, updates) {
                HealthStatus::Healthy => summary.healthy += 1,
                HealthStatus::Degraded => summary.degraded += 1,
                HealthStatus::Unavailable => summary.unavailable += 1,
                HealthStatus::Error => summary.errors += 1,
                HealthStatus::Unchecked => summary.unchecked += 1,
            }
        }
        summary
    }
}

fn manager_health_row<'a>(
    config: &'a ManagerConfig,
    status: HealthStatus,
    info: &'a ManagerHealthInfo,
    catalog: &'a ManagerCatalog,
    installed: &'a InstalledInfo,
    updates: &'a UpdatesInfo,
) -> Element<'a, Message> {
    use iced::widget::{button, column, container, row, text};

    let manager = &config.id;
    let display_name = catalog.display_name(manager);
    let record = info.records.get(manager);
    let is_current = info.current_manager.as_ref() == Some(manager) && info.is_checking;
    let (version, availability_detail) = health_record_detail(record);
    let runtime_detail = runtime_issue_detail(manager, installed, updates);
    let executable = config.executable().map_or_else(
        || "Executable: System PATH".to_owned(),
        |path| format!("Executable: {}", path.display()),
    );
    let checked_at = record.map_or("Never checked", |record| record.checked_at.as_str());

    let badge_color = status.color();
    let status_badge = container(
        text(status.label())
            .size(12)
            .font(theme::FONT_SEMIBOLD)
            .style(move |_theme| iced::widget::text::Style {
                color: Some(badge_color),
            }),
    )
    .padding([4, 8])
    .style(move |_theme| iced::widget::container::Style {
        background: Some(
            iced::Color {
                a: 0.12,
                ..badge_color
            }
            .into(),
        ),
        border: iced::Border {
            color: iced::Color {
                a: 0.32,
                ..badge_color
            },
            width: 1.0,
            radius: theme::radius::CONTROL.into(),
        },
        ..Default::default()
    });

    let settings = button(text("Configure").size(13))
        .padding([7, 12])
        .style(theme::secondary_button(true))
        .on_press(Message::OpenSettings(manager.clone()));
    let state_detail = if is_current {
        "Checking availability...".to_owned()
    } else {
        availability_detail
    };

    let mut details = column![
        row![
            icon::manager_logo(manager, display_name, 42.0),
            column![
                text(display_name)
                    .size(17)
                    .font(theme::FONT_SEMIBOLD)
                    .style(theme::text_on_surface),
                text(manager.as_str())
                    .size(12)
                    .font(theme::FONT_MONO)
                    .style(theme::text_on_surface_muted),
            ]
            .spacing(2)
            .width(Length::Fill),
            status_badge,
            settings,
        ]
        .spacing(theme::spacing::SM)
        .align_y(iced::Alignment::Center)
        .wrap(),
        text(state_detail)
            .size(14)
            .style(if matches!(status, HealthStatus::Error) {
                theme::text_error
            } else if matches!(status, HealthStatus::Degraded | HealthStatus::Unavailable) {
                theme::text_warning
            } else {
                theme::text_on_surface_muted
            })
            .width(Length::Fill)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        row![
            text(executable)
                .size(13)
                .font(theme::FONT_MONO)
                .style(theme::text_on_surface_muted),
            text(format!("Version: {version}"))
                .size(13)
                .style(theme::text_on_surface_muted),
        ]
        .spacing(theme::spacing::LG)
        .wrap(),
        text(format!("Last checked: {checked_at}"))
            .size(12)
            .style(theme::text_on_surface_alt),
    ]
    .spacing(theme::spacing::SM);
    if let Some(runtime_detail) = runtime_detail {
        details = details.push(
            text(runtime_detail)
                .size(13)
                .style(theme::text_warning)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        );
    }

    container(details)
        .padding([12, 14])
        .width(Length::Fill)
        .style(theme::surface_container)
        .into()
}

fn health_record_detail(record: Option<&ManagerHealthRecord>) -> (String, String) {
    match record.map(|record| &record.result) {
        Some(Ok(ManagerAvailability::Available { version })) => (
            version.clone().unwrap_or_else(|| "Not reported".to_owned()),
            "Availability check passed".to_owned(),
        ),
        Some(Ok(ManagerAvailability::Unavailable { reason })) => {
            ("Unavailable".to_owned(), availability_reason_detail(reason))
        }
        Some(Ok(_)) => (
            "Unavailable".to_owned(),
            "Manager is unavailable".to_owned(),
        ),
        Some(Err(error)) => ("Unknown".to_owned(), format!("Check failed: {error}")),
        None => (
            "Not checked".to_owned(),
            "Availability has not been checked".to_owned(),
        ),
    }
}

fn availability_reason_detail(reason: &AvailabilityReason) -> String {
    match reason {
        AvailabilityReason::UnsupportedPlatform { .. } => "Unsupported on this platform".to_owned(),
        AvailabilityReason::CommandMissing { command } => format!("Not found: {command}"),
        AvailabilityReason::NotExecutable { path } => {
            format!("Not executable: {}", path.display())
        }
        AvailabilityReason::VersionCheckFailed { detail } => {
            format!("Version check failed: {detail}")
        }
        _ => "Manager is unavailable".to_owned(),
    }
}

fn runtime_issue_detail(
    manager: &ManagerId,
    installed: &InstalledInfo,
    updates: &UpdatesInfo,
) -> Option<String> {
    let mut issues = Vec::new();
    if let Some(error) = installed.init_errors.get(manager) {
        issues.push(format!("Installed initialization: {error}"));
    }
    if let Some(error) = installed.load_errors.get(manager) {
        issues.push(format!("Installed listing: {error}"));
    }
    if let Some(error) = updates.init_errors.get(manager) {
        issues.push(format!("Update initialization: {error}"));
    }
    if let Some(error) = updates.load_errors.get(manager) {
        issues.push(format!("Update discovery: {error}"));
    }
    (!issues.is_empty()).then(|| issues.join(" | "))
}

fn diagnostics_report(
    config: &updater_core::Config,
    info: &ManagerHealthInfo,
    catalog: &ManagerCatalog,
    installed: &InstalledInfo,
    updates: &UpdatesInfo,
) -> String {
    let mut report = vec![
        "PackageGet Manager Health".to_owned(),
        format!("Generated: {}", activity::now_timestamp()),
        format!(
            "Last scan: {}{}",
            info.finished_at.as_deref().unwrap_or("not completed"),
            if info.last_scan_cancelled {
                " (cancelled)"
            } else {
                ""
            },
        ),
        String::new(),
    ];

    for manager in &config.managers {
        let status = info.status_for(&manager.id, installed, updates);
        let record = info.records.get(&manager.id);
        let (version, detail) = health_record_detail(record);
        let executable = manager.executable().map_or_else(
            || "System PATH".to_owned(),
            |path| path.display().to_string(),
        );
        report.extend([
            format!("{} ({})", catalog.display_name(&manager.id), manager.id),
            format!("Status: {}", status.label()),
            format!("Version: {version}"),
            format!("Executable: {executable}"),
            format!(
                "Last checked: {}",
                record.map_or("never", |record| record.checked_at.as_str())
            ),
            format!("Detail: {detail}"),
        ]);
        if let Some(runtime_detail) = runtime_issue_detail(&manager.id, installed, updates) {
            report.push(format!("Runtime: {runtime_detail}"));
        }
        report.push(String::new());
    }

    report
        .into_iter()
        .map(|line| activity::redact_detail(&line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone)]
enum ScanEvent {
    Started(ManagerId),
    Finished {
        manager: ManagerId,
        checked_at: String,
        result: Result<ManagerAvailability, String>,
    },
    Complete {
        finished_at: String,
        cancelled: bool,
    },
}

fn run_health_scan(
    generation: u64,
    managers: Vec<ManagerConfig>,
    registry: Arc<ManagerRegistry>,
    cancellation: CancellationToken,
) -> Task<Message> {
    let (sender, receiver) = mpsc::unbounded();
    let stream = Task::run(receiver, move |event| match event {
        ScanEvent::Started(manager) => Message::CheckStarted {
            generation,
            manager,
        },
        ScanEvent::Finished {
            manager,
            checked_at,
            result,
        } => Message::CheckFinished {
            generation,
            manager,
            checked_at,
            result,
        },
        ScanEvent::Complete {
            finished_at,
            cancelled,
        } => Message::ScanFinished {
            generation,
            finished_at,
            cancelled,
        },
    });
    let runner = Task::future(async move {
        for manager_config in managers {
            if cancellation.is_cancelled() {
                break;
            }
            let manager_id = manager_config.id.clone();
            let _ = sender.unbounded_send(ScanEvent::Started(manager_id.clone()));
            let result = match registry.get(&manager_id) {
                Some(manager) => manager
                    .availability(&manager_config)
                    .await
                    .map_err(|error| error.to_string()),
                None => Err("No registered implementation in this build".to_owned()),
            };
            let _ = sender.unbounded_send(ScanEvent::Finished {
                manager: manager_id,
                checked_at: activity::now_timestamp(),
                result,
            });
        }
        let _ = sender.unbounded_send(ScanEvent::Complete {
            finished_at: activity::now_timestamp(),
            cancelled: cancellation.is_cancelled(),
        });
    })
    .discard();

    Task::batch(vec![runner, stream])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_id(value: &str) -> ManagerId {
        ManagerId::parse(value).unwrap()
    }

    #[test]
    fn stale_scan_result_does_not_replace_current_generation() {
        let cargo = ManagerConfig::new(manager_id("builtin:cargo"));
        let mut info = ManagerHealthInfo::default();
        let (first, _) = info.begin_scan(std::slice::from_ref(&cargo));
        let (current, _) = info.begin_scan(std::slice::from_ref(&cargo));

        info.apply_result(
            first,
            cargo.id.clone(),
            "old".to_owned(),
            Ok(ManagerAvailability::Available {
                version: Some("old".to_owned()),
            }),
        );

        assert_eq!(current, info.generation);
        assert!(!info.records.contains_key(&cargo.id));
    }

    #[test]
    fn cancellation_is_requested_on_the_active_scan_token() {
        let cargo = ManagerConfig::new(manager_id("builtin:cargo"));
        let mut info = ManagerHealthInfo::default();
        let (_, cancellation) = info.begin_scan(&[cargo]);

        info.request_cancellation();

        assert!(cancellation.is_cancelled());
        assert!(info.cancellation_requested);
    }

    #[test]
    fn runtime_failure_degrades_an_available_manager() {
        let cargo = manager_id("builtin:cargo");
        let mut info = ManagerHealthInfo::default();
        info.scan_scope.insert(cargo.clone());
        info.records.insert(
            cargo.clone(),
            ManagerHealthRecord {
                checked_at: "now".to_owned(),
                result: Ok(ManagerAvailability::Available {
                    version: Some("1.0".to_owned()),
                }),
            },
        );
        let installed = InstalledInfo::default();
        let mut updates = UpdatesInfo::default();
        updates
            .load_errors
            .insert(cargo.clone(), "registry unavailable".to_owned());

        assert_eq!(
            info.status_for(&cargo, &installed, &updates),
            HealthStatus::Degraded
        );
    }

    #[test]
    fn diagnostics_redact_paths_and_credentials() {
        let cargo = manager_id("builtin:cargo");
        let config = updater_core::Config {
            managers: vec![
                ManagerConfig::new(cargo.clone()).with_executable("/home/user/private/cargo"),
            ],
            ..updater_core::Config::default()
        };
        let mut info = ManagerHealthInfo::default();
        info.scan_scope.insert(cargo.clone());
        info.records.insert(
            cargo,
            ManagerHealthRecord {
                checked_at: "now".to_owned(),
                result: Err("token=secret".to_owned()),
            },
        );

        let report = diagnostics_report(
            &config,
            &info,
            &ManagerCatalog::builtin(),
            &InstalledInfo::default(),
            &UpdatesInfo::default(),
        );

        assert!(!report.contains("/home/user/private"));
        assert!(!report.contains("token=secret"));
        assert!(report.contains("<redacted>"));
    }
}
