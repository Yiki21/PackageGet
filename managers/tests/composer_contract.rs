use std::{fs, path::PathBuf};

use serde_json::Value;
use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerConfig, ManagerErrorKind, PackageAction,
    PackageManager, PackageScope, Platform,
};
use updater_managers::ComposerGlobalManager;

static COMPOSER_CONTRACT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn config(manager: &ComposerGlobalManager, executable: &PathBuf) -> ManagerConfig {
    ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable)
}

#[cfg(unix)]
fn fake_composer(home: &std::path::Path, log: &std::path::Path) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Composer directory");
    let executable = directory.path().join("composer");
    let script = format!(
        r#"#!/bin/sh
printf '%s|%s\n' "$COMPOSER_HOME" "$*" >> '{}'
if [ "$1" = "--version" ]; then printf 'Composer version 2.10.0 2026-05-28 11:22:08\n'; exit 0; fi
if [ "$1" = "global" ] && [ "$2" = "config" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$COMPOSER_HOME" != '{}' ]; then exit 41; fi
if [ "$1" = "global" ] && [ "$2" = "show" ]; then
  printf '%s\n' '{{"installed":[{{"name":"dev/tool","direct-dependency":true,"version":"1.0.0","description":"Dev helper"}},{{"name":"vendor/tool","direct-dependency":true,"homepage":"https://example.test/tool","source":"https://source.example.test/tool","version":"1.0.0","description":"Example tool"}}]}}'
  exit 0
fi
if [ "$1" = "global" ] && [ "$2" = "outdated" ]; then
  printf '%s\n' '{{"installed":[{{"name":"dev/tool","direct-dependency":true,"version":"1.0.0","latest":"1.1.0","latest-status":"semver-safe-update"}},{{"name":"vendor/tool","direct-dependency":true,"version":"1.0.0","latest":"1.2.0","latest-status":"semver-safe-update","description":"Example tool"}}]}}'
  exit 0
fi
if [ "$1" = "global" ] && [ "$2" = "search" ]; then
  printf '%s\n' '[{{"name":"composer","description":"platform"}},{{"name":"vendor/new-tool","description":"New tool","url":"https://example.test/new-tool","repository":"https://source.example.test/new-tool"}}]'
  exit 0
fi
if [ "$1" = "global" ] && {{ [ "$2" = "require" ] || [ "$2" = "update" ] || [ "$2" = "remove" ]; }}; then exit 0; fi
exit 42
"#,
        log.display(),
        home.display(),
        home.display()
    );
    fs::write(&executable, script).expect("write fake Composer executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Composer executable");
    (directory, executable)
}

#[cfg(windows)]
fn fake_composer(home: &std::path::Path, log: &std::path::Path) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("create fake Composer directory");
    let executable = directory.path().join("composer.cmd");
    let script = format!(
        r#"@echo off
echo %COMPOSER_HOME%^|%*>>"{}"
if "%1"=="--version" goto version
if "%1"=="global" if "%2"=="config" goto home
if not "%COMPOSER_HOME%"=="{}" exit /b 41
if "%1"=="global" if "%2"=="show" goto show
if "%1"=="global" if "%2"=="outdated" goto outdated
if "%1"=="global" if "%2"=="search" goto search
if "%1"=="global" if "%2"=="require" goto write
if "%1"=="global" if "%2"=="update" goto write
if "%1"=="global" if "%2"=="remove" goto write
exit /b 42

:version
echo Composer version 2.10.0 2026-05-28 11:22:08
exit /b 0

:home
echo {}
exit /b 0

:show
echo {{"installed":[{{"name":"dev/tool","direct-dependency":true,"version":"1.0.0","description":"Dev helper"}},{{"name":"vendor/tool","direct-dependency":true,"homepage":"https://example.test/tool","source":"https://source.example.test/tool","version":"1.0.0","description":"Example tool"}}]}}
exit /b 0

:outdated
echo {{"installed":[{{"name":"dev/tool","direct-dependency":true,"version":"1.0.0","latest":"1.1.0","latest-status":"semver-safe-update"}},{{"name":"vendor/tool","direct-dependency":true,"version":"1.0.0","latest":"1.2.0","latest-status":"semver-safe-update","description":"Example tool"}}]}}
exit /b 0

:search
echo [{{"name":"composer","description":"platform"}},{{"name":"vendor/new-tool","description":"New tool","url":"https://example.test/new-tool","repository":"https://source.example.test/new-tool"}}]
exit /b 0

:write
exit /b 0
"#,
        log.display(),
        home.display(),
        home.display()
    );
    fs::write(&executable, script).expect("write fake Composer command file");
    (directory, executable)
}

fn write_manifest(home: &std::path::Path) {
    fs::create_dir_all(home).expect("create Composer global home");
    fs::write(
        home.join("composer.json"),
        r#"{"require":{"php":"^8.4","vendor/tool":"^1.0"},"require-dev":{"dev/tool":"^1"}}"#,
    )
    .expect("write Composer global manifest");
}

#[test]
fn composer_descriptor_advertises_direct_current_user_contract() {
    let manager = ComposerGlobalManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:composer-global");
    assert_eq!(descriptor.display_name(), "Composer Global");
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
async fn native_contract_preserves_home_direct_identity_search_and_writes() {
    let _guard = COMPOSER_CONTRACT_LOCK.lock().await;
    let manager = ComposerGlobalManager::new();
    let workspace = tempdir().expect("create Composer contract workspace");
    let home = workspace.path().join("composer-home");
    let log = workspace.path().join("composer.log");
    write_manifest(&home);
    let (_directory, executable) = fake_composer(&home, &log);
    let config = config(&manager, &executable);

    assert!(
        manager
            .availability(&config)
            .await
            .expect("Composer availability")
            .is_available()
    );
    let installed = manager
        .installed(&config)
        .await
        .expect("Composer global inventory");
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].name, "vendor/tool");
    assert_eq!(installed[0].version, "1.0.0");
    assert_eq!(installed[0].scope, PackageScope::User);
    assert_eq!(
        installed[0].homepage.as_deref(),
        Some("https://example.test/tool")
    );
    assert_eq!(
        manager
            .count_installed(&config)
            .await
            .expect("Composer global count"),
        1
    );

    let updates = manager
        .updates(&config, false)
        .await
        .expect("Composer global updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target, installed[0].target());
    assert_eq!(updates[0].available_version, "1.2.0");

    let search = manager
        .search(&config, "vendor")
        .await
        .expect("Composer global search");
    assert_eq!(search.len(), 1);
    assert_eq!(search[0].name, "vendor/new-tool");
    assert_eq!(search[0].version, "Not Installed");
    assert_eq!(search[0].scope, PackageScope::User);

    let mut install = search[0].target();
    install.version = Some("2.0.0".to_owned());
    manager
        .execute(&config, PackageAction::Install, &[install], &|_| {})
        .await
        .expect("install Composer global package");
    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&updates[0].target),
            &|_| {},
        )
        .await
        .expect("update Composer global package");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            &[installed[0].target()],
            &|_| {},
        )
        .await
        .expect("remove Composer global package");

    let lines = fs::read_to_string(&log).expect("read Composer argv log");
    let normalized = lines.replace('\\', "/").replace('"', "");
    let expected_home = home.to_string_lossy().replace('\\', "/");
    assert!(normalized.contains("|global config home --absolute --no-interaction --no-ansi"));
    assert!(
        normalized.contains(&format!(
            "{expected_home}|global show --direct --format=json --no-interaction --no-ansi"
        )),
        "unexpected Composer fixture log: {normalized:?}"
    );
    assert!(normalized.contains(&format!(
        "{expected_home}|global outdated --direct --format=json --no-interaction --no-ansi"
    )));
    assert!(normalized.contains(&format!(
        "{expected_home}|global search --format=json --no-interaction --no-ansi -- vendor"
    )));
    assert!(normalized.contains(&format!(
        "{expected_home}|global require --no-interaction --no-progress --no-ansi -- vendor/new-tool:2.0.0"
    )));
    assert!(normalized.contains(&format!(
        "{expected_home}|global update --with-dependencies --no-interaction --no-progress --no-ansi -- vendor/tool"
    )));
    assert!(normalized.contains(&format!(
        "{expected_home}|global remove --no-interaction --no-progress --no-ansi -- vendor/tool"
    )));
}

#[tokio::test]
async fn targets_reject_forged_home_constraint_scope_and_non_direct_packages() {
    let _guard = COMPOSER_CONTRACT_LOCK.lock().await;
    let manager = ComposerGlobalManager::new();
    let workspace = tempdir().expect("create Composer validation workspace");
    let home = workspace.path().join("composer-home");
    let log = workspace.path().join("composer.log");
    write_manifest(&home);
    let (_directory, executable) = fake_composer(&home, &log);
    let config = config(&manager, &executable);
    let installed = manager
        .installed(&config)
        .await
        .expect("Composer global inventory");

    let mut wrong_scope = installed[0].target();
    wrong_scope.scope = PackageScope::System;
    assert_eq!(
        manager
            .execute(&config, PackageAction::Update, &[wrong_scope], &|_| {})
            .await
            .expect_err("reject Composer system scope")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let mut forged_home = installed[0].target();
    let origin = forged_home.origin.as_mut().expect("typed Composer origin");
    let mut reference: Value = serde_json::from_str(
        origin
            .reference
            .as_deref()
            .expect("Composer origin reference"),
    )
    .expect("parse Composer origin JSON");
    reference["home"] = Value::String(workspace.path().join("other").display().to_string());
    origin.reference = Some(serde_json::to_string(&reference).expect("serialize forged origin"));
    assert_eq!(
        manager
            .execute(&config, PackageAction::Update, &[forged_home], &|_| {})
            .await
            .expect_err("reject forged Composer home")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let mut stale = installed[0].target();
    let origin = stale.origin.as_mut().expect("typed Composer origin");
    let mut reference: Value = serde_json::from_str(
        origin
            .reference
            .as_deref()
            .expect("Composer origin reference"),
    )
    .expect("parse Composer origin JSON");
    reference["constraint"] = Value::String("^9".to_owned());
    origin.reference = Some(serde_json::to_string(&reference).expect("serialize stale origin"));
    assert_eq!(
        manager
            .execute(&config, PackageAction::Uninstall, &[stale], &|_| {})
            .await
            .expect_err("reject stale Composer constraint")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let mut transitive = installed[0].target();
    transitive.name = "vendor/transitive".to_owned();
    assert_eq!(
        manager
            .execute(&config, PackageAction::Update, &[transitive], &|_| {})
            .await
            .expect_err("reject Composer transitive dependency")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[tokio::test]
async fn missing_global_manifest_is_an_empty_inventory() {
    let _guard = COMPOSER_CONTRACT_LOCK.lock().await;
    let manager = ComposerGlobalManager::new();
    let workspace = tempdir().expect("create empty Composer workspace");
    let home = workspace.path().join("composer-home");
    let log = workspace.path().join("composer.log");
    let (_directory, executable) = fake_composer(&home, &log);
    let config = config(&manager, &executable);
    assert!(
        manager
            .installed(&config)
            .await
            .expect("empty Composer inventory")
            .is_empty()
    );
    assert!(
        manager
            .updates(&config, false)
            .await
            .expect("empty Composer updates")
            .is_empty()
    );
    assert!(
        !fs::read_to_string(log)
            .expect("read empty Composer log")
            .contains("global show")
    );
}

#[tokio::test]
#[ignore = "requires host Composer and performs read-only global home, inventory, outdated, and search probes"]
async fn host_composer_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let _guard = COMPOSER_CONTRACT_LOCK.lock().await;
    let manager = ComposerGlobalManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    let _ = manager.updates(&config, false).await?;
    let _ = manager.search(&config, "composer").await?;
    Ok(())
}
