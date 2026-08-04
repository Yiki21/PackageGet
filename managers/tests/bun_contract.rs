use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapability, ManagerConfig, ManagerErrorKind,
    ManagerId, PackageAction, PackageManager, PackageOrigin, PackageScope, PackageTarget, Platform,
    ProgressEvent,
};
use updater_managers::BunManager;

fn config(manager: &BunManager, executable: &Path) -> ManagerConfig {
    ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable)
}

#[cfg(unix)]
fn fake_bun(log: &Path) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Bun directory");
    let executable = directory.path().join("bun");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then printf '1.3.14\n'; exit 0; fi
if [ "$1" = "list" ] && [ "$2" = "--global" ] && [ "$3" = "--depth" ] && [ "$4" = "0" ]; then
cat <<'EOF'
/home/test/.bun/install/global node_modules (2)
├── @scope/tool@1.0.0
└── plain@2.0.0
EOF
exit 0
fi
if [ "$1" = "outdated" ] && [ "$2" = "--global" ]; then
cat <<'EOF'
bun outdated v1.3.14 (fixture)
|-------------------------------------------|
| Package     | Current | Update | Latest   |
|-------------|---------|--------|----------|
| @scope/tool | 1.0.0   | 1.1.0  | 2.0.0    |
| plain       | 2.0.0   | 2.0.0  | 2.0.0    |
|-------------------------------------------|
EOF
exit 0
fi
if [ "$1" = "add" ] || [ "$1" = "update" ] || [ "$1" = "remove" ]; then
  printf '%s\n' "$*" >> '{}'
  exit 0
fi
exit 30
"#,
        log.display()
    );
    fs::write(&executable, script).expect("write fake Bun executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Bun executable");
    (directory, executable)
}

#[cfg(windows)]
fn fake_bun(log: &Path) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("create fake Bun directory");
    let executable = directory.path().join("bun.cmd");
    let script = format!(
        r#"@echo off
if "%1"=="--version" goto version
chcp 65001 >nul
if "%1"=="list" goto list
if "%1"=="outdated" goto outdated
if "%1"=="add" goto write
if "%1"=="update" goto write
if "%1"=="remove" goto write
exit /b 30

:version
echo 1.3.14
exit /b 0

:list
echo C:\Users\test\.bun\install\global node_modules ^(2^)
echo ├── @scope/tool@1.0.0
echo └── plain@2.0.0
exit /b 0

:outdated
echo bun outdated v1.3.14 ^(fixture^)
echo ^|-------------------------------------------^|
echo ^| Package     ^| Current ^| Update ^| Latest   ^|
echo ^|-------------^|---------^|--------^|----------^|
echo ^| @scope/tool ^| 1.0.0   ^| 1.1.0  ^| 2.0.0    ^|
echo ^| plain       ^| 2.0.0   ^| 2.0.0  ^| 2.0.0    ^|
echo ^|-------------------------------------------^|
exit /b 0

:write
echo %*>>"{}"
exit /b 0
"#,
        log.display()
    );
    fs::write(&executable, script).expect("write fake Bun command file");
    (directory, executable)
}

#[cfg(unix)]
fn empty_bun(missing_manifest: bool) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create empty Bun fixture directory");
    let executable = directory.path().join("bun");
    let (list_error, outdated_error) = if missing_manifest {
        (
            "error: No package.json was found for directory '/home/test/.bun/install/global'",
            "error: failed to initialize bun install: MissingPackageJSON",
        )
    } else {
        (
            "error: Lockfile not found",
            "error: missing lockfile, nothing outdated",
        )
    };
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "list" ]; then printf '%s\n' '{}' >&2; exit 1; fi
if [ "$1" = "outdated" ]; then
  printf 'bun outdated v1.3.14 (fixture)\n'
  printf '%s\n' '{}' >&2
  exit 1
fi
exit 30
"#,
        list_error, outdated_error
    );
    fs::write(&executable, script).expect("write empty Bun fixture");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark empty Bun fixture executable");
    (directory, executable)
}

#[cfg(windows)]
fn empty_bun(missing_manifest: bool) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("create empty Bun fixture directory");
    let executable = directory.path().join("bun.cmd");
    let (list_error, outdated_error) = if missing_manifest {
        (
            "error: No package.json was found for directory C:\\Users\\test\\.bun\\install\\global",
            "error: failed to initialize bun install: MissingPackageJSON",
        )
    } else {
        (
            "error: Lockfile not found",
            "error: missing lockfile, nothing outdated",
        )
    };
    let script = format!(
        r#"@echo off
if "%1"=="list" goto list
if "%1"=="outdated" goto outdated
exit /b 30

:list
echo {} 1>&2
exit /b 1

:outdated
echo bun outdated v1.3.14 ^(fixture^)
echo {} 1>&2
exit /b 1
"#,
        list_error, outdated_error
    );
    fs::write(&executable, script).expect("write empty Bun command file");
    (directory, executable)
}

#[cfg(unix)]
fn fake_unix_bun(script: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create malformed Bun fixture directory");
    let executable = directory.path().join("bun");
    fs::write(&executable, script).expect("write malformed Bun fixture");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark malformed Bun fixture executable");
    (directory, executable)
}

#[test]
fn descriptor_advertises_only_the_stable_bun_global_contract() {
    let manager = BunManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:bun");
    assert_eq!(descriptor.display_name(), "Bun");
    assert_eq!(descriptor.authorization(), &AuthorizationHint::None);
    for platform in [Platform::Linux, Platform::Windows, Platform::MacOs] {
        assert!(descriptor.platforms().contains(platform));
    }
    for capability in [
        ManagerCapability::Installed,
        ManagerCapability::Updates,
        ManagerCapability::Install,
        ManagerCapability::Update,
        ManagerCapability::Uninstall,
    ] {
        assert!(descriptor.capabilities().contains(capability));
    }
    assert!(
        !descriptor
            .capabilities()
            .contains(ManagerCapability::Search)
    );
}

#[tokio::test]
async fn native_contract_preserves_scoped_inventory_latest_updates_and_write_arguments() {
    let manager = BunManager::new();
    let workspace = tempdir().expect("create Bun contract workspace");
    let log = workspace.path().join("bun.log");
    let (_directory, executable) = fake_bun(&log);
    let config = config(&manager, &executable);

    let availability = manager
        .availability(&config)
        .await
        .expect("Bun availability");
    assert!(
        matches!(
            &availability,
            ManagerAvailability::Available { version: Some(version) } if version == "1.3.14"
        ),
        "unexpected Bun availability: {availability:?}"
    );
    let installed = manager.installed(&config).await.expect("Bun inventory");
    assert_eq!(installed.len(), 2);
    assert_eq!(installed[0].name, "@scope/tool");
    assert_eq!(installed[0].version, "1.0.0");
    assert_eq!(installed[0].scope, PackageScope::User);
    assert_eq!(
        installed[0].origin,
        Some(PackageOrigin::new("Bun global").with_reference("package:@scope/tool"))
    );
    assert_eq!(
        manager.count_installed(&config).await.expect("Bun count"),
        2
    );

    let updates = manager.updates(&config, false).await.expect("Bun updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "@scope/tool");
    assert_eq!(updates[0].target.version.as_deref(), Some("2.0.0"));
    assert_eq!(updates[0].current_version, "1.0.0");
    assert_eq!(updates[0].available_version, "2.0.0");
    assert_eq!(
        manager
            .search(&config, "tool")
            .await
            .expect_err("Bun search remains unadvertised")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    let mut install = PackageTarget::new(manager.descriptor().id().clone(), "@scope/tool");
    install.version = Some("3.0.0".to_owned());
    install.scope = PackageScope::User;
    install.origin = Some(package_origin("@scope/tool"));
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);
    manager
        .execute(&config, PackageAction::Install, &[install], &sink)
        .await
        .expect("install Bun package");
    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&updates[0].target),
            &sink,
        )
        .await
        .expect("update Bun package");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            &[installed[0].target()],
            &sink,
        )
        .await
        .expect("remove Bun package");

    assert_eq!(
        fs::read_to_string(log)
            .expect("read Bun argv log")
            .lines()
            .collect::<Vec<_>>(),
        [
            "add --global --no-progress --no-summary @scope/tool@3.0.0",
            "update --global --no-progress --no-summary @scope/tool@2.0.0",
            "remove --global --no-progress --no-summary @scope/tool",
        ]
    );
    assert!(matches!(
        events.lock().expect("progress lock").last(),
        Some(ProgressEvent::Finished {
            completed: 1,
            total: 1
        })
    ));
}

#[tokio::test]
async fn missing_manifest_and_lockfile_states_are_empty_global_inventories() {
    let manager = BunManager::new();
    for missing_manifest in [true, false] {
        let (_directory, executable) = empty_bun(missing_manifest);
        let config = config(&manager, &executable);
        assert!(
            manager
                .installed(&config)
                .await
                .expect("empty Bun inventory")
                .is_empty()
        );
        assert!(
            manager
                .updates(&config, false)
                .await
                .expect("empty Bun updates")
                .is_empty()
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn unrelated_nonzero_global_failure_preserves_typed_error() {
    let manager = BunManager::new();
    let (_directory, executable) =
        fake_unix_bun("#!/bin/sh\nprintf 'error: network is unreachable\n' >&2\nexit 1\n");
    let error = manager
        .installed(&config(&manager, &executable))
        .await
        .expect_err("preserve unrelated Bun failure");
    assert_eq!(error.kind(), ManagerErrorKind::Network);
}

#[cfg(unix)]
#[tokio::test]
async fn duplicate_inventory_identity_is_a_protocol_error() {
    let manager = BunManager::new();
    let (_directory, executable) = fake_unix_bun(
        "#!/bin/sh\nprintf '/tmp/global node_modules (2)\\n├── tool@1.0.0\\n└── tool@1.0.0\\n'\n",
    );

    assert_eq!(
        manager
            .installed(&config(&manager, &executable))
            .await
            .expect_err("reject duplicate Bun package")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_outdated_version_is_a_protocol_error() {
    let manager = BunManager::new();
    let (_directory, executable) = fake_unix_bun(
        r#"#!/bin/sh
printf 'bun outdated v1.3.14 (fixture)\n'
printf '|--------------------------------------|\n'
printf '| Package | Current | Update | Latest |\n'
printf '|---------|---------|--------|--------|\n'
printf '| tool    | 1.0.0   | 1.1.0  | latest |\n'
printf '|--------------------------------------|\n'
"#,
    );

    assert_eq!(
        manager
            .updates(&config(&manager, &executable), false)
            .await
            .expect_err("reject malformed Bun update version")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[tokio::test]
async fn forged_scope_origin_and_package_spec_are_rejected_before_writes() {
    let manager = BunManager::new();
    let workspace = tempdir().expect("create Bun target validation workspace");
    let log = workspace.path().join("bun.log");
    let (_directory, executable) = fake_bun(&log);
    let config = config(&manager, &executable);

    let mut wrong_scope = PackageTarget::new(manager.descriptor().id().clone(), "tool");
    wrong_scope.scope = PackageScope::System;
    wrong_scope.origin = Some(package_origin("tool"));
    assert_eq!(
        manager
            .execute(&config, PackageAction::Install, &[wrong_scope], &|_| {})
            .await
            .expect_err("reject Bun system scope")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    let mut wrong_origin = PackageTarget::new(manager.descriptor().id().clone(), "tool");
    wrong_origin.scope = PackageScope::User;
    wrong_origin.origin = Some(package_origin("other"));
    assert_eq!(
        manager
            .execute(&config, PackageAction::Install, &[wrong_origin], &|_| {})
            .await
            .expect_err("reject forged Bun origin")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let malformed = PackageTarget::new(manager.descriptor().id().clone(), "file:../tool");
    assert_eq!(
        manager
            .execute(&config, PackageAction::Install, &[malformed], &|_| {})
            .await
            .expect_err("reject Bun package spec injection")
            .kind(),
        ManagerErrorKind::Protocol
    );
    assert!(!log.exists());
}

#[tokio::test]
#[ignore = "requires host Bun and performs read-only global list/outdated probes"]
async fn host_bun_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = BunManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    let _ = manager.updates(&config, false).await?;
    Ok(())
}

fn package_origin(name: &str) -> PackageOrigin {
    PackageOrigin::new("Bun global").with_reference(format!("package:{name}"))
}

#[tokio::test]
async fn wrong_manager_identity_is_rejected() {
    let manager = BunManager::new();
    let target = PackageTarget::new(
        ManagerId::parse("builtin:npm").expect("valid foreign manager ID"),
        "tool",
    );
    let error = manager
        .execute(
            &ManagerConfig::new(manager.descriptor().id().clone()),
            PackageAction::Install,
            &[target],
            &|_| {},
        )
        .await
        .expect_err("reject foreign Bun target");
    assert_eq!(error.kind(), ManagerErrorKind::Protocol);
}
