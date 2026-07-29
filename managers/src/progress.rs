use std::{collections::VecDeque, sync::OnceLock};

use regex::Regex;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::mpsc,
};
use updater_manager_api::{ManagerError, ManagerErrorKind, ManagerResult};

use crate::command::{CommandSpec, command_status_error, io_error, piped_command};

const LINE_CHANNEL_CAPACITY: usize = 64;
const MAX_LINE_BYTES: usize = 2_048;
const TAIL_LINE_COUNT: usize = 20;

/// Bounded command progress emitted by a built-in manager.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct CommandProgress {
    fraction: f32,
    message: Option<String>,
}

impl CommandProgress {
    fn new(fraction: f32, message: Option<String>) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            message,
        }
    }

    /// Returns the normalized completion fraction from `0.0` through `1.0`.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    /// Returns the bounded command output line, when one was emitted.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Consumes the progress value into its fraction and optional message.
    #[must_use]
    pub fn into_parts(self) -> (f32, Option<String>) {
        (self.fraction, self.message)
    }
}

pub(crate) async fn run_command_with_progress(
    spec: &CommandSpec,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    run_command_with_parser(spec, ProgressParser::Percent, on_progress).await
}

pub(crate) async fn run_dnf_command_with_progress(
    spec: &CommandSpec,
    on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    run_command_with_parser(
        spec,
        ProgressParser::Dnf(DnfProgressState::default()),
        on_progress,
    )
    .await
}

async fn run_command_with_parser(
    spec: &CommandSpec,
    mut parser: ProgressParser,
    mut on_progress: impl FnMut(CommandProgress),
) -> ManagerResult<()> {
    let mut child = piped_command(spec)
        .spawn()
        .map_err(|error| io_error("failed to start package manager command", error))?;
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
    on_progress(CommandProgress::new(0.0, None));

    while let Some(line) = receiver.recv().await {
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

    join_reader(stdout_task.await)?;
    join_reader(stderr_task.await)?;

    let status = child
        .wait()
        .await
        .map_err(|error| io_error("failed to wait for package manager command", error))?;
    if !status.success() {
        let tail = tail_logs.into_iter().collect::<Vec<_>>().join("\n");
        return Err(command_status_error(spec, status, &tail));
    }

    on_progress(CommandProgress::new(1.0, None));
    Ok(())
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
    use tokio::io::{AsyncWriteExt, duplex};

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
}
