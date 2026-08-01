use std::{fs, path::PathBuf, sync::Mutex};

use tempfile::{TempDir, tempdir};
#[cfg(unix)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
#[cfg(unix)]
use updater_manager_api::ManagerErrorKind;
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapability, ManagerConfig, PackageAction,
    PackageManager, PackageOrigin, PackageScope, PackageTarget, Platform, ProgressEvent,
};
use updater_managers::CargoManager;

#[cfg(unix)]
fn fake_cargo(script: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake Cargo directory");
    let executable = directory.path().join("cargo");
    fs::write(&executable, script).expect("write fake Cargo executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake Cargo executable");
    (directory, executable)
}

#[cfg(unix)]
fn inventory_cargo() -> (TempDir, PathBuf) {
    fake_cargo(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf 'cargo 1.90.0 (abc 2026-01-01)\n'
  exit 0
fi
if [ "$1" = "install" ] && [ "$2" = "--list" ]; then
  if [ "${CARGO_TERM_COLOR:-}" != "never" ]; then exit 90; fi
  cat <<'EOF'
ripgrep v14.1.1:
    rg
local-tool v0.2.0 (/work/local-tool):
    local-tool
git-tool v1.0.0 (git+https://example.test/tools.git#deadbeef):
    git-tool
custom-tool v3.0.0 (registry+https://registry.example.test/index):
    custom-tool
vendor-tool v4.0.0 (vendor cache):
    vendor-tool
EOF
  exit 0
fi
exit 2
"#,
    )
}

#[cfg(windows)]
fn windows_cargo(log: &std::path::Path) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("create fake Cargo directory");
    let executable = directory.path().join("cargo.cmd");
    let script = format!(
        r#"@echo off
if "%1"=="--version" (
  echo cargo 1.90.0 ^(abc 2026-01-01^)
  exit /b 0
)
if not "%CARGO_TERM_COLOR%"=="never" exit /b 90
if "%1"=="install" if "%2"=="--list" (
  echo ripgrep v14.1.1:
  echo     rg.exe
  exit /b 0
)
echo %*>>"{}"
exit /b 0
"#,
        log.display()
    );
    fs::write(&executable, script).expect("write fake Cargo command file");
    (directory, executable)
}

#[cfg(unix)]
async fn serve_once(status: &str, body: &str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock registry");
    let address = listener.local_addr().expect("mock registry address");
    let status = status.to_owned();
    let body = body.to_owned();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept registry request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1_024];
        loop {
            let read = stream
                .read(&mut buffer)
                .await
                .expect("read registry request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write registry response");
        String::from_utf8(request).expect("registry request UTF-8")
    });
    (format!("http://{address}/api/v1/"), task)
}

#[cfg(unix)]
async fn serve_truncated_then(body: &str) -> (String, JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind retry registry");
    let address = listener.local_addr().expect("retry registry address");
    let body = body.to_owned();
    let task = tokio::spawn(async move {
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().await.expect("accept retry request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1_024];
            loop {
                let read = stream.read(&mut buffer).await.expect("read retry request");
                request.extend_from_slice(&buffer[..read]);
                if read == 0 || request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            if attempt == 0 {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{")
                    .await
                    .expect("write truncated response");
            } else {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write retry response");
            }
        }
        2
    });
    (format!("http://{address}/api/v1/"), task)
}

#[cfg(unix)]
fn config_with_api(manager: &CargoManager, executable: &PathBuf, api: &str) -> ManagerConfig {
    let mut config =
        ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    config.settings = serde_json::json!({ "api_base_url": api });
    config
}

#[test]
fn cargo_descriptor_exposes_the_stable_public_contract() {
    let manager = CargoManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:cargo");
    assert_eq!(descriptor.display_name(), "Cargo");
    assert_eq!(descriptor.authorization(), &AuthorizationHint::None);
    assert!(descriptor.platforms().contains(Platform::Windows));
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

#[cfg(windows)]
#[tokio::test]
async fn windows_command_contract_preserves_inventory_size_and_write_arguments() {
    let manager = CargoManager::new();
    let workspace = tempdir().expect("create Cargo Windows contract workspace");
    let log = workspace.path().join("cargo.log");
    let (_directory, executable) = windows_cargo(&log);
    let install_root = workspace.path().join("install-root");
    let bin_dir = install_root.join("bin");
    fs::create_dir_all(&bin_dir).expect("create Cargo Windows bin directory");
    fs::write(bin_dir.join("rg.exe"), b"twelve bytes").expect("write Cargo Windows binary");
    let mut config =
        ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    config.settings = serde_json::json!({ "install_root": install_root });

    assert!(matches!(
        manager.availability(&config).await.expect("Cargo availability"),
        ManagerAvailability::Available { version: Some(version) }
            if version.starts_with("cargo 1.90.0")
    ));
    let packages = manager.installed(&config).await.expect("Cargo inventory");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "ripgrep");
    assert_eq!(packages[0].version, "14.1.1");
    assert_eq!(packages[0].size, Some(12));

    let mut registry = PackageTarget::new(manager.descriptor().id().clone(), "ripgrep");
    registry.scope = PackageScope::User;
    registry.version = Some("14.2.0".to_owned());
    registry.origin =
        Some(PackageOrigin::new("crates.io").with_reference("registry:crates.io/ripgrep"));
    let legacy = PackageTarget::new(manager.descriptor().id().clone(), "cargo-edit");
    let mut uninstall = registry.clone();
    uninstall.version = None;

    manager
        .execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&registry),
            &|_| {},
        )
        .await
        .expect("install frozen registry version");
    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&legacy),
            &|_| {},
        )
        .await
        .expect("update legacy target");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            std::slice::from_ref(&uninstall),
            &|_| {},
        )
        .await
        .expect("uninstall registry target");

    assert_eq!(
        fs::read_to_string(log)
            .expect("Cargo Windows write log")
            .lines()
            .collect::<Vec<_>>(),
        [
            "install --version 14.2.0 ripgrep",
            "install --force cargo-edit",
            "uninstall ripgrep",
        ]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn installed_preserves_registry_path_git_and_custom_registry_identity() {
    let manager = CargoManager::new();
    let (_directory, executable) = inventory_cargo();
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);

    assert!(matches!(
        manager.availability(&config).await.expect("Cargo availability"),
        ManagerAvailability::Available { version: Some(version) }
            if version.starts_with("cargo 1.90.0")
    ));
    let packages = manager.installed(&config).await.expect("Cargo inventory");
    assert_eq!(packages.len(), 5);
    assert_eq!(packages[0].name, "ripgrep");
    assert_eq!(packages[0].version, "14.1.1");
    assert_eq!(packages[0].scope, PackageScope::User);
    assert_eq!(
        packages[0].origin,
        Some(PackageOrigin::new("crates.io").with_reference("registry:crates.io/ripgrep"))
    );
    assert_eq!(
        packages[1]
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("path:/work/local-tool")
    );
    assert_eq!(
        packages[2]
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("git:https://example.test/tools.git#deadbeef")
    );
    assert_eq!(
        packages[3]
            .origin
            .as_ref()
            .map(|origin| origin.name.as_str()),
        Some("cargo registry")
    );
    assert_eq!(
        packages[4]
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("other:vendor cache")
    );
    assert_eq!(
        manager.count_installed(&config).await.expect("Cargo count"),
        5
    );
    assert_eq!(
        manager
            .current_version(&config, "local-tool")
            .await
            .expect("local Cargo version"),
        "0.2.0"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn updates_query_only_crates_io_and_freeze_registry_origin() {
    let manager = CargoManager::new();
    let (_directory, executable) = inventory_cargo();
    let (api, request_task) = serve_once(
        "200 OK",
        r#"{"crate":{"name":"ripgrep","max_version":"15.0.0-beta.1","max_stable_version":"14.2.0","description":"search","homepage":null,"repository":"https://example.test/rg"}}"#,
    )
    .await;
    let config = config_with_api(&manager, &executable, &api);

    let updates = manager
        .updates(&config, false)
        .await
        .expect("Cargo updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "ripgrep");
    assert_eq!(updates[0].current_version, "14.1.1");
    assert_eq!(updates[0].available_version, "14.2.0");
    assert_eq!(updates[0].target.scope, PackageScope::User);
    assert_eq!(
        updates[0]
            .target
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("registry:crates.io/ripgrep")
    );
    let request = request_task.await.expect("mock request task");
    assert!(request.starts_with("GET /api/v1/crates/ripgrep HTTP/1.1\r\n"));
}

#[cfg(unix)]
#[tokio::test]
async fn search_uses_structured_query_encoding_and_typed_schema() {
    let manager = CargoManager::new();
    let (_directory, executable) = inventory_cargo();
    let (api, request_task) = serve_once(
        "200 OK",
        r#"{"crates":[{"name":"cargo-edit","max_version":"0.14.0-beta.1","max_stable_version":"0.13.7","description":"edit Cargo.toml","homepage":null,"repository":"https://example.test/edit"}]}"#,
    )
    .await;
    let config = config_with_api(&manager, &executable, &api);

    let packages = manager
        .search(&config, "cargo edit & tools")
        .await
        .expect("Cargo search");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "cargo-edit");
    assert_eq!(packages[0].version, "Not Installed");
    assert_eq!(
        packages[0].homepage.as_deref(),
        Some("https://example.test/edit")
    );
    let request = request_task.await.expect("mock request task");
    let request_line = request.lines().next().expect("HTTP request line");
    assert!(request_line.contains("q=cargo+edit+%26+tools"));
    assert!(request_line.contains("page=1"));
    assert!(request_line.contains("per_page=50"));
}

#[cfg(unix)]
#[tokio::test]
async fn registry_status_and_schema_failures_are_not_swallowed() {
    let manager = CargoManager::new();
    let (_directory, executable) = inventory_cargo();

    let (busy_api, busy_task) =
        serve_once("429 Too Many Requests", r#"{"error":"slow down"}"#).await;
    let busy = manager
        .search(&config_with_api(&manager, &executable, &busy_api), "cargo")
        .await
        .expect_err("surface registry rate limit");
    assert_eq!(busy.kind(), ManagerErrorKind::Busy);
    busy_task.await.expect("busy request task");

    let (policy_api, policy_task) =
        serve_once("403 Forbidden", r#"{"error":"data access policy"}"#).await;
    let policy = manager
        .search(
            &config_with_api(&manager, &executable, &policy_api),
            "cargo",
        )
        .await
        .expect_err("surface registry policy refusal");
    assert_eq!(policy.kind(), ManagerErrorKind::Busy);
    policy_task.await.expect("policy request task");

    let (schema_api, schema_task) = serve_once(
        "200 OK",
        r#"{"crates":[{"name":"cargo-edit","description":"missing version"}]}"#,
    )
    .await;
    let schema = manager
        .search(
            &config_with_api(&manager, &executable, &schema_api),
            "cargo",
        )
        .await
        .expect_err("surface invalid registry schema");
    assert_eq!(schema.kind(), ManagerErrorKind::Protocol);
    schema_task.await.expect("schema request task");
}

#[cfg(unix)]
#[tokio::test]
async fn transient_response_body_failure_is_retried_once() {
    let manager = CargoManager::new();
    let (_directory, executable) = inventory_cargo();
    let (api, request_task) = serve_truncated_then(
        r#"{"crates":[{"name":"bluetui","max_version":"0.8.0","max_stable_version":"0.8.0"}]}"#,
    )
    .await;
    let config = config_with_api(&manager, &executable, &api);

    let packages = manager
        .search(&config, "bluetui")
        .await
        .expect("retry transient response body failure");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "bluetui");
    assert_eq!(request_task.await.expect("retry request task"), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn installed_size_uses_the_configured_install_root() {
    let manager = CargoManager::new();
    let (_directory, executable) = inventory_cargo();
    let install_root = tempdir().expect("create Cargo install root");
    let bin_dir = install_root.path().join("bin");
    fs::create_dir(&bin_dir).expect("create Cargo bin directory");
    fs::write(bin_dir.join("rg"), b"twelve bytes").expect("write installed binary");
    let mut config =
        ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    config.settings = serde_json::json!({ "install_root": install_root.path() });

    let packages = manager.installed(&config).await.expect("Cargo inventory");
    assert_eq!(packages[0].name, "ripgrep");
    assert_eq!(packages[0].size, Some(12));
}

#[cfg(unix)]
#[tokio::test]
async fn execute_freezes_registry_version_and_local_write_boundaries() {
    let manager = CargoManager::new();
    let (_directory, executable) = fake_cargo(
        r#"#!/bin/sh
if [ "${CARGO_TERM_COLOR:-}" != "never" ]; then exit 90; fi
printf '%s\n' "$*" >> "$0.log"
exit 0
"#,
    );
    let log = executable.with_extension("log");
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(&executable);
    let mut registry = PackageTarget::new(manager.descriptor().id().clone(), "ripgrep");
    registry.scope = PackageScope::User;
    registry.version = Some("14.2.0".to_owned());
    registry.origin =
        Some(PackageOrigin::new("crates.io").with_reference("registry:crates.io/ripgrep"));
    let legacy = PackageTarget::new(manager.descriptor().id().clone(), "cargo-edit");
    let mut local = PackageTarget::new(manager.descriptor().id().clone(), "local-tool");
    local.scope = PackageScope::User;
    local.origin = Some(PackageOrigin::new("path").with_reference("path:/work/local-tool"));

    manager
        .execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&registry),
            &|_| {},
        )
        .await
        .expect("install frozen registry version");
    manager
        .execute(
            &config,
            PackageAction::Update,
            std::slice::from_ref(&legacy),
            &|_| {},
        )
        .await
        .expect("update legacy target");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            std::slice::from_ref(&local),
            &|_| {},
        )
        .await
        .expect("uninstall typed local target");
    assert_eq!(
        fs::read_to_string(log).expect("Cargo write log"),
        "install --version 14.2.0 ripgrep\n\
         install --force cargo-edit\n\
         uninstall local-tool\n"
    );

    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);
    assert_eq!(
        manager
            .execute(
                &config,
                PackageAction::Update,
                std::slice::from_ref(&local),
                &sink,
            )
            .await
            .expect_err("reject local package registry update")
            .kind(),
        ManagerErrorKind::Unsupported
    );
    assert!(events.lock().expect("progress lock").is_empty());
}

#[tokio::test]
async fn empty_execution_emits_stable_boundaries() {
    let manager = CargoManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);
    manager
        .execute(&config, PackageAction::Install, &[], &sink)
        .await
        .expect("empty Cargo execution");
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
            }
        ]
    );
}

#[tokio::test]
#[ignore = "machine-specific host availability and installed inventory smoke test"]
async fn host_cargo_read_only_smoke() -> Result<(), updater_manager_api::ManagerError> {
    let manager = CargoManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());
    assert!(manager.availability(&config).await?.is_available());

    let installed = manager.installed(&config).await?;
    assert_eq!(manager.count_installed(&config).await?, installed.len());
    if let Some(package) = installed.first() {
        assert_eq!(
            manager.current_version(&config, &package.name).await?,
            package.version
        );
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires read-only access to the live crates.io API"]
async fn crates_io_bluetui_detail_read_only_smoke() -> Result<(), updater_manager_api::ManagerError>
{
    let manager = CargoManager::new();
    let (_directory, executable) = fake_cargo(
        r#"#!/bin/sh
if [ "$1" = "install" ] && [ "$2" = "--list" ]; then
  printf 'bluetui v0.8.0:\n    bluetui\n'
  exit 0
fi
exit 2
"#,
    );
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);

    let updates = manager.updates(&config, false).await?;
    assert!(updates.is_empty());
    Ok(())
}
