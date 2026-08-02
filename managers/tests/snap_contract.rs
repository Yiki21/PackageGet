#![cfg(target_os = "linux")]

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerConfig, ManagerErrorKind, PackageAction,
    PackageManager, PackageOrigin, PackageScope, PackageTarget, Platform,
};
use updater_managers::SnapManager;

fn config(manager: &SnapManager, executable: &PathBuf) -> ManagerConfig {
    ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable)
}

fn fake_snap(log: &std::path::Path) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("create fake snap directory");
    let executable = directory.path().join("snap");
    let script = format!(
        r#"#!/bin/sh
printf '%s' "$1" >> '{}'
shift
for arg in "$@"; do printf '|%s' "$arg" >> '{}'; done
printf '\n' >> '{}'

case "$1" in
esac

if [ "$#" -eq 0 ]; then
  case "$(tail -n 1 '{}')" in
    version) printf 'snap 2.76\nsnapd 2.76\nseries 16\n'; exit 0 ;;
    list) printf '%s\n' 'Name Version Rev Tracking Publisher Notes' 'firefox 128.0 5000 latest/stable mozilla✓ held,classic' 'hello-world 6.4 29 latest/edge canonical✓ -'; exit 0 ;;
  esac
fi

command="$(tail -n 1 '{}')"
case "$command" in
  'refresh|--list') printf '%s\n' 'Name Version Rev Size Publisher Notes' 'firefox 129.0 5001 120MB mozilla✓ classic'; exit 0 ;;
  'find|--narrow|editor') printf '%s\n' 'Name Version Publisher Notes Summary' 'code 1.99 vscode✓ classic Code editing redefined' 'strict-app 2.0 example✓ - Strict confined application'; exit 0 ;;
  install*|refresh*|remove*) exit 0 ;;
esac
exit 30
"#,
        log.display(),
        log.display(),
        log.display(),
        log.display(),
        log.display()
    );
    fs::write(&executable, script).expect("write fake snap executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake snap executable");
    (directory, executable)
}

fn typed_target(manager: &SnapManager, name: &str, reference: &str) -> PackageTarget {
    let mut target = PackageTarget::new(manager.descriptor().id().clone(), name);
    target.scope = PackageScope::System;
    target.origin = Some(PackageOrigin::new("Snap").with_reference(reference));
    target
}

#[test]
fn snap_descriptor_freezes_linux_and_snapd_authorization_contract() {
    let manager = SnapManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:snap");
    assert_eq!(descriptor.display_name(), "Snap");
    assert!(descriptor.platforms().contains(Platform::Linux));
    assert_eq!(
        descriptor.authorization(),
        &AuthorizationHint::RequiresElevation {
            message: Some("Snap writes are authorized by snapd through Polkit.".to_owned())
        }
    );
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
async fn native_contract_preserves_state_search_and_direct_snapd_writes() {
    let manager = SnapManager::new();
    let root = tempdir().expect("create Snap fixture root");
    let log = root.path().join("snap.log");
    let (_directory, executable) = fake_snap(&log);
    let config = config(&manager, &executable);

    assert!(
        manager
            .availability(&config)
            .await
            .expect("Snap availability")
            .is_available()
    );
    let installed = manager.installed(&config).await.expect("Snap inventory");
    assert_eq!(installed.len(), 2);
    assert_eq!(installed[0].name, "firefox");
    assert_eq!(installed[0].version, "128.0");
    assert_eq!(installed[0].scope, PackageScope::System);
    assert_eq!(
        installed[0].origin,
        Some(PackageOrigin::new("Snap").with_reference(
            "snap:firefox;channel:latest/stable;confinement:classic;refresh:held;notes:held,classic"
        ))
    );
    assert!(
        installed[0]
            .description
            .as_deref()
            .expect("Snap description")
            .contains("refresh: held")
    );

    let updates = manager.updates(&config, false).await.expect("Snap updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target, installed[0].target());
    assert_eq!(updates[0].current_version, "128.0");
    assert_eq!(updates[0].available_version, "129.0");

    let search = manager
        .search(&config, "editor")
        .await
        .expect("Snap search");
    assert_eq!(search.len(), 2);
    assert_eq!(search[0].name, "code");
    assert_eq!(
        search[0].description.as_deref(),
        Some("Code editing redefined (Publisher: vscode✓)")
    );
    assert_eq!(
        search[0].origin,
        Some(PackageOrigin::new("Snap").with_reference(
            "snap:code;channel:latest/stable;confinement:classic;refresh:store;notes:classic"
        ))
    );

    let sink = |_| {};
    manager
        .execute(
            &config,
            PackageAction::Install,
            &[search[0].target()],
            &sink,
        )
        .await
        .expect("install classic Snap");
    let devmode = typed_target(
        &manager,
        "edge-app",
        "snap:edge-app;channel:latest/edge;confinement:devmode;refresh:store;notes:devmode",
    );
    manager
        .execute(&config, PackageAction::Install, &[devmode], &sink)
        .await
        .expect("install devmode edge Snap");
    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&updates[0].target),
            &sink,
        )
        .await
        .expect("refresh Snap");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            &[installed[1].target()],
            &sink,
        )
        .await
        .expect("remove Snap");

    assert_eq!(
        fs::read_to_string(log)
            .expect("read Snap argv log")
            .lines()
            .collect::<Vec<_>>(),
        [
            "version",
            "list",
            "list",
            "refresh|--list",
            "find|--narrow|editor",
            "install|code|--classic",
            "install|edge-app|--channel|latest/edge|--devmode",
            "refresh|firefox",
            "remove|hello-world",
        ]
    );
}

#[tokio::test]
async fn targets_reject_wrong_scope_origin_identity_and_revision_pins() {
    let manager = SnapManager::new();
    let root = tempdir().expect("create Snap fixture root");
    let log = root.path().join("snap.log");
    let (_directory, executable) = fake_snap(&log);
    let config = config(&manager, &executable);
    let sink = |_| {};
    let reference =
        "snap:code;channel:latest/stable;confinement:classic;refresh:store;notes:classic";

    let mut target = typed_target(&manager, "code", reference);
    target.scope = PackageScope::User;
    assert_eq!(
        manager
            .execute(&config, PackageAction::Install, &[target.clone()], &sink)
            .await
            .expect_err("user scope must be rejected")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    target.scope = PackageScope::System;
    target.origin = Some(PackageOrigin::new("Snap Store").with_reference(reference));
    assert_eq!(
        manager
            .execute(&config, PackageAction::Install, &[target.clone()], &sink)
            .await
            .expect_err("wrong origin must be rejected")
            .kind(),
        ManagerErrorKind::Protocol
    );

    target.origin = Some(PackageOrigin::new("Snap").with_reference(reference));
    target.name = "other".to_owned();
    assert_eq!(
        manager
            .execute(&config, PackageAction::Update, &[target.clone()], &sink)
            .await
            .expect_err("mismatched identity must be rejected")
            .kind(),
        ManagerErrorKind::Protocol
    );

    target.name = "code".to_owned();
    target.version = Some("123".to_owned());
    assert_eq!(
        manager
            .execute(&config, PackageAction::Uninstall, &[target], &sink)
            .await
            .expect_err("revision pin must be rejected")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    let local = typed_target(
        &manager,
        "local-app",
        "snap:local-app;channel:-;confinement:strict;refresh:automatic;notes:dangerous",
    );
    assert_eq!(
        manager
            .execute(&config, PackageAction::Install, &[local], &sink)
            .await
            .expect_err("local origin install must be rejected")
            .kind(),
        ManagerErrorKind::Unsupported
    );
}

#[tokio::test]
#[ignore = "requires host snapd and performs read-only Snap probes"]
async fn host_snap_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = SnapManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    let _ = manager.updates(&config, false).await?;
    Ok(())
}
