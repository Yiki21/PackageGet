use std::collections::BTreeMap;

use updater_core::{ManagerRegistry, register_builtin_managers};
use updater_manager_api::{ManagerDescriptor, ManagerId, Platform};

/// Read-only manager metadata used by the UI.
#[derive(Debug, Clone)]
pub struct ManagerCatalog {
    descriptors: BTreeMap<ManagerId, ManagerDescriptor>,
}

impl ManagerCatalog {
    /// Builds the catalog from the direct built-in registry.
    #[must_use]
    pub fn builtin() -> Self {
        let mut registry = ManagerRegistry::new();
        register_builtin_managers(&mut registry)
            .expect("the direct built-in manager catalog must contain unique IDs");

        let descriptors = registry
            .managers()
            .into_iter()
            .map(|manager| {
                let descriptor = manager.descriptor().clone();
                (descriptor.id().clone(), descriptor)
            })
            .collect();

        Self { descriptors }
    }

    /// Returns registered descriptor metadata for `id`.
    #[must_use]
    pub fn descriptor(&self, id: &ManagerId) -> Option<&ManagerDescriptor> {
        self.descriptors.get(id)
    }

    /// Returns a display label, falling back to the stable ID when missing.
    #[must_use]
    pub fn display_name<'a>(&'a self, id: &'a ManagerId) -> &'a str {
        self.descriptor(id)
            .map_or_else(|| id.as_str(), ManagerDescriptor::display_name)
    }

    /// Returns whether an implementation is registered in this build.
    #[must_use]
    pub fn is_registered(&self, id: &ManagerId) -> bool {
        self.descriptors.contains_key(id)
    }

    /// Returns whether the descriptor supports the current target OS.
    #[must_use]
    pub fn supports_current_platform(&self, id: &ManagerId) -> bool {
        let Some(platform) = current_platform() else {
            return false;
        };
        self.descriptor(id)
            .is_some_and(|descriptor| descriptor.platforms().contains(platform))
    }
}

fn current_platform() -> Option<Platform> {
    if cfg!(target_os = "linux") {
        Some(Platform::Linux)
    } else if cfg!(target_os = "windows") {
        Some(Platform::Windows)
    } else if cfg!(target_os = "macos") {
        Some(Platform::MacOs)
    } else {
        None
    }
}

impl Default for ManagerCatalog {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use updater_manager_api::{ManagerCapability, ManagerId};

    use super::ManagerCatalog;

    #[test]
    fn builtins_expose_direct_registry_metadata() {
        let catalog = ManagerCatalog::builtin();
        let cargo = ManagerId::parse("builtin:cargo").unwrap();
        let descriptor = catalog.descriptor(&cargo).unwrap();

        assert_eq!(catalog.display_name(&cargo), "Cargo");
        assert!(
            descriptor
                .capabilities()
                .contains(ManagerCapability::Install)
        );
        assert!(catalog.supports_current_platform(&cargo));
    }

    #[test]
    fn unknown_manager_uses_stable_id_without_runtime_fallback() {
        let catalog = ManagerCatalog::builtin();
        let unknown = ManagerId::parse("org.example:custom").unwrap();

        assert_eq!(catalog.display_name(&unknown), "org.example:custom");
        assert!(!catalog.is_registered(&unknown));
        assert!(!catalog.supports_current_platform(&unknown));
    }
}
