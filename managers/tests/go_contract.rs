use std::{fs, path::PathBuf};

#[cfg(unix)]
use std::sync::Mutex;

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId,
    ManagerResult, PackageAction, PackageManager, PackageTarget, PackageUpdate, Platform,
    SupportedPlatforms,
};
#[cfg(unix)]
use updater_manager_api::{NoopProgressSink, PackageOrigin, PackageScope, ProgressEvent};
use updater_managers::{CargoManager, GoManager, PipxManager};

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

#[cfg(windows)]
fn windows_go(log: &std::path::Path) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("create fake Go directory");
    let executable = directory.path().join("go.cmd");
    let script = format!(
        r#"@echo off
if "%1"=="version" if "%2"=="" (
  echo go version go1.24.0 windows/amd64
  exit /b 0
)
if "%1"=="version" if "%2"=="-m" if "%3"=="-json" (
  echo {{"Path":"example.com/mod/cmd/tool","Main":{{"Path":"example.com/mod","Version":"v1.0.0"}}}}
  exit /b 0
)
if "%1"=="list" if "%2"=="-m" if "%3"=="-json" if "%4"=="example.com/mod@latest" (
  echo {{"Path":"example.com/mod","Version":"v1.1.0"}}
  exit /b 0
)
if "%1"=="install" (
  echo %GOBIN%^|%2>>"{}"
  exit /b 0
)
exit /b 19
"#,
        log.display()
    );
    fs::write(&executable, script).expect("write fake Go command file");
    (directory, executable)
}

fn config(manager: &GoManager, executable: &PathBuf, bin: &std::path::Path) -> ManagerConfig {
    let mut config =
        ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    config.settings = serde_json::json!({ "go_bin_dir": bin });
    config
}

fn write_binary(path: impl AsRef<std::path::Path>, contents: &[u8]) {
    fs::write(path.as_ref(), contents).expect("write binary fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path.as_ref(), fs::Permissions::from_mode(0o755))
            .expect("mark binary fixture executable");
    }
}

#[test]
fn go_descriptor_labels_search_as_exact_identifier_lookup() {
    let manager = GoManager::new();
    assert!(manager.descriptor().exact_lookup());
}

/// The UI reads `exact_lookup` to label a Discover source as exact-identifier
/// lookup instead of hard-coding manager IDs.
#[test]
fn exact_lookup_is_advertised_by_exact_identifier_sources_only() {
    assert!(GoManager::new().descriptor().exact_lookup());
    assert!(PipxManager::new().descriptor().exact_lookup());
    assert!(!CargoManager::new().descriptor().exact_lookup());
}

#[test]
fn go_descriptor_exposes_the_stable_public_contract() {
    let manager = GoManager::new();
    let descriptor = manager.descriptor();
    assert_eq!(descriptor.id().as_str(), "builtin:go");
    assert_eq!(descriptor.display_name(), "Go");
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
async fn windows_contract_preserves_logical_identity_and_executable_removal() {
    let manager = GoManager::new();
    let workspace = tempdir().expect("create Go Windows contract workspace");
    let bin = workspace.path().join("go-bin");
    fs::create_dir_all(&bin).expect("create GOBIN");
    write_binary(bin.join("tool.exe"), b"four");
    fs::write(bin.join(".gup.lock"), b"").expect("write gup lock");
    fs::write(bin.join("README.txt"), b"installed tools").expect("write non-executable file");
    let log = workspace.path().join("go.log");
    let (_directory, executable) = windows_go(&log);
    let config = config(&manager, &executable, &bin);

    assert!(matches!(
        manager.availability(&config).await.expect("Go availability"),
        ManagerAvailability::Available { version: Some(version) }
            if version.starts_with("go version go1.24.0")
    ));
    let packages = manager.installed(&config).await.expect("Go inventory");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "tool");
    assert_eq!(packages[0].version, "v1.0.0");
    assert_eq!(packages[0].size, Some(4));

    let updates = manager.updates(&config, false).await.expect("Go updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "tool");
    assert_eq!(updates[0].available_version, "v1.1.0");

    let mut install = updates[0].target.clone();
    install.version = Some("v1.1.0".to_owned());
    manager
        .execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&install),
            &|_| {},
        )
        .await
        .expect("install typed Go target");
    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            std::slice::from_ref(&updates[0].target),
            &|_| {},
        )
        .await
        .expect("uninstall Go Windows binary");

    assert_eq!(
        fs::read_to_string(log)
            .expect("Go Windows write log")
            .trim(),
        format!("{}|example.com/mod/cmd/tool@v1.1.0", bin.display())
    );
    assert!(!bin.join("tool.exe").exists());
}

/// The manager-api default must keep the single-value `updates` contract intact
/// for every manager that does not resolve packages one by one.
#[tokio::test]
async fn default_updates_report_forwards_to_updates_without_warnings() {
    struct UpdateSource(ManagerDescriptor, bool);

    #[async_trait::async_trait]
    impl PackageManager for UpdateSource {
        fn descriptor(&self) -> &ManagerDescriptor {
            &self.0
        }

        async fn availability(
            &self,
            _config: &ManagerConfig,
        ) -> ManagerResult<ManagerAvailability> {
            Ok(ManagerAvailability::Available { version: None })
        }

        async fn updates(
            &self,
            _config: &ManagerConfig,
            _refresh: bool,
        ) -> ManagerResult<Vec<PackageUpdate>> {
            if self.1 {
                let id = self.0.id().clone();
                return Ok(vec![PackageUpdate::new(
                    PackageTarget::new(id, "example-tool"),
                    "v1.0.0",
                    "v1.1.0",
                )]);
            }
            Err(ManagerError::new(ManagerErrorKind::Other, "scan failed"))
        }
    }

    let descriptor = ManagerDescriptor::new(
        ManagerId::parse("org.example:updates").expect("valid ID"),
        "Updates",
        ManagerCategory::Development,
        SupportedPlatforms::from([Platform::Linux]),
        ManagerCapabilities::from([ManagerCapability::Updates]),
    )
    .expect("valid descriptor");
    let config = ManagerConfig::new(descriptor.id().clone());

    let report = UpdateSource(descriptor.clone(), true)
        .updates_report(&config, false)
        .await
        .expect("default report");
    assert_eq!(report.updates().len(), 1);
    assert_eq!(report.updates()[0].target.name, "example-tool");
    assert!(!report.is_degraded());
    assert!(report.warnings().is_empty());
    assert_eq!(report.into_updates().len(), 1);

    assert_eq!(
        UpdateSource(descriptor, false)
            .updates_report(&config, false)
            .await
            .expect_err("propagate the update scan failure")
            .kind(),
        ManagerErrorKind::Other
    );
}

#[cfg(unix)]
#[tokio::test]
async fn installed_is_sorted_and_preserves_binary_module_and_package_identity() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    write_binary(bin.path().join("ztool"), b"z");
    write_binary(bin.path().join("atool"), b"aaa");
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
    write_binary(bin.path().join("tool"), b"tool");
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

/// Go has no catalog to search, so a plain word such as `lint` reaches
/// `go list` and fails. The message must name the exact-module-path
/// requirement instead of reporting a bare command failure, while keeping
/// Go's own diagnostics in the detail.
#[cfg(unix)]
#[tokio::test]
async fn non_module_go_search_explains_the_exact_module_path_requirement() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    let (_directory, executable) = fake_go(
        r#"#!/bin/sh
if [ "$1" = "list" ] && [ "$2" = "-m" ] && [ "$3" = "-versions" ] && [ "$4" = "-json" ]; then
  printf '%s\n' "go: malformed module path \"$5\": missing dot in first path element" >&2
  exit 1
fi
exit 12
"#,
    );
    let config = config(&manager, &executable, bin.path());

    let error = manager
        .search(&config, "lint")
        .await
        .expect_err("a plain word is not a Go module path");
    assert!(
        error.message().contains("exact module path"),
        "search must name the exact module path requirement: {error:?}"
    );
    assert_eq!(error.kind(), ManagerErrorKind::Other);
    assert!(
        error
            .detail()
            .is_some_and(|detail| detail.contains("missing dot in first path element")),
        "the detail must keep Go's own diagnostic: {error:?}"
    );
}

/// A fake `go` whose installed binaries are `atool`, `mtool`, and `ztool`,
/// where only `mtool`'s latest-version lookup fails. `atool`'s lookup is the
/// slowest, so lookups finish in the opposite order from the installed order
/// and the update list can only be ordered by `atool`/`ztool` if the scan
/// keeps results in their binary's slot.
#[cfg(unix)]
fn unresolvable_module_fixture() -> (&'static str, [&'static str; 3]) {
    const SCRIPT: &str = r#"#!/bin/sh
if [ "$1" = "version" ] && [ "$2" = "-m" ] && [ "$3" = "-json" ]; then
  case "${4##*/}" in
    atool) printf '{"Path":"example.com/mod/cmd/atool","Main":{"Path":"example.com/mod","Version":"v1.2.0"}}\n'; exit 0 ;;
    mtool) printf '{"Path":"example.net/mtool","Main":{"Path":"example.net/mtool","Version":"v0.8.0"}}\n'; exit 0 ;;
    ztool) printf '{"Path":"example.net/ztool/cmd/ztool","Main":{"Path":"example.net/ztool","Version":"v0.5.0"}}\n'; exit 0 ;;
    *) exit 1 ;;
  esac
fi
if [ "$1" = "list" ] && [ "$2" = "-m" ] && [ "$3" = "-json" ]; then
  case "$4" in
    example.com/mod@latest) sleep 0.3; printf '{"Path":"example.com/mod","Version":"v1.3.0"}\n'; exit 0 ;;
    example.net/ztool@latest) printf '{"Path":"example.net/ztool","Version":"v0.6.0"}\n'; exit 0 ;;
    example.net/mtool@latest) printf '410 Gone: module example.net/mtool is no longer available\n' >&2; exit 1 ;;
    *) exit 1 ;;
  esac
fi
exit 20
"#;
    (SCRIPT, ["atool", "mtool", "ztool"])
}

#[cfg(unix)]
#[tokio::test]
async fn one_unresolvable_module_keeps_the_other_go_updates() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    let (script, names) = unresolvable_module_fixture();
    for name in names {
        write_binary(bin.path().join(name), name.as_bytes());
    }
    let (_directory, executable) = fake_go(script);
    let config = config(&manager, &executable, bin.path());

    let updates = manager
        .updates(&config, false)
        .await
        .expect("one unresolvable module must not fail the Go source");
    assert_eq!(
        updates
            .iter()
            .map(|update| update.target.name.as_str())
            .collect::<Vec<_>>(),
        vec!["atool", "ztool"],
        "healthy binaries keep their updates in installed order even though the \
         first lookup finished last"
    );
    assert_eq!(updates[0].available_version, "v1.3.0");
    assert_eq!(updates[1].available_version, "v0.6.0");
}

#[cfg(unix)]
#[tokio::test]
async fn degraded_go_report_records_the_binary_it_skipped() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    let (script, names) = unresolvable_module_fixture();
    for name in names {
        write_binary(bin.path().join(name), name.as_bytes());
    }
    let (_directory, executable) = fake_go(script);
    let config = config(&manager, &executable, bin.path());

    let report = manager
        .updates_report(&config, false)
        .await
        .expect("report the partial Go scan");
    assert_eq!(
        report
            .updates()
            .iter()
            .map(|update| update.target.name.as_str())
            .collect::<Vec<_>>(),
        vec!["atool", "ztool"]
    );
    assert!(report.is_degraded());
    assert_eq!(report.warnings().len(), 1);
    let warning = &report.warnings()[0];
    assert_ne!(warning.kind(), ManagerErrorKind::Protocol);
    assert!(
        warning
            .detail()
            .is_some_and(|detail| detail.contains("mtool")),
        "the recorded warning must name the skipped binary: {warning:?}"
    );
    assert!(
        warning
            .detail()
            .is_some_and(|detail| detail.contains("410 Gone")),
        "the recorded warning must keep the failing lookup's cause: {warning:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn go_latest_lookups_overlap_but_stay_within_the_concurrency_limit() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    let marks = bin.path().join("marks.log");
    let count = 6;
    for index in 0..count {
        write_binary(bin.path().join(format!("tool{index}")), b"tool");
    }
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "version" ] && [ "$2" = "-m" ] && [ "$3" = "-json" ]; then
  name=${{4##*/}}
  index=${{name#tool}}
  printf '{{"Path":"example.com/mod%s","Main":{{"Path":"example.com/mod%s","Version":"v1.0.0"}}}}\n' "$index" "$index"
  exit 0
fi
if [ "$1" = "list" ] && [ "$2" = "-m" ] && [ "$3" = "-json" ]; then
  module=${{4%@*}}
  index=${{module#example.com/mod}}
  printf 'S%s\n' "$index" >> '{marks}'
  sleep 0.2
  printf 'E%s\n' "$index" >> '{marks}'
  printf '{{"Path":"%s","Version":"v1.1.0"}}\n' "$module"
  exit 0
fi
exit 21
"#,
        marks = marks.display()
    );
    let (_directory, executable) = fake_go(&script);
    let config = config(&manager, &executable, bin.path());

    let updates = manager.updates(&config, false).await.expect("Go updates");
    assert_eq!(updates.len(), count);

    let log = fs::read_to_string(&marks).expect("read lookup mark log");
    let mut in_flight = 0_usize;
    let mut peak = 0_usize;
    for line in log.lines() {
        if line.starts_with('S') {
            in_flight += 1;
            peak = peak.max(in_flight);
        } else {
            in_flight = in_flight.saturating_sub(1);
        }
    }
    assert_eq!(in_flight, 0, "every started lookup must also finish");
    assert!(
        peak > 1,
        "lookups ran strictly one at a time (peak in flight: {peak})"
    );
    assert!(
        peak <= 4,
        "lookups exceeded the bounded concurrency limit (peak in flight: {peak})"
    );
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
    write_binary(bin.path().join("tool"), b"tool");
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
    write_binary(bin.path().join("tool"), b"tool");
    let mut target = PackageTarget::new(manager.descriptor().id().clone(), "tool");
    target.scope = PackageScope::User;
    target.origin = Some(
        PackageOrigin::new("example.com/mod").with_reference("package:example.com/mod/cmd/tool"),
    );

    manager
        .execute(
            &config,
            PackageAction::Uninstall,
            std::slice::from_ref(&target),
            &NoopProgressSink,
        )
        .await
        .expect("remove contained binary");
    assert!(!bin.path().join("tool").exists());

    let outside = tempdir().expect("create outside directory");
    fs::write(outside.path().join("outside"), b"outside").expect("write outside file");
    symlink(outside.path().join("outside"), bin.path().join("link")).expect("create symlink");
    target.name = "link".to_owned();
    assert_eq!(
        manager
            .execute(
                &config,
                PackageAction::Uninstall,
                std::slice::from_ref(&target),
                &NoopProgressSink,
            )
            .await
            .expect_err("reject symlink removal")
            .kind(),
        ManagerErrorKind::Protocol
    );
    target.name = "../outside".to_owned();
    assert_eq!(
        manager
            .execute(
                &config,
                PackageAction::Uninstall,
                std::slice::from_ref(&target),
                &NoopProgressSink,
            )
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
    write_binary(bin.path().join("broken"), b"broken");
    let (_directory, executable) =
        fake_go("#!/bin/sh\nprintf 'unreadable build info\\n' >&2\nexit 17\n");
    let error = manager
        .installed(&config(&manager, &executable, bin.path()))
        .await
        .expect_err("surface build-info failure");
    assert_ne!(error.kind(), ManagerErrorKind::Protocol);
}

#[cfg(unix)]
#[tokio::test]
async fn updates_ignore_non_executable_files_in_gobin() {
    let manager = GoManager::new();
    let bin = tempdir().expect("create GOBIN");
    fs::write(bin.path().join(".gup.lock"), b"").expect("write gup lock");
    fs::write(bin.path().join("README.txt"), b"installed tools")
        .expect("write non-executable file");
    write_binary(bin.path().join("tool"), b"tool");
    let (_directory, executable) = fake_go(
        r#"#!/bin/sh
if [ "$1" = "version" ] && [ "$2" = "-m" ] && [ "$3" = "-json" ]; then
  case "${4##*/}" in
    tool) printf '{"Path":"example.com/tool","Main":{"Path":"example.com/tool","Version":"v1.0.0"}}\n'; exit 0 ;;
    *) exit 1 ;;
  esac
fi
if [ "$1" = "list" ] && [ "$4" = "example.com/tool@latest" ]; then
  printf '{"Path":"example.com/tool","Version":"v1.1.0"}\n'
  exit 0
fi
exit 19
"#,
    );

    let updates = manager
        .updates(&config(&manager, &executable, bin.path()), false)
        .await
        .expect("a gup lock must not block Go updates");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].target.name, "tool");
    assert_eq!(updates[0].available_version, "v1.1.0");
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
