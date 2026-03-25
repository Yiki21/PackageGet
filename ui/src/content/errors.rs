use std::collections::HashMap;

use updater_core::PackageManagerType;

pub type ManagerErrors = HashMap<PackageManagerType, String>;

pub fn apply_manager_items_result<T>(
    items_by_manager: &mut HashMap<PackageManagerType, Vec<T>>,
    errors: &mut ManagerErrors,
    manager: PackageManagerType,
    result: Result<Vec<T>, String>,
) {
    match result {
        Ok(items) => {
            errors.remove(&manager);
            items_by_manager.insert(manager, items);
        }
        Err(error) => {
            items_by_manager.remove(&manager);
            errors.insert(manager, error);
        }
    }
}

pub fn apply_manager_counted_items_result<T>(
    items_by_manager: &mut HashMap<PackageManagerType, (usize, Vec<T>)>,
    errors: &mut ManagerErrors,
    manager: PackageManagerType,
    result: Result<Vec<T>, String>,
) {
    match result {
        Ok(items) => {
            errors.remove(&manager);
            let count = items.len();
            items_by_manager.insert(manager, (count, items));
        }
        Err(error) => {
            let count = items_by_manager
                .get(&manager)
                .map(|(count, _)| *count)
                .unwrap_or(0);
            items_by_manager.insert(manager, (count, Vec::new()));
            errors.insert(manager, error);
        }
    }
}

pub fn joined_manager_names(errors: &ManagerErrors) -> String {
    let mut names: Vec<_> = errors.keys().map(PackageManagerType::name).collect();
    names.sort_unstable();
    names.join(", ")
}
