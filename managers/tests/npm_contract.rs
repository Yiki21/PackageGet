use std::{fs, path::PathBuf, sync::Mutex};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapability, ManagerConfig, ManagerErrorKind,
    PackageAction, PackageManager, PackageOrigin, PackageScope, PackageTarget, ProgressEvent,
};
use updater_managers::NpmManager;

#[cfg(unix)]
fn fake_npm(script: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake npm directory");
    let executable = directory.path().join("npm");
    fs::write(&executable, script).expect("write fake npm executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake npm executable");
    (directory, executable)
}

fn config(manager: &NpmManager, executable: &PathBuf) -> ManagerConfig {
    ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable)
}

#[test]
fn npm_descriptor_exposes_the_stable_public_contract() {
    let manager = NpmManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:npm");
    assert_eq!(descriptor.display_name(), "npm");
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
async fn installed_uses_typed_dependencies_and_preserves_scoped_identity() {
    let manager = NpmManager::new();
    let fixture = tempdir().expect("create npm root fixture");
    let root = fixture.path().join("lib/node_modules");
    let scoped = root.join("@scope/tool");
    let plain = root.join("plain-tool");
    fs::create_dir_all(&scoped).expect("create scoped package");
    fs::create_dir_all(&plain).expect("create plain package");
    fs::write(scoped.join("index.js"), b"scope").expect("write scoped package");
    fs::write(plain.join("index.js"), b"plain-package").expect("write plain package");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then printf '12.0.1\n'; exit 0; fi
if [ "$1" = "root" ] && [ "$2" = "-g" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$1" = "ls" ] && [ "$2" = "-g" ] && [ "$3" = "--depth=0" ] && [ "$4" = "--json" ] && [ "$5" = "--long" ]; then
cat <<'EOF'
{{"dependencies":{{"plain-tool":{{"name":"plain-tool","version":"2.0.0","_id":"plain-tool@2.0.0","path":"{}","description":"plain","homepage":"https://example.test/plain"}},"@scope/tool":{{"name":"@scope/tool","version":"1.2.3","_id":"@scope/tool@1.2.3","path":"{}","description":"scoped","repository":{{"type":"git","url":"git+https://example.test/scoped.git"}}}}}}}}
EOF
exit 0
fi
exit 19
"#,
        root.display(),
        plain.display(),
        scoped.display()
    );
    let (_directory, executable) = fake_npm(&script);
    let config = config(&manager, &executable);

    assert!(matches!(
        manager.availability(&config).await.expect("npm availability"),
        ManagerAvailability::Available { version: Some(version) } if version == "12.0.1"
    ));
    let packages = manager.installed(&config).await.expect("npm inventory");
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].name, "@scope/tool");
    assert_eq!(packages[0].version, "1.2.3");
    assert_eq!(packages[0].size, Some(5));
    assert_eq!(
        packages[0].homepage.as_deref(),
        Some("https://example.test/scoped.git")
    );
    assert_eq!(packages[0].scope, PackageScope::User);
    assert_eq!(
        packages[0].origin,
        Some(PackageOrigin::new("npm global").with_reference("package:@scope/tool"))
    );
    assert_eq!(packages[1].name, "plain-tool");
    assert_eq!(packages[1].size, Some(13));
    assert_eq!(
        manager.count_installed(&config).await.expect("npm count"),
        2
    );
    assert_eq!(
        manager
            .current_version(&config, "@scope/tool")
            .await
            .expect("scoped version"),
        "1.2.3"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn installed_symlink_is_read_only_for_size_and_escape_is_rejected() {
    use std::os::unix::fs::symlink;

    let manager = NpmManager::new();
    let fixture = tempdir().expect("create npm root fixture");
    let root = fixture.path().join("node_modules");
    let outside = fixture.path().join("outside");
    fs::create_dir_all(&root).expect("create npm root");
    fs::create_dir_all(&outside).expect("create outside package");
    fs::write(outside.join("index.js"), b"outside").expect("write outside package");
    symlink(&outside, root.join("linked-tool")).expect("link global package");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "root" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$1" = "ls" ]; then printf '%s\n' '{{"dependencies":{{"linked-tool":{{"name":"linked-tool","version":"1.0.0","path":"{}"}}}}}}'; exit 0; fi
exit 2
"#,
        root.display(),
        root.join("linked-tool").display()
    );
    let (_directory, executable) = fake_npm(&script);
    let packages = manager
        .installed(&config(&manager, &executable))
        .await
        .expect("linked inventory");
    assert_eq!(packages[0].size, None);

    let escaping_script = format!(
        r#"#!/bin/sh
if [ "$1" = "root" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$1" = "ls" ]; then printf '%s\n' '{{"dependencies":{{"linked-tool":{{"name":"linked-tool","version":"1.0.0","path":"{}"}}}}}}'; exit 0; fi
exit 2
"#,
        root.display(),
        outside.display()
    );
    let (_other_directory, other_executable) = fake_npm(&escaping_script);
    assert_eq!(
        manager
            .installed(&config(&manager, &other_executable))
            .await
            .expect_err("reject escaped npm path")
            .kind(),
        ManagerErrorKind::Permission
    );
}

#[cfg(unix)]
#[tokio::test]
async fn outdated_accepts_only_npm_no_update_and_update_status_contracts() {
    let manager = NpmManager::new();
    let fixture = tempdir().expect("create npm root fixture");
    let root = fixture.path().join("node_modules");
    fs::create_dir_all(root.join("@scope/tool")).expect("create installed package");
    let update_script = format!(
        r#"#!/bin/sh
if [ "$1" = "config" ]; then printf 'https://registry.example.test/\n'; exit 0; fi
if [ "$1" = "root" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$1" = "outdated" ]; then
  printf '%s\n' '{{"@scope/tool":{{"current":"1.0.0","wanted":"1.1.0","latest":"2.0.0","dependent":"global","location":"{}"}}}}'
  exit 1
fi
exit 2
"#,
        root.display(),
        root.join("@scope/tool").display()
    );
    let (_directory, executable) = fake_npm(&update_script);
    let updates = manager
        .updates(&config(&manager, &executable), false)
        .await
        .expect("npm updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "@scope/tool");
    assert_eq!(updates[0].current_version, "1.0.0");
    assert_eq!(updates[0].available_version, "2.0.0");
    assert_eq!(updates[0].target.version.as_deref(), Some("2.0.0"));
    assert_eq!(
        updates[0].target.origin,
        Some(
            PackageOrigin::new("https://registry.example.test/")
                .with_reference("package:@scope/tool")
        )
    );

    let no_update_script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"config\" ]; then printf 'https://registry.example.test/\\n'; exit 0; fi\nif [ \"$1\" = \"root\" ]; then printf '%s\\n' '{}'; exit 0; fi\nif [ \"$1\" = \"outdated\" ]; then printf '{{}}\\n'; exit 0; fi\nexit 2\n",
        root.display()
    );
    let (_no_update_directory, no_update_executable) = fake_npm(&no_update_script);
    assert!(
        manager
            .updates(&config(&manager, &no_update_executable), false)
            .await
            .expect("no npm updates")
            .is_empty()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn outdated_rejects_success_with_updates_and_ambiguous_arrays() {
    let manager = NpmManager::new();
    let fixture = tempdir().expect("create npm root fixture");
    let root = fixture.path().join("node_modules");
    fs::create_dir_all(root.join("tool")).expect("create package path");
    let detail = format!(
        r#"{{"current":"1.0.0","wanted":"1.1.0","latest":"1.1.0","dependent":"global","location":"{}"}}"#,
        root.join("tool").display()
    );
    let success_script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"config\" ]; then echo https://registry.example.test/; exit 0; fi\nif [ \"$1\" = \"root\" ]; then echo '{}'; exit 0; fi\nif [ \"$1\" = \"outdated\" ]; then echo '{{\"tool\":{detail}}}'; exit 0; fi\nexit 2\n",
        root.display()
    );
    let (_directory, executable) = fake_npm(&success_script);
    assert_eq!(
        manager
            .updates(&config(&manager, &executable), false)
            .await
            .expect_err("reject incorrect npm outdated status")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let array_script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"config\" ]; then echo https://registry.example.test/; exit 0; fi\nif [ \"$1\" = \"root\" ]; then echo '{}'; exit 0; fi\nif [ \"$1\" = \"outdated\" ]; then echo '{{\"tool\":[{detail},{detail}]}}'; exit 1; fi\nexit 2\n",
        root.display()
    );
    let (_array_directory, array_executable) = fake_npm(&array_script);
    assert_eq!(
        manager
            .updates(&config(&manager, &array_executable), false)
            .await
            .expect_err("reject ambiguous npm outdated identity")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[cfg(unix)]
#[tokio::test]
async fn search_uses_typed_array_registry_origin_and_option_delimiter() {
    let manager = NpmManager::new();
    let fixture = tempdir().expect("create search fixture");
    let log = fixture.path().join("search.log");
    let root = fixture.path().join("node_modules");
    fs::create_dir_all(&root).expect("create empty npm root");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "config" ] && [ "$2" = "get" ] && [ "$3" = "registry" ]; then printf 'https://registry.example.test/npm\n'; exit 0; fi
if [ "$1" = "root" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$1" = "ls" ]; then printf '%s\n' '{{"dependencies":{{}}}}'; exit 0; fi
if [ "$1" = "search" ]; then
  printf '%s|%s|%s|%s\n' "$1" "$2" "$3" "$4" > '{}'
  printf '%s\n' '[{{"name":"z-tool","version":"2.0.0","description":"z"}},{{"name":"@scope/tool","version":"1.2.3","description":"scoped","links":{{"homepage":"https://example.test/tool"}}}}]'
  exit 0
fi
exit 2
"#,
        root.display(),
        log.display()
    );
    let (_directory, executable) = fake_npm(&script);
    let packages = manager
        .search(&config(&manager, &executable), "@scope/tool")
        .await
        .expect("npm search");
    assert_eq!(
        fs::read_to_string(log).expect("read search argv").trim(),
        "search|--json|--|@scope/tool"
    );
    assert_eq!(packages[0].name, "@scope/tool");
    assert_eq!(packages[0].version, "Not Installed");
    assert_eq!(
        packages[0].homepage.as_deref(),
        Some("https://example.test/tool")
    );
    assert_eq!(
        packages[0].origin,
        Some(
            PackageOrigin::new("https://registry.example.test/npm/")
                .with_reference("package:@scope/tool")
        )
    );
    assert_eq!(packages[1].name, "z-tool");
}

#[cfg(unix)]
#[tokio::test]
async fn typed_writes_use_exact_scoped_spec_and_validate_registry() {
    let manager = NpmManager::new();
    let fixture = tempdir().expect("create write fixture");
    let log = fixture.path().join("write.log");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "config" ]; then printf 'https://registry.example.test/\n'; exit 0; fi
if [ "$1" = "install" ]; then printf '%s|%s|%s\n' "$1" "$2" "$3" > '{}'; exit 0; fi
if [ "$1" = "uninstall" ]; then printf '%s|%s|%s\n' "$1" "$2" "$3" > '{}'; exit 0; fi
exit 2
"#,
        log.display(),
        log.display()
    );
    let (_directory, executable) = fake_npm(&script);
    let config = config(&manager, &executable);
    let mut target = PackageTarget::new(manager.descriptor().id().clone(), "@scope/tool");
    target.version = Some("2.3.4-beta.1".to_owned());
    target.scope = PackageScope::User;
    target.origin = Some(
        PackageOrigin::new("https://registry.example.test/").with_reference("package:@scope/tool"),
    );
    let events = Mutex::new(Vec::new());

    manager
        .execute(
            &config,
            PackageAction::Update,
            &[target.clone()],
            &|event| {
                events.lock().expect("progress lock").push(event);
            },
        )
        .await
        .expect("update typed npm target");
    assert_eq!(
        fs::read_to_string(&log).expect("read update argv").trim(),
        "install|-g|@scope/tool@2.3.4-beta.1"
    );
    assert!(matches!(
        events.lock().expect("progress lock").last(),
        Some(ProgressEvent::Finished {
            completed: 1,
            total: 1
        })
    ));

    target.version = None;
    target.origin = Some(
        PackageOrigin::new("https://another.example.test/").with_reference("package:@scope/tool"),
    );
    assert_eq!(
        manager
            .execute(&config, PackageAction::Update, &[target], &|_| {})
            .await
            .expect_err("reject mismatched npm registry")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[cfg(unix)]
#[tokio::test]
async fn installed_origin_allows_only_uninstall_and_legacy_update_uses_latest() {
    let manager = NpmManager::new();
    let fixture = tempdir().expect("create write fixture");
    let log = fixture.path().join("write.log");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"install\" ] || [ \"$1\" = \"uninstall\" ]; then printf '%s|%s|%s\\n' \"$1\" \"$2\" \"$3\" > '{}'; exit 0; fi\nexit 2\n",
        log.display()
    );
    let (_directory, executable) = fake_npm(&script);
    let config = config(&manager, &executable);
    let mut installed = PackageTarget::new(manager.descriptor().id().clone(), "tool");
    installed.scope = PackageScope::User;
    installed.origin = Some(PackageOrigin::new("npm global").with_reference("package:tool"));
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            &[installed.clone()],
            &|_| {},
        )
        .await
        .expect("uninstall installed npm target");
    assert_eq!(
        fs::read_to_string(&log)
            .expect("read uninstall argv")
            .trim(),
        "uninstall|-g|tool"
    );
    assert_eq!(
        manager
            .execute(&config, PackageAction::Update, &[installed], &|_| {})
            .await
            .expect_err("installed npm origin is not replayable")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    let legacy = PackageTarget::new(manager.descriptor().id().clone(), "@scope/tool");
    manager
        .execute(&config, PackageAction::Update, &[legacy], &|_| {})
        .await
        .expect("update legacy npm target");
    assert_eq!(
        fs::read_to_string(log).expect("read legacy argv").trim(),
        "install|-g|@scope/tool@latest"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_name_and_protocol_output_are_not_silently_accepted() {
    let manager = NpmManager::new();
    let fixture = tempdir().expect("create protocol fixture");
    let root = fixture.path().join("node_modules");
    fs::create_dir_all(&root).expect("create npm root");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"root\" ]; then echo '{}'; exit 0; fi\nif [ \"$1\" = \"ls\" ]; then echo '{{\"dependencies\":[]}}'; exit 0; fi\nexit 2\n",
        root.display()
    );
    let (_directory, executable) = fake_npm(&script);
    assert_eq!(
        manager
            .installed(&config(&manager, &executable))
            .await
            .expect_err("reject npm dependencies array")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let target = PackageTarget::new(manager.descriptor().id().clone(), "--registry=evil");
    assert_eq!(
        manager
            .execute(
                &config(&manager, &executable),
                PackageAction::Install,
                &[target],
                &|_| {},
            )
            .await
            .expect_err("reject option-like package name")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[tokio::test]
#[ignore = "requires host npm and performs read-only global inventory and registry probes"]
async fn host_npm_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = NpmManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    let _ = manager.updates(&config, false).await?;
    let _ = manager.search(&config, "npm").await?;
    Ok(())
}
