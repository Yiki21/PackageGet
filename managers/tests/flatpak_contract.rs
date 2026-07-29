use std::{fs, path::PathBuf, sync::Mutex};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapability, ManagerConfig,
    ManagerErrorKind, PackageAction, PackageManager, PackageScope, PackageTarget, ProgressEvent,
};
use updater_managers::FlatpakManager;

#[cfg(unix)]
fn fake_flatpak(script: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Flatpak directory");
    let executable = directory.path().join("flatpak");
    fs::write(&executable, script).expect("write fake Flatpak executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Flatpak executable");
    (directory, executable)
}

#[test]
fn flatpak_descriptor_exposes_the_stable_public_contract() {
    let manager = FlatpakManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:flatpak");
    assert_eq!(descriptor.display_name(), "Flatpak");
    assert!(matches!(
        descriptor.authorization(),
        AuthorizationHint::MayRequireElevation { .. }
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
    let manager = FlatpakManager::new();
    let directory = tempdir().expect("create temporary directory");
    let missing = directory.path().join("missing-flatpak");
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
async fn empty_execution_emits_boundaries_without_running_flatpak() {
    let manager = FlatpakManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[], &sink)
        .await
        .expect("execute empty Flatpak group");

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
    let manager = FlatpakManager::new();
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
        "org.example.App",
    );
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);
    let error = manager
        .execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&target),
            &sink,
        )
        .await
        .expect_err("reject mismatched target");

    assert_eq!(error.kind(), ManagerErrorKind::Protocol);
    assert!(events.lock().expect("progress lock").is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn public_reads_preserve_scope_ref_origin_cache_and_search_remotes() {
    let manager = FlatpakManager::new();
    let (_directory, executable) = fake_flatpak(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf 'Flatpak 1.18.0\n'
  exit 0
fi
if [ "${LC_ALL:-}" != "C" ] || [ "${LANG:-}" != "C" ]; then
  printf 'locale contract violated\n' >&2
  exit 90
fi
if [ "$1" = "list" ] && [ "$2" = "--app" ]; then
  case "$3" in
    --columns=application:f,name:f,version:f,branch:f,size,origin:f,installation:f,ref:f) ;;
    *) exit 91 ;;
  esac
  printf 'org.example.App\tSystem App\t1.0\tstable\t14.8 MB\tflathub\tsystem\torg.example.App/x86_64/stable\n'
  printf 'org.example.App\tUser Beta\t2.0\tbeta\t2 MiB\tflathub-beta\tuser\torg.example.App/x86_64/beta\n'
  exit 0
fi
if [ "$1" = "remote-ls" ]; then
  if [ "$3" != "--updates" ] || [ "$4" != "--app" ] || [ "$5" != "--cached" ]; then
    exit 92
  fi
  case "$6" in
    --columns=application:f,ref:f,branch:f,version:f,commit:f,origin:f) ;;
    *) exit 93 ;;
  esac
  if [ "$2" = "--system" ]; then
    printf 'org.example.App\tapp/org.example.App/x86_64/stable\tstable\t1.0\tnew-commit\tflathub\n'
  elif [ "$2" = "--user" ]; then
    printf 'org.example.App\tapp/org.example.App/x86_64/beta\tbeta\t2.1\tnew-beta\tflathub-beta\n'
  else
    exit 94
  fi
  exit 0
fi
if [ "$1" = "--default-arch" ]; then
  printf 'x86_64\n'
  exit 0
fi
if [ "$1" = "search" ]; then
  case "$3" in
    --columns=application:f,name:f,description:f,version:f,branch:f,remotes:f) ;;
    *) exit 95 ;;
  esac
  if [ "$2" = "--system" ]; then
    printf 'org.example.App\tExample\tSystem result\t1.0\tstable\tfedora,flathub\n'
  elif [ "$2" = "--user" ]; then
    printf 'org.example.App\tExample\tUser result\t2.0\tbeta\tflathub-beta\n'
  else
    exit 96
  fi
  exit 0
fi
exit 2
"#,
    );
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check fake Flatpak availability"),
        ManagerAvailability::Available {
            version: Some("Flatpak 1.18.0".to_owned()),
        }
    );

    let installed = manager
        .installed(&config)
        .await
        .expect("list fake Flatpak installations");
    assert_eq!(installed.len(), 2);
    assert_eq!(installed[0].name, "org.example.App");
    assert_eq!(installed[0].scope, PackageScope::System);
    assert_eq!(installed[0].size, Some(14_800_000));
    assert_eq!(
        installed[0]
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("app/org.example.App/x86_64/stable")
    );
    assert_eq!(installed[1].scope, PackageScope::User);
    assert_eq!(installed[1].size, Some(2_097_152));
    assert_eq!(
        manager
            .count_installed(&config)
            .await
            .expect("count fake Flatpak installations"),
        2
    );
    assert_eq!(
        manager
            .current_version(&config, "org.example.App")
            .await
            .expect_err("bare duplicate Flatpak ID is ambiguous")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let updates = manager
        .updates(&config, false)
        .await
        .expect("list cached fake Flatpak updates");
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].target.scope, PackageScope::System);
    assert_eq!(updates[0].current_version, "1.0 (stable)");
    assert_eq!(updates[0].available_version, "new build (1.0 (stable))");
    assert_eq!(updates[1].target.scope, PackageScope::User);
    assert_eq!(updates[1].current_version, "2.0 (beta)");
    assert_eq!(updates[1].available_version, "2.1 (beta)");

    let search = manager
        .search(&config, "example")
        .await
        .expect("search fake Flatpak catalogs");
    assert_eq!(search.len(), 3);
    assert_eq!(search[0].scope, PackageScope::System);
    assert_eq!(search[0].version, "1.0 (stable)");
    assert_eq!(
        search[0].origin.as_ref().map(|origin| origin.name.as_str()),
        Some("fedora")
    );
    assert_eq!(
        search[1].origin.as_ref().map(|origin| origin.name.as_str()),
        Some("flathub")
    );
    assert_eq!(search[2].scope, PackageScope::User);
    assert_eq!(search[2].version, "2.0 (beta)");
    assert!(search.iter().all(|package| {
        package
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref())
            .is_some_and(|reference| reference.starts_with("app/org.example.App/x86_64/"))
    }));
}

#[cfg(unix)]
#[tokio::test]
async fn public_execute_freezes_scoped_and_legacy_argv() {
    let manager = FlatpakManager::new();
    let (_directory, executable) = fake_flatpak(
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$0.log"
exit 0
"#,
    );
    let log = executable.with_extension("log");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(&executable);
    let mut scoped = PackageTarget::new(manager.descriptor().id().clone(), "org.example.App");
    scoped.scope = PackageScope::User;
    scoped.origin = Some(
        updater_manager_api::PackageOrigin::new("flathub")
            .with_reference("app/org.example.App/x86_64/stable"),
    );
    let legacy = PackageTarget::new(manager.descriptor().id().clone(), "org.example.Legacy");

    manager
        .execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&scoped),
            &|_| {},
        )
        .await
        .expect("execute scoped fake Flatpak install");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            std::slice::from_ref(&legacy),
            &|_| {},
        )
        .await
        .expect("execute legacy fake Flatpak uninstall");

    assert_eq!(
        fs::read_to_string(log).expect("read fake Flatpak command log"),
        "install --user -y --noninteractive flathub app/org.example.App/x86_64/stable\n\
         uninstall -y org.example.Legacy\n"
    );
}

#[tokio::test]
#[ignore = "requires Flatpak with readable user and system installations"]
async fn host_flatpak_read_only_smoke() {
    let manager = FlatpakManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    assert!(matches!(
        manager
            .availability(&config)
            .await
            .expect("check host Flatpak availability"),
        ManagerAvailability::Available { version: Some(version) } if version.starts_with("Flatpak ")
    ));
    let installed = manager
        .installed(&config)
        .await
        .expect("list host Flatpak installations");
    assert_eq!(
        manager
            .count_installed(&config)
            .await
            .expect("count host Flatpak installations"),
        installed.len()
    );
    assert!(
        installed
            .iter()
            .any(|package| package.scope == PackageScope::System)
    );
    assert!(
        installed
            .iter()
            .any(|package| package.scope == PackageScope::User)
    );
    assert!(installed.iter().all(|package| {
        package
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref())
            .is_some_and(|reference| reference.starts_with("app/"))
    }));

    let updates = manager
        .updates(&config, false)
        .await
        .expect("list cached host Flatpak app updates");
    assert!(updates.iter().all(|update| {
        matches!(
            update.target.scope,
            PackageScope::System | PackageScope::User
        ) && update
            .target
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref())
            .is_some_and(|reference| reference.starts_with("app/"))
    }));
}
