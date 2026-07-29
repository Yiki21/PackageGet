use std::{collections::VecDeque, process::Stdio};

use regex::Regex;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::mpsc,
};

use crate::{CoreResult, error::CoreError, pm::common::manager_command};

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

pub async fn run_command_with_progress(
    command: &str,
    args: &[String],
    on_progress: impl FnMut(CommandProgressEvent),
) -> CoreResult<()> {
    run_command_with_progress_env(command, args, &[], on_progress).await
}

pub async fn run_command_with_progress_env(
    command: &str,
    args: &[String],
    env: &[(&str, &str)],
    mut on_progress: impl FnMut(CommandProgressEvent),
) -> CoreResult<()> {
    let mut child = manager_command(command);
    child.args(args).envs(env.iter().copied());
    let mut child = child
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

        if let Some(value) = parse_percent(&line, &percent_pattern) {
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
        return Err(CoreError::from_command_failure(detail));
    }

    on_progress(CommandProgressEvent {
        progress: 1.0,
        command_message: None,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_parser_keeps_the_last_bounded_value() {
        let pattern =
            Regex::new(r"([0-9]{1,3}(?:\.[0-9]+)?)%").expect("percent regex should compile");

        assert_eq!(
            parse_percent("download 10% verify 42.5%", &pattern),
            Some(0.425)
        );
        assert_eq!(parse_percent("unexpected 125%", &pattern), Some(1.0));
        assert_eq!(parse_percent("no progress", &pattern), None);
    }
}
