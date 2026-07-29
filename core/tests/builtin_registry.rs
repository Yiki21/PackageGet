use std::sync::Arc;

use updater_core::{
    ALL_PACKAGE_MANAGERS, ManagerRegistry, PackageManagerType, RegistryError,
    register_builtin_managers,
};
use updater_managers::{
    AptManager, DnfManager, FlatpakManager, HomebrewManager, PacmanManager, ZypperManager,
};

#[test]
fn mixed_builtin_registration_preserves_all_stable_ids() {
    let mut registry = ManagerRegistry::new();
    register_builtin_managers(&mut registry).expect("register mixed built-in managers");

    assert_eq!(registry.len(), ALL_PACKAGE_MANAGERS.len());
    for manager_type in ALL_PACKAGE_MANAGERS {
        let manager = registry
            .get(&manager_type.manager_id())
            .expect("registered manager ID");
        assert_eq!(manager.descriptor().id(), &manager_type.manager_id());
    }
}

#[test]
fn mixed_registration_rejects_a_preexisting_direct_apt_manager() {
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(AptManager::new()))
        .expect("register direct APT manager");

    assert!(matches!(
        register_builtin_managers(&mut registry),
        Err(RegistryError::DuplicateManager { id })
            if id == PackageManagerType::Apt.manager_id()
    ));
}

#[test]
fn mixed_registration_rejects_a_preexisting_direct_dnf_manager() {
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(DnfManager::new()))
        .expect("register direct DNF manager");

    assert!(matches!(
        register_builtin_managers(&mut registry),
        Err(RegistryError::DuplicateManager { id })
            if id == PackageManagerType::Dnf.manager_id()
    ));
}

#[test]
fn mixed_registration_rejects_a_preexisting_direct_pacman_manager() {
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(PacmanManager::new()))
        .expect("register direct Pacman manager");

    assert!(matches!(
        register_builtin_managers(&mut registry),
        Err(RegistryError::DuplicateManager { id })
            if id == PackageManagerType::Pacman.manager_id()
    ));
}

#[test]
fn mixed_registration_rejects_a_preexisting_direct_zypper_manager() {
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(ZypperManager::new()))
        .expect("register direct Zypper manager");

    assert!(matches!(
        register_builtin_managers(&mut registry),
        Err(RegistryError::DuplicateManager { id })
            if id == PackageManagerType::Zypper.manager_id()
    ));
}

#[test]
fn mixed_registration_rejects_a_preexisting_direct_flatpak_manager() {
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(FlatpakManager::new()))
        .expect("register direct Flatpak manager");

    assert!(matches!(
        register_builtin_managers(&mut registry),
        Err(RegistryError::DuplicateManager { id })
            if id == PackageManagerType::Flatpak.manager_id()
    ));
}

#[test]
fn mixed_registration_rejects_a_preexisting_direct_homebrew_manager() {
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(HomebrewManager::new()))
        .expect("register direct Homebrew manager");

    assert!(matches!(
        register_builtin_managers(&mut registry),
        Err(RegistryError::DuplicateManager { id })
            if id == PackageManagerType::Homebrew.manager_id()
    ));
}
