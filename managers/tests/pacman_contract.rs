use std::sync::Mutex;

use tempfile::tempdir;
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapability, ManagerConfig,
    ManagerErrorKind, PackageAction, PackageManager, PackageTarget, ProgressEvent,
};
use updater_managers::PacmanManager;

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

#[tokio::test]
#[ignore = "requires Pacman and a readable local package database"]
async fn arch_container_pacman_read_only_smoke() {
    let manager = PacmanManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    let availability = manager
        .availability(&config)
        .await
        .expect("check Pacman availability");
    assert!(availability.is_available());

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
