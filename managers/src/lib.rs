//! Built-in package manager implementations for Updater.
//!
//! This crate contains concrete command execution without depending on Iced or
//! the Updater user interface. Managers implement the object-safe contracts
//! from `updater-manager-api` and migrate here incrementally.

#![deny(missing_docs)]

mod apt;
mod command;
mod dnf;
mod progress;

pub use apt::AptManager;
pub use dnf::DnfManager;
pub use progress::CommandProgress;
