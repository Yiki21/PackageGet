use crate::{ManagerRegistry, RegistryError};
use updater_manager_api::Platform;

/// Registers direct built-in managers supported by the current target.
///
/// Managers are registered in the stable order provided by
/// `updater_managers::builtin_managers`. Registration remains incremental, so
/// the first duplicate manager returns its existing stable ID.
///
/// # Errors
///
/// Returns [`RegistryError::DuplicateManager`] if the registry already
/// contains a built-in manager ID.
pub fn register_builtin_managers(registry: &mut ManagerRegistry) -> Result<(), RegistryError> {
    for manager in updater_managers::builtin_managers() {
        registry.register(manager)?;
    }
    Ok(())
}

/// Registers built-in managers supported by `platform`.
///
/// # Errors
///
/// Returns [`RegistryError::DuplicateManager`] if the registry already
/// contains a built-in manager ID for the selected platform.
pub fn register_builtin_managers_for(
    registry: &mut ManagerRegistry,
    platform: Platform,
) -> Result<(), RegistryError> {
    for manager in updater_managers::builtin_managers_for(platform) {
        registry.register(manager)?;
    }
    Ok(())
}
