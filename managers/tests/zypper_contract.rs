use std::{fs, path::PathBuf, sync::Mutex};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapability, ManagerConfig,
    ManagerErrorKind, PackageAction, PackageManager, PackageScope, PackageTarget, ProgressEvent,
};
use updater_managers::ZypperManager;

#[cfg(unix)]
fn fake_zypper(script: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Zypper directory");
    let executable = directory.path().join("zypper");
    fs::write(&executable, script).expect("write fake Zypper executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Zypper executable");
    (directory, executable)
}

#[test]
fn zypper_descriptor_exposes_the_stable_public_contract() {
    let manager = ZypperManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:zypper");
    assert_eq!(descriptor.display_name(), "Zypper");
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
    let manager = ZypperManager::new();
    let directory = tempdir().expect("create temporary directory");
    let missing = directory.path().join("missing-zypper");
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
async fn empty_execution_emits_boundaries_without_running_zypper() {
    let manager = ZypperManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[], &sink)
        .await
        .expect("execute empty Zypper group");

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
    let manager = ZypperManager::new();
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
#[tokio::test]
async fn public_updates_enforce_c_locale_and_table_contracts() {
    let manager = ZypperManager::new();
    let (_directory, executable) = fake_zypper(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf 'zypper 1.14.93\n'
  exit 0
fi
if [ "${LC_ALL:-}" != "C" ]; then
  printf 'LC_ALL was not C\n' >&2
  exit 90
fi
if [ "$1" = "--non-interactive" ] && [ "$2" = "list-updates" ]; then
  printf 'Loading repository data...\n'
  printf 'Available Version | Repository | Name | Current Version | Arch\n'
  printf '------------------+------------+------+-----------------+-------\n'
  printf '5.2-4.1 | repo-update | bash | 5.2-3.1 | x86_64\n'
  printf '5.2-5.1 | repo-testing | bash | 5.2-4.1 | x86_64\n'
  printf '9.1-3.1 | repo-update | vim | 9.1-2.1 | x86_64\n'
  exit 0
fi
exit 2
"#,
    );
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check fake Zypper availability"),
        ManagerAvailability::Available {
            version: Some("zypper 1.14.93".to_owned()),
        }
    );

    let updates = manager
        .updates(&config, false)
        .await
        .expect("list fake Zypper updates");
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].target.name, "bash");
    assert_eq!(updates[0].target.scope, PackageScope::System);
    assert_eq!(updates[0].current_version, "5.2-3.1");
    assert_eq!(updates[0].available_version, "5.2-4.1");
    assert_eq!(updates[1].target.name, "vim");
    assert!(
        updates
            .iter()
            .all(|update| update.target.manager_id == *manager.descriptor().id())
    );
}

#[cfg(unix)]
#[tokio::test]
async fn public_search_maps_zypper_statuses_and_only_104_to_empty() {
    let manager = ZypperManager::new();
    let (_directory, executable) = fake_zypper(
        r#"#!/bin/sh
if [ "${LC_ALL:-}" != "C" ]; then
  printf 'LC_ALL was not C\n' >&2
  exit 90
fi
if [ "$1" != "--non-interactive" ] || [ "$2" != "search" ] || [ "$3" != "--details" ]; then
  exit 2
fi
case "$4" in
  code-5) exit 5 ;;
  code-7) exit 7 ;;
  code-102) exit 102 ;;
  code-103) exit 103 ;;
  code-104) exit 104 ;;
  code-105) exit 105 ;;
  code-106)
    printf 'S | Name | Type | Version | Arch | Repository\n'
    printf '  | partial | package | 1.0 | x86_64 | skipped-repo\n'
    printf 'repository skipped\n' >&2
    exit 106
    ;;
  code-107) exit 107 ;;
  not-authorized) printf 'not authorized\n' >&2; exit 126 ;;
  cancelled-text) printf 'operation was cancelled\n' >&2; exit 127 ;;
  *) exit 104 ;;
esac
"#,
    );
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);

    assert!(
        manager
            .search(&config, "code-104")
            .await
            .expect("treat Zypper search 104 as no match")
            .is_empty()
    );

    for (query, expected) in [
        ("code-5", ManagerErrorKind::Permission),
        ("code-7", ManagerErrorKind::Busy),
        ("code-102", ManagerErrorKind::RebootRequired),
        ("code-103", ManagerErrorKind::Other),
        ("code-105", ManagerErrorKind::Cancelled),
        ("code-106", ManagerErrorKind::Network),
        ("code-107", ManagerErrorKind::Other),
        ("not-authorized", ManagerErrorKind::Permission),
        ("cancelled-text", ManagerErrorKind::Cancelled),
    ] {
        let error = manager
            .search(&config, query)
            .await
            .expect_err("surface Zypper search failure");
        assert_eq!(error.kind(), expected, "query {query}");
        assert!(error.detail().is_some(), "query {query} retains detail");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn update_failures_reject_partial_rows_and_do_not_treat_104_as_empty() {
    let manager = ZypperManager::new();
    let (_directory, executable) = fake_zypper(
        r#"#!/bin/sh
if [ "${LC_ALL:-}" != "C" ]; then
  exit 90
fi
printf 'S | Repository | Name | Current Version | Available Version | Arch\n'
printf 'v | repo-update | bash | 5.2-3.1 | 5.2-4.1 | x86_64\n'
printf 'repository skipped\n' >&2
exit 106
"#,
    );
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);

    let error = manager
        .updates(&config, false)
        .await
        .expect_err("reject partial Zypper update result");
    assert_eq!(error.kind(), ManagerErrorKind::Network);
    assert!(
        error
            .detail()
            .is_some_and(|detail| detail.contains("repository skipped"))
    );

    let (_directory, executable) = fake_zypper(
        r#"#!/bin/sh
if [ "${LC_ALL:-}" != "C" ]; then
  exit 90
fi
printf 'no update capability matched\n' >&2
exit 104
"#,
    );
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    let error = manager
        .updates(&config, false)
        .await
        .expect_err("surface Zypper update status 104");
    assert_eq!(error.kind(), ManagerErrorKind::Other);
}

#[tokio::test]
#[ignore = "requires Zypper and a readable RPM database"]
async fn tumbleweed_container_zypper_read_only_smoke() {
    let manager = ZypperManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    let availability = manager
        .availability(&config)
        .await
        .expect("check Zypper availability");
    assert!(matches!(
        availability,
        ManagerAvailability::Available { version: Some(version) }
            if version.to_ascii_lowercase().contains("zypper")
    ));

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
    assert!(installed.iter().all(|package| {
        package.manager_id == *manager.descriptor().id() && package.scope == PackageScope::System
    }));

    let first = installed.first().expect("at least one installed package");
    assert_eq!(
        manager
            .current_version(&config, &first.name)
            .await
            .expect("query one installed RPM package"),
        first.version
    );
}
