use std::sync::Mutex;

use tempfile::tempdir;
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapability, ManagerConfig,
    ManagerErrorKind, PackageAction, PackageManager, PackageScope, PackageTarget, ProgressEvent,
};
use updater_managers::PortageManager;

#[cfg(target_os = "linux")]
fn fake_portage() -> (tempfile::TempDir, std::path::PathBuf) {
    use std::{fs, os::unix::fs::PermissionsExt};

    let directory = tempdir().expect("create fake Portage directory");
    let script = r#"#!/bin/sh
command_name=${0##*/}
case "$command_name:$1" in
  emerge:--version)
    printf 'Portage 3.0.72 (python 3.13.7-final-0, default/linux/amd64/23.0)\n'
    ;;
  qlist:--version)
    printf 'portage-utils-0.100.1\n'
    ;;
  qlist:--installed)
    if [ "$#" -eq 4 ]; then
      case "$4" in
        dev-lang/python:3.13)
          printf 'dev-lang/python\t3.13.14\t3.13\tgentoo\n'
          ;;
        *)
          exit 1
          ;;
      esac
    else
      printf 'dev-lang/python\t3.13.14\t3.13\tgentoo\n'
      printf 'dev-python/librt\t0.12.0\t0\tgentoo\n'
    fi
    ;;
  emerge:--pretend)
    printf '[ebuild     UD] dev-python/librt-0.11.0 [0.12.0] USE=test\n'
    printf '[ebuild   R   ] dev-lang/python-3.13.14 USE=ssl\n'
    ;;
  emerge:--search)
    printf '*  app-shells/bash\n'
    printf '      Latest version available: 5.3_p9-r2\n'
    printf '      Latest version installed: [ Not Installed ]\n'
    printf '      Homepage: https://www.gnu.org/software/bash/\n'
    printf '      Description: The standard shell\n'
    ;;
  *)
    printf 'unexpected fake Portage command: %s %s\n' "$command_name" "$*" >&2
    exit 64
    ;;
esac
"#;
    for command in ["emerge", "qlist"] {
        let path = directory.path().join(command);
        fs::write(&path, script).expect("write fake Portage executable");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("mark fake Portage executable");
    }
    let emerge = directory.path().join("emerge");
    (directory, emerge)
}

#[test]
fn portage_descriptor_exposes_the_gentoo_system_contract() {
    let manager = PortageManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:portage");
    assert_eq!(descriptor.display_name(), "Portage");
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
async fn missing_custom_emerge_is_an_offline_availability_result() {
    let manager = PortageManager::new();
    let directory = tempdir().expect("create temporary directory");
    let missing = directory.path().join("missing-emerge");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(&missing);

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check missing emerge"),
        ManagerAvailability::Unavailable {
            reason: AvailabilityReason::CommandMissing {
                command: missing.to_string_lossy().into_owned(),
            },
        }
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn fake_portage_drives_every_read_only_public_api() {
    let manager = PortageManager::new();
    let (_directory, emerge) = fake_portage();
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(emerge);

    assert!(matches!(
        manager
            .availability(&config)
            .await
            .expect("check fake Portage availability"),
        ManagerAvailability::Available { version: Some(version) }
            if version.starts_with("Portage 3.0.72")
    ));

    let installed = manager
        .installed(&config)
        .await
        .expect("list fake Portage packages");
    assert_eq!(installed.len(), 2);
    assert_eq!(installed[0].name, "dev-lang/python:3.13");
    assert_eq!(installed[0].version, "3.13.14");
    assert_eq!(
        installed[0]
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("repo:gentoo;slot:3.13")
    );
    assert_eq!(
        manager
            .count_installed(&config)
            .await
            .expect("count fake Portage packages"),
        2
    );
    assert_eq!(
        manager
            .current_version(&config, "dev-lang/python:3.13")
            .await
            .expect("query fake Portage version"),
        "3.13.14"
    );

    let updates = manager
        .updates(&config, false)
        .await
        .expect("list fake Portage updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "dev-python/librt:0");
    assert_eq!(updates[0].available_version, "0.11.0");

    let search = manager
        .search(&config, "bash")
        .await
        .expect("search fake Portage catalog");
    assert_eq!(search.len(), 1);
    assert_eq!(search[0].name, "app-shells/bash");
    assert_eq!(search[0].version, "Not Installed");
}

#[tokio::test]
async fn empty_execution_emits_boundaries_without_running_portage() {
    let manager = PortageManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[], &sink)
        .await
        .expect("execute empty Portage group");

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
    let manager = PortageManager::new();
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
        "dev-lang/python:3.13",
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
#[ignore = "requires Gentoo Portage and a readable package database"]
async fn gentoo_container_portage_read_only_smoke() {
    let manager = PortageManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    let availability = manager
        .availability(&config)
        .await
        .expect("check Portage availability");
    assert!(matches!(
        availability,
        ManagerAvailability::Available { version: Some(version) }
            if version.starts_with("Portage ")
    ));
    let installed = manager
        .installed(&config)
        .await
        .expect("list Portage packages");
    let count = manager
        .count_installed(&config)
        .await
        .expect("count Portage packages");
    assert!(!installed.is_empty());
    assert_eq!(count, installed.len());
    assert!(installed.iter().all(|package| {
        package.manager_id == *manager.descriptor().id()
            && package.scope == PackageScope::System
            && package.name.contains('/')
            && package.name.contains(':')
    }));
    let first = installed.first().expect("at least one Portage package");
    assert_eq!(
        manager
            .current_version(&config, &first.name)
            .await
            .expect("query one Portage package"),
        first.version
    );
}
