use std::{collections::VecDeque, process::Stdio};

use regex::Regex;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::mpsc,
};

use crate::{CoreResult, error::CoreError};

#[derive(Debug, Clone)]
pub struct CommandProgressEvent {
    pub progress: f32,
    pub command_message: Option<String>,
}

async fn forward_lines<R: AsyncRead + Unpin>(mut reader: R, tx: mpsc::UnboundedSender<String>) {
    let mut buf = [0u8; 4096];
    let mut current = Vec::new();

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !current.is_empty() {
                            let line = String::from_utf8_lossy(&current).trim().to_string();
                            if !line.is_empty() {
                                let _ = tx.send(line);
                            }
                            current.clear();
                        }
                    } else {
                        current.push(b);
                    }
                }
            }
            Err(_) => break,
        }
    }

    if !current.is_empty() {
        let line = String::from_utf8_lossy(&current).trim().to_string();
        if !line.is_empty() {
            let _ = tx.send(line);
        }
    }
}

fn parse_percent(line: &str, pattern: &Regex) -> Option<f32> {
    let mut best = None;
    for cap in pattern.captures_iter(line) {
        let value = cap.get(1)?.as_str().parse::<f32>().ok()?;
        best = Some(value.clamp(0.0, 100.0) / 100.0);
    }
    best
}

fn parse_step_ratio(line: &str, pattern: &Regex) -> Option<f32> {
    let mut best = None;
    for cap in pattern.captures_iter(line) {
        let current = cap.get(1)?.as_str().parse::<f32>().ok()?;
        let total = cap.get(2)?.as_str().parse::<f32>().ok()?;
        if total > 0.0 {
            best = Some((current / total).clamp(0.0, 1.0));
        }
    }
    best
}

pub async fn run_command_with_progress(
    command: &str,
    args: &[String],
    mut on_progress: impl FnMut(CommandProgressEvent),
) -> CoreResult<()> {
    let mut child = Command::new(command)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CoreError::CommandError("failed to capture stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| CoreError::CommandError("failed to capture stderr".to_string()))?;

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let tx_out = tx.clone();
    let tx_err = tx.clone();

    tokio::spawn(async move {
        forward_lines(stdout, tx_out).await;
    });
    tokio::spawn(async move {
        forward_lines(stderr, tx_err).await;
    });
    drop(tx);

    let percent_pattern =
        Regex::new(r"([0-9]{1,3}(?:\.[0-9]+)?)%").expect("valid regex for percent parsing");
    let step_pattern =
        Regex::new(r"\[([0-9]+)\s*/\s*([0-9]+)\]").expect("valid regex for step parsing");
    let is_dnf = command == "dnf"
        || command.ends_with("/dnf")
        || args
            .first()
            .is_some_and(|first| first == "dnf" || first.ends_with("/dnf"));

    let mut in_transaction_phase = false;
    let mut max_progress = 0.0f32;
    let mut tail_logs: VecDeque<String> = VecDeque::new();

    on_progress(CommandProgressEvent {
        progress: 0.0,
        command_message: None,
    });

    while let Some(line) = rx.recv().await {
        if tail_logs.len() >= 20 {
            tail_logs.pop_front();
        }
        tail_logs.push_back(line.clone());

        on_progress(CommandProgressEvent {
            progress: max_progress,
            command_message: Some(line.clone()),
        });

        if is_dnf {
            if line.contains("Running transaction") {
                in_transaction_phase = true;
            }

            if let Some(step_ratio) = parse_step_ratio(&line, &step_pattern) {
                // DNF typically has two phases:
                // 1) download and resolve
                // 2) running transaction
                // Map them to distinct ranges to avoid jumping straight to 99%.
                let value = if in_transaction_phase {
                    0.60 + (step_ratio * 0.39)
                } else {
                    step_ratio * 0.60
                };

                let value = value.min(0.99);
                if value > max_progress {
                    max_progress = value;
                    on_progress(CommandProgressEvent {
                        progress: value,
                        command_message: None,
                    });
                }
                continue;
            }

            if let Some(percent_ratio) = parse_percent(&line, &percent_pattern) {
                let value = if in_transaction_phase {
                    0.60 + (percent_ratio * 0.39)
                } else {
                    percent_ratio * 0.60
                };

                let value = value.min(0.99);
                if value > max_progress {
                    max_progress = value;
                    on_progress(CommandProgressEvent {
                        progress: value,
                        command_message: None,
                    });
                }
            }
        } else if let Some(value) = parse_percent(&line, &percent_pattern) {
            let value = value.min(0.99);
            if value > max_progress {
                max_progress = value;
                on_progress(CommandProgressEvent {
                    progress: value,
                    command_message: None,
                });
            }
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        let tail = tail_logs.into_iter().collect::<Vec<_>>().join("\n");
        let detail = if tail.trim().is_empty() {
            format!("{} {:?} exited with {}", command, args, status)
        } else {
            format!("{} {:?} failed:\n{}", command, args, tail)
        };
        return Err(CoreError::UnknownError(detail));
    }

    on_progress(CommandProgressEvent {
        progress: 1.0,
        command_message: None,
    });
    Ok(())
}
