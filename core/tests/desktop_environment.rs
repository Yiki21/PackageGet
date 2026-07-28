use updater_core::{Config, PackageManagerType};

#[tokio::test]
#[ignore = "requires locally installed pnpm and Homebrew"]
async fn desktop_environment_finds_and_runs_user_managed_package_managers() {
    let config = Config::default();

    for manager in [PackageManagerType::Pnpm, PackageManagerType::Homebrew] {
        let availability = manager.availability_with_config(&config).await;
        assert!(
            availability.is_available(),
            "{} availability check failed: {}",
            manager.name(),
            availability.message()
        );
    }

    let (pnpm_count, pnpm_updates, homebrew_count, homebrew_updates) = tokio::join!(
        PackageManagerType::Pnpm.count_installed(&config),
        PackageManagerType::Pnpm.list_updates_with_refresh(&config, false),
        PackageManagerType::Homebrew.count_installed(&config),
        PackageManagerType::Homebrew.list_updates_with_refresh(&config, false),
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
