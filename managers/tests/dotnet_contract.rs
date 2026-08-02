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
use updater_managers::DotnetToolManager;

fn config(manager: &DotnetToolManager, executable: &PathBuf) -> ManagerConfig {
    ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable)
}

#[cfg(unix)]
fn fake_dotnet(log: &std::path::Path) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake dotnet directory");
    let executable = directory.path().join("dotnet");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then printf '%s|%s|%s|%s|%s|%s\n' "$1" "$2" "$3" "$4" "$5" "$6" >> '{}'; printf '10.0.302\n'; exit 0; fi
if [ "$1" = "tool" ] && [ "$2" = "list" ]; then
  printf '%s|%s|%s|%s|%s|%s\n' "$1" "$2" "$3" "$4" "$5" "$6" >> '{}'
  printf '%s\n' '{{"version":1,"data":[{{"packageId":"example.tool","version":"1.0.0","commands":["example","example-alias"]}}]}}'
  exit 0
fi
if [ "$1" = "package" ] && [ "$2" = "search" ]; then
  printf '%s|%s|%s|%s|%s|%s\n' "$1" "$2" "$3" "$4" "$5" "$6" >> '{}'
  printf '%s\n' '{{"version":2,"problems":[],"searchResult":[{{"sourceName":"private","packages":[{{"id":"Example.Tool","version":"1.5.0"}}]}},{{"sourceName":"nuget.org","packages":[{{"id":"example.tool","version":"2.0.0"}}]}}]}}'
  exit 0
fi
if [ "$1" = "tool" ] && [ "$2" = "search" ]; then
  printf '%s|%s|%s|%s|%s|%s\n' "$1" "$2" "$3" "$4" "$5" "$6" >> '{}'
  printf '%s\n' '----------------' 'example.tool' 'Latest Version: 2.0.0' 'Authors: Example' 'Downloads: 20' 'Verified: False' 'Description: Example global tool' 'Versions:' '  2.0.0 Downloads: 20'
  exit 0
fi
if [ "$1" = "tool" ]; then printf '%s|%s|%s|%s|%s|%s\n' "$1" "$2" "$3" "$4" "$5" "$6" >> '{}'; exit 0; fi
exit 30
"#,
        log.display(),
        log.display(),
        log.display(),
        log.display(),
        log.display()
    );
    fs::write(&executable, script).expect("write fake dotnet executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake dotnet executable");
    (directory, executable)
}

#[cfg(windows)]
fn fake_dotnet(log: &std::path::Path) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("create fake dotnet directory");
    let executable = directory.path().join("dotnet.cmd");
    let script = format!(
        r#"@echo off
if "%1"=="--version" goto version
if "%1"=="tool" if "%2"=="list" goto list
if "%1"=="package" if "%2"=="search" goto metadata
if "%1"=="tool" if "%2"=="search" goto search
if "%1"=="tool" goto write
exit /b 30

:version
echo %~1^|%~2^|%~3^|%~4^|%~5^|%~6>>"{}"
echo 10.0.302
exit /b 0

:list
echo %~1^|%~2^|%~3^|%~4^|%~5^|%~6>>"{}"
echo {{"version":1,"data":[{{"packageId":"example.tool","version":"1.0.0","commands":["example","example-alias"]}}]}}
exit /b 0

:metadata
echo %~1^|%~2^|%~3^|%~4^|%~5^|%~6>>"{}"
echo {{"version":2,"problems":[],"searchResult":[{{"sourceName":"private","packages":[{{"id":"Example.Tool","version":"1.5.0"}}]}},{{"sourceName":"nuget.org","packages":[{{"id":"example.tool","version":"2.0.0"}}]}}]}}
exit /b 0

:search
echo %~1^|%~2^|%~3^|%~4^|%~5^|%~6>>"{}"
echo ----------------
echo example.tool
echo Latest Version: 2.0.0
echo Authors: Example
echo Downloads: 20
echo Verified: False
echo Description: Example global tool
echo Versions:
echo   2.0.0 Downloads: 20
exit /b 0

:write
echo %~1^|%~2^|%~3^|%~4^|%~5^|%~6>>"{}"
exit /b 0
"#,
        log.display(),
        log.display(),
        log.display(),
        log.display(),
        log.display()
    );
    fs::write(&executable, script).expect("write fake dotnet command file");
    (directory, executable)
}

#[test]
fn dotnet_descriptor_advertises_current_user_global_contract() {
    let manager = DotnetToolManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:dotnet-tool");
    assert_eq!(descriptor.display_name(), ".NET global tools");
    assert_eq!(descriptor.authorization(), &AuthorizationHint::None);
    for platform in [Platform::Linux, Platform::Windows, Platform::MacOs] {
        assert!(descriptor.platforms().contains(platform));
    }
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
async fn native_contract_preserves_global_identity_updates_search_and_writes() {
    let manager = DotnetToolManager::new();
    let root = tempdir().expect("create dotnet fixture root");
    let log = root.path().join("dotnet.log");
    let (_directory, executable) = fake_dotnet(&log);
    let config = config(&manager, &executable);

    assert!(
        manager
            .availability(&config)
            .await
            .expect("dotnet availability")
            .is_available()
    );
    let installed = manager.installed(&config).await.expect("dotnet inventory");
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].name, "example.tool");
    assert_eq!(installed[0].version, "1.0.0");
    assert_eq!(installed[0].scope, PackageScope::User);
    assert_eq!(
        installed[0].origin,
        Some(PackageOrigin::new(".NET global tool").with_reference("global:example.tool"))
    );

    let updates = manager
        .updates(&config, false)
        .await
        .expect("dotnet updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target, installed[0].target());
    assert_eq!(updates[0].available_version, "2.0.0");

    let search = manager
        .search(&config, "example")
        .await
        .expect("dotnet search");
    assert_eq!(search.len(), 1);
    assert_eq!(search[0].name, "example.tool");
    assert_eq!(search[0].version, "2.0.0");
    assert_eq!(
        search[0].description.as_deref(),
        Some("Example global tool")
    );

    let mut install = PackageTarget::new(manager.descriptor().id().clone(), "example.tool");
    install.scope = PackageScope::User;
    install.version = Some("2.1.0".to_owned());
    install.origin =
        Some(PackageOrigin::new(".NET global tool").with_reference("global:example.tool"));
    #[cfg(unix)]
    let events = Mutex::new(Vec::new());
    #[cfg(unix)]
    let sink = |event| events.lock().expect("progress lock").push(event);
    #[cfg(windows)]
    let sink = |_| {};

    manager
        .execute(&config, PackageAction::Install, &[install], &sink)
        .await
        .expect("install dotnet tool");
    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&updates[0].target),
            &sink,
        )
        .await
        .expect("update dotnet tool");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            &[installed[0].target()],
            &sink,
        )
        .await
        .expect("uninstall dotnet tool");
    assert_eq!(
        fs::read_to_string(log)
            .expect("read dotnet argv log")
            .lines()
            .collect::<Vec<_>>(),
        [
            "--version|||||",
            "tool|list|--global|--format|json|",
            "tool|list|--global|--format|json|",
            "package|search|example.tool|--exact-match|--format|json",
            "tool|search|example|--detail|--take|50",
            "tool|install|example.tool|--global|--version|2.1.0",
            "tool|update|example.tool|--global||",
            "tool|uninstall|example.tool|--global||",
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

#[tokio::test]
async fn targets_reject_wrong_scope_origin_and_pinned_non_install_actions() {
    let manager = DotnetToolManager::new();
    let root = tempdir().expect("create dotnet fixture root");
    let log = root.path().join("dotnet.log");
    let (_directory, executable) = fake_dotnet(&log);
    let config = config(&manager, &executable);
    let sink = |_| {};

    let mut target = PackageTarget::new(manager.descriptor().id().clone(), "example.tool");
    target.scope = PackageScope::Project;
    assert_eq!(
        manager
            .execute(&config, PackageAction::Install, &[target.clone()], &sink)
            .await
            .expect_err("project scope must be rejected")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    target.scope = PackageScope::User;
    target.origin = Some(PackageOrigin::new("NuGet").with_reference("global:example.tool"));
    assert_eq!(
        manager
            .execute(&config, PackageAction::Update, &[target.clone()], &sink)
            .await
            .expect_err("wrong origin must be rejected")
            .kind(),
        ManagerErrorKind::Protocol
    );

    target.origin =
        Some(PackageOrigin::new(".NET global tool").with_reference("global:example.tool"));
    target.version = Some("2.0.0".to_owned());
    assert_eq!(
        manager
            .execute(&config, PackageAction::Uninstall, &[target], &sink)
            .await
            .expect_err("pinned uninstall must be rejected")
            .kind(),
        ManagerErrorKind::Unsupported
    );
}

#[tokio::test]
#[ignore = "requires host dotnet SDK and performs read-only global tool probes"]
async fn host_dotnet_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = DotnetToolManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    Ok(())
}
