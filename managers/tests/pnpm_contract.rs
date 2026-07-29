use std::{fs, path::PathBuf, sync::Mutex};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerConfig, ManagerErrorKind, PackageAction,
    PackageManager, PackageOrigin, PackageScope, PackageTarget, ProgressEvent,
};
use updater_managers::PnpmManager;

#[cfg(unix)]
fn fake_pnpm(script: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake pnpm directory");
    let executable = directory.path().join("pnpm");
    fs::write(&executable, script).expect("write fake pnpm executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake pnpm executable");
    (directory, executable)
}

fn config(manager: &PnpmManager, executable: &PathBuf) -> ManagerConfig {
    ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable)
}

#[test]
fn pnpm_descriptor_exposes_the_stable_public_contract() {
    let manager = PnpmManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:pnpm");
    assert_eq!(descriptor.display_name(), "pnpm");
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
async fn installed_preserves_scoped_identity_metadata_origin_and_strict_size() {
    let manager = PnpmManager::new();
    let global = tempdir().expect("create global root");
    let scoped = global.path().join("instance/node_modules/@scope/tool");
    let plain = global.path().join("instance/node_modules/plain");
    fs::create_dir_all(&scoped).expect("create scoped package");
    fs::create_dir_all(&plain).expect("create plain package");
    fs::write(scoped.join("index.js"), b"12345").expect("write scoped package");
    fs::write(plain.join("index.js"), b"123").expect("write plain package");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then printf '11.5.1\n'; exit 0; fi
if [ "$1" = "list" ] && [ "$2" = "--global" ] && [ "$3" = "--depth" ] && [ "$4" = "0" ] && [ "$5" = "--json" ] && [ "$6" = "--long" ]; then
cat <<'EOF'
[{{"path":"{}","private":true,"dependencies":{{
  "@scope/tool":{{"from":"@scope/tool","version":"2.1.0","description":"Scoped tool","homepage":"https://example.test/tool","repository":"git+https://example.test/scoped.git","path":"{}"}},
  "plain":{{"from":"plain","version":"1.0.0","repository":{{"url":"git+https://example.test/plain.git"}},"path":"{}"}}
}}}}]
EOF
exit 0
fi
exit 20
"#,
        global.path().display(),
        scoped.display(),
        plain.display()
    );
    let (_directory, executable) = fake_pnpm(&script);
    let config = config(&manager, &executable);

    let packages = manager.installed(&config).await.expect("pnpm inventory");
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].name, "@scope/tool");
    assert_eq!(packages[0].version, "2.1.0");
    assert_eq!(packages[0].size, Some(5));
    assert_eq!(packages[0].scope, PackageScope::User);
    assert_eq!(
        packages[0].origin,
        Some(PackageOrigin::new("pnpm global").with_reference("package:@scope/tool"))
    );
    assert_eq!(
        packages[1].homepage.as_deref(),
        Some("https://example.test/plain.git")
    );
    assert_eq!(
        manager
            .current_version(&config, "@scope/tool")
            .await
            .expect("version"),
        "2.1.0"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn installed_symlink_keeps_identity_without_following_size_target() {
    use std::os::unix::fs::symlink;

    let manager = PnpmManager::new();
    let global = tempdir().expect("create global root");
    let outside = tempdir().expect("create linked package");
    fs::write(outside.path().join("index.js"), b"outside").expect("write linked package");
    let package = global.path().join("instance/node_modules/tool");
    fs::create_dir_all(package.parent().expect("package parent")).expect("create package parent");
    symlink(outside.path(), &package).expect("link pnpm package");
    let script = format!(
        "#!/bin/sh\nprintf '[{{\"path\":\"{}\",\"private\":true,\"dependencies\":{{\"tool\":{{\"from\":\"tool\",\"version\":\"1.0.0\",\"path\":\"{}\"}}}}}}]\\n'\n",
        global.path().display(),
        package.display()
    );
    let (_directory, executable) = fake_pnpm(&script);

    let packages = manager
        .installed(&config(&manager, &executable))
        .await
        .expect("linked pnpm inventory");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "tool");
    assert_eq!(packages[0].size, None);
}

#[cfg(unix)]
#[tokio::test]
async fn duplicate_and_escaping_package_paths_are_rejected() {
    let manager = PnpmManager::new();
    let first = tempdir().expect("create first root");
    let second = tempdir().expect("create second root");
    let package = first.path().join("node_modules/tool");
    fs::create_dir_all(&package).expect("create package");
    let duplicate = format!(
        "#!/bin/sh\nprintf '[{{\"path\":\"{}\",\"private\":true,\"dependencies\":{{\"tool\":{{\"from\":\"tool\",\"version\":\"1.0.0\",\"path\":\"{}\"}}}}}},{{\"path\":\"{}\",\"private\":true,\"dependencies\":{{\"tool\":{{\"from\":\"tool\",\"version\":\"1.0.0\",\"path\":\"{}\"}}}}}}]\\n'\n",
        first.path().display(),
        package.display(),
        second.path().display(),
        package.display()
    );
    let (_directory, executable) = fake_pnpm(&duplicate);
    assert_eq!(
        manager
            .installed(&config(&manager, &executable))
            .await
            .expect_err("reject duplicate")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let escaping = format!(
        "#!/bin/sh\nprintf '[{{\"path\":\"{}\",\"private\":true,\"dependencies\":{{\"tool\":{{\"from\":\"tool\",\"version\":\"1.0.0\",\"path\":\"{}\"}}}}}}]\\n'\n",
        second.path().display(),
        package.display()
    );
    let (_directory, executable) = fake_pnpm(&escaping);
    assert_eq!(
        manager
            .installed(&config(&manager, &executable))
            .await
            .expect_err("reject escaping path")
            .kind(),
        ManagerErrorKind::Permission
    );
}

#[cfg(unix)]
#[tokio::test]
async fn outdated_empty_object_and_scoped_updates_preserve_registry_origin() {
    let manager = PnpmManager::new();
    let (_directory, executable) = fake_pnpm(
        r#"#!/bin/sh
if [ "$1" = "outdated" ] && [ "$2" = "--global" ] && [ "$3" = "--format" ] && [ "$4" = "json" ]; then
  if [ "${EMPTY:-}" = "1" ]; then printf '{}\n'; else printf '{"@scope/tool":{"current":"1.0.0","wanted":"1.1.0","latest":"2.0.0"}}\n'; fi
  exit 0
fi
if [ "$1" = "config" ]; then printf 'https://registry.example.test/\n'; exit 0; fi
exit 21
"#,
    );
    let updates = manager
        .updates(&config(&manager, &executable), false)
        .await
        .expect("pnpm updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "@scope/tool");
    assert_eq!(updates[0].target.scope, PackageScope::User);
    assert_eq!(updates[0].target.version.as_deref(), Some("2.0.0"));
    assert_eq!(
        updates[0].target.origin,
        Some(
            PackageOrigin::new("https://registry.example.test/")
                .with_reference("package:@scope/tool")
        )
    );
    assert_eq!(updates[0].available_version, "2.0.0");

    let (_directory, executable) = fake_pnpm(
        "#!/bin/sh\nif [ \"$1\" = \"config\" ]; then echo https://registry.example.test/; else echo '{}'; fi\nexit 0\n",
    );
    assert!(
        manager
            .updates(&config(&manager, &executable), false)
            .await
            .expect("empty pnpm updates")
            .is_empty()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn nonzero_outdated_is_failure_even_when_stdout_is_valid_json() {
    let manager = PnpmManager::new();
    let (_directory, executable) = fake_pnpm(
        "#!/bin/sh\nprintf '{\"tool\":{\"current\":\"1.0.0\",\"latest\":\"2.0.0\"}}\\n'\nprintf 'registry unavailable\\n' >&2\nexit 22\n",
    );
    assert_ne!(
        manager
            .updates(&config(&manager, &executable), false)
            .await
            .expect_err("surface command failure")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[cfg(unix)]
#[tokio::test]
async fn search_uses_typed_registry_origin_full_scope_and_bounded_argv() {
    let manager = PnpmManager::new();
    let global = tempdir().expect("create empty global root");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "config" ] && [ "$2" = "get" ] && [ "$3" = "registry" ]; then printf 'https://registry.example.test/\n'; exit 0; fi
if [ "$1" = "list" ]; then printf '[{{"path":"{}","private":true,"dependencies":{{}}}}]\n'; exit 0; fi
if [ "$1" = "search" ] && [ "$2" = "scope" ] && [ "$3" = "--json" ] && [ "$4" = "--search-limit" ] && [ "$5" = "50" ]; then
  printf '[{{"name":"@scope/tool","version":"3.0.0","description":"tool","links":{{"npm":"https://npm.test/tool"}}}}]\n'
  exit 0
fi
exit 23
"#,
        global.path().display()
    );
    let (_directory, executable) = fake_pnpm(&script);
    let packages = manager
        .search(&config(&manager, &executable), "scope")
        .await
        .expect("pnpm search");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "@scope/tool");
    assert_eq!(packages[0].version, "Not Installed");
    assert_eq!(
        packages[0].origin,
        Some(
            PackageOrigin::new("https://registry.example.test/")
                .with_reference("package:@scope/tool")
        )
    );
}

#[cfg(unix)]
#[tokio::test]
async fn writes_freeze_scoped_specs_origins_and_global_commands() {
    let manager = PnpmManager::new();
    let log_dir = tempdir().expect("create log directory");
    let log = log_dir.path().join("argv.log");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "config" ]; then printf 'https://registry.example.test/\n'; exit 0; fi
printf '%s|%s|%s\n' "$1" "$2" "$3" >> '{}'
exit 0
"#,
        log.display()
    );
    let (_directory, executable) = fake_pnpm(&script);
    let config = config(&manager, &executable);
    let mut install = PackageTarget::new(manager.descriptor().id().clone(), "@scope/tool");
    install.scope = PackageScope::User;
    install.version = Some("3.1.0".to_owned());
    install.origin = Some(
        PackageOrigin::new("https://registry.example.test/").with_reference("package:@scope/tool"),
    );
    let mut update = PackageTarget::new(manager.descriptor().id().clone(), "plain");
    update.scope = PackageScope::User;
    update.version = Some("2.0.0".to_owned());
    update.origin =
        Some(PackageOrigin::new("https://registry.example.test/").with_reference("package:plain"));
    let mut uninstall = PackageTarget::new(manager.descriptor().id().clone(), "plain");
    uninstall.scope = PackageScope::User;
    uninstall.origin = Some(PackageOrigin::new("pnpm global").with_reference("package:plain"));
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[install], &sink)
        .await
        .expect("install package");
    manager
        .execute(&config, PackageAction::Update, &[update], &sink)
        .await
        .expect("update package");
    manager
        .execute(&config, PackageAction::Uninstall, &[uninstall], &sink)
        .await
        .expect("remove package");
    assert_eq!(
        fs::read_to_string(log).expect("read argv log"),
        "add|-g|@scope/tool@3.1.0\nadd|-g|plain@2.0.0\nremove|-g|plain\n"
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
async fn malformed_json_and_target_origin_are_protocol_errors() {
    let manager = PnpmManager::new();
    let (_directory, executable) = fake_pnpm("#!/bin/sh\nprintf '{broken json\\n'\n");
    assert_eq!(
        manager
            .installed(&config(&manager, &executable))
            .await
            .expect_err("reject malformed JSON")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let (_directory, executable) = fake_pnpm("#!/bin/sh\nexit 0\n");
    let mut target = PackageTarget::new(manager.descriptor().id().clone(), "@scope/tool");
    target.scope = PackageScope::User;
    target.origin = Some(PackageOrigin::new("pnpm global").with_reference("package:@scope/other"));
    assert_eq!(
        manager
            .execute_target_with_progress(
                &config(&manager, &executable),
                PackageAction::Uninstall,
                &target,
                |_| {},
            )
            .await
            .expect_err("reject mismatched identity")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let malformed = PackageTarget::new(manager.descriptor().id().clone(), "file:../tool");
    assert_eq!(
        manager
            .execute(
                &config(&manager, &executable),
                PackageAction::Install,
                &[malformed],
                &|_| {},
            )
            .await
            .expect_err("reject package spec injection")
            .kind(),
        ManagerErrorKind::Protocol
    );
    assert_eq!(
        manager
            .search(&config(&manager, &executable), "--registry=evil")
            .await
            .expect_err("reject option-like search query")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[tokio::test]
#[ignore = "requires host pnpm and performs read-only global list/outdated probes"]
async fn host_pnpm_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = PnpmManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    let _ = manager.updates(&config, false).await?;
    let _ = manager.search(&config, "pnpm").await?;
    Ok(())
}
