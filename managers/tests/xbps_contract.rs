use std::sync::Mutex;

use tempfile::tempdir;
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapability, ManagerConfig,
    ManagerErrorKind, PackageAction, PackageManager, PackageScope, PackageTarget, ProgressEvent,
};
use updater_managers::XbpsManager;

#[cfg(target_os = "linux")]
fn fake_xbps() -> (tempfile::TempDir, std::path::PathBuf) {
    use std::{fs, os::unix::fs::PermissionsExt};

    let directory = tempdir().expect("create fake XBPS directory");
    let script = r#"#!/bin/sh
command_name=${0##*/}
case "$command_name:$1" in
  xbps-query:--version|xbps-install:--version|xbps-remove:--version)
    printf 'XBPS: 0.60.7 API: 20240314 GIT: UNSET\n'
    ;;
  xbps-query:--list-pkgs)
    printf 'ii base-files-0.142_11 Void Linux base system files\n'
    printf 'ii xbps-0.60.7_1 XBPS package system utilities\n'
    ;;
  xbps-query:--property)
    printf '%s-0.60.7_1\n' "$3"
    ;;
  xbps-query:--repository)
    printf '[*] xbps-0.60.8_1 XBPS package system utilities\n'
    printf '[-] xtools-0.70_1 helper tools\n'
    ;;
  xbps-install:--update)
    printf 'dependency-1.0_1 install x86_64 repo 1 1\n'
    printf 'xbps-0.60.8_1 update x86_64 repo 2 2\n'
    ;;
  *)
    printf 'unexpected fake XBPS command: %s %s\n' "$command_name" "$*" >&2
    exit 64
    ;;
esac
"#;
    for command in ["xbps-query", "xbps-install", "xbps-remove"] {
        let path = directory.path().join(command);
        fs::write(&path, script).expect("write fake XBPS executable");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("mark fake XBPS executable");
    }
    let query = directory.path().join("xbps-query");
    (directory, query)
}

#[test]
fn xbps_descriptor_exposes_the_void_system_contract() {
    let manager = XbpsManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:xbps");
    assert_eq!(descriptor.display_name(), "XBPS");
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
async fn missing_custom_xbps_query_is_an_offline_availability_result() {
    let manager = XbpsManager::new();
    let directory = tempdir().expect("create temporary directory");
    let missing = directory.path().join("missing-xbps-query");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(&missing);

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check missing xbps-query"),
        ManagerAvailability::Unavailable {
            reason: AvailabilityReason::CommandMissing {
                command: missing.to_string_lossy().into_owned(),
            },
        }
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn fake_xbps_drives_every_read_only_public_api() {
    let manager = XbpsManager::new();
    let (_directory, query) = fake_xbps();
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(query);

    assert!(matches!(
        manager
            .availability(&config)
            .await
            .expect("check fake XBPS availability"),
        ManagerAvailability::Available { version: Some(version) }
            if version.starts_with("XBPS: 0.60.7")
    ));

    let installed = manager
        .installed(&config)
        .await
        .expect("list fake XBPS packages");
    assert_eq!(installed.len(), 2);
    assert_eq!(installed[0].name, "base-files");
    assert_eq!(installed[1].name, "xbps");
    assert_eq!(
        manager
            .count_installed(&config)
            .await
            .expect("count fake XBPS packages"),
        2
    );
    assert_eq!(
        manager
            .current_version(&config, "xbps")
            .await
            .expect("query fake XBPS version"),
        "0.60.7_1"
    );

    let updates = manager
        .updates(&config, false)
        .await
        .expect("list fake XBPS updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "xbps");
    assert_eq!(updates[0].available_version, "0.60.8_1");

    let search = manager
        .search(&config, "tools")
        .await
        .expect("search fake XBPS repository");
    assert_eq!(search.len(), 2);
    assert_eq!(search[0].version, "0.60.7_1");
    assert_eq!(search[1].version, "Not Installed");
}

#[tokio::test]
async fn empty_execution_emits_boundaries_without_running_xbps() {
    let manager = XbpsManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[], &sink)
        .await
        .expect("execute empty XBPS group");

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
    let manager = XbpsManager::new();
    let wrong_config = ManagerConfig::new(
        updater_manager_api::ManagerId::parse("builtin:cargo").expect("valid Cargo ID"),
    );
    assert_eq!(
        manager
            .availability(&wrong_config)
            .await
            .expect_err("reject mismatched config")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let target = PackageTarget::new(
        updater_manager_api::ManagerId::parse("org.example:other")
            .expect("valid external manager ID"),
        "xbps",
    );
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);
    let error = manager
        .execute(&config, PackageAction::Update, &[target], &sink)
        .await
        .expect_err("reject mismatched target");
    assert_eq!(error.kind(), ManagerErrorKind::Protocol);
    assert!(events.lock().expect("progress lock").is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires Void Linux XBPS and a readable package database"]
async fn void_container_xbps_read_only_smoke() {
    let manager = XbpsManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    let availability = manager
        .availability(&config)
        .await
        .expect("check XBPS availability");
    assert!(matches!(
        availability,
        ManagerAvailability::Available { version: Some(version) }
            if version.contains("XBPS")
    ));
    let installed = manager
        .installed(&config)
        .await
        .expect("list XBPS packages");
    let count = manager
        .count_installed(&config)
        .await
        .expect("count XBPS packages");
    assert!(!installed.is_empty());
    assert_eq!(count, installed.len());
    assert!(installed.iter().all(|package| {
        package.manager_id == *manager.descriptor().id()
            && package.scope == PackageScope::System
            && package.origin.as_ref().map(|origin| origin.name.as_str()) == Some("XBPS")
    }));
    let first = installed.first().expect("at least one XBPS package");
    assert_eq!(
        manager
            .current_version(&config, &first.name)
            .await
            .expect("query one XBPS package"),
        first.version
    );
}
