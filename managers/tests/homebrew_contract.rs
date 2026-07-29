use std::{fs, path::PathBuf, sync::Mutex};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, AvailabilityReason, ManagerAvailability, ManagerCapability, ManagerConfig,
    ManagerErrorKind, PackageAction, PackageManager, PackageOrigin, PackageScope, PackageTarget,
    ProgressEvent,
};
use updater_managers::HomebrewManager;

#[cfg(unix)]
fn fake_brew(script: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Homebrew directory");
    let executable = directory.path().join("brew");
    fs::write(&executable, script).expect("write fake Homebrew executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Homebrew executable");
    (directory, executable)
}

#[cfg(unix)]
fn read_fixture_brew() -> (TempDir, PathBuf) {
    fake_brew(
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$0.log"
if [ "$1" = "--version" ]; then
  printf 'Homebrew 6.0.13\n'
  exit 0
fi
if [ "$1" = "update" ]; then
  if [ "${HOMEBREW_NO_AUTO_UPDATE+x}" = "x" ] ||
     [ "${HOMEBREW_NO_ANALYTICS:-}" != "1" ] ||
     [ "${LC_ALL:-}" != "C" ] || [ "${LANG:-}" != "C" ]; then
    printf 'refresh environment contract violated\n' >&2
    exit 89
  fi
  exit 0
fi
if [ "${HOMEBREW_NO_AUTO_UPDATE:-}" != "1" ] ||
   [ "${HOMEBREW_NO_ANALYTICS:-}" != "1" ] ||
   [ "${HOMEBREW_NO_ASK:-}" != "1" ] ||
   [ "${LC_ALL:-}" != "C" ] || [ "${LANG:-}" != "C" ]; then
  printf 'command-local environment contract violated\n' >&2
  exit 90
fi
if [ "$1" = "info" ] && [ "$2" = "--json=v2" ] && [ "$3" = "--installed" ]; then
  printf '%s\n' '{
    "formulae": [{
      "name": "jq",
      "full_name": "jq",
      "tap": "homebrew/core",
      "desc": "JSON processor",
      "homepage": "https://jqlang.github.io/jq/",
      "installed": [{"version":"1.7.1"},{"version":"1.8.0"}],
      "linked_keg": "1.8.0"
    }],
    "casks": [{
      "token": "jq",
      "full_token": "jq",
      "tap": "homebrew/cask",
      "desc": "Cask collision",
      "homepage": "https://example.test/jq-cask",
      "installed": "3.0"
    }]
  }'
  exit 0
fi
if [ "$1" = "outdated" ] && [ "$2" = "--json=v2" ]; then
  printf '%s\n' '{
    "formulae": [{"name":"jq","installed_versions":["1.7.1","1.8.0"],"current_version":"1.9.0","pinned":false,"pinned_version":null}],
    "casks": [{"name":"jq","installed_versions":["3.0"],"current_version":"3.1","pinned":false,"pinned_version":null}]
  }'
  exit 0
fi
if [ "$1" = "search" ]; then
  if [ "$3" = "definitely-missing" ]; then
    printf 'Error: No formulae or casks found for "definitely-missing".\n' >&2
    exit 1
  fi
  if [ "$2" = "--formula" ] || [ "$2" = "--cask" ]; then
    printf 'jq\n'
    exit 0
  fi
  exit 91
fi
if [ "$1" = "info" ] && [ "$2" = "--json=v2" ] && [ "$3" = "--formula" ]; then
  printf '%s\n' '{
    "formulae": [{
      "name":"jq","full_name":"jq","tap":"homebrew/core",
      "desc":"JSON processor","homepage":"https://jqlang.github.io/jq/",
      "installed":[{"version":"1.7.1"},{"version":"1.8.0"}],"linked_keg":"1.8.0"
    }],
    "casks": []
  }'
  exit 0
fi
if [ "$1" = "info" ] && [ "$2" = "--json=v2" ] && [ "$3" = "--cask" ]; then
  printf '%s\n' '{
    "formulae": [],
    "casks": [{
      "token":"jq","full_token":"jq","tap":"homebrew/cask",
      "desc":"Cask collision","homepage":"https://example.test/jq-cask","installed":"3.0"
    }]
  }'
  exit 0
fi
exit 2
"#,
    )
}

#[test]
fn homebrew_descriptor_exposes_the_stable_public_contract() {
    let manager = HomebrewManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:homebrew");
    assert_eq!(descriptor.display_name(), "Homebrew");
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
    let manager = HomebrewManager::new();
    let directory = tempdir().expect("create temporary directory");
    let missing = directory.path().join("missing-brew");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(&missing);

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check missing Homebrew executable"),
        ManagerAvailability::Unavailable {
            reason: AvailabilityReason::CommandMissing {
                command: missing.to_string_lossy().into_owned(),
            },
        }
    );
}

#[tokio::test]
async fn empty_execution_emits_boundaries_without_running_homebrew() {
    let manager = HomebrewManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[], &sink)
        .await
        .expect("execute empty Homebrew group");

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
async fn mismatched_config_and_target_are_rejected_before_progress() {
    let manager = HomebrewManager::new();
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
        "jq",
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
async fn public_reads_preserve_formula_cask_tap_and_multikeg_identity() {
    let manager = HomebrewManager::new();
    let (_directory, executable) = read_fixture_brew();
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(&executable);

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check fake Homebrew availability"),
        ManagerAvailability::Available {
            version: Some("Homebrew 6.0.13".to_owned()),
        }
    );
    let installed = manager
        .installed(&config)
        .await
        .expect("list fake Homebrew packages");
    assert_eq!(installed.len(), 2);
    assert_eq!(installed[0].name, "jq");
    assert_eq!(installed[0].version, "1.7.1, 1.8.0");
    assert_eq!(installed[0].scope, PackageScope::User);
    assert_eq!(
        installed[0]
            .origin
            .as_ref()
            .map(|origin| origin.name.as_str()),
        Some("homebrew/core")
    );
    assert_eq!(
        installed[0]
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("formula:homebrew/core/jq")
    );
    assert_eq!(installed[1].name, "jq");
    assert_eq!(
        installed[1]
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("cask:homebrew/cask/jq")
    );
    assert_eq!(
        manager
            .count_installed(&config)
            .await
            .expect("count fake Homebrew packages"),
        2
    );
    assert_eq!(
        manager
            .current_version(&config, "formula:homebrew/core/jq")
            .await
            .expect("query linked formula version"),
        "1.8.0"
    );
    assert_eq!(
        manager
            .current_version(&config, "cask:homebrew/cask/jq")
            .await
            .expect("query installed cask version"),
        "3.0"
    );
    assert_eq!(
        manager
            .current_version(&config, "jq")
            .await
            .expect_err("same-name formula and cask are ambiguous")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let updates = manager
        .updates(&config, false)
        .await
        .expect("list fake Homebrew updates");
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0].target.name, "jq");
    assert_eq!(updates[0].current_version, "1.7.1, 1.8.0");
    assert_eq!(updates[0].available_version, "1.9.0");
    assert_eq!(
        updates[0]
            .target
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("formula:homebrew/core/jq")
    );
    assert_eq!(updates[1].current_version, "3.0");
    assert_eq!(updates[1].available_version, "3.1");

    let search = manager
        .search(&config, "jq")
        .await
        .expect("search fake Homebrew catalogs");
    assert_eq!(search.len(), 2);
    assert_eq!(search[0].description.as_deref(), Some("JSON processor"));
    assert_eq!(search[0].version, "1.7.1, 1.8.0");
    assert_eq!(search[1].description.as_deref(), Some("Cask collision"));
    assert!(
        manager
            .search(&config, "definitely-missing")
            .await
            .expect("map semantic Homebrew no-match to empty")
            .is_empty()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn refresh_is_explicit_and_precedes_inventory_and_outdated() {
    let manager = HomebrewManager::new();
    let (_directory, executable) = read_fixture_brew();
    let log = executable.with_extension("log");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(&executable);

    manager
        .updates(&config, true)
        .await
        .expect("refresh and list fake Homebrew updates");
    assert_eq!(
        fs::read_to_string(log).expect("read fake Homebrew command log"),
        "update\ninfo --json=v2 --installed\noutdated --json=v2\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn public_execute_freezes_kind_tap_and_legacy_argv() {
    let manager = HomebrewManager::new();
    let (_directory, executable) = fake_brew(
        r#"#!/bin/sh
if [ "${HOMEBREW_NO_AUTO_UPDATE:-}" != "1" ] ||
   [ "${HOMEBREW_NO_ANALYTICS:-}" != "1" ] ||
   [ "${HOMEBREW_NO_ASK:-}" != "1" ] ||
   [ "${HOMEBREW_NO_INSTALL_CLEANUP:-}" != "1" ]; then
  exit 90
fi
printf '%s\n' "$*" >> "$0.log"
exit 0
"#,
    );
    let log = executable.with_extension("log");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(&executable);
    let mut formula = PackageTarget::new(manager.descriptor().id().clone(), "jq");
    formula.scope = PackageScope::User;
    formula.origin =
        Some(PackageOrigin::new("homebrew/core").with_reference("formula:homebrew/core/jq"));
    let mut cask = PackageTarget::new(manager.descriptor().id().clone(), "claude-code");
    cask.scope = PackageScope::User;
    cask.origin =
        Some(PackageOrigin::new("homebrew/cask").with_reference("cask:homebrew/cask/claude-code"));
    let legacy = PackageTarget::new(manager.descriptor().id().clone(), "ripgrep");

    manager
        .execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&formula),
            &|_| {},
        )
        .await
        .expect("execute typed formula install");
    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&cask),
            &|_| {},
        )
        .await
        .expect("execute typed cask upgrade");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            std::slice::from_ref(&formula),
            &|_| {},
        )
        .await
        .expect("execute typed formula uninstall");
    manager
        .execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&legacy),
            &|_| {},
        )
        .await
        .expect("execute legacy Homebrew install");

    assert_eq!(
        fs::read_to_string(log).expect("read fake Homebrew write log"),
        "install --formula --yes homebrew/core/jq\n\
         upgrade --cask --yes homebrew/cask/claude-code\n\
         uninstall --formula homebrew/core/jq\n\
         install ripgrep\n"
    );
}

#[tokio::test]
async fn invalid_scoped_targets_are_rejected_before_progress() {
    let manager = HomebrewManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let mut target = PackageTarget::new(manager.descriptor().id().clone(), "jq");
    target.scope = PackageScope::User;
    target.origin =
        Some(PackageOrigin::new("wrong/tap").with_reference("formula:homebrew/core/jq"));
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    assert_eq!(
        manager
            .execute(
                &config,
                PackageAction::Install,
                std::slice::from_ref(&target),
                &sink,
            )
            .await
            .expect_err("reject mismatched Homebrew tap")
            .kind(),
        ManagerErrorKind::Protocol
    );
    assert!(events.lock().expect("progress lock").is_empty());

    target.origin =
        Some(PackageOrigin::new("homebrew/core").with_reference("formula:homebrew/core/jq"));
    target.version = Some("1.9.0".to_owned());
    assert_eq!(
        manager
            .execute(
                &config,
                PackageAction::Install,
                std::slice::from_ref(&target),
                &sink,
            )
            .await
            .expect_err("reject ignored Homebrew target version")
            .kind(),
        ManagerErrorKind::Unsupported
    );
    assert!(events.lock().expect("progress lock").is_empty());
}

#[tokio::test]
#[ignore = "requires a readable local Homebrew installation"]
async fn host_homebrew_read_only_smoke() -> Result<(), updater_manager_api::ManagerError> {
    let manager = HomebrewManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    assert!(matches!(
        manager
            .availability(&config)
            .await
            .expect("check host Homebrew availability"),
        ManagerAvailability::Available { version: Some(version) } if version.starts_with("Homebrew ")
    ));
    let installed = manager
        .installed(&config)
        .await
        .expect("list host Homebrew packages");
    assert_eq!(
        manager
            .count_installed(&config)
            .await
            .expect("count host Homebrew packages"),
        installed.len()
    );
    assert!(installed.iter().all(|package| {
        package.scope == PackageScope::User
            && package.origin.as_ref().is_some_and(|origin| {
                !origin.name.is_empty()
                    && origin.reference.as_deref().is_some_and(|reference| {
                        reference.starts_with("formula:") || reference.starts_with("cask:")
                    })
            })
    }));
    if let Some(package) = installed.first() {
        let reference = package
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref())
            .expect("host package typed reference");
        assert!(
            !manager
                .current_version(&config, reference)
                .await?
                .is_empty()
        );
    }
    let updates = manager
        .updates(&config, false)
        .await
        .expect("list host Homebrew updates without refresh");
    assert!(updates.iter().all(|update| {
        update.target.scope == PackageScope::User
            && update
                .target
                .origin
                .as_ref()
                .and_then(|origin| origin.reference.as_deref())
                .is_some_and(|reference| {
                    reference.starts_with("formula:") || reference.starts_with("cask:")
                })
    }));

    Ok::<(), updater_manager_api::ManagerError>(())
}
