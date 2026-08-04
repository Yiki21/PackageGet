# Iteration 054: Scoop Manager

## Status

Completed

## Goal

Add the first post-1.0 Windows user-application manager without introducing a
new UI or core dispatch path. Scoop is a good next candidate because it is
user-oriented, has an explicit global scope, and does not require a privileged
helper for normal installs.

## Scope

- [x] Add `builtin:scoop` with Windows-only platform metadata and no elevation hint.
- [x] Cover availability, installed inventory, update listing, search, install,
  update, and uninstall through the existing object-safe `PackageManager` trait.
- [x] Read installed packages through the official `scoop export` JSON contract.
- [x] Parse `scoop status` and `scoop search` table output with header detection,
  preserving package name, version, bucket source, and scope.
- [x] Freeze bucket identity and local/global scope into `PackageTarget` before writes.
- [x] Reject version-pinned targets and ambiguous duplicate local/global names
  instead of guessing a write target.
- [x] Add offline parser and command-construction tests plus a Windows-native
  contract test for platform availability.

## Non-goals

- Scoop buckets management, hold/unhold, cache cleanup, or Scoop self-update.
- Treating global apps as ordinary user apps.
- Executing `scoop update *` or another manager-wide write through the core group API.
- Adding Chocolatey, MAS, or another Windows manager in the same iteration.

## Verification

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --locked --jobs 1`
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- `cargo build --workspace --locked --jobs 1`
- Windows CI runs `scoop_contract` on the native runner; real Scoop availability remains an explicit ignored smoke.

## Follow-up

Run the native Windows contract and a real read-only Scoop smoke before
choosing Chocolatey or extending Scoop with bucket management.
