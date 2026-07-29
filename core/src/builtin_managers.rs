use crate::{ManagerRegistry, RegistryError};

/// Registers the complete direct built-in manager catalog.
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
