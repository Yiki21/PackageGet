use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{ExitStatus, Output, Stdio},
    time::Duration,
};

use directories_next::UserDirs;
use tokio::{
    process::Command,
    time::{sleep, timeout},
};
use updater_manager_api::{
    AvailabilityReason, ManagerAvailability, ManagerConfig, ManagerDescriptor, ManagerError,
    ManagerErrorKind, ManagerResult, Platform,
};

const MAX_DIAGNOSTIC_CHARS: usize = 8_192;
const PKEXEC_PATH: &str = "/usr/bin/pkexec";
const SYSTEM_HELPER_PATH: &str = "/usr/lib/updater/updater-system-helper";
const DEFAULT_WINDOWS_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";
const EXECUTABLE_BUSY_RETRIES: usize = 3;
const EXECUTABLE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    program: PathBuf,
    args: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    removed_environment: Vec<OsString>,
}

impl CommandSpec {
    pub(crate) fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            environment: Vec::new(),
            removed_environment: Vec::new(),
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

    pub(crate) fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.environment.push((key.into(), value.into()));
        self
    }

    pub(crate) fn env_remove(mut self, key: impl Into<OsString>) -> Self {
        self.removed_environment.push(key.into());
        self
    }

    pub(crate) fn program(&self) -> &Path {
        &self.program
    }

    pub(crate) fn arguments(&self) -> &[OsString] {
        &self.args
    }

    pub(crate) fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    pub(crate) fn removed_environment(&self) -> &[OsString] {
        &self.removed_environment
    }
}

pub(crate) fn system_helper_command(action: &str, manager: &str) -> CommandSpec {
    CommandSpec::new(PKEXEC_PATH).args([SYSTEM_HELPER_PATH, action, manager])
}

pub(crate) async fn manager_availability(
    descriptor: &ManagerDescriptor,
    config: &ManagerConfig,
    default_program: &str,
    version_args: &[&str],
) -> ManagerAvailability {
    manager_availability_with_version(
        descriptor,
        config,
        default_program,
        version_args,
        detected_version,
    )
    .await
}

pub(crate) fn unsupported_platform(descriptor: &ManagerDescriptor) -> Option<ManagerAvailability> {
    let platform = Platform::current();
    platform
        .filter(|platform| descriptor.platforms().contains(*platform))
        .is_none()
        .then_some(ManagerAvailability::Unavailable {
            reason: AvailabilityReason::UnsupportedPlatform { platform },
        })
}

pub(crate) async fn manager_availability_with_version(
    descriptor: &ManagerDescriptor,
    config: &ManagerConfig,
    default_program: &str,
    version_args: &[&str],
    detect_version: fn(&Output) -> Option<String>,
) -> ManagerAvailability {
    if let Some(availability) = unsupported_platform(descriptor) {
        return availability;
    }

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
            version: detect_version(&output),
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
            find_executable(
                default_program,
                &manager_search_directories(),
                windows_pathext().as_deref(),
            )
            .unwrap_or_else(|| PathBuf::from(default_program))
        },
        Path::to_path_buf,
    )
}

pub(crate) async fn run_output(spec: &CommandSpec) -> ManagerResult<Output> {
    let mut attempt = 0;
    loop {
        match build_command(spec).output().await {
            Ok(output) => return Ok(output),
            Err(error) if is_executable_busy(&error) && attempt < EXECUTABLE_BUSY_RETRIES => {
                attempt += 1;
                sleep(EXECUTABLE_BUSY_RETRY_DELAY).await;
            }
            Err(error) => {
                return Err(io_error("failed to execute package manager command", error));
            }
        }
    }
}

fn is_executable_busy(error: &std::io::Error) -> bool {
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
    command.kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        command.as_std_mut().process_group(0);
    }
    command.args(spec.arguments());
    for key in spec.removed_environment() {
        command.env_remove(key);
    }
    command.envs(spec.environment().iter().cloned());
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
            "operation was cancelled",
            "operation was canceled",
            "cancelled",
            "canceled",
        ],
    ) {
        ManagerErrorKind::Cancelled
    } else if contains_any(
        &detail,
        &[
            "could not resolve host",
            "couldn't resolve host",
            "network is unreachable",
            "failed to download",
            "connection refused",
        ],
    ) {
        ManagerErrorKind::Network
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
            "not allowed for user",
            "operation not permitted",
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
            "failed to init transaction",
            "unable to lock database",
            "could not lock database",
            "another system helper transaction is already active",
            "transaction is already in progress",
            "another active homebrew process",
            "has already locked",
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
    if let Some(bun_install) = env::var_os("BUN_INSTALL") {
        let bun_install = PathBuf::from(bun_install);
        if !bun_install.as_os_str().is_empty() {
            push_search_directory(&mut directories, bun_install.join("bin"));
        }
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
            home.join(".bun/bin"),
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join("go/bin"),
            home.join(".linuxbrew/bin"),
        ] {
            push_search_directory(&mut directories, directory);
        }
    }

    for directory in Platform::current()
        .into_iter()
        .flat_map(platform_search_directories)
    {
        push_search_directory(&mut directories, directory);
    }

    if matches!(Platform::current(), Some(Platform::Windows)) {
        for (variable, suffix) in [
            ("LOCALAPPDATA", "Microsoft/WindowsApps"),
            ("APPDATA", "npm"),
            ("USERPROFILE", "scoop/shims"),
            ("ProgramData", "chocolatey/bin"),
        ] {
            if let Some(root) = env::var_os(variable) {
                push_search_directory(&mut directories, PathBuf::from(root).join(suffix));
            }
        }
    }

    directories
}

fn platform_search_directories(platform: Platform) -> Vec<PathBuf> {
    match platform {
        Platform::Linux => vec![
            PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ],
        Platform::MacOs => vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ],
        Platform::Windows => Vec::new(),
        _ => Vec::new(),
    }
}

#[cfg(target_os = "windows")]
fn windows_pathext() -> Option<OsString> {
    Some(env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(DEFAULT_WINDOWS_PATHEXT)))
}

#[cfg(not(target_os = "windows"))]
fn windows_pathext() -> Option<OsString> {
    None
}

fn find_executable(
    command: &str,
    directories: &[PathBuf],
    windows_pathext: Option<&OsStr>,
) -> Option<PathBuf> {
    let candidates = executable_candidates(command, windows_pathext);
    directories.iter().find_map(|directory| {
        candidates.iter().find_map(|candidate| {
            let candidate = directory.join(candidate);
            let metadata = std::fs::metadata(&candidate).ok()?;
            (metadata.is_file() && is_executable(&metadata)).then_some(candidate)
        })
    })
}

fn executable_candidates(command: &str, windows_pathext: Option<&OsStr>) -> Vec<OsString> {
    let command = OsString::from(command);
    let mut candidates = vec![command.clone()];
    let Some(pathext) = windows_pathext else {
        return candidates;
    };
    if Path::new(&command).extension().is_some() {
        return candidates;
    }

    let pathext = pathext.to_string_lossy();
    let extensions = if pathext
        .split(';')
        .any(|extension| !extension.trim().is_empty())
    {
        pathext.as_ref()
    } else {
        DEFAULT_WINDOWS_PATHEXT
    };
    let mut seen = std::collections::HashSet::new();
    for extension in extensions
        .split(';')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let extension = if extension.starts_with('.') {
            extension.to_owned()
        } else {
            format!(".{extension}")
        };
        if !seen.insert(extension.to_ascii_lowercase()) {
            continue;
        }
        let mut candidate = command.clone();
        candidate.push(extension);
        candidates.push(candidate);
    }
    candidates
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
    use updater_manager_api::{
        ManagerCapabilities, ManagerCapability, ManagerCategory, ManagerConfig, ManagerDescriptor,
        ManagerId, SupportedPlatforms,
    };

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
                &[first.path().to_path_buf(), second.path().to_path_buf()],
                None,
            ),
            Some(first_command)
        );
    }

    #[tokio::test]
    async fn availability_returns_unsupported_before_command_probe() {
        let unsupported_platform = match Platform::current() {
            Some(Platform::Windows) => Platform::Linux,
            _ => Platform::Windows,
        };
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse("test:windows-only").expect("valid test manager ID"),
            "Unsupported test manager",
            ManagerCategory::Development,
            SupportedPlatforms::from([unsupported_platform]),
            ManagerCapabilities::from([ManagerCapability::Installed]),
        )
        .expect("valid test descriptor");
        let config = ManagerConfig::new(descriptor.id().clone());

        let availability =
            manager_availability(&descriptor, &config, "command-that-does-not-exist", &[]).await;

        assert_eq!(
            availability,
            ManagerAvailability::Unavailable {
                reason: updater_manager_api::AvailabilityReason::UnsupportedPlatform {
                    platform: Platform::current(),
                },
            }
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
            find_executable("apt", &[directory.path().to_path_buf()], None),
            None
        );
    }

    #[test]
    fn windows_candidates_follow_pathext_without_duplicate_extensions() {
        assert_eq!(
            executable_candidates("winget", Some(OsStr::new(".EXE;.CMD;exe;;.Bat"))),
            ["winget", "winget.EXE", "winget.CMD", "winget.Bat"].map(OsString::from)
        );
        assert_eq!(
            executable_candidates("pnpm.cmd", Some(OsStr::new(".EXE;.CMD"))),
            [OsString::from("pnpm.cmd")]
        );
        assert_eq!(
            executable_candidates("winget", Some(OsStr::new(";;"))),
            [
                "winget",
                "winget.COM",
                "winget.EXE",
                "winget.BAT",
                "winget.CMD",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn finds_windows_pathext_candidate_in_directory_order() {
        let directory = tempdir().expect("create Windows candidate directory");
        let executable = directory.path().join("winget.EXE");
        fs::write(&executable, b"fake executable").expect("write Windows executable candidate");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
                .expect("mark Windows candidate executable on Unix host");
        }

        assert_eq!(
            find_executable(
                "winget",
                &[directory.path().to_path_buf()],
                Some(OsStr::new(".COM;.EXE")),
            ),
            Some(executable)
        );
    }

    #[test]
    fn classifies_command_failures_without_treating_every_pkexec_error_as_permission() {
        for (detail, expected) in [
            ("command not found", ManagerErrorKind::CommandMissing),
            ("operation was cancelled", ManagerErrorKind::Cancelled),
            ("pkexec: canceled", ManagerErrorKind::Cancelled),
            ("could not resolve host", ManagerErrorKind::Network),
            ("failed to download object", ManagerErrorKind::Network),
            ("pkexec: not authorized", ManagerErrorKind::Permission),
            ("not allowed for user", ManagerErrorKind::Permission),
            ("operation not permitted", ManagerErrorKind::Permission),
            ("could not get lock", ManagerErrorKind::Busy),
            (
                "another app is currently holding the dnf lock",
                ManagerErrorKind::Busy,
            ),
            ("failed to init transaction", ManagerErrorKind::Busy),
            ("unable to lock database", ManagerErrorKind::Busy),
            (
                "another system helper transaction is already active",
                ManagerErrorKind::Busy,
            ),
            (
                "another active Homebrew process is already using this directory",
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

    #[test]
    fn system_helper_commands_use_the_policy_bound_path_and_action_first() {
        let spec = system_helper_command("install", "apt").args(["bash", "curl"]);

        assert_eq!(spec.program(), Path::new(PKEXEC_PATH));
        assert_eq!(
            spec.arguments(),
            [SYSTEM_HELPER_PATH, "install", "apt", "bash", "curl",]
                .map(OsString::from)
                .as_slice()
        );
    }

    #[test]
    fn command_specs_preserve_command_local_environment() {
        let spec = CommandSpec::new("zypper")
            .env_remove("INHERITED_SETTING")
            .env("LC_ALL", "C")
            .args(["--non-interactive", "list-updates"]);

        assert_eq!(
            spec.environment(),
            [(OsString::from("LC_ALL"), OsString::from("C"))]
        );
        assert_eq!(
            spec.removed_environment(),
            [OsString::from("INHERITED_SETTING")]
        );
    }

    #[test]
    fn platform_search_paths_freeze_linux_and_macos_homebrew_prefixes() {
        assert_eq!(
            platform_search_directories(Platform::Linux),
            [
                PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ]
        );
        assert_eq!(
            platform_search_directories(Platform::MacOs),
            [
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
            ]
        );
    }
}
