use std::{fs, path::PathBuf};

#[cfg(unix)]
use std::sync::Mutex;

use tempfile::{TempDir, tempdir};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerConfig, PackageAction, PackageManager,
    PackageOrigin, PackageScope, PackageTarget, Platform,
};
#[cfg(unix)]
use updater_manager_api::{ManagerErrorKind, ProgressEvent};
use updater_managers::PipxManager;

#[cfg(unix)]
fn fake_pipx(script: &str) -> (TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempdir().expect("create fake pipx directory");
    let executable = directory.path().join("pipx");
    fs::write(&executable, script).expect("write fake pipx executable");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake pipx executable");
    (directory, executable)
}

#[cfg(windows)]
fn windows_pipx(home: &std::path::Path, log: &std::path::Path) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("create fake pipx directory");
    let executable = directory.path().join("pipx.cmd");
    let script = format!(
        r#"@echo off
if "%1"=="--version" goto version
if "%1"=="environment" goto environment
if "%1"=="list" goto list
if "%1"=="install" goto write
if "%1"=="upgrade" goto write
if "%1"=="uninstall" goto write
exit /b 30

:version
echo 1.15.0
exit /b 0

:environment
echo {}
exit /b 0

:list
echo {{"pipx_spec_version":"0.1","venvs":{{"tool-env":{{"metadata":{{"main_package":{{"package":"Example_Tool","package_or_url":"example-tool==1.0.0","package_version":"1.0.0","pinned":false,"pip_args":[]}}}}}}}}}}
exit /b 0

:write
echo %*>>"{}"
exit /b 0
"#,
        home.display(),
        log.display()
    );
    fs::write(&executable, script).expect("write fake pipx command file");
    (directory, executable)
}

fn config(manager: &PipxManager, executable: &PathBuf) -> ManagerConfig {
    ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable)
}

fn config_with_api(manager: &PipxManager, executable: &PathBuf, api: &str) -> ManagerConfig {
    let mut config = config(manager, executable);
    config.settings = serde_json::json!({ "pypi_api_base_url": api });
    config
}

async fn serve_once(status: &str, body: &str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock PyPI");
    let address = listener.local_addr().expect("mock PyPI address");
    let status = status.to_owned();
    let body = body.to_owned();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept PyPI request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1_024];
        loop {
            let read = stream.read(&mut buffer).await.expect("read PyPI request");
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
            .expect("write PyPI response");
        String::from_utf8(request).expect("request is UTF-8")
    });
    (format!("http://{address}/pypi/"), task)
}

#[cfg(unix)]
fn inventory_pipx(home: &std::path::Path) -> (TempDir, PathBuf) {
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then printf '1.15.0\n'; exit 0; fi
if [ "$1" = "environment" ] && [ "$2" = "--value" ] && [ "$3" = "PIPX_HOME" ]; then printf '{}\n'; exit 0; fi
if [ "$1" = "list" ] && [ "$2" = "--json" ]; then
cat <<'EOF'
{{"pipx_spec_version":"0.1","venvs":{{
  "tool-env":{{"metadata":{{"main_package":{{"package":"Example_Tool","package_or_url":"example-tool==1.0.0","package_version":"1.0.0","pinned":false,"pip_args":[]}}}}}},
  "git-env":{{"metadata":{{"main_package":{{"package":"git-tool","package_or_url":"git+https://example.test/git-tool.git","package_version":"0.4.0"}}}}}}
}}}}
EOF
exit 0
fi
exit 30
"#,
        home.display()
    );
    fake_pipx(&script)
}

#[cfg(unix)]
fn create_inventory_home() -> TempDir {
    let home = tempdir().expect("create PIPX_HOME");
    let tool = home.path().join("venvs/tool-env");
    let git = home.path().join("venvs/git-env");
    fs::create_dir_all(&tool).expect("create registry venv");
    fs::create_dir_all(&git).expect("create git venv");
    fs::write(tool.join("tool.py"), b"12345").expect("write registry venv file");
    fs::write(git.join("git.py"), b"123").expect("write git venv file");
    home
}

#[test]
fn pipx_descriptor_exposes_the_stable_public_contract() {
    let manager = PipxManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:pipx");
    assert_eq!(descriptor.display_name(), "pipx");
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
async fn windows_contract_preserves_venv_paths_identity_pypi_and_write_arguments() {
    let manager = PipxManager::new();
    let home = tempdir().expect("create Windows PIPX_HOME");
    let venv = home.path().join("venvs/tool-env");
    fs::create_dir_all(&venv).expect("create Windows pipx venv");
    fs::write(venv.join("tool.py"), b"12345").expect("write Windows pipx venv file");
    let log = home.path().join("pipx.log");
    let (_directory, executable) = windows_pipx(home.path(), &log);

    let (updates_api, updates_task) = serve_once(
        "200 OK",
        r#"{"info":{"name":"example-tool","version":"2.0.0","summary":"tool"}}"#,
    )
    .await;
    let updates_config = config_with_api(&manager, &executable, &updates_api);
    assert!(
        manager
            .availability(&updates_config)
            .await
            .expect("pipx availability")
            .is_available()
    );
    let packages = manager
        .installed(&updates_config)
        .await
        .expect("pipx inventory");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "tool-env");
    assert_eq!(packages[0].version, "1.0.0");
    assert_eq!(packages[0].size, Some(5));
    assert_eq!(packages[0].scope, PackageScope::User);
    assert_eq!(
        packages[0].origin,
        Some(
            PackageOrigin::new("PyPI")
                .with_reference("registry:venv=tool-env;distribution=Example_Tool")
        )
    );

    let updates = manager
        .updates(&updates_config, false)
        .await
        .expect("pipx updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "tool-env");
    assert_eq!(updates[0].available_version, "2.0.0");
    updates_task.await.expect("pipx update metadata request");

    let (search_api, search_task) = serve_once(
        "200 OK",
        r#"{"info":{"name":"example-tool","version":"2.0.0","summary":"tool"}}"#,
    )
    .await;
    let search = manager
        .search(
            &config_with_api(&manager, &executable, &search_api),
            "Example.Tool",
        )
        .await
        .expect("pipx exact search");
    assert_eq!(search.len(), 1);
    assert_eq!(search[0].name, "example-tool");
    assert_eq!(search[0].version, "1.0.0");
    search_task.await.expect("pipx search metadata request");

    let mut install = PackageTarget::new(manager.descriptor().id().clone(), "example-tool");
    install.scope = PackageScope::User;
    install.version = Some("2.1.0".to_owned());
    install.origin =
        Some(PackageOrigin::new("PyPI").with_reference("registry:distribution=example-tool"));
    manager
        .execute(&updates_config, PackageAction::Install, &[install], &|_| {})
        .await
        .expect("install pipx distribution");
    manager
        .execute(
            &updates_config,
            PackageAction::Update,
            std::slice::from_ref(&updates[0].target),
            &|_| {},
        )
        .await
        .expect("upgrade pipx venv");
    manager
        .execute(
            &updates_config,
            PackageAction::Uninstall,
            &[packages[0].target()],
            &|_| {},
        )
        .await
        .expect("uninstall pipx venv");
    assert_eq!(
        fs::read_to_string(log)
            .expect("pipx Windows write log")
            .lines()
            .collect::<Vec<_>>(),
        [
            "install example-tool==2.1.0",
            "upgrade tool-env",
            "uninstall tool-env",
        ]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn installed_preserves_venv_distribution_source_and_size_identity() {
    let manager = PipxManager::new();
    let home = create_inventory_home();
    let (_directory, executable) = inventory_pipx(home.path());
    let config = config(&manager, &executable);

    let packages = manager.installed(&config).await.expect("pipx inventory");
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].name, "tool-env");
    assert_eq!(packages[0].version, "1.0.0");
    assert_eq!(packages[0].size, Some(5));
    assert_eq!(packages[0].scope, PackageScope::User);
    assert_eq!(
        packages[0].origin,
        Some(
            PackageOrigin::new("PyPI")
                .with_reference("registry:venv=tool-env;distribution=Example_Tool")
        )
    );
    assert_eq!(packages[1].name, "git-env");
    assert_eq!(
        packages[1].homepage.as_deref(),
        Some("https://example.test/git-tool.git")
    );
    assert_eq!(
        packages[1]
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("git:venv=git-env;distribution=git-tool")
    );
    assert_eq!(
        manager
            .current_version(&config, "tool-env")
            .await
            .expect("version by venv"),
        "1.0.0"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn suffix_collision_is_ambiguous_but_symlink_and_missing_metadata_are_rejected() {
    use std::os::unix::fs::symlink;

    let manager = PipxManager::new();
    let home = tempdir().expect("create PIPX_HOME");
    fs::create_dir_all(home.path().join("venvs/a")).expect("create first venv");
    fs::create_dir_all(home.path().join("venvs/b")).expect("create second venv");
    let collision = format!(
        "#!/bin/sh\nif [ \"$1\" = \"environment\" ]; then printf '{}\\n'; else printf '{{\"pipx_spec_version\":\"0.1\",\"venvs\":{{\"a\":{{\"metadata\":{{\"main_package\":{{\"package\":\"some_tool\",\"package_or_url\":\"some_tool\",\"package_version\":\"1\"}}}}}},\"b\":{{\"metadata\":{{\"main_package\":{{\"package\":\"some-tool\",\"package_or_url\":\"some-tool\",\"package_version\":\"2\"}}}}}}}}}}\\n'; fi\n",
        home.path().display()
    );
    let (_directory, executable) = fake_pipx(&collision);
    let collision_config = config(&manager, &executable);
    assert_eq!(
        manager
            .installed(&collision_config)
            .await
            .expect("suffix venvs are valid")
            .len(),
        2
    );
    assert_eq!(
        manager
            .current_version(&collision_config, "some.tool")
            .await
            .expect_err("distribution lookup is ambiguous")
            .kind(),
        ManagerErrorKind::Protocol
    );

    let outside = tempdir().expect("create outside venv");
    let symlink_home = tempdir().expect("create symlink PIPX_HOME");
    fs::create_dir_all(symlink_home.path().join("venvs")).expect("create venv root");
    symlink(outside.path(), symlink_home.path().join("venvs/tool")).expect("create venv symlink");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"environment\" ]; then printf '{}\\n'; else printf '{{\"pipx_spec_version\":\"0.1\",\"venvs\":{{\"tool\":{{\"metadata\":{{\"main_package\":{{\"package\":\"tool\",\"package_or_url\":\"tool\",\"package_version\":\"1\"}}}}}}}}}}\\n'; fi\n",
        symlink_home.path().display()
    );
    let (_directory, executable) = fake_pipx(&script);
    assert_eq!(
        manager
            .installed(&config(&manager, &executable))
            .await
            .expect_err("reject symlink venv")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    let missing = format!(
        "#!/bin/sh\nif [ \"$1\" = \"environment\" ]; then printf '{}\\n'; else printf '{{\"pipx_spec_version\":\"0.1\",\"venvs\":{{\"tool\":{{\"metadata\":{{}}}}}}}}\\n'; fi\n",
        home.path().display()
    );
    let (_directory, executable) = fake_pipx(&missing);
    assert_eq!(
        manager
            .installed(&config(&manager, &executable))
            .await
            .expect_err("reject missing main package")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[cfg(unix)]
#[tokio::test]
async fn updates_query_only_registry_sources_and_freeze_venv_identity() {
    let manager = PipxManager::new();
    let home = create_inventory_home();
    let (_directory, executable) = inventory_pipx(home.path());
    let (api, request_task) = serve_once(
        "200 OK",
        r#"{"info":{"name":"example-tool","version":"2.0.0","summary":"tool","home_page":"https://example.test/home"}}"#,
    )
    .await;

    let updates = manager
        .updates(&config_with_api(&manager, &executable, &api), false)
        .await
        .expect("pipx updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "tool-env");
    assert_eq!(updates[0].available_version, "2.0.0");
    assert_eq!(
        updates[0]
            .target
            .origin
            .as_ref()
            .and_then(|origin| origin.reference.as_deref()),
        Some("registry:venv=tool-env;distribution=Example_Tool")
    );
    let request = request_task.await.expect("mock PyPI request");
    assert!(request.starts_with("GET /pypi/Example_Tool/json HTTP/1.1\r\n"));
}

#[cfg(unix)]
#[tokio::test]
async fn exact_search_handles_404_status_and_malformed_json_distinctly() {
    let manager = PipxManager::new();
    let home = create_inventory_home();
    let (_directory, executable) = inventory_pipx(home.path());

    let (api, task) = serve_once("404 Not Found", r#"{"message":"not found"}"#).await;
    assert!(
        manager
            .search(&config_with_api(&manager, &executable, &api), "missing")
            .await
            .expect("404 is an exact miss")
            .is_empty()
    );
    task.await.expect("404 request");

    let (api, task) = serve_once("503 Service Unavailable", r#"{"message":"busy"}"#).await;
    assert_eq!(
        manager
            .search(&config_with_api(&manager, &executable, &api), "tool")
            .await
            .expect_err("surface server error")
            .kind(),
        ManagerErrorKind::Network
    );
    task.await.expect("status request");

    let (api, task) = serve_once("403 Forbidden", r#"{"message":"forbidden"}"#).await;
    assert_eq!(
        manager
            .search(&config_with_api(&manager, &executable, &api), "tool")
            .await
            .expect_err("surface permission status")
            .kind(),
        ManagerErrorKind::Permission
    );
    task.await.expect("permission request");

    let (api, task) = serve_once("200 OK", "{broken").await;
    assert_eq!(
        manager
            .search(&config_with_api(&manager, &executable, &api), "tool")
            .await
            .expect_err("surface malformed body")
            .kind(),
        ManagerErrorKind::Protocol
    );
    task.await.expect("malformed request");

    let oversized = "x".repeat((2 * 1024 * 1024) + 1);
    let (api, task) = serve_once("200 OK", &oversized).await;
    assert_eq!(
        manager
            .search(&config_with_api(&manager, &executable, &api), "tool")
            .await
            .expect_err("reject oversized body")
            .kind(),
        ManagerErrorKind::Protocol
    );
    task.await.expect("oversized request");
}

#[cfg(unix)]
#[tokio::test]
async fn search_preserves_pypi_canonical_name_and_installed_normalization() {
    let manager = PipxManager::new();
    let home = create_inventory_home();
    let (_directory, executable) = inventory_pipx(home.path());
    let (api, task) = serve_once(
        "200 OK",
        r#"{"info":{"name":"example-tool","version":"2.0.0","summary":"summary","home_page":null,"project_urls":{"Source":"https://example.test/src"}}}"#,
    )
    .await;
    let packages = manager
        .search(
            &config_with_api(&manager, &executable, &api),
            "Example.Tool",
        )
        .await
        .expect("exact PyPI search");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "example-tool");
    assert_eq!(packages[0].version, "1.0.0");
    assert_eq!(
        packages[0].homepage.as_deref(),
        Some("https://example.test/src")
    );
    assert_eq!(
        packages[0].origin,
        Some(PackageOrigin::new("PyPI").with_reference("registry:distribution=example-tool"))
    );
    task.await.expect("search request");
}

#[cfg(unix)]
#[tokio::test]
async fn pinned_registry_and_editable_sources_remain_read_only() {
    let manager = PipxManager::new();
    let home = tempdir().expect("create PIPX_HOME");
    for venv in ["pinned-env", "editable-env"] {
        fs::create_dir_all(home.path().join("venvs").join(venv)).expect("create pipx venv");
    }
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "environment" ]; then printf '{}\n'; exit 0; fi
if [ "$1" = "list" ]; then
printf '%s\n' '{{"pipx_spec_version":"0.1","venvs":{{"pinned-env":{{"metadata":{{"main_package":{{"package":"pinned-tool","package_or_url":"pinned-tool==1.0","package_version":"1.0","pinned":true}}}}}},"editable-env":{{"metadata":{{"main_package":{{"package":"editable-tool","package_or_url":"./editable-tool","package_version":"0.1","pip_args":["--editable"]}}}}}}}}}}'
exit 0
fi
exit 30
"#,
        home.path().display()
    );
    let (_directory, executable) = fake_pipx(&script);
    let config = config_with_api(&manager, &executable, "http://127.0.0.1:9/pypi/");

    let packages = manager.installed(&config).await.expect("pipx inventory");
    assert_eq!(packages.len(), 2);
    assert!(packages.iter().any(|package| {
        package.name == "editable-env"
            && package.origin.as_ref().is_some_and(|origin| {
                origin.reference.as_deref()
                    == Some("editable:venv=editable-env;distribution=editable-tool")
            })
    }));
    assert!(
        manager
            .updates(&config, false)
            .await
            .expect("read-only sources do not query PyPI")
            .is_empty()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn writes_use_distribution_for_install_and_venv_for_upgrade_uninstall() {
    let manager = PipxManager::new();
    let log_dir = tempdir().expect("create log directory");
    let log = log_dir.path().join("argv.log");
    let script = format!(
        "#!/bin/sh\nprintf '%s|%s\\n' \"$1\" \"$2\" >> '{}'\nexit 0\n",
        log.display()
    );
    let (_directory, executable) = fake_pipx(&script);
    let config = config(&manager, &executable);
    let mut install = PackageTarget::new(manager.descriptor().id().clone(), "example-tool");
    install.scope = PackageScope::User;
    install.version = Some("2.1.0".to_owned());
    install.origin =
        Some(PackageOrigin::new("PyPI").with_reference("registry:distribution=example-tool"));
    let mut update = PackageTarget::new(manager.descriptor().id().clone(), "tool-env");
    update.scope = PackageScope::User;
    update.origin = Some(
        PackageOrigin::new("PyPI")
            .with_reference("registry:venv=tool-env;distribution=Example_Tool"),
    );
    let mut uninstall = PackageTarget::new(manager.descriptor().id().clone(), "git-env");
    uninstall.scope = PackageScope::User;
    uninstall.origin =
        Some(PackageOrigin::new("git").with_reference("git:venv=git-env;distribution=git-tool"));
    let events = Mutex::new(Vec::new());
    let sink = |event| events.lock().expect("progress lock").push(event);

    manager
        .execute(&config, PackageAction::Install, &[install], &sink)
        .await
        .expect("install distribution");
    manager
        .execute(&config, PackageAction::Update, &[update], &sink)
        .await
        .expect("upgrade venv");
    manager
        .execute(&config, PackageAction::Uninstall, &[uninstall], &sink)
        .await
        .expect("uninstall venv");
    assert_eq!(
        fs::read_to_string(log).expect("read argv log"),
        "install|example-tool==2.1.0\nupgrade|tool-env\nuninstall|git-env\n"
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
async fn malformed_origin_and_non_registry_upgrade_are_rejected_before_execution() {
    let manager = PipxManager::new();
    let (_directory, executable) = fake_pipx("#!/bin/sh\nexit 40\n");
    let config = config(&manager, &executable);
    let mut target = PackageTarget::new(manager.descriptor().id().clone(), "tool");
    target.scope = PackageScope::User;
    target.name = "other".to_owned();
    target.origin =
        Some(PackageOrigin::new("PyPI").with_reference("registry:venv=other;distribution=tool"));
    assert_eq!(
        manager
            .execute_target_with_progress(&config, PackageAction::Update, &target, |_| {})
            .await
            .expect_err("command failure remains visible")
            .kind(),
        ManagerErrorKind::Other
    );

    target.name = "tool".to_owned();
    target.origin =
        Some(PackageOrigin::new("git").with_reference("git:venv=tool;distribution=tool"));
    assert_eq!(
        manager
            .execute_target_with_progress(&config, PackageAction::Update, &target, |_| {})
            .await
            .expect_err("git upgrade is read-only")
            .kind(),
        ManagerErrorKind::Unsupported
    );

    target.origin = Some(
        PackageOrigin::new("PyPI")
            .with_reference("registry:venv=tool;venv=other;distribution=tool"),
    );
    assert_eq!(
        manager
            .execute_target_with_progress(&config, PackageAction::Uninstall, &target, |_| {})
            .await
            .expect_err("reject mismatched distribution")
            .kind(),
        ManagerErrorKind::Protocol
    );
}

#[tokio::test]
#[ignore = "requires host pipx and performs read-only environment/list probes"]
async fn host_pipx_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = PipxManager::new();
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
#[ignore = "requires live PyPI network access and performs an exact read-only query"]
async fn live_pypi_read_only_smoke_is_explicitly_opt_in()
-> Result<(), updater_manager_api::ManagerError> {
    let manager = PipxManager::new();
    let home = create_inventory_home();
    let (_directory, executable) = inventory_pipx(home.path());
    let packages = manager
        .search(&config(&manager, &executable), "docx2txt")
        .await?;
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "docx2txt");
    Ok(())
}
