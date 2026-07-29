use std::sync::Arc;

use updater_manager_api::PackageManager;

use crate::{
    AptManager, CargoManager, DnfManager, FlatpakManager, GoManager, HomebrewManager, NpmManager,
    PacmanManager, PipxManager, PnpmManager, ZypperManager,
};

/// Creates the complete catalog of direct built-in package managers.
///
/// The returned managers are object-safe shared instances in stable product
/// order: system managers first, followed by application and development
/// managers. Each call creates an independent catalog; callers may register,
/// retain, or clone the returned [`Arc`] values without shared global state.
#[must_use]
pub fn builtin_managers() -> Vec<Arc<dyn PackageManager>> {
    vec![
        Arc::new(AptManager::new()),
        Arc::new(DnfManager::new()),
        Arc::new(PacmanManager::new()),
        Arc::new(ZypperManager::new()),
        Arc::new(FlatpakManager::new()),
        Arc::new(HomebrewManager::new()),
        Arc::new(CargoManager::new()),
        Arc::new(GoManager::new()),
        Arc::new(NpmManager::new()),
        Arc::new(PnpmManager::new()),
        Arc::new(PipxManager::new()),
    ]
}
