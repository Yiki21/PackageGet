use std::sync::Arc;

use updater_manager_api::{PackageManager, Platform};

use crate::{
    AptManager, CargoManager, ComposerGlobalManager, DnfManager, DotnetToolManager, FlatpakManager,
    GoManager, HomebrewManager, NpmManager, PacmanManager, PipxManager, PnpmManager,
    RubyGemsManager, SnapManager, UvManager, WingetManager, ZypperManager,
};

/// Creates the direct built-in package managers for the current target.
///
/// The returned managers are object-safe shared instances in stable product
/// order after platform filtering. Each call creates an independent catalog;
/// callers may register, retain, or clone the returned [`Arc`] values without
/// shared global state.
#[must_use]
pub fn builtin_managers() -> Vec<Arc<dyn PackageManager>> {
    Platform::current().map_or_else(Vec::new, builtin_managers_for)
}

/// Creates the built-in manager catalog supported by `platform`.
///
/// Managers retain their stable product order after filtering.
#[must_use]
pub fn builtin_managers_for(platform: Platform) -> Vec<Arc<dyn PackageManager>> {
    all_builtin_managers()
        .into_iter()
        .filter(|manager| manager.descriptor().platforms().contains(platform))
        .collect()
}

fn all_builtin_managers() -> Vec<Arc<dyn PackageManager>> {
    vec![
        Arc::new(AptManager::new()),
        Arc::new(DnfManager::new()),
        Arc::new(PacmanManager::new()),
        Arc::new(ZypperManager::new()),
        Arc::new(WingetManager::new()),
        Arc::new(FlatpakManager::new()),
        Arc::new(SnapManager::new()),
        Arc::new(HomebrewManager::new()),
        Arc::new(CargoManager::new()),
        Arc::new(GoManager::new()),
        Arc::new(NpmManager::new()),
        Arc::new(PnpmManager::new()),
        Arc::new(PipxManager::new()),
        Arc::new(UvManager::new()),
        Arc::new(DotnetToolManager::new()),
        Arc::new(RubyGemsManager::new()),
        Arc::new(ComposerGlobalManager::new()),
    ]
}
