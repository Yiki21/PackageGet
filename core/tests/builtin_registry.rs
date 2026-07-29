use updater_core::{
    ALL_PACKAGE_MANAGERS, ManagerRegistry, PackageManagerType, RegistryError,
    register_builtin_managers,
};
use updater_managers::builtin_managers;

#[test]
fn direct_builtin_registration_preserves_all_stable_ids() {
    let mut registry = ManagerRegistry::new();
    register_builtin_managers(&mut registry).expect("register direct built-in managers");

    assert_eq!(registry.len(), ALL_PACKAGE_MANAGERS.len());
    for manager_type in ALL_PACKAGE_MANAGERS {
        let manager = registry
            .get(&manager_type.manager_id())
            .expect("registered manager ID");
        assert_eq!(manager.descriptor().id(), &manager_type.manager_id());
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
    let catalog_ids = builtin_managers()
        .into_iter()
        .map(|manager| manager.descriptor().id().clone())
        .collect::<Vec<_>>();
    let runtime_ids = ALL_PACKAGE_MANAGERS
        .iter()
        .map(|manager_type| PackageManagerType::manager_id(*manager_type))
        .collect::<Vec<_>>();

    assert_eq!(catalog_ids, runtime_ids);
}
