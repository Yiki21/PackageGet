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

#[cfg(unix)]
#[tokio::test]
async fn update_listing_never_syncs_the_live_database() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create logging fake Pacman directory");
    let log = directory.path().join("pacman.log");
    let executable = directory.path().join("pacman");
    fs::write(
        &executable,
        format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
if [ "$1" = "-Qu" ]; then printf 'curl 8.15.0-1 -> 8.16.0-1\n'; exit 0; fi
exit 2
"#,
            log.display()
        ),
    )
    .expect("write logging fake Pacman executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark logging fake Pacman executable");

    let manager = PacmanManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);

    // A refreshed listing would run the privileged helper, which the test
    // host cannot authorise, so this pins the query half of the contract:
    // pacman is only ever asked to list, never to sync, without a temporary
    // database path.
    let updates = manager
        .updates(&config, false)
        .await
        .expect("list Pacman updates");

    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "curl");
    let invoked = fs::read_to_string(&log).expect("read fake Pacman invocation log");
    for invocation in invoked.lines() {
        assert!(
            !invocation
                .split_whitespace()
                .any(|word| word.starts_with("-S")),
            "Pacman must never sync the live database from a listing: {invocation}"
        );
    }
    assert_eq!(invoked.lines().count(), 1, "{invoked}");
    assert!(invoked.starts_with("-Qu"), "{invoked}");
}

/// The fake `pacman` used by the result-reporting tests below.
#[cfg(unix)]
fn fake_pacman_script(mode: &str) -> String {
    match mode {
        "search-fails" => {
            r#"#!/bin/sh
if [ "$1" = "-Ss" ]; then
  printf 'error: failed to init transaction (unable to lock database)\n' >&2
  exit 1
fi
exit 2
"#
        }
        "search-exits-100" => {
            r#"#!/bin/sh
if [ "$1" = "-Ss" ]; then
  printf 'error: could not access database directory\n' >&2
  exit 100
fi
exit 2
"#
        }
        "search-no-match" => {
            r#"#!/bin/sh
if [ "$1" = "-Ss" ]; then
  exit 1
fi
exit 2
"#
        }
        "search-usage-error" => {
            r#"#!/bin/sh
if [ "$1" = "-Ss" ]; then
  exit 2
fi
exit 2
"#
        }
        "version-map-fails" => {
            r#"#!/bin/sh
if [ "$1" = "-Ss" ]; then
  printf 'core/bash 5.2.037-1\n    The GNU Bourne Again shell\n'
  exit 0
fi
if [ "$1" = "-Q" ]; then
  printf 'error: could not open database\n' >&2
  exit 1
fi
exit 2
"#
        }
        other => panic!("unknown fake Pacman mode: {other}"),
    }
    .to_owned()
}

#[cfg(unix)]
fn write_fake_pacman(mode: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Pacman directory");
    let executable = directory.path().join("pacman");
    fs::write(&executable, fake_pacman_script(mode)).expect("write fake Pacman executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Pacman executable");
    (directory, executable)
}

#[cfg(unix)]
#[tokio::test]
async fn search_failure_is_reported_instead_of_an_empty_result() {
    let manager = PacmanManager::new();

    // A locked or uninitializable database is a failure, not 'no matches'.
    let (_directory, executable) = write_fake_pacman("search-fails");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    let error = manager
        .search(&config, "bash")
        .await
        .expect_err("a failed Pacman search must not read as an empty result");
    assert_eq!(error.kind(), ManagerErrorKind::Busy);
    assert!(
        error.detail().is_some_and(|detail| detail.contains("lock")),
        "the diagnostic must reach the caller: {error:?}"
    );

    // Any other non-zero exit stays a classified failure too.
    let (_directory, executable) = write_fake_pacman("search-exits-100");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    let error = manager
        .search(&config, "bash")
        .await
        .expect_err("exit 100 must not read as an empty result");
    assert_eq!(error.kind(), ManagerErrorKind::Other);
}

#[cfg(unix)]
#[tokio::test]
async fn search_without_matches_stays_an_empty_listing() {
    let manager = PacmanManager::new();

    // `pacman -Ss` exits 1 with empty stdout and stderr when nothing matched,
    // which is the one exit code the tool defines as 'no results'.
    let (_directory, executable) = write_fake_pacman("search-no-match");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    assert!(
        manager
            .search(&config, "zzz")
            .await
            .expect("a no-match search is an empty listing")
            .is_empty()
    );

    // Exit 2 is a usage error even without a diagnostic.
    let (_directory, executable) = write_fake_pacman("search-usage-error");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    let error = manager
        .search(&config, "bash")
        .await
        .expect_err("exit 2 is not a no-match result");
    assert_eq!(error.kind(), ManagerErrorKind::Other);
}

#[cfg(unix)]
#[tokio::test]
async fn installed_version_failure_does_not_publish_not_installed_rows() {
    let manager = PacmanManager::new();
    let (_directory, executable) = write_fake_pacman("version-map-fails");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);

    let error = manager
        .search(&config, "bash")
        .await
        .expect_err("a broken local database must fail the search, not label rows absent");
    assert_eq!(error.kind(), ManagerErrorKind::Other);
}
