use std::collections::HashMap;

use updater_manager_api::ManagerId;

pub type ManagerErrors = HashMap<ManagerId, String>;

pub fn apply_manager_items_result<T>(
    items_by_manager: &mut HashMap<ManagerId, Vec<T>>,
    errors: &mut ManagerErrors,
    manager: ManagerId,
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
    items_by_manager: &mut HashMap<ManagerId, (usize, Vec<T>)>,
    errors: &mut ManagerErrors,
    manager: ManagerId,
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
            items_by_manager.insert(manager.clone(), (count, Vec::new()));
            errors.insert(manager, error);
        }
    }
}
