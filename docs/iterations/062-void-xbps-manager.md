# Iteration 062: Void Linux XBPS Manager

- Date: 2026-08-04
- Status: In progress
- ROADMAP phase: Phase 7 - package-manager ecosystem expansion

## Goal

Add a Linux-only XBPS system manager for Void Linux using the official query
and dry-run transaction formats, without deriving updates from a mutating
upgrade command.

## Confirmed contract

- The manager ID is `builtin:xbps`; availability requires `xbps-query`,
  `xbps-install`, and `xbps-remove` from one command family.
- Installed inventory uses `xbps-query --list-pkgs` and accepts only the `ii`
  installed state, preserving package name, version, description, system scope,
  and XBPS origin.
- Update discovery uses read-only `xbps-install --update --dry-run`; the official
  six-field transaction output is joined to installed inventory, and only
  `update` actions are exposed.
- Search uses repository mode `xbps-query --repository --search`, preserving
  installed identity where present and reporting other results as not installed.
- Install/update/remove and repository sync use only fixed
  `/usr/bin/xbps-install` and `/usr/bin/xbps-remove` helper plans with explicit
  non-interactive arguments.

## Scope

- [ ] Add the direct `XbpsManager` implementation and Linux catalog entry.
- [ ] Extend the fixed-path system helper with XBPS install, update, remove, and
  repository-sync plans.
- [ ] Add parser, identity, command, unsupported-platform, and offline public API
  contracts.
- [ ] Add a Void Linux read-only CLI smoke to Linux CI.
- [ ] Include the fixed helper and Polkit assets in portable tar archives behind
  an explicit root-run installer, so Void Linux release users can enable writes.
- [ ] Update user documentation, brand attribution, ROADMAP progress, and the
  iteration index.

## Non-goals

- Repository/key management, hold/repolock toggles, orphan cleanup, cache
  cleanup, alternative selection, or arbitrary root/config directories.
- Running a real upgrade during update discovery or CI.
- Version-pinned writes in the first contract.

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --workspace --all-targets --locked --jobs 1`
- [ ] `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- [ ] `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- [ ] `cargo build --workspace --locked --jobs 1`
- [ ] Void Linux real read-only CLI smoke.
- [ ] GitHub Actions Linux/Windows/macOS CI.

## Official CLI basis

- [xbps-query(1)](https://man.voidlinux.org/xbps-query.1)
- [xbps-install(1)](https://man.voidlinux.org/xbps-install.1)
- [xbps-remove(1)](https://man.voidlinux.org/xbps-remove.1)
- [Official Void Linux OCI images](https://github.com/void-linux/void-containers)
