#![cfg(unix)]

//! Package writes run for as long as the package manager needs. A wall-clock
//! deadline around a write dropped the command future and killed only the
//! direct child, leaving half-applied global prefixes and orphaned
//! grandchildren, so these tests hold each write open past every former
//! deadline on tokio's paused clock.

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use tempfile::{TempDir, tempdir};
use updater_manager_api::{
    ManagerConfig, ManagerId, PackageAction, PackageManager, PackageOrigin, PackageScope,
    PackageTarget, ProgressEvent,
};
use updater_managers::{
    BunManager, ChocolateyManager, DotnetToolManager, NpmManager, PipxManager, PnpmManager,
    ScoopManager, SnapManager, UvManager,
};

/// Longer than the former 30 second pnpm and 90 second write deadlines.
const WRITE_DURATION: Duration = Duration::from_secs(120);

/// A fake package manager whose write only finishes once the test releases it.
fn blocking_write_executable(name: &str) -> (TempDir, PathBuf, PathBuf) {
    let directory = tempdir().expect("create fake package manager directory");
    let executable = directory.path().join(name);
    let release = directory.path().join("release");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\nwhile [ ! -e '{}' ]; do sleep 0.05; done\nexit 0\n",
            release.display()
        ),
    )
    .expect("write fake package manager");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
        .expect("mark fake package manager executable");
    (directory, executable, release)
}

async fn assert_install_outlives_former_deadline<M: PackageManager>(
    manager: M,
    executable_name: &str,
    target: impl FnOnce(&ManagerId) -> PackageTarget,
) {
    let (_directory, executable, release) = blocking_write_executable(executable_name);
    let config = ManagerConfig::new(manager.descriptor().id().clone()).with_executable(executable);
    let target = target(manager.descriptor().id());
    let sink = |_event: ProgressEvent| {};
    let started = tokio::time::Instant::now();

    let release_after_former_deadline = async {
        tokio::time::sleep(WRITE_DURATION).await;
        fs::write(&release, b"").expect("release fake package write");
    };
    let (result, ()) = tokio::join!(
        manager.execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&target),
            &sink,
        ),
        release_after_former_deadline,
    );

    result.unwrap_or_else(|error| {
        panic!(
            "{} install must not hit a wall-clock deadline: {error:?}",
            manager.descriptor().id()
        )
    });
    assert!(started.elapsed() >= WRITE_DURATION);
}

fn plain_target(manager_id: &ManagerId) -> PackageTarget {
    PackageTarget::new(manager_id.clone(), "tool")
}

#[tokio::test(start_paused = true)]
async fn pnpm_install_outlives_the_former_wall_clock_deadline() {
    assert_install_outlives_former_deadline(PnpmManager::new(), "pnpm", plain_target).await;
}

#[tokio::test(start_paused = true)]
async fn npm_install_outlives_the_former_wall_clock_deadline() {
    assert_install_outlives_former_deadline(NpmManager::new(), "npm", plain_target).await;
}

#[tokio::test(start_paused = true)]
async fn bun_install_outlives_the_former_wall_clock_deadline() {
    assert_install_outlives_former_deadline(BunManager::new(), "bun", plain_target).await;
}

#[tokio::test(start_paused = true)]
async fn pipx_install_outlives_the_former_wall_clock_deadline() {
    assert_install_outlives_former_deadline(PipxManager::new(), "pipx", plain_target).await;
}

#[tokio::test(start_paused = true)]
async fn uv_install_outlives_the_former_wall_clock_deadline() {
    assert_install_outlives_former_deadline(UvManager::new(), "uv", plain_target).await;
}

#[tokio::test(start_paused = true)]
async fn dotnet_install_outlives_the_former_wall_clock_deadline() {
    assert_install_outlives_former_deadline(DotnetToolManager::new(), "dotnet", plain_target).await;
}

#[tokio::test(start_paused = true)]
async fn snap_install_outlives_the_former_wall_clock_deadline() {
    assert_install_outlives_former_deadline(SnapManager::new(), "snap", |manager_id| {
        let mut target = PackageTarget::new(manager_id.clone(), "code");
        target.scope = PackageScope::System;
        target.origin = Some(PackageOrigin::new("Snap").with_reference(
            "snap:code;channel:latest/stable;confinement:classic;refresh:store;notes:classic",
        ));
        target
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn scoop_install_outlives_the_former_wall_clock_deadline() {
    assert_install_outlives_former_deadline(ScoopManager::new(), "scoop", |manager_id| {
        let mut target = PackageTarget::new(manager_id.clone(), "7zip");
        target.scope = PackageScope::User;
        target.origin = Some(PackageOrigin::new("Scoop").with_reference("bucket:main"));
        target
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn chocolatey_install_outlives_the_former_wall_clock_deadline() {
    assert_install_outlives_former_deadline(ChocolateyManager::new(), "choco", |manager_id| {
        let mut target = PackageTarget::new(manager_id.clone(), "git");
        target.scope = PackageScope::System;
        target.origin = Some(PackageOrigin::new("Chocolatey"));
        target
    })
    .await;
}
