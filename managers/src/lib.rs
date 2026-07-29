//! Built-in package manager implementations for Updater.
//!
//! This crate contains concrete command execution without depending on Iced or
//! the Updater user interface. Managers implement the object-safe contracts
//! from `updater-manager-api` and migrate here incrementally.

#![deny(missing_docs)]
