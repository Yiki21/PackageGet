use std::{collections::VecDeque, io, process::ExitStatus, sync::OnceLock, time::Duration};

use regex::Regex;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Child,
    sync::mpsc,
    time::{Instant, MissedTickBehavior, interval},
};
use updater_manager_api::{ManagerError, ManagerErrorKind, ManagerResult, ProgressSink};

use crate::command::{CommandSpec, command_status_error, io_error, piped_command};

const LINE_CHANNEL_CAPACITY: usize = 64;
const MAX_LINE_BYTES: usize = 2_048;
const TAIL_LINE_COUNT: usize = 20;
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);
const TERMINATION_GRACE_PERIOD: Duration = Duration::from_secs(2);
/// How long the runner keeps draining output after the direct child exits.
/// A descendant that inherited the pipes can hold them open indefinitely.
const OUTPUT_DRAIN_DEADLINE: Duration = Duration::from_secs(2);
const TRUNCATED_OUTPUT_NOTICE: &str =
    "command output was truncated: a child process kept the output pipe open";
const EXECUTABLE_BUSY_RETRIES: usize = 3;
const EXECUTABLE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(20);

/// Bounded command progress emitted by a built-in manager.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub(crate) struct CommandProgress {
    fraction: f32,
    message: Option<String>,
}

impl CommandProgress {
    pub(crate) fn new(fraction: f32, message: Option<String>) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            message,
        }
    }

    /// Returns the normalized completion fraction from `0.0` through `1.0`.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn fraction(&self) -> f32 {
        self.fraction
    }

    /// Returns the bounded command output line, when one was emitted.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Consumes the progress value into its fraction and optional message.
    #[must_use]
    pub(crate) fn into_parts(self) -> (f32, Option<String>) {
        (self.fraction, self.message)
    }
}

pub(crate) async fn run_command_with_progress(
    spec: &CommandSpec,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    run_command_with_parser(
        spec,
        ProgressParser::Percent,
        command_status_error,
        || false,
        on_progress,
    )
    .await
}

pub(crate) async fn run_cancellable_command_with_progress(
    spec: &CommandSpec,
    cancellation: &dyn ProgressSink,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    run_command_with_parser(
        spec,
        ProgressParser::Percent,
        command_status_error,
        || cancellation.is_cancelled(),
        on_progress,
    )
    .await
}

pub(crate) async fn run_command_with_progress_and_status(
    spec: &CommandSpec,
    status_error: fn(&CommandSpec, ExitStatus, &str) -> ManagerError,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    run_command_with_parser(
        spec,
        ProgressParser::Percent,
        status_error,
        || false,
        on_progress,
    )
    .await
}

pub(crate) async fn run_cancellable_command_with_progress_and_status(
    spec: &CommandSpec,
    cancellation: &dyn ProgressSink,
    status_error: fn(&CommandSpec, ExitStatus, &str) -> ManagerError,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    run_command_with_parser(
        spec,
        ProgressParser::Percent,
        status_error,
        || cancellation.is_cancelled(),
        on_progress,
    )
    .await
}

#[allow(dead_code)]
pub(crate) async fn run_dnf_command_with_progress(
    spec: &CommandSpec,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    run_command_with_parser(
        spec,
        ProgressParser::Dnf(DnfProgressState::default()),
        command_status_error,
        || false,
        on_progress,
    )
    .await
}

pub(crate) async fn run_cancellable_dnf_command_with_progress(
    spec: &CommandSpec,
    cancellation: &dyn ProgressSink,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    run_command_with_parser(
        spec,
        ProgressParser::Dnf(DnfProgressState::default()),
        command_status_error,
        || cancellation.is_cancelled(),
        on_progress,
    )
    .await
}

async fn run_command_with_parser(
    spec: &CommandSpec,
    mut parser: ProgressParser,
    status_error: fn(&CommandSpec, ExitStatus, &str) -> ManagerError,
    is_cancelled: impl Fn() -> bool,
    mut on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    if is_cancelled() {
        return Err(cancelled_error());
    }
    let mut attempt = 0;
    let mut child = loop {
        let result = piped_command(spec).spawn();
        match result {
            Ok(process) => break process,
            Err(error) if is_executable_busy(&error) && attempt < EXECUTABLE_BUSY_RETRIES => {
                attempt += 1;
                tokio::time::sleep(EXECUTABLE_BUSY_RETRY_DELAY).await;
            }
            Err(error) => {
                return Err(io_error("failed to start package manager command", error));
            }
        }
    };
    // `Process::id` is only available while the direct child runs; the
    // process group outlives it when descendants inherit the pipes. Only
    // Unix signals the group after the child is reaped, so only Unix saves it.
    #[cfg(unix)]
    let process_group = child.id();
    let stdout = child.stdout.take().ok_or_else(|| {
        ManagerError::new(
            ManagerErrorKind::Other,
            "failed to capture package manager stdout",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        ManagerError::new(
            ManagerErrorKind::Other,
            "failed to capture package manager stderr",
        )
    })?;

    let (sender, mut receiver) = mpsc::channel::<String>(LINE_CHANNEL_CAPACITY);
    let stdout_task = tokio::spawn(forward_lines(stdout, sender.clone()));
    let stderr_task = tokio::spawn(forward_lines(stderr, sender.clone()));
    drop(sender);

    let mut max_progress = 0.0_f32;
    let mut tail_logs = VecDeque::with_capacity(TAIL_LINE_COUNT);
    let mut status = None;
    let mut output_open = true;
    let mut cancellation_requested = false;
    let mut termination_error = None;
    let mut force_at = None;
    let mut forced = false;
    let mut drain_deadline = None;
    let mut output_truncated = false;
    // A privileged transaction runs as root behind `pkexec`. An unprivileged
    // sender cannot signal that process group, and if the GUI runs as root,
    // signalling it would interrupt a package database transaction, so the
    // cancellation is recorded and the transaction is allowed to finish.
    let privileged = spec.is_privileged();
    let mut poll = interval(CANCELLATION_POLL_INTERVAL);
    poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    on_progress(CommandProgress::new(0.0, None));

    while status.is_none() || output_open {
        tokio::select! {
            line = receiver.recv(), if output_open => {
                let Some(line) = line else {
                    output_open = false;
                    continue;
                };
                if tail_logs.len() == TAIL_LINE_COUNT {
                    tail_logs.pop_front();
                }
                tail_logs.push_back(line.clone());

                on_progress(CommandProgress::new(max_progress, Some(line.clone())));
                if let Some(progress) = parser.parse(&line)
                    && progress > max_progress
                {
                    max_progress = progress.min(0.99);
                    on_progress(CommandProgress::new(max_progress, None));
                }
            }
            _ = poll.tick() => {
                if status.is_none() {
                    status = child.try_wait().map_err(|error| {
                        io_error("failed to query package manager command", error)
                    })?;
                    if status.is_some() {
                        // The direct child exited but a descendant may still
                        // hold the pipes open, so bound the remaining drain
                        // instead of waiting for an end of file that may never
                        // arrive.
                        drain_deadline = Some(Instant::now() + OUTPUT_DRAIN_DEADLINE);
                        continue;
                    }

                    if !cancellation_requested && is_cancelled() {
                        cancellation_requested = true;
                        if !privileged {
                            force_at = Some(Instant::now() + TERMINATION_GRACE_PERIOD);
                            if let Err(error) = terminate_process_tree(&mut child, false).await {
                                termination_error = Some(error);
                            }
                        }
                    } else if !privileged
                        && !forced
                        && force_at.is_some_and(|deadline| Instant::now() >= deadline)
                    {
                        forced = true;
                        if let Err(error) = terminate_process_tree(&mut child, true).await {
                            termination_error.get_or_insert(error);
                        }
                    }
                    continue;
                }

                let cancelled = !cancellation_requested && is_cancelled();
                let drain_expired =
                    drain_deadline.is_some_and(|deadline| Instant::now() >= deadline);
                if !cancelled && !drain_expired {
                    continue;
                }

                cancellation_requested |= cancelled;
                output_truncated = true;
                output_open = false;
                // `try_wait` has already reaped the direct child here. On Unix
                // the process-group ID stays reserved while any member of the
                // group is alive, so signalling it cannot reach an unrelated
                // process. On Windows the reaped child's PID may already be
                // reused, and `taskkill /T /F` would kill whatever tree now
                // owns it, so the readers are only aborted there.
                #[cfg(unix)]
                {
                    if !privileged
                        && let Some(pid) = process_group
                        && let Err(error) = terminate_process_group(pid, true).await
                    {
                        termination_error.get_or_insert(error);
                    }
                }
                if tail_logs.len() == TAIL_LINE_COUNT {
                    tail_logs.pop_front();
                }
                tail_logs.push_back(TRUNCATED_OUTPUT_NOTICE.to_owned());
                on_progress(CommandProgress::new(
                    max_progress,
                    Some(TRUNCATED_OUTPUT_NOTICE.to_owned()),
                ));
            }
        }
    }

    let cancellation_delivered =
        cancellation_requested && !privileged && termination_error.is_none();
    if output_truncated || cancellation_delivered {
        // A descendant still holds the pipes, so the readers never see an end
        // of file; abort them instead of waiting for one.
        stdout_task.abort();
        stderr_task.abort();
        let _ = stdout_task.await;
        let _ = stderr_task.await;
    } else {
        join_reader(stdout_task.await)?;
        join_reader(stderr_task.await)?;
    }

    let status = match status {
        Some(status) => status,
        None => child
            .wait()
            .await
            .map_err(|error| io_error("failed to wait for package manager command", error))?,
    };
    if cancellation_delivered {
        return Err(cancelled_error());
    }
    if !status.success() {
        let tail = tail_logs.into_iter().collect::<Vec<_>>().join("\n");
        return Err(status_error(spec, status, &tail));
    }

    on_progress(CommandProgress::new(1.0, None));
    Ok(())
}

fn is_executable_busy(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ETXTBSY)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

fn cancelled_error() -> ManagerError {
    ManagerError::new(
        ManagerErrorKind::Cancelled,
        "package manager command was cancelled",
    )
}

#[cfg(unix)]
async fn terminate_process_tree(child: &mut Child, force: bool) -> io::Result<()> {
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("child PID is unavailable"))?;
    terminate_process_group(pid, force).await
}

#[cfg(unix)]
async fn terminate_process_group(pid: u32, force: bool) -> io::Result<()> {
    let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
    let result = unsafe { libc::kill(-(pid as i32), signal) };
    if result == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(windows)]
async fn terminate_process_group(pid: u32, _force: bool) -> io::Result<()> {
    let status = tokio::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("taskkill exited with {status}")))
    }
}

#[cfg(windows)]
async fn terminate_process_tree(child: &mut Child, force: bool) -> io::Result<()> {
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("child PID is unavailable"))?;
    terminate_process_group(pid, force).await
}

#[cfg(not(any(unix, windows)))]
async fn terminate_process_tree(child: &mut Child, _force: bool) -> io::Result<()> {
    child.start_kill()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ProgressParser {
    Percent,
    Dnf(DnfProgressState),
}

impl ProgressParser {
    fn parse(&mut self, line: &str) -> Option<f32> {
        match self {
            Self::Percent => parse_percent(line),
            Self::Dnf(state) => state.parse(line),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum DnfPhase {
    #[default]
    Download,
    Transaction,
}

impl DnfPhase {
    fn scale(self, ratio: f32) -> f32 {
        match self {
            Self::Download => ratio * 0.60,
            Self::Transaction => 0.60 + (ratio * 0.39),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct DnfProgressState {
    phase: DnfPhase,
    previous_step_ratio: Option<f32>,
}

impl DnfProgressState {
    fn parse(&mut self, line: &str) -> Option<f32> {
        if is_dnf_transaction_marker(line) {
            self.phase = DnfPhase::Transaction;
        }

        if let Some(step_ratio) = parse_step_ratio(line) {
            if self.phase == DnfPhase::Download
                && self
                    .previous_step_ratio
                    .is_some_and(|previous| previous >= 0.9 && step_ratio < previous - 0.2)
            {
                self.phase = DnfPhase::Transaction;
            }
            self.previous_step_ratio = Some(step_ratio);
            return Some(self.phase.scale(step_ratio));
        }

        parse_percent(line).map(|ratio| self.phase.scale(ratio))
    }
}

async fn forward_lines<R>(mut reader: R, sender: mpsc::Sender<String>)
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 4_096];
    let mut current = Vec::with_capacity(MAX_LINE_BYTES);

    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => {
                for &byte in &buffer[..read] {
                    if matches!(byte, b'\n' | b'\r') {
                        if !send_line(&sender, &mut current).await {
                            return;
                        }
                    } else if current.len() < MAX_LINE_BYTES {
                        current.push(byte);
                    }
                }
            }
            Err(_) => break,
        }
    }

    let _ = send_line(&sender, &mut current).await;
}

async fn send_line(sender: &mpsc::Sender<String>, current: &mut Vec<u8>) -> bool {
    if current.is_empty() {
        return true;
    }

    let line = String::from_utf8_lossy(current).trim().to_owned();
    current.clear();
    line.is_empty() || sender.send(line).await.is_ok()
}

fn parse_percent(line: &str) -> Option<f32> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r"([0-9]{1,3}(?:\.[0-9]+)?)%").expect("percent progress regex must remain valid")
    });

    pattern
        .captures_iter(line)
        .filter_map(|capture| capture.get(1)?.as_str().parse::<f32>().ok())
        .map(|value| value.clamp(0.0, 100.0) / 100.0)
        .last()
}

fn parse_step_ratio(line: &str) -> Option<f32> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r"(?:[\[(]\s*)?([0-9]+)\s*/\s*([0-9]+)(?:\s*[\])])?")
            .expect("DNF step progress regex must remain valid")
    });

    pattern
        .captures_iter(line)
        .filter_map(|capture| {
            let current = capture.get(1)?.as_str().parse::<usize>().ok()?;
            let total = capture.get(2)?.as_str().parse::<usize>().ok()?;
            (total > 0 && current > 0 && current <= total).then_some(current as f32 / total as f32)
        })
        .reduce(f32::max)
}

fn is_dnf_transaction_marker(line: &str) -> bool {
    line.to_ascii_lowercase().contains("running transaction") || line.contains("运行事务")
}

fn join_reader(result: Result<(), tokio::task::JoinError>) -> ManagerResult<()> {
    result.map_err(|error| {
        ManagerError::new(
            ManagerErrorKind::Other,
            "package manager output reader failed",
        )
        .with_detail(error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use tokio::io::{AsyncWriteExt, duplex};
    use updater_manager_api::{ProgressEvent, ProgressSink};

    use super::*;

    #[test]
    fn percent_parser_uses_the_last_bounded_value() {
        assert_eq!(parse_percent("download 10% verify 42.5%"), Some(0.425));
        assert_eq!(parse_percent("unexpected 125%"), Some(1.0));
        assert_eq!(parse_percent("no progress"), None);
    }

    #[test]
    fn step_parser_supports_common_dnf_formats_and_rejects_dates() {
        assert_eq!(parse_step_ratio("[3/10] package"), Some(0.3));
        assert_eq!(parse_step_ratio("(8 / 10) package"), Some(0.8));
        assert_eq!(parse_step_ratio("step 2/5 then 4/5"), Some(0.8));
        assert_eq!(parse_step_ratio("release 2026/03"), None);
        assert_eq!(parse_step_ratio("invalid 0/10 12/10"), None);
    }

    #[test]
    fn dnf_progress_maps_download_and_transaction_phases() {
        let mut state = DnfProgressState::default();

        assert_eq!(state.parse("[5/10] Downloading"), Some(0.3));
        assert_eq!(state.phase, DnfPhase::Download);
        assert_eq!(state.parse("Running transaction"), None);
        assert_eq!(state.phase, DnfPhase::Transaction);
        assert_eq!(state.parse("[5/10] Installing"), Some(0.795));
        assert_eq!(state.parse("验证 50%"), Some(0.795));
    }

    #[test]
    fn dnf_progress_detects_localized_marker_and_step_reset() {
        let mut localized = DnfProgressState::default();
        assert_eq!(localized.parse("开始运行事务"), None);
        assert_eq!(localized.phase, DnfPhase::Transaction);

        let mut reset = DnfProgressState::default();
        assert_eq!(reset.parse("[10/10] Downloading"), Some(0.6));
        let transaction_progress = reset
            .parse("[1/5] Installing")
            .expect("parse transaction progress after a step reset");
        assert!((transaction_progress - 0.678).abs() < f32::EPSILON * 2.0);
        assert_eq!(reset.phase, DnfPhase::Transaction);
    }

    #[test]
    fn command_progress_clamps_fraction_and_preserves_message() {
        let progress = CommandProgress::new(1.5, Some("done".to_owned()));
        assert_eq!(progress.fraction(), 1.0);
        assert_eq!(progress.message(), Some("done"));
        assert_eq!(progress.into_parts(), (1.0, Some("done".to_owned())));
    }

    #[tokio::test]
    async fn line_forwarding_bounds_long_command_output() {
        let (reader, mut writer) = duplex(MAX_LINE_BYTES * 2);
        let (sender, mut receiver) = mpsc::channel(1);
        let task = tokio::spawn(forward_lines(reader, sender));

        writer
            .write_all(&vec![b'x'; MAX_LINE_BYTES + 512])
            .await
            .expect("write long command line");
        writer.write_all(b"\n").await.expect("finish command line");
        drop(writer);

        let line = receiver.recv().await.expect("receive bounded line");
        assert_eq!(line.len(), MAX_LINE_BYTES);
        task.await.expect("line forwarding task");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn commands_read_end_of_file_from_a_closed_stdin() {
        let mut lines = Vec::new();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_command_with_progress(
                &CommandSpec::new("sh").args(["-c", "read line; echo \"read exited $?\""]),
                |progress| lines.extend(progress.into_parts().1),
            ),
        )
        .await
        .expect("a command reading stdin must not wait for input");

        result.expect("run stdin reader");
        assert_eq!(lines, ["read exited 1"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_terminates_a_silent_command_and_waits_for_exit() {
        struct Cancellation(Arc<AtomicBool>);

        impl ProgressSink for Cancellation {
            fn emit(&self, _event: ProgressEvent) {}

            fn is_cancelled(&self) -> bool {
                self.0.load(Ordering::Acquire)
            }
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        let request = Arc::clone(&cancelled);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            request.store(true, Ordering::Release);
        });

        let started = std::time::Instant::now();
        let result = run_cancellable_command_with_progress(
            &CommandSpec::new("sh").args(["-c", "sleep 30 & wait"]),
            &Cancellation(cancelled),
            |_| {},
        )
        .await;

        let error = result.expect_err("command should be cancelled");
        assert_eq!(error.kind(), ManagerErrorKind::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn surviving_grandchild_does_not_block_the_runner_past_the_drain_deadline() {
        let mut lines = Vec::new();
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            run_command_with_progress(
                &CommandSpec::new("sh").args(["-c", "sleep 600 &"]),
                |progress| lines.extend(progress.into_parts().1),
            ),
        )
        .await
        .expect("a surviving grandchild must not keep the runner alive");

        result.expect("the direct child exits successfully");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            lines.iter().any(|line| line == TRUNCATED_OUTPUT_NOTICE),
            "truncated output must be reported: {lines:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_during_the_post_exit_drain_is_honoured() {
        struct Cancellation(Arc<AtomicBool>);

        impl ProgressSink for Cancellation {
            fn emit(&self, _event: ProgressEvent) {}

            fn is_cancelled(&self) -> bool {
                self.0.load(Ordering::Acquire)
            }
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        let request = Arc::clone(&cancelled);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            request.store(true, Ordering::Release);
        });

        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            run_cancellable_command_with_progress(
                &CommandSpec::new("sh").args(["-c", "sleep 600 &"]),
                &Cancellation(cancelled),
                |_| {},
            ),
        )
        .await
        .expect("cancellation during the post-exit drain must not hang");

        let error = result.expect_err("cancelling the drain reports a cancellation");
        assert_eq!(error.kind(), ManagerErrorKind::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn privileged_cancellation_is_recorded_and_reports_the_real_result() {
        struct Cancellation(Arc<AtomicBool>);

        impl ProgressSink for Cancellation {
            fn emit(&self, _event: ProgressEvent) {}

            fn is_cancelled(&self) -> bool {
                self.0.load(Ordering::Acquire)
            }
        }

        let directory = tempfile::tempdir().expect("create marker directory");
        let marker = directory.path().join("transaction-finished");
        let cancelled = Arc::new(AtomicBool::new(false));
        let request = Arc::clone(&cancelled);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            request.store(true, Ordering::Release);
        });

        let result = run_cancellable_command_with_progress(
            &CommandSpec::new("sh")
                .arg("-c")
                .arg(format!("sleep 0.4; printf done > '{}'", marker.display()))
                .privileged(),
            &Cancellation(cancelled),
            |_| {},
        )
        .await;

        result.expect("a privileged transaction that exits 0 succeeds");
        assert!(
            marker.exists(),
            "the privileged transaction must not be signalled"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unprivileged_cancellation_still_terminates_the_command() {
        struct Cancellation(Arc<AtomicBool>);

        impl ProgressSink for Cancellation {
            fn emit(&self, _event: ProgressEvent) {}

            fn is_cancelled(&self) -> bool {
                self.0.load(Ordering::Acquire)
            }
        }

        let directory = tempfile::tempdir().expect("create marker directory");
        let marker = directory.path().join("command-finished");
        let cancelled = Arc::new(AtomicBool::new(false));
        let request = Arc::clone(&cancelled);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            request.store(true, Ordering::Release);
        });

        let error = run_cancellable_command_with_progress(
            &CommandSpec::new("sh")
                .arg("-c")
                .arg(format!("sleep 0.4; printf done > '{}'", marker.display())),
            &Cancellation(cancelled),
            |_| {},
        )
        .await
        .expect_err("an unprivileged command is cancelled");

        assert_eq!(error.kind(), ManagerErrorKind::Cancelled);
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            !marker.exists(),
            "the unprivileged command must be terminated"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn privileged_cancellation_reports_a_failing_transaction_as_a_status_error() {
        struct Cancellation(Arc<AtomicBool>);

        impl ProgressSink for Cancellation {
            fn emit(&self, _event: ProgressEvent) {}

            fn is_cancelled(&self) -> bool {
                self.0.load(Ordering::Acquire)
            }
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        let request = Arc::clone(&cancelled);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            request.store(true, Ordering::Release);
        });

        let error = run_cancellable_command_with_progress(
            &CommandSpec::new("sh")
                .args(["-c", "sleep 0.4; exit 7"])
                .privileged(),
            &Cancellation(cancelled),
            |_| {},
        )
        .await
        .expect_err("a failing privileged transaction reports its status");

        assert_ne!(error.kind(), ManagerErrorKind::Cancelled);
        assert_eq!(error.message(), "package manager command failed");
        assert!(
            error.detail().is_some_and(|detail| detail.contains("exit")),
            "{error:?}"
        );
    }
}
