use std::{fs, path::PathBuf, sync::Mutex};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapability, ManagerConfig,
    ManagerErrorKind, PackageAction, PackageManager, PackageTarget, ProgressEvent,
};
use updater_managers::PacmanManager;

#[cfg(unix)]
fn fake_pacman() -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Pacman directory");
    let executable = directory.path().join("pacman");
    fs::write(
        &executable,
        r#"#!/bin/sh
case "$1" in
  --version)
    printf ' .--.\n/ _.- Pacman v7.0.0 - libalpm v15.0.0\n'
    ;;
  -Q)
    if [ "$#" -eq 2 ]; then
      case "$2" in
        bash) printf 'bash 5.2.037-1\n' ;;
        curl) printf 'curl 8.15.0-1\n' ;;
        *) exit 1 ;;
      esac
    else
      printf 'bash 5.2.037-1\ncurl 8.15.0-1\n'
    fi
    ;;
  -Qq)
    printf 'bash\ncurl\n'
    ;;
  -Qu)
    printf 'curl 8.15.0-1 -> 8.16.0-1\n'
    ;;
  -Ss)
    printf 'core/bash 5.2.037-1\n    The GNU Bourne Again shell\nextra/fzf 0.65.0-1\n    Command-line fuzzy finder\n'
    ;;
  *)
    exit 2
    ;;
esac
"#,
    )
    .expect("write fake Pacman executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Pacman executable");
    (directory, executable)
}

#[test]
fn pacman_descriptor_exposes_the_stable_public_contract() {
    let manager = PacmanManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:pacman");
    assert_eq!(descriptor.display_name(), "Pacman");
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
    let manager = PacmanManager::new();
    let directory = tempdir().expect("create temporary directory");
    let missing = directory.path().join("missing-pacman");
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
async fn empty_execution_emits_boundaries_without_running_pacman() {
    let manager = PacmanManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[], &sink)
        .await
        .expect("execute empty Pacman group");

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
    let manager = PacmanManager::new();
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
async fn public_read_apis_preserve_pacman_output_contracts() {
    let manager = PacmanManager::new();
    let (_directory, executable) = fake_pacman();
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check fake Pacman availability"),
        ManagerAvailability::Available {
            version: Some("/ _.- Pacman v7.0.0 - libalpm v15.0.0".to_owned()),
        }
    );
    assert_eq!(
        manager
            .current_version(&config, "bash")
            .await
            .expect("query fake package version"),
        "5.2.037-1"
    );

    let installed = manager
        .installed(&config)
        .await
        .expect("list fake installed packages");
    assert_eq!(installed.len(), 2);
    assert_eq!(installed[0].name, "bash");
    assert_eq!(installed[0].version, "5.2.037-1");
    assert_eq!(
        manager
            .count_installed(&config)
            .await
            .expect("count fake installed packages"),
        installed.len()
    );

    let updates = manager
        .updates(&config, false)
        .await
        .expect("list fake Pacman updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "curl");
    assert_eq!(updates[0].current_version, "8.15.0-1");
    assert_eq!(updates[0].available_version, "8.16.0-1");

    let search = manager
        .search(&config, "shell")
        .await
        .expect("search fake Pacman repositories");
    assert_eq!(search.len(), 2);
    assert_eq!(search[0].name, "bash");
    assert_eq!(search[0].version, "5.2.037-1");
    assert_eq!(
        search[0].description.as_deref(),
        Some("The GNU Bourne Again shell")
    );
    assert_eq!(search[1].name, "fzf");
    assert_eq!(search[1].version, "Not Installed");
}

#[tokio::test]
#[ignore = "requires Pacman and a readable local package database"]
async fn arch_container_pacman_read_only_smoke() {
    let manager = PacmanManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    let availability = manager
        .availability(&config)
        .await
        .expect("check Pacman availability");
    assert!(matches!(
        availability,
        ManagerAvailability::Available { version: Some(version) }
            if version.contains("Pacman v")
    ));

    let installed = manager
        .installed(&config)
        .await
        .expect("list installed Pacman packages");
    let count = manager
        .count_installed(&config)
        .await
        .expect("count installed Pacman packages");

    assert!(!installed.is_empty());
    assert_eq!(count, installed.len());
    assert!(
        installed
            .iter()
            .all(|package| package.manager_id == *manager.descriptor().id())
    );

    let first = installed.first().expect("at least one installed package");
    assert_eq!(
        manager
            .current_version(&config, &first.name)
            .await
            .expect("query one installed Pacman package"),
        first.version
    );
}

#[tokio::test]
#[ignore = "machine-specific host availability smoke test"]
async fn local_pacman_availability_is_structured() {
    let manager = PacmanManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    let availability = manager
        .availability(&config)
        .await
        .expect("check local Pacman availability");

    match availability {
        ManagerAvailability::Available { .. } => {}
        ManagerAvailability::Unavailable {
            reason: AvailabilityReason::CommandMissing { command },
        } => assert_eq!(command, "pacman"),
        other => panic!("unexpected local Pacman availability: {other:?}"),
    }
}
