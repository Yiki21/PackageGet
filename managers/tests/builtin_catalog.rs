use std::{collections::BTreeSet, sync::Arc};

use updater_manager_api::{AuthorizationHint, ManagerCategory, PackageManager, Platform};
use updater_managers::builtin_managers_for;

const EXPECTED_IDS: [&str; 20] = [
    "builtin:apt",
    "builtin:dnf",
    "builtin:pacman",
    "builtin:zypper",
    "builtin:portage",
    "builtin:xbps",
    "builtin:flatpak",
    "builtin:snap",
    "builtin:homebrew",
    "builtin:cargo",
    "builtin:go",
    "builtin:npm",
    "builtin:pnpm",
    "builtin:bun",
    "builtin:pipx",
    "builtin:uv",
    "builtin:dotnet-tool",
    "builtin:rubygems",
    "builtin:composer-global",
    "builtin:nix-profile",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthorizationClass {
    None,
    MayRequireElevation,
    RequiresElevation,
}

#[test]
fn catalog_contains_every_direct_builtin_in_stable_product_order() {
    let managers = builtin_managers_for(Platform::Linux);
    let ids = managers
        .iter()
        .map(|manager| manager.descriptor().id().as_str())
        .collect::<Vec<_>>();

    assert_eq!(ids, EXPECTED_IDS);
    assert_eq!(
        ids.iter().copied().collect::<BTreeSet<_>>().len(),
        ids.len()
    );
}

#[test]
fn catalog_is_object_safe_and_descriptors_have_valid_contracts() {
    let managers: Vec<Arc<dyn PackageManager>> = builtin_managers_for(Platform::Linux);

    for manager in managers {
        let descriptor = manager.descriptor();
        assert!(descriptor.id().as_str().starts_with("builtin:"));
        assert!(!descriptor.capabilities().is_empty());
        assert!(!descriptor.display_name().trim().is_empty());
        assert!(!descriptor.description().trim().is_empty());
    }
}

#[test]
fn catalog_freezes_descriptor_display_category_platform_and_authorization() {
    let expected = [
        (
            "APT",
            ManagerCategory::System,
            vec![Platform::Linux],
            AuthorizationClass::RequiresElevation,
        ),
        (
            "DNF",
            ManagerCategory::System,
            vec![Platform::Linux],
            AuthorizationClass::RequiresElevation,
        ),
        (
            "Pacman",
            ManagerCategory::System,
            vec![Platform::Linux],
            AuthorizationClass::RequiresElevation,
        ),
        (
            "Zypper",
            ManagerCategory::System,
            vec![Platform::Linux],
            AuthorizationClass::RequiresElevation,
        ),
        (
            "Portage",
            ManagerCategory::System,
            vec![Platform::Linux],
            AuthorizationClass::RequiresElevation,
        ),
        (
            "XBPS",
            ManagerCategory::System,
            vec![Platform::Linux],
            AuthorizationClass::RequiresElevation,
        ),
        (
            "Flatpak",
            ManagerCategory::Application,
            vec![Platform::Linux],
            AuthorizationClass::MayRequireElevation,
        ),
        (
            "Snap",
            ManagerCategory::Application,
            vec![Platform::Linux],
            AuthorizationClass::RequiresElevation,
        ),
        (
            "Homebrew",
            ManagerCategory::Application,
            vec![Platform::Linux, Platform::MacOs],
            AuthorizationClass::MayRequireElevation,
        ),
        (
            "Cargo",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::None,
        ),
        (
            "Go",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::None,
        ),
        (
            "npm",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::None,
        ),
        (
            "pnpm",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::None,
        ),
        (
            "Bun",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::None,
        ),
        (
            "pipx",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::None,
        ),
        (
            "uv tool",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::None,
        ),
        (
            ".NET global tools",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::None,
        ),
        (
            "RubyGems",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::MayRequireElevation,
        ),
        (
            "Composer Global",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::Windows, Platform::MacOs],
            AuthorizationClass::None,
        ),
        (
            "Nix profile",
            ManagerCategory::Development,
            vec![Platform::Linux, Platform::MacOs],
            AuthorizationClass::None,
        ),
    ];

    for (manager, (display_name, category, platforms, authorization)) in
        builtin_managers_for(Platform::Linux).iter().zip(expected)
    {
        let descriptor = manager.descriptor();
        assert_eq!(descriptor.display_name(), display_name);
        assert_eq!(descriptor.category(), category);
        assert_eq!(
            descriptor.platforms().iter().copied().collect::<Vec<_>>(),
            platforms
        );
        let actual_authorization = match descriptor.authorization() {
            AuthorizationHint::None => AuthorizationClass::None,
            AuthorizationHint::MayRequireElevation { .. } => {
                AuthorizationClass::MayRequireElevation
            }
            AuthorizationHint::RequiresElevation { .. } => AuthorizationClass::RequiresElevation,
            _ => panic!("unexpected authorization hint for {}", descriptor.id()),
        };
        assert_eq!(actual_authorization, authorization);
    }
}

#[test]
fn each_catalog_call_returns_independent_arc_instances() {
    let first = builtin_managers_for(Platform::Linux);
    let second = builtin_managers_for(Platform::Linux);

    assert_eq!(first.len(), second.len());
    for (left, right) in first.iter().zip(&second) {
        assert_eq!(left.descriptor().id(), right.descriptor().id());
        assert!(!Arc::ptr_eq(left, right));
    }
}

#[test]
fn platform_catalogs_only_include_advertised_managers() {
    let linux = builtin_managers_for(Platform::Linux)
        .into_iter()
        .map(|manager| manager.descriptor().id().as_str().to_owned())
        .collect::<Vec<_>>();
    let macos = builtin_managers_for(Platform::MacOs)
        .into_iter()
        .map(|manager| manager.descriptor().id().as_str().to_owned())
        .collect::<Vec<_>>();
    let windows = builtin_managers_for(Platform::Windows)
        .into_iter()
        .map(|manager| manager.descriptor().id().as_str().to_owned())
        .collect::<Vec<_>>();

    assert_eq!(linux, EXPECTED_IDS);
    assert_eq!(
        macos,
        [
            "builtin:homebrew",
            "builtin:cargo",
            "builtin:go",
            "builtin:npm",
            "builtin:pnpm",
            "builtin:bun",
            "builtin:pipx",
            "builtin:uv",
            "builtin:dotnet-tool",
            "builtin:rubygems",
            "builtin:composer-global",
            "builtin:nix-profile",
        ]
    );
    assert_eq!(
        windows,
        [
            "builtin:winget",
            "builtin:scoop",
            "builtin:chocolatey",
            "builtin:cargo",
            "builtin:go",
            "builtin:npm",
            "builtin:pnpm",
            "builtin:bun",
            "builtin:pipx",
            "builtin:uv",
            "builtin:dotnet-tool",
            "builtin:rubygems",
            "builtin:composer-global",
        ]
    );
}
