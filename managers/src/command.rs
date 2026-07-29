use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{ExitStatus, Output, Stdio},
    time::Duration,
};

use directories_next::UserDirs;
use tokio::{process::Command, time::timeout};
use updater_manager_api::{
    AvailabilityReason, ManagerAvailability, ManagerConfig, ManagerError, ManagerErrorKind,
    ManagerResult,
};

const MAX_DIAGNOSTIC_CHARS: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    program: PathBuf,
    args: Vec<OsString>,
}

impl CommandSpec {
    pub(crate) fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    pub(crate) fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub(crate) fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub(crate) fn program(&self) -> &Path {
        &self.program
    }

    pub(crate) fn arguments(&self) -> &[OsString] {
        &self.args
    }
}

pub(crate) async fn manager_availability(
    config: &ManagerConfig,
    default_program: &str,
    version_args: &[&str],
) -> ManagerAvailability {
    let program = resolve_executable(config, default_program);

    if config.executable().is_some() {
        let metadata = match tokio::fs::metadata(&program).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return ManagerAvailability::Unavailable {
                    reason: AvailabilityReason::CommandMissing {
                        command: program.to_string_lossy().into_owned(),
                    },
                };
            }
            Err(error) => {
                return ManagerAvailability::Unavailable {
                    reason: AvailabilityReason::VersionCheckFailed {
                        detail: bounded_text(&error.to_string()),
                    },
                };
            }
        };

        if !metadata.is_file() || !is_executable(&metadata) {
            return ManagerAvailability::Unavailable {
                reason: AvailabilityReason::NotExecutable { path: program },
            };
        }
    }

    let spec = CommandSpec::new(program.clone()).args(version_args.iter().copied());
    match timeout(Duration::from_secs(5), run_output(&spec)).await {
        Ok(Ok(output)) if output.status.success() => ManagerAvailability::Available {
            version: detected_version(&output),
        },
        Ok(Ok(output)) => ManagerAvailability::Unavailable {
            reason: AvailabilityReason::VersionCheckFailed {
                detail: output_detail(&output, &format_command(&spec)),
            },
        },
        Ok(Err(error)) if error.kind() == ManagerErrorKind::CommandMissing => {
            ManagerAvailability::Unavailable {
                reason: AvailabilityReason::CommandMissing {
                    command: program.to_string_lossy().into_owned(),
                },
            }
        }
        Ok(Err(error)) => ManagerAvailability::Unavailable {
            reason: AvailabilityReason::VersionCheckFailed {
                detail: error.detail().unwrap_or_else(|| error.message()).to_owned(),
            },
        },
        Err(_) => ManagerAvailability::Unavailable {
            reason: AvailabilityReason::VersionCheckFailed {
                detail: format!("{} timed out", format_command(&spec)),
            },
        },
    }
}

pub(crate) fn resolve_executable(config: &ManagerConfig, default_program: &str) -> PathBuf {
    config.executable().map_or_else(
        || {
            find_executable(default_program, &manager_search_directories())
                .unwrap_or_else(|| PathBuf::from(default_program))
        },
        Path::to_path_buf,
    )
}

pub(crate) async fn run_output(spec: &CommandSpec) -> ManagerResult<Output> {
    build_command(spec)
        .output()
        .await
        .map_err(|error| io_error("failed to execute package manager command", error))
}

pub(crate) fn require_success(
    spec: &CommandSpec,
    output: Output,
    message: &str,
) -> ManagerResult<Output> {
    if output.status.success() {
        return Ok(output);
    }

    Err(command_failure(
        message,
        &output_detail(&output, &format_command(spec)),
    ))
}

pub(crate) fn decode_stdout(output: Output, message: &str) -> ManagerResult<String> {
    String::from_utf8(output.stdout).map_err(|error| {
        ManagerError::new(ManagerErrorKind::Protocol, message).with_detail(error.to_string())
    })
}

pub(crate) fn build_command(spec: &CommandSpec) -> Command {
    let mut command = Command::new(spec.program());
    command.args(spec.arguments());
    if let Ok(path) = env::join_paths(manager_search_directories()) {
        command.env("PATH", path);
    }
    command
}

pub(crate) fn piped_command(spec: &CommandSpec) -> Command {
    let mut command = build_command(spec);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

pub(crate) fn io_error(message: &str, error: std::io::Error) -> ManagerError {
    let kind = match error.kind() {
        std::io::ErrorKind::NotFound => ManagerErrorKind::CommandMissing,
        std::io::ErrorKind::PermissionDenied => ManagerErrorKind::Permission,
        std::io::ErrorKind::TimedOut => ManagerErrorKind::Timeout,
        _ => ManagerErrorKind::Other,
    };
    ManagerError::new(kind, message).with_detail(error.to_string())
}

pub(crate) fn command_status_error(
    spec: &CommandSpec,
    status: ExitStatus,
    tail: &str,
) -> ManagerError {
    let command = format_command(spec);
    let detail = if tail.trim().is_empty() {
        format!("{command} exited with {status}")
    } else {
        format!("{command} failed:\n{}", bounded_text(tail))
    };
    command_failure("package manager command failed", &detail)
}

fn command_failure(message: &str, detail: &str) -> ManagerError {
    ManagerError::new(classify_command_failure(detail), message).with_detail(bounded_text(detail))
}

fn classify_command_failure(detail: &str) -> ManagerErrorKind {
    let detail = detail.to_ascii_lowercase();

    if contains_any(
        &detail,
        &[
            "command not found",
            "executable file not found",
            "failed to spawn",
            "no such file or directory",
            "not found in path",
            "os error 2",
        ],
    ) {
        ManagerErrorKind::CommandMissing
    } else if contains_any(
        &detail,
        &[
            "permission denied",
            "not authorized",
            "authorization failed",
            "authentication failure",
            "no authentication agent",
            "polkit",
            "pkexec must be setuid root",
            "must be root",
            "requires root",
            "access is denied",
            "eacces",
        ],
    ) {
        ManagerErrorKind::Permission
    } else if contains_any(
        &detail,
        &[
            "could not get lock",
            "could not acquire lock",
            "unable to acquire",
            "unable to lock",
            "database is locked",
            "holding the apt lock",
            "dpkg frontend lock",
            "holding the yum lock",
            "holding the dnf lock",
            "system management is locked",
        ],
    ) {
        ManagerErrorKind::Busy
    } else if contains_any(&detail, &["timed out", "timeout", "deadline exceeded"]) {
        ManagerErrorKind::Timeout
    } else if contains_any(
        &detail,
        &[
            "reboot required",
            "restart required",
            "restart the computer",
        ],
    ) {
        ManagerErrorKind::RebootRequired
    } else {
        ManagerErrorKind::Other
    }
}

fn output_detail(output: &Output, fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        fallback
    };
    bounded_text(detail)
}

fn detected_version(output: &Output) -> Option<String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(bounded_text)
}

fn format_command(spec: &CommandSpec) -> String {
    let mut command = spec.program().to_string_lossy().into_owned();
    for argument in spec.arguments() {
        command.push(' ');
        command.push_str(&argument.to_string_lossy());
    }
    bounded_text(&command)
}

fn bounded_text(value: &str) -> String {
    value.chars().take(MAX_DIAGNOSTIC_CHARS).collect()
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

fn push_search_directory(directories: &mut Vec<PathBuf>, directory: impl Into<PathBuf>) {
    let directory = directory.into();
    if !directory.as_os_str().is_empty() && !directories.contains(&directory) {
        directories.push(directory);
    }
}

fn manager_search_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();

    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            push_search_directory(&mut directories, directory);
        }
    }
    if let Some(asdf_data_dir) = env::var_os("ASDF_DATA_DIR") {
        push_search_directory(&mut directories, PathBuf::from(asdf_data_dir).join("shims"));
    }
    if let Some(pnpm_home) = env::var_os("PNPM_HOME") {
        push_search_directory(&mut directories, pnpm_home);
    }
    if let Some(homebrew_prefix) = env::var_os("HOMEBREW_PREFIX") {
        push_search_directory(&mut directories, PathBuf::from(homebrew_prefix).join("bin"));
    }

    if let Some(user_dirs) = UserDirs::new() {
        let home = user_dirs.home_dir();
        for directory in [
            home.join(".asdf/shims"),
            home.join(".local/share/pnpm"),
            home.join(".local/share/pnpm/bin"),
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join("go/bin"),
            home.join(".linuxbrew/bin"),
        ] {
            push_search_directory(&mut directories, directory);
        }
    }

    for directory in [
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ] {
        push_search_directory(&mut directories, directory);
    }

    directories
}

fn find_executable(command: &str, directories: &[PathBuf]) -> Option<PathBuf> {
    directories.iter().find_map(|directory| {
        let candidate = directory.join(command);
        let metadata = std::fs::metadata(&candidate).ok()?;
        (metadata.is_file() && is_executable(&metadata)).then_some(candidate)
    })
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn finds_the_first_executable_candidate() {
        let first = tempdir().expect("create first directory");
        let second = tempdir().expect("create second directory");
        let first_command = first.path().join("apt");
        let second_command = second.path().join("apt");
        fs::write(&first_command, "#!/bin/sh\n").expect("write first executable");
        fs::write(&second_command, "#!/bin/sh\n").expect("write second executable");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&first_command, fs::Permissions::from_mode(0o755))
                .expect("mark first executable");
            fs::set_permissions(&second_command, fs::Permissions::from_mode(0o755))
                .expect("mark second executable");
        }

        assert_eq!(
            find_executable(
                "apt",
                &[first.path().to_path_buf(), second.path().to_path_buf()]
            ),
            Some(first_command)
        );
    }

    #[cfg(unix)]
    #[test]
    fn ignores_non_executable_candidates() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().expect("create directory");
        let command = directory.path().join("apt");
        fs::write(&command, "#!/bin/sh\n").expect("write candidate");
        fs::set_permissions(&command, fs::Permissions::from_mode(0o644))
            .expect("mark candidate non-executable");

        assert_eq!(
            find_executable("apt", &[directory.path().to_path_buf()]),
            None
        );
    }

    #[test]
    fn classifies_command_failures_without_treating_every_pkexec_error_as_permission() {
        for (detail, expected) in [
            ("command not found", ManagerErrorKind::CommandMissing),
            ("pkexec: not authorized", ManagerErrorKind::Permission),
            ("could not get lock", ManagerErrorKind::Busy),
            (
                "another app is currently holding the dnf lock",
                ManagerErrorKind::Busy,
            ),
            ("operation timed out", ManagerErrorKind::Timeout),
            ("reboot required", ManagerErrorKind::RebootRequired),
            (
                "pkexec apt install failed: broken package",
                ManagerErrorKind::Other,
            ),
        ] {
            assert_eq!(classify_command_failure(detail), expected, "{detail}");
        }
    }

    #[test]
    fn command_specs_preserve_non_utf8_safe_os_arguments() {
        let spec = CommandSpec::new("pkexec")
            .arg(Path::new("/custom/apt").as_os_str())
            .args(["install", "-y"]);

        assert_eq!(spec.program(), Path::new("pkexec"));
        assert_eq!(
            spec.arguments(),
            ["/custom/apt", "install", "-y"]
                .map(OsString::from)
                .as_slice()
        );
    }
}
