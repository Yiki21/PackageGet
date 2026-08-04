# Iteration 061: Gentoo Portage Manager

- Date: 2026-08-04
- Status: Complete
- ROADMAP phase: Phase 7 - package-manager ecosystem expansion

## Goal

Add a Linux-only Portage system manager for Gentoo without flattening Portage
SLOT identity or treating rebuilds and newly pulled dependencies as ordinary
package updates.

## Confirmed contract

- The manager ID is `builtin:portage`; availability requires both `emerge` and
  `qlist`, and reports the Portage version returned by `emerge --version`.
- Installed inventory uses `qlist -I -F` with an explicit tab-separated format
  for category/package, PVR, SLOT, and repository.
- Installed identity is `category/package:slot`, including SLOT `0`, so
  concurrently installed SLOTs remain independently selectable.
- Update discovery uses read-only `emerge --pretend --update --deep --newuse
  --with-bdeps=y --package-moves=n --color=n --quiet --verbose @world` and only
  admits `U`/`D` version transitions that can be joined to one installed
  category/package, version, and SLOT. Rebuilds and new dependencies are not
  exposed as package updates.
- Search uses `emerge --search --package-moves=n --color=n`; masked entries are
  excluded, while visible results retain current/available version, homepage,
  and description.
- Install and update merge validated atoms through the fixed privileged helper;
  installed writes preserve `category/package:SLOT::repository`. Uninstall uses
  dependency-aware `emerge --depclean` rather than unsafe `--unmerge`.
  Metadata refresh uses `emerge --sync`.

## Scope

- [x] Add the direct `PortageManager` implementation and Linux catalog entry.
- [x] Extend the fixed-path system helper with manager-specific Portage atom
  validation and fixed `emerge` command plans.
- [x] Add parser, identity, command, unsupported-platform, and offline public API
  contracts.
- [x] Add a Gentoo stage3 read-only CLI smoke to Linux CI.
- [x] Include the fixed helper and Polkit assets in portable tar archives behind
  an explicit root-run installer, so Gentoo release users can enable writes.
- [x] Update user documentation, brand attribution, ROADMAP progress, and the
  iteration index.

## Non-goals

- Editing USE flags, keywords, masks, licenses, news, profiles, sets, overlays,
  binary-package policy, or Portage configuration files.
- Presenting USE-driven rebuilds, new dependencies, or new SLOT installs as
  ordinary version updates.
- Bypassing dependency safety with `emerge --unmerge`.

## Verification

- [x] `cargo fmt --all -- --check`
- [x] `cargo check --workspace --all-targets --locked --jobs 1`
- [x] `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- [x] `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- [x] `cargo build --workspace --locked --jobs 1`
- [x] Gentoo stage3 real read-only manager smoke.
- [x] GitHub Actions Linux/Windows/macOS CI.

## Evidence

- Offline public API and parser/command contracts pass; the ordinary suite does
  not invoke Portage or a privileged write.
- The ignored manager smoke passes inside pinned official Gentoo stage3 image
  `sha256:08914e15d306...`, covering availability, installed inventory, count,
  SLOT/system scope, repository origin, and exact current version.
- Main CI run `30927472564` and 17-asset Package prebuild run `30927472410`
  completed successfully.

## Official CLI basis

- [Portage emerge manual](https://dev.gentoo.org/~zmedico/portage/doc/man/emerge.1.html)
- [Portage `portageq` implementation and command documentation](https://github.com/gentoo/portage/blob/master/bin/portageq)
- [portage-utils `qlist` implementation](https://github.com/gentoo/portage-utils/blob/master/qlist.c)
