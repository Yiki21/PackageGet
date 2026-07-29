use std::{fs, path::PathBuf, sync::Mutex};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerConfig, ManagerErrorKind, PackageAction,
    PackageManager, PackageOrigin, PackageScope, PackageTarget, ProgressEvent,
};
use updater_managers::GoManager;

#[cfg(unix)]
fn fake_go(script: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Go directory");
    let executable = directory.path().join("go");
    fs::write(&executable, script).expect("write fake Go executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Go executable");
    (directory, executable)
}

fn config(manager: &GoManager, executable: &PathBuf, bin: &std::path::Path) -> ManagerConfig {
    let mut config =
        ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    config.settings = serde_json::json!({ "go_bin_dir": bin });
    config
}

#[test]
fn go_descriptor_exposes_the_stable_public_contract() {
    let manager = GoManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:go");
    assert_eq!(descriptor.display_name(), "Go");
    assert_eq!(descriptor.authorization(), &AuthorizationHint::None);
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

#[cfg(unix)]
#[tokio::test]
async fn installed_is_sorted_and_preserves_binary_module_and_package_identity() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    fs::write(bin.path().join("ztool"), b"z").expect("write ztool");
    fs::write(bin.path().join("atool"), b"aaa").expect("write atool");
    let (_directory, executable) = fake_go(
        r#"#!/bin/sh
if [ "$1" = "version" ] && [ "$2" = "-m" ] && [ "$3" = "-json" ]; then
  name=${4##*/}
  case "$name" in
    atool) printf '{"Path":"example.com/mod/cmd/atool","Main":{"Path":"example.com/mod","Version":"v1.2.0"}}\n' ;;
    ztool) printf '{"Path":"example.net/ztool","Main":{"Path":"example.net/ztool","Version":"v0.8.0"}}\n' ;;
    *) exit 8 ;;
  esac
  exit 0
fi
exit 9
"#,
    );

    let packages = manager
        .installed(&config(&manager, &executable, bin.path()))
        .await
        .expect("Go inventory");
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].name, "atool");
    assert_eq!(packages[0].version, "v1.2.0");
    assert_eq!(packages[0].size, Some(3));
    assert_eq!(packages[0].scope, PackageScope::User);
    assert_eq!(
        packages[0].origin,
        Some(
            PackageOrigin::new("example.com/mod")
                .with_reference("package:example.com/mod/cmd/atool")
        )
    );
    assert_eq!(packages[1].name, "ztool");
}

#[cfg(unix)]
#[tokio::test]
async fn updates_and_exact_search_use_module_versions_without_swallowing_failures() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    fs::write(bin.path().join("tool"), b"tool").expect("write tool");
    let (_directory, executable) = fake_go(
        r#"#!/bin/sh
if [ "$1" = "version" ] && [ "$2" = "-m" ] && [ "$3" = "-json" ]; then
  printf '{"Path":"example.com/mod/cmd/tool","Main":{"Path":"example.com/mod","Version":"v1.0.0"}}\n'
  exit 0
fi
if [ "$1" = "list" ] && [ "$2" = "-m" ] && [ "$3" = "-json" ] && [ "$4" = "example.com/mod@latest" ]; then
  printf '{"Path":"example.com/mod","Version":"v1.1.0"}\n'
  exit 0
fi
if [ "$1" = "list" ] && [ "$2" = "-m" ] && [ "$3" = "-versions" ] && [ "$4" = "-json" ] && [ "$5" = "example.com/mod" ]; then
  printf '{"Path":"example.com/mod","Versions":["v0.9.0","v1.0.0","v1.1.0"]}\n'
  exit 0
fi
exit 11
"#,
    );
    let config = config(&manager, &executable, bin.path());

    let updates = manager.updates(&config, false).await.expect("Go updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "tool");
    assert_eq!(updates[0].available_version, "v1.1.0");
    assert_eq!(
        updates[0]
            .target
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("package:example.com/mod/cmd/tool")
    );

    let result = manager
        .search(&config, "example.com/mod")
        .await
        .expect("exact Go lookup");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "tool");

    let error = manager
        .search(&config, "example.com/missing")
        .await
        .expect_err("surface go list failure");
    assert_ne!(error.kind(), ManagerErrorKind::Protocol);
}

#[cfg(unix)]
#[tokio::test]
async fn typed_writes_use_package_identity_and_command_local_gobin() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    let log = bin.path().join("write.log");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "install" ]; then
  printf '%s|%s\n' "$GOBIN" "$2" > '{}'
  exit 0
fi
exit 12
"#,
        log.display()
    );
    let (_directory, executable) = fake_go(&script);
    let config = config(&manager, &executable, bin.path());
    let mut target = PackageTarget::new(manager.descriptor().id().clone(), "tool");
    target.scope = PackageScope::User;
    target.origin = Some(
        PackageOrigin::new("example.com/mod").with_reference("package:example.com/mod/cmd/tool"),
    );
    target.version = Some("v1.4.0".to_owned());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[target], &sink)
        .await
        .expect("install typed Go target");
    assert_eq!(
        fs::read_to_string(log).expect("read write log").trim(),
        format!("{}|example.com/mod/cmd/tool@v1.4.0", bin.path().display())
    );
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
async fn legacy_binary_update_resolves_the_installed_package_path() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    fs::write(bin.path().join("tool"), b"tool").expect("write tool");
    let log = bin.path().join("write.log");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "version" ] && [ "$2" = "-m" ] && [ "$3" = "-json" ]; then
  printf '{{"Path":"example.com/mod/cmd/tool","Main":{{"Path":"example.com/mod","Version":"v1.0.0"}}}}\n'
  exit 0
fi
if [ "$1" = "install" ]; then
  printf '%s\n' "$2" > '{}'
  exit 0
fi
exit 14
"#,
        log.display()
    );
    let (_directory, executable) = fake_go(&script);
    let config = config(&manager, &executable, bin.path());
    let target = PackageTarget::new(manager.descriptor().id().clone(), "tool");

    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&target),
            &|_| {},
        )
        .await
        .expect("update legacy binary target");
    assert_eq!(
        fs::read_to_string(log).expect("read update log").trim(),
        "example.com/mod/cmd/tool@latest"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn uninstall_only_removes_a_regular_basename_inside_gobin() {
    use std::os::unix::fs::symlink;

    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    let (_directory, executable) = fake_go(
        "#!/bin/sh\nif [ \"$1\" = \"version\" ] && [ \"$2\" = \"-m\" ] && [ \"$3\" = \"-json\" ]; then printf '{\"Path\":\"example.com/mod/cmd/tool\",\"Main\":{\"Path\":\"example.com/mod\",\"Version\":\"v1.0.0\"}}\\n'; exit 0; fi\nexit 13\n",
    );
    let config = config(&manager, &executable, bin.path());
    fs::write(bin.path().join("tool"), b"tool").expect("write tool");
    let mut target = PackageTarget::new(manager.descriptor().id().clone(), "tool");
    target.scope = PackageScope::User;
    target.origin = Some(
        PackageOrigin::new("example.com/mod").with_reference("package:example.com/mod/cmd/tool"),
    );

    manager
        .execute_target_with_progress(&config, PackageAction::Uninstall, &target, |_| {})
        .await
        .expect("remove contained binary");
    assert!(!bin.path().join("tool").exists());

    let outside = tempdir().expect("create outside directory");
    fs::write(outside.path().join("outside"), b"outside").expect("write outside file");
    symlink(outside.path().join("outside"), bin.path().join("link")).expect("create symlink");
    target.name = "link".to_owned();
    assert_eq!(
        manager
            .execute_target_with_progress(&config, PackageAction::Uninstall, &target, |_| {})
            .await
            .expect_err("reject symlink removal")
            .kind(),
        ManagerErrorKind::Protocol
    );
    target.name = "../outside".to_owned();
    assert_eq!(
        manager
            .execute_target_with_progress(&config, PackageAction::Uninstall, &target, |_| {})
            .await
            .expect_err("reject traversal")
            .kind(),
        ManagerErrorKind::Protocol
    );
    assert!(outside.path().join("outside").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn build_info_probe_failure_is_not_silently_dropped() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    fs::write(bin.path().join("broken"), b"broken").expect("write binary");
    let (_directory, executable) =
        fake_go("#!/bin/sh\nprintf 'unreadable build info\\n' >&2\nexit 17\n");
    let error = manager
        .installed(&config(&manager, &executable, bin.path()))
        .await
        .expect_err("surface build-info failure");
    assert_ne!(error.kind(), ManagerErrorKind::Protocol);
}

#[tokio::test]
#[ignore = "requires host Go and performs read-only GOBIN/build-info probes"]
async fn host_go_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = GoManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    assert!(!installed.is_empty());
    Ok(())
}
