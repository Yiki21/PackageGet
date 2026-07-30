use updater_core::{Config, ManagerConfig, ManagerRegistry, register_builtin_managers};
use updater_manager_api::ManagerId;

#[tokio::test]
#[ignore = "requires locally installed pnpm and Homebrew"]
async fn desktop_environment_finds_and_runs_user_managed_package_managers() {
    let pnpm_id = ManagerId::parse("builtin:pnpm").expect("valid pnpm ID");
    let homebrew_id = ManagerId::parse("builtin:homebrew").expect("valid Homebrew ID");
    let config = Config {
        managers: vec![
            ManagerConfig::new(pnpm_id.clone()),
            ManagerConfig::new(homebrew_id.clone()),
        ],
        ..Config::default()
    };
    let mut registry = ManagerRegistry::new();
    register_builtin_managers(&mut registry).expect("register built-in managers");
    let pnpm = registry.get(&pnpm_id).expect("registered pnpm manager");
    let homebrew = registry
        .get(&homebrew_id)
        .expect("registered Homebrew manager");

    for (manager, id) in [(&pnpm, &pnpm_id), (&homebrew, &homebrew_id)] {
        let manager_config = config.manager(id).expect("configured manager");
        let availability = manager
            .availability(manager_config)
            .await
            .expect("run availability check");
        assert!(
            availability.is_available(),
            "{} availability check failed: {availability:?}",
            manager.descriptor().display_name(),
        );
    }

    let (pnpm_count, pnpm_updates, homebrew_count, homebrew_updates) = tokio::join!(
        pnpm.count_installed(config.manager(&pnpm_id).unwrap()),
        pnpm.updates(config.manager(&pnpm_id).unwrap(), false),
        homebrew.count_installed(config.manager(&homebrew_id).unwrap()),
        homebrew.updates(config.manager(&homebrew_id).unwrap(), false),
    );

    let pnpm_count = pnpm_count.unwrap_or_else(|error| panic!("pnpm count failed: {error}"));
    let pnpm_updates = pnpm_updates.unwrap_or_else(|error| panic!("pnpm updates failed: {error}"));
    let homebrew_count =
        homebrew_count.unwrap_or_else(|error| panic!("Homebrew count failed: {error}"));
    let homebrew_updates =
        homebrew_updates.unwrap_or_else(|error| panic!("Homebrew updates failed: {error}"));

    println!(
        "pnpm: {pnpm_count} installed, {} updates",
        pnpm_updates.len()
    );
    println!(
        "Homebrew: {homebrew_count} installed, {} updates",
        homebrew_updates.len()
    );
}
