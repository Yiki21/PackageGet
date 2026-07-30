use std::{collections::BTreeMap, fmt, sync::Arc};

use thiserror::Error;
use updater_manager_api::{ManagerCapability, ManagerDescriptor, ManagerId, PackageManager};

/// Deterministic registry for compile-time package manager extensions.
///
/// Managers are registered explicitly as [`Arc<dyn PackageManager>`]. The
/// registry owns no runtime plugin loader and performs no dynamic-library
/// discovery.
#[derive(Default)]
pub struct ManagerRegistry {
    managers: BTreeMap<ManagerId, Arc<dyn PackageManager>>,
}

impl ManagerRegistry {
    /// Creates an empty manager registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a manager implementation.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DuplicateManager`] when a manager with the
    /// same validated [`ManagerId`] is already registered.
    pub fn register(&mut self, manager: Arc<dyn PackageManager>) -> Result<(), RegistryError> {
        let id = manager.descriptor().id().clone();
        if self.managers.contains_key(&id) {
            return Err(RegistryError::DuplicateManager { id });
        }

        self.managers.insert(id, manager);
        Ok(())
    }

    /// Returns the number of registered managers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.managers.len()
    }

    /// Returns whether the registry contains no managers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.managers.is_empty()
    }

    /// Returns whether `id` is registered.
    #[must_use]
    pub fn contains(&self, id: &ManagerId) -> bool {
        self.managers.contains_key(id)
    }

    /// Returns a shared manager instance by ID.
    #[must_use]
    pub fn get(&self, id: &ManagerId) -> Option<Arc<dyn PackageManager>> {
        self.managers.get(id).map(Arc::clone)
    }

    /// Returns registered descriptor metadata by ID.
    #[must_use]
    pub fn descriptor(&self, id: &ManagerId) -> Option<&ManagerDescriptor> {
        self.managers.get(id).map(|manager| manager.descriptor())
    }

    /// Returns a manager after checking an advertised capability.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::UnknownManager`] when `id` is not registered.
    /// Returns [`RegistryError::UnsupportedCapability`] when the descriptor
    /// does not advertise `capability`.
    pub fn manager_for(
        &self,
        id: &ManagerId,
        capability: ManagerCapability,
    ) -> Result<Arc<dyn PackageManager>, RegistryError> {
        let manager = self
            .get(id)
            .ok_or_else(|| RegistryError::UnknownManager { id: id.clone() })?;

        if !manager.descriptor().capabilities().contains(capability) {
            return Err(RegistryError::UnsupportedCapability {
                id: id.clone(),
                capability,
            });
        }

        Ok(manager)
    }

    /// Returns managers in stable descriptor order.
    ///
    /// Ordering uses category, display name, then manager ID. The final ID
    /// tie-breaker keeps results deterministic when display metadata matches.
    #[must_use]
    pub fn managers(&self) -> Vec<Arc<dyn PackageManager>> {
        let mut managers: Vec<_> = self.managers.values().map(Arc::clone).collect();
        managers.sort_by(|left, right| {
            let left = left.descriptor();
            let right = right.descriptor();

            left.category()
                .cmp(&right.category())
                .then_with(|| left.display_name().cmp(right.display_name()))
                .then_with(|| left.id().cmp(right.id()))
        });
        managers
    }
}

impl fmt::Debug for ManagerRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagerRegistry")
            .field("manager_ids", &self.managers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Failures produced by [`ManagerRegistry`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// A manager ID was registered more than once.
    #[error("manager is already registered: {id}")]
    DuplicateManager {
        /// Duplicated manager ID.
        id: ManagerId,
    },
    /// A requested manager ID is not registered.
    #[error("manager is not registered: {id}")]
    UnknownManager {
        /// Missing manager ID.
        id: ManagerId,
    },
    /// A requested manager does not advertise the required capability.
    #[error("manager {id} does not support {capability}")]
    UnsupportedCapability {
        /// Registered manager ID.
        id: ManagerId,
        /// Required capability.
        capability: ManagerCapability,
    },
}
