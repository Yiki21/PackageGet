//! Built-in package manager implementations for Updater.
//!
//! This crate contains concrete command execution without depending on Iced or
//! the Updater user interface. Managers implement the object-safe contracts
//! from `updater-manager-api` and migrate here incrementally.

#![deny(missing_docs)]

mod apt;
mod bun;
mod cargo;
mod catalog;
mod chocolatey;
mod command;
mod composer;
mod dnf;
mod dotnet;
mod flatpak;
mod go;
mod homebrew;
mod nix_profile;
mod npm;
mod pacman;
mod pipx;
mod pnpm;
mod progress;
mod rubygems;
mod scoop;
mod snap;
mod uv;
mod winget;
mod zypper;

pub use apt::AptManager;
pub use bun::BunManager;
pub use cargo::CargoManager;
pub use catalog::{builtin_managers, builtin_managers_for};
pub use chocolatey::ChocolateyManager;
pub use composer::ComposerGlobalManager;
pub use dnf::DnfManager;
pub use dotnet::DotnetToolManager;
pub use flatpak::FlatpakManager;
pub use go::{GoManager, configured_go_bin_dir, set_configured_go_bin_dir};
pub use homebrew::HomebrewManager;
pub use nix_profile::{NixProfileManager, configured_nix_profile, set_configured_nix_profile};
pub use npm::NpmManager;
pub use pacman::PacmanManager;
pub use pipx::PipxManager;
pub use pnpm::PnpmManager;
pub use rubygems::RubyGemsManager;
pub use scoop::ScoopManager;
pub use snap::SnapManager;
pub use uv::UvManager;
pub use winget::WingetManager;
pub use zypper::ZypperManager;
