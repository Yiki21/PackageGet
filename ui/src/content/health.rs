//! Read-only health checks for configured package managers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::channel::mpsc;
use iced::{Element, Task};
use updater_core::{CancellationToken, ManagerRegistry};
use updater_manager_api::{AvailabilityReason, ManagerAvailability, ManagerConfig, ManagerId};

use crate::{
    activity,
    content::{InstalledInfo, UpdatesInfo, shared},
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
}

#[derive(Debug, Clone, Default)]
pub struct HealthCenter;

#[derive(Debug, Clone)]
pub enum Message {
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
}

#[derive(Debug)]
pub enum Action {
    None,
    Run(Task<Message>),
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
        use iced::widget::{button, column, row, text};

        let summary = HealthSummary::new(config, info, installed, updates);
        let check = button(
            text(if info.has_results() {
                "Recheck"
            } else {
                "Check all"
            })
            .size(14)
            .font(theme::FONT_SEMIBOLD),
        )
        .padding([8, 12])
        .style(theme::secondary_button(!info.is_checking))
        .on_press_maybe((!info.is_checking).then_some(Message::StartScan));
        let cancel = button(
            text(if info.cancellation_requested {
                "Stopping..."
            } else {
                "Cancel"
            })
            .size(14)
            .font(theme::FONT_SEMIBOLD),
        )
        .padding([8, 12])
        .style(theme::secondary_button(
            info.is_checking && !info.cancellation_requested,
        ))
        .on_press_maybe(
            (info.is_checking && !info.cancellation_requested).then_some(Message::CancelScan),
        );
        let copy = button(text("Copy report").size(14).font(theme::FONT_SEMIBOLD))
            .padding([8, 12])
            .style(theme::secondary_button(info.has_results()))
            .on_press_maybe(info.has_results().then_some(Message::CopyDiagnostics));

        let toolbar = shared::toolbar(row![check, cancel, copy].spacing(theme::spacing::SM).wrap());

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
        ]
        .spacing(theme::spacing::LG)
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
