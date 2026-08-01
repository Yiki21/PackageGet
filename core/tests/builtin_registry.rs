use updater_core::{
    ManagerRegistry, RegistryError, register_builtin_managers, register_builtin_managers_for,
};
use updater_manager_api::Platform;
use updater_managers::{builtin_managers, builtin_managers_for};

#[test]
fn direct_builtin_registration_preserves_all_stable_ids() {
    let expected_ids = builtin_managers()
        .into_iter()
        .map(|manager| manager.descriptor().id().clone())
        .collect::<Vec<_>>();
    let mut registry = ManagerRegistry::new();
    register_builtin_managers(&mut registry).expect("register direct built-in managers");

    assert_eq!(registry.len(), expected_ids.len());
    for id in expected_ids {
        let manager = registry.get(&id).expect("registered manager ID");
        assert_eq!(manager.descriptor().id(), &id);
    }
}

#[test]
fn direct_registration_rejects_each_preexisting_builtin_id() {
    for manager in builtin_managers() {
        let id = manager.descriptor().id().clone();
        let mut registry = ManagerRegistry::new();
        registry
            .register(manager)
            .expect("register one direct built-in manager");

        assert!(matches!(
            register_builtin_managers(&mut registry),
            Err(RegistryError::DuplicateManager { id: duplicate }) if duplicate == id
        ));
    }
}

#[test]
fn direct_catalog_ids_match_the_builtin_runtime_set() {
    let mut catalog_ids = builtin_managers()
        .into_iter()
        .map(|manager| manager.descriptor().id().clone())
        .collect::<Vec<_>>();
    let mut registry = ManagerRegistry::new();
    register_builtin_managers(&mut registry).expect("register direct built-in managers");
    let mut runtime_ids = registry
        .managers()
        .into_iter()
        .map(|manager| manager.descriptor().id().clone())
        .collect::<Vec<_>>();
    catalog_ids.sort();
    runtime_ids.sort();

    assert_eq!(catalog_ids, runtime_ids);
}

#[test]
fn explicit_platform_registration_matches_filtered_catalog() {
    for platform in [Platform::Linux, Platform::Windows, Platform::MacOs] {
        let mut expected_ids = builtin_managers_for(platform)
            .into_iter()
            .map(|manager| manager.descriptor().id().clone())
            .collect::<Vec<_>>();
        let mut registry = ManagerRegistry::new();

        register_builtin_managers_for(&mut registry, platform)
            .expect("register platform built-in managers");

        let mut actual_ids = registry
            .managers()
            .into_iter()
            .map(|manager| manager.descriptor().id().clone())
            .collect::<Vec<_>>();
        expected_ids.sort();
        actual_ids.sort();
        assert_eq!(actual_ids, expected_ids);
    }
}
