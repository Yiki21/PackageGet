use std::{collections::VecDeque, path::Path, process::Stdio};

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
        let current = cap.get(1)?.as_str().parse::<usize>().ok()?;
        let total = cap.get(2)?.as_str().parse::<usize>().ok()?;

        // Filter out non-progress fragments such as "2026/03".
        if total > 0 && current > 0 && current <= total {
            let ratio = current as f32 / total as f32;
            best = Some(best.map_or(ratio, |prev: f32| prev.max(ratio)));
        }
    }
    best
}

fn command_looks_like_dnf(command: &str) -> bool {
    let executable = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
        .to_ascii_lowercase();

    executable == "dnf" || executable.starts_with("dnf")
}

fn is_dnf_command(command: &str, args: &[String]) -> bool {
    command_looks_like_dnf(command)
        || args
            .first()
            .is_some_and(|first| command_looks_like_dnf(first))
}

fn is_dnf_transaction_marker(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("running transaction") || line.contains("运行事务")
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
    let step_pattern = Regex::new(r"(?:[\[(]\s*)?([0-9]+)\s*/\s*([0-9]+)(?:\s*[\])])?")
        .expect("valid regex for step parsing");
    let is_dnf = is_dnf_command(command, args);

    let mut in_transaction_phase = false;
    let mut previous_step_ratio = None::<f32>;
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
            if is_dnf_transaction_marker(&line) {
                in_transaction_phase = true;
            }

            if let Some(step_ratio) = parse_step_ratio(&line, &step_pattern) {
                // Some DNF outputs do not include a localized transaction marker.
                // A significant ratio reset is typically the phase boundary.
                if !in_transaction_phase
                    && previous_step_ratio
                        .is_some_and(|prev| prev >= 0.9 && step_ratio < prev - 0.2)
                {
                    in_transaction_phase = true;
                }
                previous_step_ratio = Some(step_ratio);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn step_pattern() -> Regex {
        Regex::new(r"(?:[\[(]\s*)?([0-9]+)\s*/\s*([0-9]+)(?:\s*[\])])?")
            .expect("step regex should compile")
    }

    #[test]
    fn parse_step_ratio_supports_common_dnf_formats() {
        let pattern = step_pattern();

        let bracketed = parse_step_ratio("Progress: [3/10]", &pattern);
        let parenthesized = parse_step_ratio("(4/10): package.rpm", &pattern);
        let plain = parse_step_ratio("Upgrading : foo.x86_64 7/10", &pattern);

        assert_eq!(bracketed, Some(0.3));
        assert_eq!(parenthesized, Some(0.4));
        assert_eq!(plain, Some(0.7));
    }

    #[test]
    fn parse_step_ratio_ignores_non_progress_pairs() {
        let pattern = step_pattern();

        // Year/month style pairs should be ignored.
        assert_eq!(parse_step_ratio("mirror path 2026/03", &pattern), None);
        // Zero-total is not a valid progress signal.
        assert_eq!(parse_step_ratio("Transaction: 0/0", &pattern), None);
    }

    #[test]
    fn is_dnf_command_detects_wrapped_and_custom_binary_names() {
        assert!(is_dnf_command("dnf", &[]));
        assert!(is_dnf_command("/usr/bin/dnf5", &[]));
        assert!(is_dnf_command(
            "pkexec",
            &[String::from("/usr/local/bin/dnf5"), String::from("upgrade")]
        ));
        assert!(!is_dnf_command("pkexec", &[String::from("/usr/bin/apt")]));
    }

    #[test]
    fn is_dnf_transaction_marker_supports_multiple_locales() {
        assert!(is_dnf_transaction_marker("Running transaction"));
        assert!(is_dnf_transaction_marker("开始运行事务"));
        assert!(!is_dnf_transaction_marker("Downloading packages"));
    }
}
