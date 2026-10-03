use std::{env, ffi::OsString, process::ExitCode};

#[cfg(target_os = "linux")]
use std::{
    fs, io,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::Command,
};

const MAX_PACKAGES: usize = 4_096;
const MAX_PACKAGE_NAME_BYTES: usize = 255;
const MAX_PORTAGE_ATOM_BYTES: usize = 512;
/// Fixed database the privileged helper mirrors for update discovery. It is
/// never configurable and never derived from the caller's arguments, so the
/// helper cannot be steered into syncing the live database.
const PACMAN_SYNC_DATABASE: &str = "/var/lib/updater/pacman-sync";
#[cfg(target_os = "linux")]
const PACMAN_LOCAL_DATABASE: &str = "/var/lib/pacman/local";
#[cfg(target_os = "linux")]
const PACMAN_DIRECTORY_MODE: u32 = 0o755;

/// The privileged work a plan needs before its command runs.
#[derive(Debug, PartialEq, Eq)]
enum Preparation {
    None,
    /// Mirror the pacman sync databases into [`PACMAN_SYNC_DATABASE`].
    PacmanSyncDatabase,
}

#[derive(Debug, PartialEq, Eq)]
struct CommandPlan {
    program: &'static str,
    arguments: Vec<OsString>,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    preparation: Preparation,
}

fn main() -> ExitCode {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    let plan = match command_plan(&arguments) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("updater-system-helper: {error}");
            return ExitCode::from(2);
        }
    };

    execute(plan)
}

#[cfg(target_os = "linux")]
fn execute(plan: CommandPlan) -> ExitCode {
    use std::os::unix::process::CommandExt;

    if let Err(error) = prepare_plan(&plan) {
        eprintln!("updater-system-helper: {error}");
        return ExitCode::from(2);
    }

    let program = plan.program;
    let error = system_command(plan).exec();
    eprintln!("updater-system-helper: failed to execute {program}: {error}");
    ExitCode::from(127)
}

#[cfg(target_os = "linux")]
fn prepare_plan(plan: &CommandPlan) -> Result<(), String> {
    match plan.preparation {
        Preparation::None => Ok(()),
        Preparation::PacmanSyncDatabase => prepare_pacman_sync_database(
            Path::new(PACMAN_SYNC_DATABASE),
            Path::new(PACMAN_LOCAL_DATABASE),
        ),
    }
}

/// Creates the fixed temporary sync database the refresh query reads.
///
/// The caller supplies the paths so this can be tested against a temporary
/// directory; the helper itself always passes the fixed constants above.
#[cfg(target_os = "linux")]
fn prepare_pacman_sync_database(directory: &Path, local_database: &Path) -> Result<(), String> {
    let parent = directory
        .parent()
        .ok_or_else(|| format!("invalid sync database path {}", directory.display()))?;
    // `/var/lib` is root owned, so nothing unprivileged can plant a path
    // component here, but every component is still verified.
    ensure_owned_directory(parent)?;
    ensure_owned_directory(directory)?;

    // pacman only reads local packages when they are reachable from the
    // database path, exactly as the pacman-contrib `checkupdates` script does.
    let local = directory.join("local");
    match fs::symlink_metadata(&local) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = fs::read_link(&local)
                .map_err(|error| format!("failed to read {}: {error}", local.display()))?;
            if target != local_database {
                replace_symlink(&local, local_database)?;
            }
        }
        Ok(_) => {
            return Err(format!("refusing unexpected {}", local.display()));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_symlink(&local, local_database)?;
        }
        Err(error) => return Err(format!("failed to inspect {}: {error}", local.display())),
    }

    // The sync mirror is written by pacman itself, but an existing `sync`
    // entry must still be a real directory and not a symlink.
    let sync = directory.join("sync");
    if let Ok(metadata) = fs::symlink_metadata(&sync)
        && (metadata.file_type().is_symlink() || !metadata.is_dir())
    {
        return Err(format!("refusing unexpected {}", sync.display()));
    }

    Ok(())
}

/// Creates `path` with mode 0755 when it is missing, and otherwise requires a
/// real directory. Symlinks and non-directories are refused, so a planted
/// entry cannot redirect root; a directory is tightened to 0755 regardless of
/// the umask `pkexec` inherited.
#[cfg(target_os = "linux")]
fn ensure_owned_directory(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path)
                .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
            fs::symlink_metadata(path)
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
        }
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("refusing unexpected {}", path.display()));
    }
    if metadata.permissions().mode() & 0o777 != PACMAN_DIRECTORY_MODE {
        fs::set_permissions(path, fs::Permissions::from_mode(PACMAN_DIRECTORY_MODE))
            .map_err(|error| format!("failed to secure {}: {error}", path.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn create_symlink(link: &Path, target: &Path) -> Result<(), String> {
    symlink(target, link).map_err(|error| format!("failed to create {}: {error}", link.display()))
}

#[cfg(target_os = "linux")]
fn replace_symlink(link: &Path, target: &Path) -> Result<(), String> {
    fs::remove_file(link)
        .map_err(|error| format!("failed to replace {}: {error}", link.display()))?;
    create_symlink(link, target)
}

#[cfg(target_os = "linux")]
fn system_command(plan: CommandPlan) -> Command {
    let mut command = Command::new(plan.program);
    command
        .args(plan.arguments)
        .env_clear()
        // The helper has no terminal, so debconf must never wait for an answer.
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("HOME", "/root")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("LOGNAME", "root")
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("USER", "root")
        .current_dir("/");
    command
}

#[cfg(not(target_os = "linux"))]
fn execute(_plan: CommandPlan) -> ExitCode {
    eprintln!("updater-system-helper: Linux is required");
    ExitCode::from(2)
}

fn command_plan(arguments: &[OsString]) -> Result<CommandPlan, String> {
    let [action, manager, packages @ ..] = arguments else {
        return Err("expected <action> <manager> [package ...]".to_owned());
    };
    let action = action
        .to_str()
        .ok_or_else(|| "action must be valid UTF-8".to_owned())?;
    let manager = manager
        .to_str()
        .ok_or_else(|| "manager must be valid UTF-8".to_owned())?;

    let (program, command_arguments): (&str, &[&str]) = match (action, manager) {
        ("install", "apt") => (
            "/usr/bin/apt-get",
            &[
                "-o",
                "Dpkg::Options::=--force-confdef",
                "-o",
                "Dpkg::Options::=--force-confold",
                "install",
                "-y",
            ],
        ),
        ("update", "apt") => (
            "/usr/bin/apt-get",
            &[
                "-o",
                "Dpkg::Options::=--force-confdef",
                "-o",
                "Dpkg::Options::=--force-confold",
                "install",
                "-y",
                "--only-upgrade",
            ],
        ),
        ("remove", "apt") => (
            "/usr/bin/apt-get",
            &[
                "-o",
                "Dpkg::Options::=--force-confdef",
                "-o",
                "Dpkg::Options::=--force-confold",
                "remove",
                "-y",
            ],
        ),
        ("refresh", "apt") => ("/usr/bin/apt-get", &["update"]),
        ("install", "dnf") => ("/usr/bin/dnf", &["install", "-y"]),
        ("update", "dnf") => ("/usr/bin/dnf", &["upgrade", "-y", "--skip-unavailable"]),
        ("remove", "dnf") => ("/usr/bin/dnf", &["remove", "-y"]),
        ("refresh", "dnf") => ("/usr/bin/dnf", &["check-upgrade", "--refresh"]),
        ("install", "pacman") => ("/usr/bin/pacman", &["-S", "--needed", "--noconfirm"]),
        // Arch does not support partial upgrades, so an update is always the
        // whole `-Syu` system transaction and the live sync database is never
        // refreshed on its own.
        ("update", "pacman") => ("/usr/bin/pacman", &["-Syu", "--noconfirm"]),
        ("remove", "pacman") => ("/usr/bin/pacman", &["-R", "--noconfirm"]),
        // A refresh mirrors the sync databases into the fixed temporary
        // database prepared by `prepare_pacman_sync_database`; the live
        // database only ever moves together with `-u`.
        ("refresh", "pacman") => (
            "/usr/bin/pacman",
            &[
                "-Sy",
                "--dbpath",
                PACMAN_SYNC_DATABASE,
                "--logfile",
                "/dev/null",
                "--noconfirm",
            ],
        ),
        ("install", "zypper") => ("/usr/bin/zypper", &["--non-interactive", "install", "-y"]),
        ("update", "zypper") => ("/usr/bin/zypper", &["--non-interactive", "update", "-y"]),
        ("remove", "zypper") => ("/usr/bin/zypper", &["--non-interactive", "remove", "-y"]),
        ("refresh", "zypper") => ("/usr/bin/zypper", &["--non-interactive", "refresh"]),
        ("install", "portage") => ("/usr/bin/emerge", &["--ask=n", "--color=n"]),
        ("update", "portage") => ("/usr/bin/emerge", &["--ask=n", "--color=n", "--update"]),
        ("remove", "portage") => ("/usr/bin/emerge", &["--ask=n", "--color=n", "--depclean"]),
        ("refresh", "portage") => ("/usr/bin/emerge", &["--sync"]),
        ("install", "xbps") => ("/usr/bin/xbps-install", &["--yes"]),
        ("update", "xbps") => ("/usr/bin/xbps-install", &["--yes", "--update"]),
        ("remove", "xbps") => ("/usr/bin/xbps-remove", &["--yes"]),
        ("refresh", "xbps") => ("/usr/bin/xbps-install", &["--sync"]),
        (_, "apt" | "dnf" | "pacman" | "zypper" | "portage" | "xbps") => {
            return Err(format!("unsupported action: {action}"));
        }
        _ => return Err(format!("unsupported manager: {manager}")),
    };

    if action == "refresh" {
        if !packages.is_empty() {
            return Err("refresh does not accept package names".to_owned());
        }
    } else if (action, manager) == ("update", "pacman") {
        if !packages.is_empty() {
            return Err(
                "pacman update is a full system upgrade and does not accept package names"
                    .to_owned(),
            );
        }
    } else if packages.is_empty() {
        return Err(format!("{action} requires at least one package"));
    }
    if packages.len() > MAX_PACKAGES {
        return Err(format!("package batch exceeds {MAX_PACKAGES} entries"));
    }

    for package in packages {
        match manager {
            "portage" => validate_portage_atom(package, action != "install")?,
            "xbps" => validate_xbps_package_name(package)?,
            _ => validate_package_name(package)?,
        }
    }

    let mut resolved_arguments = command_arguments
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    resolved_arguments.extend(packages.iter().cloned());
    let preparation = match (action, manager) {
        ("refresh", "pacman") => Preparation::PacmanSyncDatabase,
        _ => Preparation::None,
    };
    Ok(CommandPlan {
        program,
        arguments: resolved_arguments,
        preparation,
    })
}

fn validate_portage_atom(package: &OsString, require_slot: bool) -> Result<(), String> {
    let Some(atom) = package.to_str() else {
        return Err("Portage atom must be valid UTF-8".to_owned());
    };
    if atom.is_empty() || atom.len() > MAX_PORTAGE_ATOM_BYTES {
        return Err(format!(
            "Portage atom must contain 1 to {MAX_PORTAGE_ATOM_BYTES} bytes"
        ));
    }
    let (identity, repository) = atom
        .split_once("::")
        .map_or((atom, None), |(identity, repository)| {
            (identity, Some(repository))
        });
    let (package, slot) = identity
        .split_once(':')
        .map_or((identity, None), |(package, slot)| (package, Some(slot)));
    let mut package_parts = package.split('/');
    let category = package_parts.next().unwrap_or_default();
    let name = package_parts.next().unwrap_or_default();
    if package_parts.next().is_some()
        || !valid_package_component(category)
        || !valid_package_component(name)
        || slot.is_some_and(|slot| !valid_package_component(slot))
        || repository.is_some_and(|repository| !valid_package_component(repository))
        || (require_slot && slot.is_none())
    {
        return Err(format!("invalid Portage atom: {atom}"));
    }
    Ok(())
}

fn validate_xbps_package_name(package: &OsString) -> Result<(), String> {
    let Some(package) = package.to_str() else {
        return Err("XBPS package name must be valid UTF-8".to_owned());
    };
    if package.len() > MAX_PACKAGE_NAME_BYTES || !valid_package_component(package) {
        return Err(format!("invalid XBPS package name: {package}"));
    }
    Ok(())
}

fn valid_package_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.' | b'_'))
}

fn validate_package_name(package: &OsString) -> Result<(), String> {
    let Some(package) = package.to_str() else {
        return Err("package name must be valid UTF-8".to_owned());
    };
    if package.is_empty() || package.len() > MAX_PACKAGE_NAME_BYTES {
        return Err(format!(
            "package name must contain 1 to {MAX_PACKAGE_NAME_BYTES} bytes"
        ));
    }

    let mut bytes = package.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes.all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'+' | b'-' | b'.' | b'_' | b':' | b'@' | b'%' | b'=' | b'~'
                )
        })
    {
        return Err(format!("invalid package name: {package}"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    /// Owns a unique temporary directory without adding a test-only crate.
    #[cfg(target_os = "linux")]
    struct Scratch(std::path::PathBuf);

    #[cfg(target_os = "linux")]
    impl Scratch {
        fn new(name: &str) -> Self {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "updater-helper-test-{}-{name}-{unique}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create scratch directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn maps_every_supported_manager_action_to_a_fixed_command() {
        for (action, manager, program, expected) in [
            (
                "install",
                "apt",
                "/usr/bin/apt-get",
                vec![
                    "-o",
                    "Dpkg::Options::=--force-confdef",
                    "-o",
                    "Dpkg::Options::=--force-confold",
                    "install",
                    "-y",
                    "bash",
                ],
            ),
            (
                "update",
                "apt",
                "/usr/bin/apt-get",
                vec![
                    "-o",
                    "Dpkg::Options::=--force-confdef",
                    "-o",
                    "Dpkg::Options::=--force-confold",
                    "install",
                    "-y",
                    "--only-upgrade",
                    "bash",
                ],
            ),
            (
                "remove",
                "apt",
                "/usr/bin/apt-get",
                vec![
                    "-o",
                    "Dpkg::Options::=--force-confdef",
                    "-o",
                    "Dpkg::Options::=--force-confold",
                    "remove",
                    "-y",
                    "bash",
                ],
            ),
            ("refresh", "apt", "/usr/bin/apt-get", vec!["update"]),
            (
                "install",
                "dnf",
                "/usr/bin/dnf",
                vec!["install", "-y", "bash"],
            ),
            (
                "update",
                "dnf",
                "/usr/bin/dnf",
                vec!["upgrade", "-y", "--skip-unavailable", "bash"],
            ),
            (
                "remove",
                "dnf",
                "/usr/bin/dnf",
                vec!["remove", "-y", "bash"],
            ),
            (
                "refresh",
                "dnf",
                "/usr/bin/dnf",
                vec!["check-upgrade", "--refresh"],
            ),
            (
                "install",
                "pacman",
                "/usr/bin/pacman",
                vec!["-S", "--needed", "--noconfirm", "bash"],
            ),
            (
                "update",
                "pacman",
                "/usr/bin/pacman",
                vec!["-Syu", "--noconfirm"],
            ),
            (
                "remove",
                "pacman",
                "/usr/bin/pacman",
                vec!["-R", "--noconfirm", "bash"],
            ),
            (
                "refresh",
                "pacman",
                "/usr/bin/pacman",
                vec![
                    "-Sy",
                    "--dbpath",
                    PACMAN_SYNC_DATABASE,
                    "--logfile",
                    "/dev/null",
                    "--noconfirm",
                ],
            ),
            (
                "install",
                "zypper",
                "/usr/bin/zypper",
                vec!["--non-interactive", "install", "-y", "bash"],
            ),
            (
                "update",
                "zypper",
                "/usr/bin/zypper",
                vec!["--non-interactive", "update", "-y", "bash"],
            ),
            (
                "remove",
                "zypper",
                "/usr/bin/zypper",
                vec!["--non-interactive", "remove", "-y", "bash"],
            ),
            (
                "refresh",
                "zypper",
                "/usr/bin/zypper",
                vec!["--non-interactive", "refresh"],
            ),
        ] {
            let mut input = vec![action, manager];
            if action != "refresh" && (action, manager) != ("update", "pacman") {
                input.push("bash");
            }
            let plan = command_plan(&arguments(&input)).expect("build allowed command");
            assert_eq!(plan.program, program, "{action} {manager}");
            assert_eq!(plan.arguments, arguments(&expected), "{action} {manager}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn system_commands_run_debconf_noninteractively_in_a_cleared_environment() {
        let plan = command_plan(&arguments(&["update", "apt", "bash"])).expect("build apt plan");
        let command = system_command(plan);
        let environment = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();

        assert!(
            environment.contains(&(
                "DEBIAN_FRONTEND".to_owned(),
                Some("noninteractive".to_owned())
            )),
            "{environment:?}"
        );
        assert!(environment.contains(&(
            "PATH".to_owned(),
            Some("/usr/sbin:/usr/bin:/sbin:/bin".to_owned())
        )));
    }

    #[test]
    fn pacman_update_is_a_full_system_transaction_without_package_names() {
        let plan = command_plan(&arguments(&["update", "pacman"])).expect("build pacman update");
        assert_eq!(plan.program, "/usr/bin/pacman");
        assert_eq!(plan.arguments, arguments(&["-Syu", "--noconfirm"]));

        let error = command_plan(&arguments(&["update", "pacman", "bash"]))
            .expect_err("reject a partial pacman update");
        assert!(error.contains("full system upgrade"), "{error}");
    }

    #[test]
    fn pacman_syncs_only_the_fixed_temporary_database_without_partial_upgrade_flags() {
        let plan =
            command_plan(&arguments(&["refresh", "pacman"])).expect("build pacman refresh plan");
        assert_eq!(plan.program, "/usr/bin/pacman");
        assert_eq!(plan.preparation, Preparation::PacmanSyncDatabase);
        assert_eq!(
            plan.arguments,
            arguments(&[
                "-Sy",
                "--dbpath",
                PACMAN_SYNC_DATABASE,
                "--logfile",
                "/dev/null",
                "--noconfirm",
            ])
        );

        let error = command_plan(&arguments(&["refresh", "pacman", "bash"]))
            .expect_err("reject a refresh with package names");
        assert!(
            error.contains("refresh does not accept package names"),
            "{error}"
        );
    }

    #[test]
    fn every_pacman_sync_plan_is_temporary_or_a_full_upgrade() {
        for input in [
            vec!["refresh", "pacman"],
            vec!["install", "pacman", "bash"],
            vec!["update", "pacman"],
            vec!["remove", "pacman", "bash"],
        ] {
            let plan = command_plan(&arguments(&input)).expect("build pacman plan");
            let arguments = plan
                .arguments
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let Some(sync_flag) = arguments
                .iter()
                .find(|argument| argument.starts_with("-S") && argument.contains('y'))
            else {
                continue;
            };

            // `-Syu` refreshes and upgrades in one transaction; any other
            // sync flag must read the fixed temporary database instead of the
            // live one.
            let temporary = arguments
                .windows(2)
                .any(|pair| pair == ["--dbpath", PACMAN_SYNC_DATABASE]);
            assert!(
                temporary || sync_flag.contains('u'),
                "pacman {sync_flag} without --dbpath {PACMAN_SYNC_DATABASE} or -u: {input:?}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pacman_sync_database_creates_a_secure_directory_with_a_local_symlink() {
        let scratch = Scratch::new("create");
        let directory = scratch.path();
        let database = directory.join("updater/pacman-sync");
        let local = directory.join("pacman/local");
        fs::create_dir_all(&local).expect("create local database directory");

        prepare_pacman_sync_database(&database, &local).expect("prepare sync database");

        let metadata = fs::symlink_metadata(&database).expect("inspect sync database");
        assert!(metadata.is_dir());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o755);
        assert_eq!(
            fs::read_link(database.join("local")).expect("read local symlink"),
            local
        );

        // A second run keeps the correct symlink and stays idempotent.
        prepare_pacman_sync_database(&database, &local).expect("prepare sync database twice");
        assert_eq!(
            fs::read_link(database.join("local")).expect("read local symlink"),
            local
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pacman_sync_database_replaces_a_stale_local_symlink() {
        let scratch = Scratch::new("stale");
        let directory = scratch.path();
        let database = directory.join("updater/pacman-sync");
        let local = directory.join("pacman/local");
        let stale = directory.join("stale/local");
        fs::create_dir_all(&local).expect("create local database directory");
        fs::create_dir_all(&stale).expect("create stale directory");
        fs::create_dir_all(&database).expect("create sync database directory");
        symlink(&stale, database.join("local")).expect("plant stale local symlink");

        prepare_pacman_sync_database(&database, &local).expect("prepare sync database");

        assert_eq!(
            fs::read_link(database.join("local")).expect("read local symlink"),
            local
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pacman_sync_database_refuses_an_unexpected_local_entry() {
        let scratch = Scratch::new("unexpected");
        let directory = scratch.path();
        let database = directory.join("updater/pacman-sync");
        let local = directory.join("pacman/local");
        fs::create_dir_all(&local).expect("create local database directory");
        fs::create_dir_all(&database).expect("create sync database directory");
        fs::write(database.join("local"), b"not a directory")
            .expect("plant unexpected local entry");

        let error = prepare_pacman_sync_database(&database, &local)
            .expect_err("refuse an unexpected local entry");

        assert!(error.contains("refusing unexpected"), "{error}");
        assert!(
            fs::read_link(database.join("local")).is_err(),
            "the unexpected entry must not be replaced"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pacman_sync_database_refuses_a_planted_symlink() {
        let scratch = Scratch::new("planted-database");
        let directory = scratch.path();
        let real = directory.join("real");
        let database = directory.join("updater/pacman-sync");
        let local = directory.join("pacman/local");
        fs::create_dir_all(&real).expect("create real directory");
        fs::create_dir_all(&local).expect("create local database directory");
        fs::create_dir_all(database.parent().expect("sync database parent"))
            .expect("create sync database parent");
        symlink(&real, &database).expect("plant database symlink");

        let error = prepare_pacman_sync_database(&database, &local)
            .expect_err("refuse a planted database symlink");

        assert!(error.contains("refusing unexpected"), "{error}");
        assert!(real.is_dir());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pacman_sync_database_refuses_a_planted_sync_symlink() {
        let scratch = Scratch::new("planted-sync");
        let directory = scratch.path();
        let real = directory.join("real");
        let database = directory.join("updater/pacman-sync");
        let local = directory.join("pacman/local");
        fs::create_dir_all(&real).expect("create real directory");
        fs::create_dir_all(&local).expect("create local database directory");
        fs::create_dir_all(&database).expect("create sync database directory");
        symlink(&real, database.join("sync")).expect("plant sync symlink");

        let error = prepare_pacman_sync_database(&database, &local)
            .expect_err("refuse a planted sync symlink");

        assert!(error.contains("refusing unexpected"), "{error}");
    }

    #[test]
    fn accepts_distribution_package_identifiers_without_accepting_paths() {
        let plan = command_plan(&arguments(&[
            "install",
            "apt",
            "libstdc++6:amd64",
            "kernel-core.x86_64",
            "foo_bar@1%2=3~4",
        ]))
        .expect("accept package identifiers");
        assert_eq!(plan.arguments.len(), 9);

        for package in [
            "-oDebug::NoLocking=1",
            "../tmp/package",
            "name/path",
            "two words",
            "$(id)",
        ] {
            let error = command_plan(&arguments(&["install", "apt", package]))
                .expect_err("reject unsafe package name");
            assert!(error.contains("invalid package name"), "{package}: {error}");
        }
    }

    #[test]
    fn maps_portage_and_xbps_to_fixed_commands() {
        for (input, program, expected) in [
            (
                vec!["install", "portage", "dev-lang/python:3.13::gentoo"],
                "/usr/bin/emerge",
                vec!["--ask=n", "--color=n", "dev-lang/python:3.13::gentoo"],
            ),
            (
                vec!["update", "portage", "dev-lang/python:3.13::gentoo"],
                "/usr/bin/emerge",
                vec![
                    "--ask=n",
                    "--color=n",
                    "--update",
                    "dev-lang/python:3.13::gentoo",
                ],
            ),
            (
                vec!["remove", "portage", "dev-lang/python:3.13::gentoo"],
                "/usr/bin/emerge",
                vec![
                    "--ask=n",
                    "--color=n",
                    "--depclean",
                    "dev-lang/python:3.13::gentoo",
                ],
            ),
            (
                vec!["refresh", "portage"],
                "/usr/bin/emerge",
                vec!["--sync"],
            ),
            (
                vec!["install", "xbps", "base-files"],
                "/usr/bin/xbps-install",
                vec!["--yes", "base-files"],
            ),
            (
                vec!["update", "xbps", "base-files"],
                "/usr/bin/xbps-install",
                vec!["--yes", "--update", "base-files"],
            ),
            (
                vec!["remove", "xbps", "base-files"],
                "/usr/bin/xbps-remove",
                vec!["--yes", "base-files"],
            ),
            (
                vec!["refresh", "xbps"],
                "/usr/bin/xbps-install",
                vec!["--sync"],
            ),
        ] {
            let plan = command_plan(&arguments(&input)).expect("build fixed command");
            assert_eq!(plan.program, program, "{input:?}");
            assert_eq!(plan.arguments, arguments(&expected), "{input:?}");
        }
    }

    #[test]
    fn applies_manager_specific_package_validation() {
        for atom in [
            "@world",
            ">=dev-lang/python-3.13",
            "dev-lang/python:3.13::gentoo::other",
            "dev-lang/python:3.13::",
            "dev-lang/python:3.13/3.13",
            "/tmp/python:3.13",
        ] {
            let error = command_plan(&arguments(&["install", "portage", atom]))
                .expect_err("reject unsafe or unqualified Portage atom");
            assert!(error.contains("invalid Portage atom"), "{atom}: {error}");
        }

        command_plan(&arguments(&["install", "portage", "dev-lang/python"]))
            .expect("allow unqualified Portage install atom");
        command_plan(&arguments(&[
            "update",
            "portage",
            "dev-lang/python:3.13::gentoo",
        ]))
        .expect("allow SLOT and repository qualified Portage update atom");
        for action in ["update", "remove"] {
            let error = command_plan(&arguments(&[action, "portage", "dev-lang/python"]))
                .expect_err("require SLOT for an existing Portage package");
            assert!(error.contains("invalid Portage atom"));
        }

        for package in ["-base", "base/files", "base:1", "base>=1", "two words"] {
            let error = command_plan(&arguments(&["install", "xbps", package]))
                .expect_err("reject invalid XBPS package name");
            assert!(
                error.contains("invalid XBPS package name"),
                "{package}: {error}"
            );
        }
    }

    #[test]
    fn rejects_unknown_or_incomplete_requests() {
        for input in [
            vec![],
            vec!["install"],
            vec!["install", "flatpak", "org.example.App"],
            vec!["shell", "apt", "bash"],
            vec!["install", "apt"],
            vec!["refresh", "apt", "bash"],
            vec!["refresh", "pacman", "bash"],
        ] {
            assert!(command_plan(&arguments(&input)).is_err(), "{input:?}");
        }
    }

    #[test]
    fn rejects_oversized_names_and_batches() {
        let long_name = "a".repeat(MAX_PACKAGE_NAME_BYTES + 1);
        assert!(
            command_plan(&[
                OsString::from("install"),
                OsString::from("apt"),
                OsString::from(long_name),
            ])
            .is_err()
        );

        let mut batch = arguments(&["install", "apt"]);
        batch.extend((0..=MAX_PACKAGES).map(|_| OsString::from("bash")));
        assert!(command_plan(&batch).is_err());
    }

    #[test]
    fn policy_binds_each_action_to_the_fixed_helper_and_icon() {
        let document = roxmltree::Document::parse_with_options(
            include_str!("../../../assets/linux/com.ayi.updater.policy"),
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .expect("parse Updater Polkit policy");
        let actions = document
            .descendants()
            .filter(|node| node.has_tag_name("action"))
            .collect::<Vec<_>>();
        assert_eq!(actions.len(), 4);

        for (id, argument) in [
            ("com.ayi.updater.install-system-packages", "install"),
            ("com.ayi.updater.update-system-packages", "update"),
            ("com.ayi.updater.remove-system-packages", "remove"),
            ("com.ayi.updater.refresh-system-package-metadata", "refresh"),
        ] {
            let action = actions
                .iter()
                .find(|node| node.attribute("id") == Some(id))
                .unwrap_or_else(|| panic!("missing policy action {id}"));
            assert_eq!(
                action
                    .children()
                    .find(|node| node.has_tag_name("icon_name"))
                    .and_then(|node| node.text()),
                Some("updater")
            );
            assert_eq!(
                action
                    .children()
                    .filter(|node| node.has_tag_name("description"))
                    .count(),
                2
            );
            assert_eq!(
                action
                    .children()
                    .filter(|node| node.has_tag_name("message"))
                    .count(),
                2
            );

            let annotations = action
                .children()
                .filter(|node| node.has_tag_name("annotate"))
                .collect::<Vec<_>>();
            assert!(annotations.iter().any(|node| {
                node.attribute("key") == Some("org.freedesktop.policykit.exec.path")
                    && node.text() == Some("/usr/lib/updater/updater-system-helper")
            }));
            assert!(annotations.iter().any(|node| {
                node.attribute("key") == Some("org.freedesktop.policykit.exec.argv1")
                    && node.text() == Some(argument)
            }));
        }
    }
}
