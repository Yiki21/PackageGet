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
        if let Some(progress) = parse_percent(&line)
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

fn join_reader(result: Result<(), tokio::task::JoinError>) -> ManagerResult<()> {
    result.map_err(|error| {
        ManagerError::new(
            ManagerErrorKind::Other,
            "package manager output reader failed",
        )
        .with_detail(error.to_string())
    })
}
