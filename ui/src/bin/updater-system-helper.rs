use std::{
    env,
    ffi::OsString,
    process::{Command, ExitCode},
};

const MAX_PACKAGES: usize = 4_096;
const MAX_PACKAGE_NAME_BYTES: usize = 255;

#[derive(Debug, PartialEq, Eq)]
struct CommandPlan {
    program: &'static str,
    arguments: Vec<OsString>,
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

    let error = Command::new(plan.program)
        .args(plan.arguments)
        .env_clear()
        .env("HOME", "/root")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("LOGNAME", "root")
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("USER", "root")
        .current_dir("/")
        .exec();
    eprintln!(
        "updater-system-helper: failed to execute {}: {error}",
        plan.program
    );
    ExitCode::from(127)
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
        ("install", "apt") => ("/usr/bin/apt-get", &["install", "-y"]),
        ("update", "apt") => ("/usr/bin/apt-get", &["install", "-y", "--only-upgrade"]),
        ("remove", "apt") => ("/usr/bin/apt-get", &["remove", "-y"]),
        ("refresh", "apt") => ("/usr/bin/apt-get", &["update"]),
        ("install", "dnf") => ("/usr/bin/dnf", &["install", "-y"]),
        ("update", "dnf") => ("/usr/bin/dnf", &["upgrade", "-y", "--skip-unavailable"]),
        ("remove", "dnf") => ("/usr/bin/dnf", &["remove", "-y"]),
        ("refresh", "dnf") => ("/usr/bin/dnf", &["check-upgrade", "--refresh"]),
        ("install" | "update", "pacman") => ("/usr/bin/pacman", &["-S", "--needed", "--noconfirm"]),
        ("remove", "pacman") => ("/usr/bin/pacman", &["-R", "--noconfirm"]),
        ("refresh", "pacman") => ("/usr/bin/pacman", &["-Sy", "--noconfirm"]),
        ("install", "zypper") => ("/usr/bin/zypper", &["--non-interactive", "install", "-y"]),
        ("update", "zypper") => ("/usr/bin/zypper", &["--non-interactive", "update", "-y"]),
        ("remove", "zypper") => ("/usr/bin/zypper", &["--non-interactive", "remove", "-y"]),
        ("refresh", "zypper") => ("/usr/bin/zypper", &["--non-interactive", "refresh"]),
        (_, "apt" | "dnf" | "pacman" | "zypper") => {
            return Err(format!("unsupported action: {action}"));
        }
        _ => return Err(format!("unsupported manager: {manager}")),
    };

    if action == "refresh" {
        if !packages.is_empty() {
            return Err("refresh does not accept package names".to_owned());
        }
    } else if packages.is_empty() {
        return Err(format!("{action} requires at least one package"));
    }
    if packages.len() > MAX_PACKAGES {
        return Err(format!("package batch exceeds {MAX_PACKAGES} entries"));
    }

    for package in packages {
        validate_package_name(package)?;
    }

    let mut resolved_arguments = command_arguments
        .iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
    resolved_arguments.extend(packages.iter().cloned());
    Ok(CommandPlan {
        program,
        arguments: resolved_arguments,
    })
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

    #[test]
    fn maps_every_supported_manager_action_to_a_fixed_command() {
        for (action, manager, program, expected) in [
            (
                "install",
                "apt",
                "/usr/bin/apt-get",
                vec!["install", "-y", "bash"],
            ),
            (
                "update",
                "apt",
                "/usr/bin/apt-get",
                vec!["install", "-y", "--only-upgrade", "bash"],
            ),
            (
                "remove",
                "apt",
                "/usr/bin/apt-get",
                vec!["remove", "-y", "bash"],
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
                vec!["-S", "--needed", "--noconfirm", "bash"],
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
                vec!["-Sy", "--noconfirm"],
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
            if action != "refresh" {
                input.push("bash");
            }
            let plan = command_plan(&arguments(&input)).expect("build allowed command");
            assert_eq!(plan.program, program, "{action} {manager}");
            assert_eq!(plan.arguments, arguments(&expected), "{action} {manager}");
        }
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
        assert_eq!(plan.arguments.len(), 5);

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
    fn rejects_unknown_or_incomplete_requests() {
        for input in [
            vec![],
            vec!["install"],
            vec!["install", "flatpak", "org.example.App"],
            vec!["shell", "apt", "bash"],
            vec!["install", "apt"],
            vec!["refresh", "apt", "bash"],
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
