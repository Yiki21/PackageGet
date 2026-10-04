use std::sync::Mutex;

#[cfg(unix)]
use std::{
    fs,
    path::{Path, PathBuf},
};

use tempfile::tempdir;
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapability, ManagerConfig,
    ManagerErrorKind, PackageAction, PackageManager, PackageTarget, ProgressEvent,
};
use updater_managers::DnfManager;

#[test]
fn dnf_descriptor_exposes_the_stable_public_contract() {
    let manager = DnfManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:dnf");
    assert_eq!(descriptor.display_name(), "DNF");
    assert!(matches!(
        descriptor.authorization(),
        AuthorizationHint::RequiresElevation { .. }
    ));
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
    let manager = DnfManager::new();
    let directory = tempdir().expect("create temporary directory");
    let missing = directory.path().join("missing-dnf");
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
async fn empty_execution_emits_boundaries_without_running_dnf() {
    let manager = DnfManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[], &sink)
        .await
        .expect("execute empty DNF group");

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
    let manager = DnfManager::new();
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

    let info_error = manager
        .package_info(&config, &target)
        .await
        .expect_err("reject mismatched package information target");
    assert_eq!(info_error.kind(), ManagerErrorKind::Protocol);
}

/// Fake `dnf` that reports three updates and two search results.
///
/// The `search-fails` marker makes `dnf search` fail the way an unreachable
/// repository or a locked cache would; dnf exits 0 with "No matches found."
/// when a search simply matches nothing.
#[cfg(unix)]
const FAKE_DNF_SCRIPT: &str = r#"#!/bin/sh
case "$1" in
  check-upgrade)
    if [ -e "${0%/*}/no-updates" ]; then
      exit 100
    fi
    printf 'kernel-core.x86_64 6.9.1-200.fc40 updates\n'
    printf 'bash.x86_64 5.2.30-1.fc40 updates\n'
    printf 'fzf.x86_64 0.65.0-1.fc40 updates\n'
    exit 100
    ;;
  search)
    if [ -e "${0%/*}/search-fails" ]; then
      printf 'Error: Failed to download metadata for repo\n' >&2
      exit 1
    fi
    if [ -e "${0%/*}/search-empty" ]; then
      printf 'No matches found.\n'
      exit 0
    fi
    printf 'Matched fields: name, summary\n'
    printf 'kernel-core.x86_64\tThe Linux kernel\n'
    printf 'fzf.x86_64\tA command-line fuzzy finder\n'
    exit 0
    ;;
esac
exit 2
"#;

/// Fake `rpm` that logs every invocation and fails while `rpm-fails` exists.
///
/// kernel-core has three installed instances. Like real RPM, `rpm -q` prints
/// one version per instance and only separates them when the query format
/// ends in a newline; otherwise the instances are glued together.
///
/// `rpm-fails` reports an unreadable rpmdb, which is a failure and not a
/// missing package; with `rpmdb-broken` present even a single-package query
/// fails the same way.
#[cfg(unix)]
const FAKE_RPM_SCRIPT: &str = r#"#!/bin/sh
directory=${0%/*}
printf 'rpm %s\n' "$(printf '%s' "$*" | tr '\n' ' ')" >> "$directory/rpm-invocations.log"
if [ -e "$directory/rpmdb-broken" ]; then
  printf 'error: cannot open Packages database in /var/lib/rpm\n' >&2
  exit 1
fi
if [ -e "$directory/rpm-fails" ]; then
  printf 'rpmdb open failed\n' >&2
  exit 1
fi
newline=$(printf 'x\n')
newline=${newline%x}
case "$1" in
  -qa)
    printf 'kernel-core\t6.8.5-300.fc40\n'
    printf 'kernel-core\t6.8.10-300.fc40\n'
    printf 'kernel-core\t6.8.7-300.fc40\n'
    printf 'bash\t5.2.26-3.fc40\n'
    exit 0
    ;;
  -q)
    case "$4" in
      kernel-core)
        case "$3" in
          *"$newline") printf '6.8.5-300.fc40\n6.8.10-300.fc40\n6.8.7-300.fc40\n' ;;
          *) printf '6.8.5-300.fc406.8.10-300.fc406.8.7-300.fc40' ;;
        esac
        ;;
      bash) printf '5.2.26-3.fc40\n' ;;
      *)
        printf 'package %s is not installed\n' "$4" >&2
        exit 1
        ;;
    esac
    exit 0
    ;;
esac
exit 2
"#;

/// Names the fake `dnf`/`rpm` directory for the child test process.
#[cfg(unix)]
const FAKE_DNF_DIRECTORY_ENV: &str = "UPDATER_DNF_CONTRACT_FAKE_DIRECTORY";
#[cfg(unix)]
const FAKE_RPM_CHILD_TEST: &str = "fake_rpm_on_path_child_checks_dnf_rpm_lookups";

#[cfg(unix)]
fn write_fake_executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, script).expect("write fake executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("mark fake executable");
}

#[cfg(unix)]
fn take_rpm_invocations(directory: &Path) -> Vec<String> {
    let log = directory.join("rpm-invocations.log");
    let invocations = fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    let _ = fs::remove_file(log);
    invocations
}

/// DNF resolves `rpm` through `PATH`, which is process-global. Instead of
/// mutating this multi-threaded test process, the assertions run in a child
/// copy of this test binary whose `PATH` starts with the fake `rpm`.
#[cfg(unix)]
#[test]
fn dnf_updates_and_search_query_rpm_once_per_call() {
    let directory = tempdir().expect("create fake DNF directory");
    write_fake_executable(&directory.path().join("dnf"), FAKE_DNF_SCRIPT);
    write_fake_executable(&directory.path().join("rpm"), FAKE_RPM_SCRIPT);
    let path = std::env::join_paths(
        std::iter::once(directory.path().to_path_buf()).chain(
            std::env::var_os("PATH")
                .iter()
                .flat_map(std::env::split_paths),
        ),
    )
    .expect("join fake RPM PATH");

    let output = std::process::Command::new(std::env::current_exe().expect("locate test binary"))
        .args([
            "--ignored",
            "--exact",
            FAKE_RPM_CHILD_TEST,
            "--test-threads=1",
        ])
        .env("PATH", path)
        .env(FAKE_DNF_DIRECTORY_ENV, directory.path())
        .output()
        .expect("run fake RPM child test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "child test failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("1 passed"),
        "child test did not run\nstdout:\n{stdout}"
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "child process of dnf_updates_and_search_query_rpm_once_per_call"]
async fn fake_rpm_on_path_child_checks_dnf_rpm_lookups() {
    let Some(directory) = std::env::var_os(FAKE_DNF_DIRECTORY_ENV).map(PathBuf::from) else {
        eprintln!("skipped: run through dnf_updates_and_search_query_rpm_once_per_call");
        return;
    };
    let manager = DnfManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone())
        .with_executable(directory.join("dnf"));

    let updates = manager
        .updates(&config, false)
        .await
        .expect("list fake DNF updates");
    assert_eq!(updates.len(), 3);
    assert_eq!(updates[0].target.name, "kernel-core");
    assert_eq!(updates[0].current_version, "6.8.10-300.fc40");
    assert_eq!(updates[0].available_version, "6.9.1-200.fc40");
    assert_eq!(updates[1].current_version, "5.2.26-3.fc40");
    assert_eq!(updates[2].current_version, "unknown");
    let invocations = take_rpm_invocations(&directory);
    assert_eq!(
        invocations.len(),
        1,
        "three update rows must share one RPM query: {invocations:?}"
    );
    assert!(
        invocations[0].starts_with("rpm -qa --queryformat "),
        "updates must read every installed version in one query: {invocations:?}"
    );

    let search = manager
        .search(&config, "python")
        .await
        .expect("search fake DNF repositories");
    assert_eq!(search.len(), 2);
    assert_eq!(search[0].name, "fzf");
    assert_eq!(search[0].version, "Not Installed");
    assert_eq!(search[1].name, "kernel-core");
    assert_eq!(search[1].version, "6.8.10-300.fc40");
    let invocations = take_rpm_invocations(&directory);
    assert_eq!(
        invocations.len(),
        1,
        "two search rows must share one RPM query: {invocations:?}"
    );

    assert_eq!(
        manager
            .current_version(&config, "kernel-core")
            .await
            .expect("query fake multi-instance kernel-core version"),
        "6.8.10-300.fc40"
    );
    let _ = take_rpm_invocations(&directory);

    fs::write(directory.join("no-updates"), "").expect("make fake DNF report no updates");
    assert!(
        manager
            .updates(&config, false)
            .await
            .expect("list no fake DNF updates")
            .is_empty()
    );
    assert!(
        take_rpm_invocations(&directory).is_empty(),
        "an up-to-date system must not query RPM"
    );
    fs::remove_file(directory.join("no-updates")).expect("restore fake DNF updates");

    fs::write(directory.join("rpm-fails"), "").expect("make fake RPM fail");
    let error = manager
        .updates(&config, false)
        .await
        .expect_err("an unreadable rpmdb must fail the update listing");
    assert_eq!(error.kind(), ManagerErrorKind::Other);
    let error = manager
        .search(&config, "python")
        .await
        .expect_err("an unreadable rpmdb must fail the search, not label rows absent");
    assert_eq!(error.kind(), ManagerErrorKind::Other);
    assert_eq!(take_rpm_invocations(&directory).len(), 2);
    fs::remove_file(directory.join("rpm-fails")).expect("restore fake RPM");

    // A package RPM cannot find is still an absent package, but a database
    // that cannot be opened is not: the two must not collapse into one answer.
    fs::write(directory.join("rpmdb-broken"), "").expect("break the fake rpmdb");
    let error = manager
        .package_info(
            &config,
            &PackageTarget::new(manager.descriptor().id().clone(), "bash"),
        )
        .await
        .expect_err("a broken rpmdb must not read as an absent package");
    assert_eq!(error.kind(), ManagerErrorKind::Other);
    let _ = take_rpm_invocations(&directory);
    fs::remove_file(directory.join("rpmdb-broken")).expect("restore the fake rpmdb");

    // `rpm -q` for a package that is genuinely absent keeps returning `None`.
    assert!(
        manager
            .package_info(
                &config,
                &PackageTarget::new(manager.descriptor().id().clone(), "nosuchpackage"),
            )
            .await
            .expect("query a missing package")
            .is_none()
    );
    let _ = take_rpm_invocations(&directory);

    // dnf exits 0 with "No matches found.", so an empty search is still an
    // empty listing rather than an error.
    fs::write(directory.join("search-empty"), "").expect("make fake DNF match nothing");
    assert!(
        manager
            .search(&config, "zzz")
            .await
            .expect("a no-match search is an empty listing")
            .is_empty()
    );
    assert!(
        take_rpm_invocations(&directory).is_empty(),
        "an empty search must not query RPM"
    );
    let _ = fs::remove_file(directory.join("search-empty"));

    fs::write(directory.join("search-fails"), "").expect("make fake DNF search fail");
    let error = manager
        .search(&config, "python")
        .await
        .expect_err("a failed dnf search must not read as an empty result");
    assert_eq!(error.kind(), ManagerErrorKind::Network);
    let _ = fs::remove_file(directory.join("search-fails"));
}

#[tokio::test]
#[ignore = "requires a local DNF installation and RPM database"]
async fn local_dnf_availability_and_installed_listing_smoke() {
    let manager = DnfManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    let availability = manager
        .availability(&config)
        .await
        .expect("check local DNF availability");
    assert!(availability.is_available());

    let installed = manager
        .installed(&config)
        .await
        .expect("list installed RPM packages");
    let count = manager
        .count_installed(&config)
        .await
        .expect("count installed RPM packages");

    assert!(!installed.is_empty());
    assert_eq!(count, installed.len());
    assert!(
        installed
            .iter()
            .all(|package| package.manager_id == *manager.descriptor().id())
    );
}
