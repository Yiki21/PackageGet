//! Built-in package manager implementations for Updater.
//!
//! This crate contains concrete command execution without depending on Iced or
//! the Updater user interface. Managers implement the object-safe contracts
//! from `updater-manager-api` and migrate here incrementally.

#![deny(missing_docs)]

mod apt;
mod command;
mod dnf;
mod flatpak;
mod homebrew;
mod pacman;
mod progress;
mod zypper;

pub use apt::AptManager;
pub use dnf::DnfManager;
pub use flatpak::FlatpakManager;
pub use homebrew::HomebrewManager;
pub use pacman::PacmanManager;
pub use progress::CommandProgress;
pub use zypper::ZypperManager;
