use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerConfig, PackageManager, Platform,
};
#[cfg(not(target_os = "windows"))]
use updater_manager_api::{AvailabilityReason, ManagerAvailability};
use updater_managers::ChocolateyManager;

#[test]
fn chocolatey_descriptor_exposes_the_machine_scope_contract() {
    let manager = ChocolateyManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:chocolatey");
    assert_eq!(descriptor.display_name(), "Chocolatey");
    assert_eq!(
        descriptor.platforms().iter().copied().collect::<Vec<_>>(),
        vec![Platform::Windows]
    );
    assert_eq!(
        descriptor.authorization(),
        &AuthorizationHint::RequiresElevation {
            message: Some("Chocolatey writes machine-wide packages and requires elevation.".into()),
        }
    );
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

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn availability_reports_unsupported_host_without_spawning_choco() {
    let manager = ChocolateyManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check Chocolatey availability"),
        ManagerAvailability::Unavailable {
            reason: AvailabilityReason::UnsupportedPlatform {
                platform: Platform::current(),
            },
        }
    );
}

#[cfg(target_os = "windows")]
#[tokio::test]
#[ignore = "requires a real Chocolatey installation"]
async fn real_chocolatey_read_only_smoke() {
    let manager = ChocolateyManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    assert!(manager.availability(&config).await.unwrap().is_available());
    let packages = manager
        .installed(&config)
        .await
        .expect("read Chocolatey installed inventory");
    assert!(!packages.is_empty());
    assert!(packages.iter().all(|package| {
        package.manager_id == *manager.descriptor().id()
            && package.scope == updater_manager_api::PackageScope::System
            && package.origin.as_ref().map(|origin| origin.name.as_str()) == Some("Chocolatey")
    }));
}
