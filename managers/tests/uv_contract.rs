use std::{fs, path::PathBuf};

#[cfg(unix)]
use std::sync::Mutex;

use tempfile::{TempDir, tempdir};
#[cfg(unix)]
use updater_manager_api::ProgressEvent;
use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerConfig, ManagerErrorKind, PackageAction,
    PackageManager, PackageOrigin, PackageScope, PackageTarget, Platform,
};
use updater_managers::UvManager;

fn config(manager: &UvManager, executable: &PathBuf) -> ManagerConfig {
    ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable)
}

#[cfg(unix)]
fn fake_uv(root: &std::path::Path, log: &std::path::Path) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake uv directory");
    let executable = directory.path().join("uv");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then printf 'uv 0.11.32\n'; exit 0; fi
if [ "$1" = "tool" ] && [ "$2" = "dir" ]; then printf '{}\n'; exit 0; fi
if [ "$1" = "tool" ] && [ "$2" = "list" ]; then
  if [ "$3" = "--outdated" ]; then
    printf 'example-tool v1.0.0 [latest: 2.0.0] ({}/example-tool)\n'
  else
    printf 'example-tool v1.0.0 ({}/example-tool)\n- example-tool ({}/bin/example-tool)\n'
  fi
  exit 0
fi
if [ "$1" = "tool" ]; then printf '%s|%s|%s\n' "$1" "$2" "$3" >> '{}'; exit 0; fi
exit 30
"#,
        root.display(),
        root.display(),
        root.display(),
        root.display(),
        log.display()
    );
    fs::write(&executable, script).expect("write fake uv executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake uv executable");
    (directory, executable)
}

#[cfg(windows)]
fn fake_uv(root: &std::path::Path, log: &std::path::Path) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("create fake uv directory");
    let executable = directory.path().join("uv.cmd");
    let script = format!(
        r#"@echo off
if "%1"=="--version" goto version
if "%1"=="tool" if "%2"=="dir" goto dir
if "%1"=="tool" if "%2"=="list" goto list
if "%1"=="tool" goto write
exit /b 30

:version
echo uv 0.11.32
exit /b 0

:dir
echo {}
exit /b 0

:list
if "%3"=="--outdated" goto outdated
echo example-tool v1.0.0 ^({}\example-tool^)
echo - example-tool ^({}\bin\example-tool.exe^)
exit /b 0

:outdated
echo example-tool v1.0.0 [latest: 2.0.0] ^({}\example-tool^)
exit /b 0

:write
echo %~1^|%~2^|%~3>>"{}"
exit /b 0
"#,
        root.display(),
        root.display(),
        root.display(),
        root.display(),
        log.display()
    );
    fs::write(&executable, script).expect("write fake uv command file");
    (directory, executable)
}

#[test]
fn uv_descriptor_advertises_only_implemented_capabilities() {
    let manager = UvManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:uv");
    assert_eq!(descriptor.display_name(), "uv tool");
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
async fn native_contract_preserves_tool_identity_paths_updates_and_writes() {
    let manager = UvManager::new();
    let root = tempdir().expect("create uv tool root");
    let tool = root.path().join("example-tool");
    fs::create_dir(&tool).expect("create uv tool environment");
    fs::write(tool.join("tool.py"), b"12345").expect("write uv tool file");
    let log = root.path().join("uv.log");
    let (_directory, executable) = fake_uv(root.path(), &log);
    let config = config(&manager, &executable);

    assert!(
        manager
            .availability(&config)
            .await
            .expect("uv availability")
            .is_available()
    );
    let installed = manager.installed(&config).await.expect("uv inventory");
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].name, "example-tool");
    assert_eq!(installed[0].version, "1.0.0");
    assert_eq!(installed[0].scope, PackageScope::User);
    assert_eq!(installed[0].size, Some(5));
    assert_eq!(
        installed[0].origin,
        Some(PackageOrigin::new("uv tool").with_reference("tool:example-tool"))
    );

    let updates = manager.updates(&config, false).await.expect("uv updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target, installed[0].target());
    assert_eq!(updates[0].available_version, "2.0.0");
    assert_eq!(
        manager
            .search(&config, "example-tool")
            .await
            .expect_err("uv search remains unadvertised")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    let mut install = PackageTarget::new(manager.descriptor().id().clone(), "example-tool");
    install.scope = PackageScope::User;
    install.version = Some("2.1.0".to_owned());
    install.origin = Some(PackageOrigin::new("uv tool").with_reference("tool:example-tool"));
    #[cfg(unix)]
    let events = Mutex::new(Vec::new());
    #[cfg(unix)]
    let sink = |event| events.lock().expect("progress lock").push(event);
    #[cfg(windows)]
    let sink = |_| {};

    manager
        .execute(&config, PackageAction::Install, &[install], &sink)
        .await
        .expect("install uv tool");
    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&updates[0].target),
            &sink,
        )
        .await
        .expect("upgrade uv tool");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            &[installed[0].target()],
            &sink,
        )
        .await
        .expect("uninstall uv tool");
    assert_eq!(
        fs::read_to_string(log)
            .expect("read uv argv log")
            .lines()
            .collect::<Vec<_>>(),
        [
            "tool|install|example-tool==2.1.0",
            "tool|upgrade|example-tool",
            "tool|uninstall|example-tool",
        ]
    );
    #[cfg(unix)]
    assert!(matches!(
        events.lock().expect("progress lock").last(),
        Some(ProgressEvent::Finished {
            completed: 1,
            total: 1
        })
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_and_escaping_tool_paths_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let manager = UvManager::new();
    let root = tempdir().expect("create uv tool root");
    let outside = tempdir().expect("create outside directory");
    symlink(outside.path(), root.path().join("escape")).expect("create escaping tool link");
    let directory = tempdir().expect("create fake uv directory");
    let executable = directory.path().join("uv");
    let script = format!(
        "#!/bin/sh\nif [ \"$2\" = \"dir\" ]; then printf '{}\\n'; else printf 'escape v1.0.0 ({}/escape)\\n'; fi\n",
        root.path().display(),
        root.path().display()
    );
    fs::write(&executable, script).expect("write escaping uv fixture");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark escaping uv fixture executable");
    assert_eq!(
        manager
            .installed(&config(&manager, &executable))
            .await
            .expect_err("reject symlink tool environment")
            .kind(),
        ManagerErrorKind::Unsupported
    );
}

#[tokio::test]
#[ignore = "requires host uv and performs read-only tool directory/list probes"]
async fn host_uv_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = UvManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    Ok(())
}
