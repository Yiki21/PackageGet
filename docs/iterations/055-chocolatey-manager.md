# Iteration 055: Chocolatey Manager

## Status

Completed

## Goal

Add Chocolatey as the second Windows application manager while preserving the
object-safe manager boundary and explicit machine-scope authorization policy.

## Scope

- [x] Add `builtin:chocolatey` with Windows-only platform metadata.
- [x] Cover availability, installed inventory, updates, search, install,
  upgrade, and uninstall through the existing manager trait.
- [x] Parse `choco list/outdated/search --limit-output` pipe contracts without
  depending on localized table headings or the removed `--local-only` switch.
- [x] Freeze system scope and Chocolatey origin into package targets.
- [x] Reject version-pinned, user-scope, wrong-origin, and ambiguous targets.
- [x] Add offline parser/argv tests and a Windows-native contract test.

## Non-goals

- Chocolatey source administration, package pinning, feature configuration, or
  package approval workflows.
- User-local Chocolatey installations with a different command or scope model.
- Running a real package install in CI.

## Verification

- `cargo fmt --all -- --check`
- `cargo check -p updater-managers --all-targets --locked --jobs 1`
- `cargo test -p updater-managers --all-targets --locked --jobs 1 -- --test-threads=1`
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- Windows CI runs `chocolatey_contract`; real Chocolatey availability remains
  an explicit ignored smoke.

## Follow-up

Run the native Windows availability/read-only inventory smoke before adding
Chocolatey source selection or another Windows manager.
