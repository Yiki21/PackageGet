use std::{fs, path::PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerConfig, ManagerErrorKind, PackageAction,
    PackageManager, PackageScope, PackageTarget, Platform,
};
use updater_managers::RubyGemsManager;

struct Fixture {
    _directory: TempDir,
    executable: PathBuf,
    log: PathBuf,
    system: PathBuf,
    user: PathBuf,
}

fn config(manager: &RubyGemsManager, executable: &PathBuf) -> ManagerConfig {
    ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable)
}

#[cfg(unix)]
fn fake_gem() -> Fixture {
    let directory = tempdir().expect("create fake RubyGems directory");
    let executable = directory.path().join("gem");
    let log = directory.path().join("gem.log");
    let system = directory.path().join("system-gems");
    let user = directory.path().join("user-gems");
    let gem_path = std::env::join_paths([&user, &system]).expect("join GEM_PATH");
    let script = format!(
        r#"#!/bin/sh
printf '%s|%s' "$GEM_HOME" "$GEM_PATH" >> '{}'
for arg in "$@"; do printf '|%s' "$arg" >> '{}'; done
printf '\n' >> '{}'

if [ "$1" = "--version" ]; then printf '4.0.10\n'; exit 0; fi
if [ "$1" = "environment" ] && [ "$2" = "home" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$1" = "environment" ] && [ "$2" = "user_gemhome" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$1" = "environment" ] && [ "$2" = "path" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$1" = "list" ] && [ "$GEM_HOME" = '{}' ]; then
  printf '%s\n' 'rake (13.0.6, 12.3.3)' '    Installed at (13.0.6): {}' '                 (12.3.3): {}'
  exit 0
fi
if [ "$1" = "list" ] && [ "$GEM_HOME" = '{}' ]; then
  printf '%s\n' 'bundler (4.0.10)' '    Installed at (default): {}' '' 'rake (13.0.6)' '    Installed at: {}'
  exit 0
fi
if [ "$1" = "outdated" ] && [ "$GEM_HOME" = '{}' ]; then printf '%s\n' 'rake (13.0.6 < 13.3.0)' 'bundler (4.0.10 < 4.0.17)'; exit 0; fi
if [ "$1" = "outdated" ] && [ "$GEM_HOME" = '{}' ]; then printf '%s\n' 'bundler (4.0.10 < 4.0.17)'; exit 0; fi
if [ "$1" = "search" ]; then printf '%s\n' 'rake (13.3.0, 13.2.1)' 'rake-compiler (1.3.0)'; exit 0; fi
case "$1" in install|update|uninstall) exit 0 ;; esac
exit 30
"#,
        log.display(),
        log.display(),
        log.display(),
        system.display(),
        user.display(),
        gem_path.to_string_lossy(),
        user.display(),
        user.display(),
        user.display(),
        system.display(),
        system.display(),
        system.display(),
        user.display(),
        system.display(),
    );
    fs::write(&executable, script).expect("write fake gem executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake gem executable");
    Fixture {
        _directory: directory,
        executable,
        log,
        system,
        user,
    }
}

#[cfg(windows)]
fn fake_gem() -> Fixture {
    let directory = tempdir().expect("create fake RubyGems directory");
    let executable = directory.path().join("gem.cmd");
    let log = directory.path().join("gem.log");
    let system = directory.path().join("system-gems");
    let user = directory.path().join("user-gems");
    let gem_path = std::env::join_paths([&user, &system]).expect("join GEM_PATH");
    let script = format!(
        r#"@echo off
echo %GEM_HOME%^|%GEM_PATH%^|%*>>"{}"
if "%1"=="--version" goto version
if "%1"=="environment" goto environment
if "%1"=="list" goto list
if "%1"=="outdated" goto outdated
if "%1"=="search" goto search
if "%1"=="install" goto write
if "%1"=="update" goto write
if "%1"=="uninstall" goto write
exit /b 30
:version
echo 4.0.10
exit /b 0
:environment
if "%2"=="home" goto home
if "%2"=="user_gemhome" goto userhome
if "%2"=="path" goto path
exit /b 31
:home
echo {}
exit /b 0
:userhome
echo {}
exit /b 0
:path
echo {}
exit /b 0
:list
if "%GEM_HOME%"=="{}" goto userlist
if "%GEM_HOME%"=="{}" goto systemlist
exit /b 32
:userlist
echo rake (13.0.6, 12.3.3)
echo     Installed at (13.0.6): {}
echo                  (12.3.3): {}
exit /b 0
:systemlist
echo bundler (4.0.10)
echo     Installed at (default): {}
echo.
echo rake (13.0.6)
echo     Installed at: {}
exit /b 0
:outdated
if "%GEM_HOME%"=="{}" goto useroutdated
if "%GEM_HOME%"=="{}" goto systemoutdated
exit /b 33
:useroutdated
echo rake (13.0.6 ^< 13.3.0)
echo bundler (4.0.10 ^< 4.0.17)
exit /b 0
:systemoutdated
echo bundler (4.0.10 ^< 4.0.17)
exit /b 0
:search
echo rake (13.3.0, 13.2.1)
echo rake-compiler (1.3.0)
exit /b 0
:write
exit /b 0
"#,
        log.display(),
        system.display(),
        user.display(),
        gem_path.to_string_lossy(),
        user.display(),
        system.display(),
        user.display(),
        user.display(),
        system.display(),
        system.display(),
        user.display(),
        system.display(),
    );
    fs::write(&executable, script).expect("write fake gem command file");
    Fixture {
        _directory: directory,
        executable,
        log,
        system,
        user,
    }
}

fn fake_gem_reporting_update_error() -> Fixture {
    let fixture = fake_gem();
    let script = fs::read_to_string(&fixture.executable).expect("read fake RubyGems executable");
    #[cfg(unix)]
    let (needle, replacement) = (
        "case \"$1\" in install|update|uninstall) exit 0 ;; esac",
        "if [ \"$1\" = \"update\" ]; then printf '%s\\n' 'ERROR:  Error installing rake' >&2; exit 0; fi\ncase \"$1\" in install|update|uninstall) exit 0 ;; esac",
    );
    #[cfg(windows)]
    let (needle, replacement) = (
        "if \"%1\"==\"update\" goto write",
        "if \"%1\"==\"update\" (\n  >&2 echo ERROR:  Error installing rake\n  exit /b 0\n)",
    );
    let failing_script = script.replace(needle, replacement);
    assert_ne!(failing_script, script, "inject fake RubyGems update error");
    fs::write(&fixture.executable, failing_script).expect("write failing fake RubyGems executable");
    fixture
}

fn origin_value(package: &updater_manager_api::PackageInfo) -> serde_json::Value {
    serde_json::from_str(
        package
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref())
            .expect("RubyGems origin reference"),
    )
    .expect("RubyGems origin JSON")
}

#[test]
fn rubygems_descriptor_advertises_three_platform_repository_contract() {
    let manager = RubyGemsManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:rubygems");
    assert_eq!(descriptor.display_name(), "RubyGems");
    assert!(matches!(
        descriptor.authorization(),
        AuthorizationHint::MayRequireElevation { .. }
    ));
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
async fn native_contract_preserves_repositories_versions_updates_search_and_writes() {
    let manager = RubyGemsManager::new();
    let fixture = fake_gem();
    let config = config(&manager, &fixture.executable);

    assert!(
        manager
            .availability(&config)
            .await
            .expect("RubyGems availability")
            .is_available()
    );
    let installed = manager
        .installed(&config)
        .await
        .expect("RubyGems inventory");
    assert_eq!(installed.len(), 4);
    let user_rake_versions = installed
        .iter()
        .filter(|package| {
            package.name == "rake"
                && origin_value(package)["repository"] == fixture.user.to_string_lossy().as_ref()
        })
        .map(|package| package.version.as_str())
        .collect::<Vec<_>>();
    assert_eq!(user_rake_versions, ["12.3.3", "13.0.6"]);
    assert!(installed.iter().any(|package| {
        package.name == "rake"
            && package.version == "13.0.6"
            && origin_value(package)["repository"] == fixture.system.to_string_lossy().as_ref()
    }));
    let bundler = installed
        .iter()
        .find(|package| package.name == "bundler")
        .expect("default bundler");
    assert_eq!(bundler.scope, PackageScope::System);
    assert_eq!(origin_value(bundler)["default"], true);

    let updates = manager
        .updates(&config, false)
        .await
        .expect("RubyGems updates");
    assert_eq!(updates.len(), 2);
    let rake_update = updates
        .iter()
        .find(|update| update.target.name == "rake")
        .expect("rake update");
    assert_eq!(rake_update.current_version, "13.0.6");
    assert_eq!(rake_update.available_version, "13.3.0");

    let search = manager
        .search(&config, "rake")
        .await
        .expect("RubyGems search");
    assert_eq!(search.len(), 2);
    assert_eq!(search[0].name, "rake");
    assert_eq!(search[0].version, "13.3.0");
    assert_eq!(search[0].scope, PackageScope::System);
    assert_eq!(origin_value(&search[0])["version"], serde_json::Value::Null);

    let sink = |_| {};
    let mut install = search[0].target();
    install.version = Some("13.3.0".to_owned());
    manager
        .execute(&config, PackageAction::Install, &[install], &sink)
        .await
        .expect("install Ruby gem");
    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&rake_update.target),
            &sink,
        )
        .await
        .expect("update Ruby gem");
    let old_rake = installed
        .iter()
        .find(|package| {
            package.name == "rake"
                && package.version == "12.3.3"
                && package.scope == PackageScope::User
        })
        .expect("old user rake");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            &[old_rake.target()],
            &sink,
        )
        .await
        .expect("uninstall exact Ruby gem version");

    let log = fs::read_to_string(&fixture.log).expect("read RubyGems argv log");
    assert!(log.lines().any(|line| {
        line.contains("install")
            && line.contains("rake")
            && line.contains("--version")
            && line.contains("13.3.0")
            && line.contains(&fixture.system.to_string_lossy().to_string())
    }));
    assert!(log.lines().any(|line| {
        line.contains("update")
            && line.contains("rake")
            && line.contains(&fixture.user.to_string_lossy().to_string())
    }));
    assert!(log.lines().any(|line| {
        line.contains("uninstall")
            && line.contains("12.3.3")
            && line.contains("--abort-on-dependent")
            && line.contains(&fixture.user.to_string_lossy().to_string())
    }));
}

#[tokio::test]
async fn targets_reject_default_uninstall_wrong_scope_and_forged_repository() {
    let manager = RubyGemsManager::new();
    let fixture = fake_gem();
    let config = config(&manager, &fixture.executable);
    let installed = manager
        .installed(&config)
        .await
        .expect("RubyGems inventory");
    let sink = |_| {};

    let default = installed
        .iter()
        .find(|package| package.name == "bundler")
        .expect("default bundler")
        .target();
    assert_eq!(
        manager
            .execute(&config, PackageAction::Uninstall, &[default], &sink)
            .await
            .expect_err("default gem uninstall must fail")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    let mut wrong_scope = installed
        .iter()
        .find(|package| package.name == "rake" && package.scope == PackageScope::User)
        .expect("user rake")
        .target();
    wrong_scope.scope = PackageScope::System;
    assert_eq!(
        manager
            .execute(&config, PackageAction::Update, &[wrong_scope], &sink)
            .await
            .expect_err("wrong scope must fail")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let outside = if cfg!(windows) {
        r"C:\outside"
    } else {
        "/outside"
    };
    let reference = serde_json::json!({
        "repository": outside,
        "version": "1.0.0",
        "default": false
    });
    let mut forged = PackageTarget::new(manager.descriptor().id().clone(), "rake");
    forged.scope = PackageScope::System;
    forged.origin = Some(
        updater_manager_api::PackageOrigin::new("RubyGems").with_reference(reference.to_string()),
    );
    assert_eq!(
        manager
            .execute(&config, PackageAction::Uninstall, &[forged], &sink)
            .await
            .expect_err("forged repository must fail")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[tokio::test]
async fn update_rejects_error_output_even_when_rubygems_exits_successfully() {
    let manager = RubyGemsManager::new();
    let fixture = fake_gem_reporting_update_error();
    let config = config(&manager, &fixture.executable);
    let target = manager
        .installed(&config)
        .await
        .expect("RubyGems inventory")
        .into_iter()
        .find(|package| package.name == "rake" && package.scope == PackageScope::User)
        .expect("user rake")
        .target();
    let sink = |_| {};

    let error = manager
        .execute(&config, PackageAction::Update, &[target], &sink)
        .await
        .expect_err("RubyGems ERROR output must fail the update");

    assert_eq!(error.kind(), ManagerErrorKind::Other);
    assert_eq!(
        error.message(),
        "RubyGems reported a package operation failure"
    );
    assert_eq!(error.detail(), Some("ERROR:  Error installing rake"));
}

#[tokio::test]
#[ignore = "requires host RubyGems and performs read-only environment/inventory probes"]
async fn host_rubygems_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = RubyGemsManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    Ok(())
}
