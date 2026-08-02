use updater_manager_api::{AuthorizationHint, ManagerCapability, PackageManager, Platform};
use updater_managers::NixProfileManager;

#[cfg(unix)]
use serde_json::{Value, json};
#[cfg(unix)]
use std::{fs, path::PathBuf};
#[cfg(unix)]
use tempfile::{TempDir, tempdir};
#[cfg(unix)]
use updater_manager_api::{
    ManagerConfig, ManagerErrorKind, PackageAction, PackageOrigin, PackageScope, PackageTarget,
};

#[cfg(unix)]
static NIX_CONTRACT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(unix)]
fn config(
    manager: &NixProfileManager,
    executable: &PathBuf,
    profile: &std::path::Path,
) -> ManagerConfig {
    let mut config =
        ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    config.settings = json!({ "profile": profile });
    config
}

#[cfg(unix)]
fn manifest() -> &'static str {
    r#"{"version":3,"elements":{"hello":{"active":true,"priority":5,"storePaths":["/nix/store/0123456789abcdfghijklmnpqrsvwxyz-hello-2.12.2"],"originalUrl":"flake:nixpkgs","url":"github:NixOS/nixpkgs/7b38b03d76ab71bdc8dc325e3f6338d984cc35ca","attrPath":"legacyPackages.x86_64-linux.hello","outputs":["out"]},"local-tool":{"active":true,"priority":5,"storePaths":["/nix/store/0123456789abcdfghijklmnpqrsvwxyz-local-tool-1.0"]},"pinned":{"active":true,"priority":5,"storePaths":["/nix/store/0123456789abcdfghijklmnpqrsvwxyz-pinned-3.0"],"originalUrl":"github:NixOS/nixpkgs/0123456789abcdef0123456789abcdef01234567","url":"github:NixOS/nixpkgs/0123456789abcdef0123456789abcdef01234567","attrPath":"legacyPackages.x86_64-linux.pinned","outputs":["out"]}}}"#
}

#[cfg(unix)]
fn fake_nix(log: &std::path::Path) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Nix directory");
    let executable = directory.path().join("nix");
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
if [ "$1" = "--version" ]; then printf 'nix (Nix) 2.32.9\n'; exit 0; fi
if [ "$1" = "profile" ] && [ "$2" = "list" ]; then printf '%s\n' '{}'; exit 0; fi
if [ "$1" = "profile" ] && {{ [ "$2" = "install" ] || [ "$2" = "upgrade" ] || [ "$2" = "remove" ]; }}; then exit 0; fi
exit 42
"#,
        log.display(),
        manifest()
    );
    fs::write(&executable, script).expect("write fake Nix executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Nix executable");
    (directory, executable)
}

#[test]
fn descriptor_advertises_explicit_single_profile_capabilities() {
    let manager = NixProfileManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:nix-profile");
    assert_eq!(descriptor.display_name(), "Nix profile");
    assert_eq!(descriptor.authorization(), &AuthorizationHint::None);
    assert!(descriptor.platforms().contains(Platform::Linux));
    assert!(descriptor.platforms().contains(Platform::MacOs));
    assert!(!descriptor.platforms().contains(Platform::Windows));
    for capability in [
        ManagerCapability::Installed,
        ManagerCapability::Install,
        ManagerCapability::Update,
        ManagerCapability::Uninstall,
    ] {
        assert!(descriptor.capabilities().contains(capability));
    }
    assert!(
        !descriptor
            .capabilities()
            .contains(ManagerCapability::Updates)
    );
    assert!(
        !descriptor
            .capabilities()
            .contains(ManagerCapability::Search)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn native_contract_preserves_profile_source_identity_and_write_argv() {
    let _guard = NIX_CONTRACT_LOCK.lock().await;
    let manager = NixProfileManager::new();
    let workspace = tempdir().expect("create Nix contract workspace");
    let profile = workspace.path().join("profiles/current");
    let log = workspace.path().join("nix.log");
    let (_directory, executable) = fake_nix(&log);
    let config = config(&manager, &executable, &profile);

    assert!(
        manager
            .availability(&config)
            .await
            .expect("Nix availability")
            .is_available()
    );
    let installed = manager.installed(&config).await.expect("Nix inventory");
    assert_eq!(installed.len(), 3);
    assert_eq!(
        manager.count_installed(&config).await.expect("Nix count"),
        3
    );
    let hello = installed
        .iter()
        .find(|package| package.name == "hello")
        .expect("hello element");
    assert_eq!(hello.version, "2.12.2");
    assert_eq!(hello.scope, PackageScope::User);
    let reference: Value = serde_json::from_str(
        hello
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref())
            .expect("typed origin"),
    )
    .expect("parse Nix origin");
    assert_eq!(reference["profile"], profile.to_string_lossy().as_ref());
    assert_eq!(reference["original_url"], "flake:nixpkgs");
    assert_eq!(reference["attr_path"], "legacyPackages.x86_64-linux.hello");

    let installable = "nixpkgs#ripgrep";
    let mut install = PackageTarget::new(manager.descriptor().id().clone(), installable);
    install.scope = PackageScope::User;
    install.origin = Some(PackageOrigin::new("Nix profile").with_reference(
        json!({"kind":"installable","profile":profile,"installable":installable}).to_string(),
    ));
    manager
        .execute(&config, PackageAction::Install, &[install], &|_| {})
        .await
        .expect("install Nix package");
    manager
        .execute(&config, PackageAction::Update, &[hello.target()], &|_| {})
        .await
        .expect("upgrade Nix package");
    let local = installed
        .iter()
        .find(|package| package.name == "local-tool")
        .expect("local element");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            &[local.target()],
            &|_| {},
        )
        .await
        .expect("remove Nix package");

    let normalized = fs::read_to_string(log)
        .expect("read Nix argv log")
        .replace('\\', "/");
    let profile = profile.to_string_lossy().replace('\\', "/");
    assert!(normalized.contains(&format!("profile list --json --profile {profile}")));
    assert!(normalized.contains(&format!(
        "profile install nixpkgs#ripgrep --profile {profile}"
    )));
    assert!(normalized.contains(&format!("profile upgrade hello --profile {profile}")));
    assert!(normalized.contains(&format!("profile remove local-tool --profile {profile}")));
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_missing_profile_locked_update_and_forged_origin() {
    let _guard = NIX_CONTRACT_LOCK.lock().await;
    let manager = NixProfileManager::new();
    let empty = ManagerConfig::new(manager.descriptor().id().clone());
    assert_eq!(
        manager
            .availability(&empty)
            .await
            .expect_err("profile is required")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let workspace = tempdir().expect("create Nix validation workspace");
    let profile = workspace.path().join("profiles/current");
    let log = workspace.path().join("nix.log");
    let (_directory, executable) = fake_nix(&log);
    let config = config(&manager, &executable, &profile);
    let installed = manager.installed(&config).await.expect("Nix inventory");
    let pinned = installed
        .iter()
        .find(|package| package.name == "pinned")
        .expect("pinned element");
    assert_eq!(
        manager
            .execute(&config, PackageAction::Update, &[pinned.target()], &|_| {})
            .await
            .expect_err("locked source must not update")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let hello = installed
        .iter()
        .find(|package| package.name == "hello")
        .expect("hello element");
    let mut forged = hello.target();
    let origin = forged.origin.as_mut().expect("typed Nix origin");
    let mut reference: Value =
        serde_json::from_str(origin.reference.as_deref().expect("origin reference"))
            .expect("parse origin");
    reference["store_paths"] = json!(["/nix/store/0123456789abcdfghijklmnpqrsvwxyz-forged-9.9"]);
    origin.reference = Some(reference.to_string());
    assert_eq!(
        manager
            .execute(&config, PackageAction::Uninstall, &[forged], &|_| {})
            .await
            .expect_err("forged store path must fail")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires host Nix and UPDATER_NIX_PROFILE; performs read-only availability and profile listing"]
async fn host_nix_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let _guard = NIX_CONTRACT_LOCK.lock().await;
    let manager = NixProfileManager::new();
    let profile = std::env::var("UPDATER_NIX_PROFILE").map_err(|error| {
        updater_manager_api::ManagerError::new(
            updater_manager_api::ManagerErrorKind::Protocol,
            "UPDATER_NIX_PROFILE is required for the host smoke",
        )
        .with_detail(error.to_string())
    })?;
    let mut config = ManagerConfig::new(manager.descriptor().id().clone());
    config.settings = json!({"profile": profile});
    assert!(manager.availability(&config).await?.is_available());
    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    Ok(())
}
