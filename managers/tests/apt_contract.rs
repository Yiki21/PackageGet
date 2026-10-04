use std::sync::Mutex;

use tempfile::tempdir;
use updater_manager_api::{
    AvailabilityReason, ManagerAvailability, ManagerCapability, ManagerConfig, ManagerErrorKind,
    PackageAction, PackageManager, PackageTarget, ProgressEvent,
};
use updater_managers::AptManager;

#[test]
fn apt_descriptor_exposes_the_stable_public_contract() {
    let manager = AptManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:apt");
    assert_eq!(descriptor.display_name(), "APT");
    for capability in [
        ManagerCapability::Installed,
        ManagerCapability::Updates,
        ManagerCapability::Search,
        ManagerCapability::Install,
        ManagerCapability::Update,
        ManagerCapability::Uninstall,
    ] {
        assert!(descriptor.capabilities().contains(capability));
    }
}

#[tokio::test]
async fn missing_custom_executable_is_an_offline_availability_result() {
    let manager = AptManager::new();
    let directory = tempdir().expect("create temporary directory");
    let missing = directory.path().join("missing-apt");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(&missing);

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check missing executable"),
        ManagerAvailability::Unavailable {
            reason: AvailabilityReason::CommandMissing {
                command: missing.to_string_lossy().into_owned(),
            },
        }
    );
}

#[tokio::test]
async fn empty_execution_emits_boundaries_without_running_apt() {
    let manager = AptManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[], &sink)
        .await
        .expect("execute empty APT group");

    assert_eq!(
        *events.lock().expect("progress lock"),
        vec![
            ProgressEvent::Started {
                action: PackageAction::Install,
                total: 0,
            },
            ProgressEvent::Finished {
                completed: 0,
                total: 0,
            },
        ]
    );
}

#[tokio::test]
async fn mismatched_config_and_targets_are_rejected_before_progress() {
    let manager = AptManager::new();
    let wrong_config = ManagerConfig::new(
        updater_manager_api::ManagerId::parse("builtin:cargo").expect("valid Cargo ID"),
    );
    let config_error = manager
        .availability(&wrong_config)
        .await
        .expect_err("reject mismatched config");
    assert_eq!(config_error.kind(), ManagerErrorKind::Protocol);

    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let target = PackageTarget::new(
        updater_manager_api::ManagerId::parse("org.example:other")
            .expect("valid external manager ID"),
        "bash",
    );
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);
    let target_error = manager
        .execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&target),
            &sink,
        )
        .await
        .expect_err("reject mismatched target");

    assert_eq!(target_error.kind(), ManagerErrorKind::Protocol);
    assert!(events.lock().expect("progress lock").is_empty());
}

#[cfg(unix)]
mod fake_path {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use tempfile::TempDir;
    use updater_manager_api::{ManagerConfig, PackageManager};
    use updater_managers::AptManager;

    /// Names the fake `apt`/`dpkg-query` directory for the child test process.
    const FAKE_APT_DIRECTORY_ENV: &str = "UPDATER_APT_CONTRACT_FAKE_DIRECTORY";
    const BATCHING_CHILD_TEST: &str =
        "fake_path::apt_listing_batching_child_checks_locale_and_batching";

    /// Fake `dpkg-query` that logs every invocation and fails while
    /// `dpkg-query-fails` exists.
    ///
    /// The map query (`-W -f=<name>\t<version>`) answers for every installed
    /// package; a single-package query (`-W -f=<version> <name>`) answers with
    /// its own marker version, so the two paths cannot be confused.
    const FAKE_DPKG_QUERY_SCRIPT: &str = r#"#!/bin/sh
directory=${0%/*}
printf 'dpkg-query %s\n' "$(printf '%s' "$*" | tr '\n' ' ')" >> "$directory/dpkg-query-invocations.log"
if [ -e "$directory/dpkg-query-fails" ]; then
  printf 'dpkg-query: error: failed to open package info file\n' >&2
  exit 2
fi
case "$1" in
  -W)
    if [ "$#" -ge 3 ]; then
      printf '3.3.3-single\n'
    else
      printf 'bash\t5.2.26-1\nvim\t9.1.0-1\n'
    fi
    ;;
esac
exit 0
"#;

    /// Fake `apt` that logs its invocation and environment, then prints one
    /// update row with an English marker only when the locale is fixed.
    ///
    /// A real apt prints `[aktualisierbar von: 5.2]` under a German locale,
    /// which the parser cannot read; the marker is translated by gettext. The
    /// fake reproduces that so the test fails if `LC_ALL=C` is not enforced.
    const FAKE_APT_SCRIPT: &str = r#"#!/bin/sh
directory=${0%/*}
printf 'apt %s\n' "$*" >> "$directory/apt-invocations.log"
printf 'lc_all=%s lang=%s\n' "${LC_ALL:-unset}" "${LANG:-unset}" >> "$directory/apt-locale.log"
if [ "$1" = "list" ] && [ "$2" = "--upgradable" ]; then
  if [ -e "$directory/apt-no-marker" ]; then
    printf 'bash/stable 5.2.30-1 amd64\n'
    printf 'vim/stable 2:9.1.1234 amd64 [upgradable from: 9.1.0-1]\n'
  elif [ "${LC_ALL:-}" = "C" ]; then
    printf 'bash/stable 5.2.30-1 amd64 [upgradable from: 5.1.0-marker]\n'
    printf 'vim/stable 2:9.1.1234 amd64 [upgradable from: 9.1.0-1]\n'
  else
    printf 'bash/stable 5.2.30-1 amd64 [aktualisierbar von: 5.1.0-marker]\n'
    printf 'vim/stable 2:9.1.1234 amd64 [aktualisierbar von: 9.1.0-1]\n'
  fi
  exit 0
fi
exit 2
"#;

    fn write_fake_executable(path: &Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, script).expect("write fake executable");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("mark fake executable");
    }

    fn fake_apt_directory() -> TempDir {
        let directory = tempfile::tempdir().expect("create fake apt directory");
        for (name, script) in [
            ("apt", FAKE_APT_SCRIPT),
            ("dpkg-query", FAKE_DPKG_QUERY_SCRIPT),
        ] {
            write_fake_executable(&directory.path().join(name), script);
        }
        directory
    }

    fn take_invocations(directory: &Path, name: &str) -> Vec<String> {
        let log = directory.join(format!("{name}-invocations.log"));
        let invocations = fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        let _ = fs::remove_file(log);
        invocations
    }

    /// `apt-cache` and `dpkg-query` are resolved through `PATH`, which is
    /// process-global, so the assertions run in a child copy of this test
    /// binary whose `PATH` starts with the fake executables.
    fn run_fake_apt_child(child_test: &str, parent: &str) -> String {
        let directory = fake_apt_directory();
        let path = std::env::join_paths(
            std::iter::once(directory.path().to_path_buf()).chain(
                std::env::var_os("PATH")
                    .iter()
                    .flat_map(std::env::split_paths),
            ),
        )
        .expect("join fake apt PATH");

        let output =
            std::process::Command::new(std::env::current_exe().expect("locate test binary"))
                .args(["--ignored", "--exact", child_test, "--test-threads=1"])
                .env("PATH", path)
                // A German locale is what a translated environment looks like;
                // the manager must pin the locale itself instead of inheriting
                // it.
                .env("LC_ALL", "de_DE.UTF-8")
                .env("LANG", "de_DE.UTF-8")
                .env(FAKE_APT_DIRECTORY_ENV, directory.path())
                .output()
                .expect("run fake apt child test");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "child test {parent} failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.contains("1 passed"),
            "child test {parent} did not run\nstdout:\n{stdout}"
        );
        stdout
    }

    /// The fake `apt` and `dpkg-query` live on `PATH`, so the assertions run in
    /// a child process; the argument is the child that must run.
    #[test]
    fn apt_listing_runs_in_a_fixed_locale_with_one_batched_version_lookup() {
        run_fake_apt_child(
            BATCHING_CHILD_TEST,
            "apt_listing_runs_in_a_fixed_locale_with_one_batched_version_lookup",
        );
    }

    #[ignore = "child process of apt_listing_runs_in_a_fixed_locale_with_one_batched_version_lookup"]
    #[tokio::test]
    async fn apt_listing_batching_child_checks_locale_and_batching() {
        let Some(directory) = std::env::var_os(FAKE_APT_DIRECTORY_ENV).map(PathBuf::from) else {
            eprintln!(
                "skipped: run through apt_listing_runs_in_a_fixed_locale_with_one_batched_version_lookup"
            );
            return;
        };
        let manager = AptManager::new();
        let config = ManagerConfig::new(manager.descriptor().id().clone());

        // The listing runs under a fixed locale, so the English marker is read
        // directly and no per-row `dpkg-query` fallback is needed. The marker
        // version differs from the map version, so a fallback would be visible.
        let updates = manager
            .updates(&config, false)
            .await
            .expect("list fake apt updates");
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].target.name, "bash");
        assert_eq!(updates[0].current_version, "5.1.0-marker");
        assert_eq!(updates[0].available_version, "5.2.30-1");
        assert_eq!(updates[1].current_version, "9.1.0-1");
        assert!(
            take_invocations(&directory, "dpkg-query").is_empty(),
            "a normal system must never run one dpkg-query per update"
        );
        let locale =
            fs::read_to_string(directory.join("apt-locale.log")).expect("read fake apt locale log");
        assert_eq!(locale.lines().count(), 1, "{locale}");
        for line in locale.lines() {
            assert_eq!(line, "lc_all=C lang=C", "apt must run in the C locale");
        }

        // When the marker is genuinely missing, one batched lookup fills every
        // affected row instead of one process per row.
        fs::write(directory.join("apt-no-marker"), "").expect("hide the upgrade marker");
        let updates = manager
            .updates(&config, false)
            .await
            .expect("list fake apt updates without markers");
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].current_version, "5.2.26-1");
        assert_eq!(updates[1].current_version, "9.1.0-1");
        let invocations = take_invocations(&directory, "dpkg-query");
        assert_eq!(
            invocations.len(),
            1,
            "both unknown rows must share one batched dpkg-query: {invocations:?}"
        );
        fs::remove_file(directory.join("apt-no-marker")).expect("restore the upgrade marker");
    }
}
