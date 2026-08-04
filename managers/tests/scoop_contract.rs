use updater_manager_api::{
    AuthorizationHint, ManagerCapability, ManagerConfig, PackageManager, Platform,
};
#[cfg(not(target_os = "windows"))]
use updater_manager_api::{AvailabilityReason, ManagerAvailability};
use updater_managers::ScoopManager;

#[test]
fn scoop_descriptor_exposes_the_stable_public_contract() {
    let manager = ScoopManager::new();
    let descriptor = manager.descriptor();

    assert_eq!(descriptor.id().as_str(), "builtin:scoop");
    assert_eq!(descriptor.display_name(), "Scoop");
    assert_eq!(
        descriptor.platforms().iter().copied().collect::<Vec<_>>(),
        vec![Platform::Windows]
    );
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

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn availability_reports_the_unsupported_host_without_spawning_scoop() {
    let manager = ScoopManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    assert_eq!(
        manager
            .availability(&config)
            .await
            .expect("check Scoop availability"),
        ManagerAvailability::Unavailable {
            reason: AvailabilityReason::UnsupportedPlatform {
                platform: Platform::current(),
            },
        }
    );
}

#[cfg(target_os = "windows")]
#[tokio::test]
#[ignore = "requires a real Scoop installation"]
async fn real_scoop_availability_smoke() {
    let manager = ScoopManager::new();
    let config = ManagerConfig::new(manager.descriptor().id().clone());

    assert!(manager.availability(&config).await.unwrap().is_available());
}
