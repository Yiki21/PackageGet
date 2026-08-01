//! Built-in package manager implementations for Updater.
//!
//! This crate contains concrete command execution without depending on Iced or
//! the Updater user interface. Managers implement the object-safe contracts
//! from `updater-manager-api` and migrate here incrementally.

#![deny(missing_docs)]

mod apt;
mod cargo;
mod catalog;
mod command;
mod dnf;
mod flatpak;
mod go;
mod homebrew;
mod npm;
mod pacman;
mod pipx;
mod pnpm;
mod progress;
mod zypper;

pub use apt::AptManager;
pub use cargo::CargoManager;
pub use catalog::{builtin_managers, builtin_managers_for};
pub use dnf::DnfManager;
pub use flatpak::FlatpakManager;
pub use go::GoManager;
pub use homebrew::HomebrewManager;
pub use npm::NpmManager;
pub use pacman::PacmanManager;
pub use pipx::PipxManager;
pub use pnpm::PnpmManager;
pub use progress::CommandProgress;
pub use zypper::ZypperManager;
