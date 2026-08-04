# Iteration 056: Windows Manager Native Read-only Smoke

## Status

Completed

## Goal

Exercise at least one Windows application manager against a real hosted-runner
installation while keeping the default test suite deterministic and offline.

## Scope

- [x] Extend Chocolatey's ignored smoke from availability to installed inventory.
- [x] Assert Chocolatey inventory preserves manager ID, machine scope, and origin.
- [x] Run Chocolatey's ignored smoke when the hosted Windows runner exposes
  `choco`; skip with an explicit message when the image no longer includes it.
- [x] Extend Winget and Scoop ignored smokes to read installed inventory.
- [x] Run Winget/Scoop smokes only when their commands already exist on the
  hosted runner, with an explicit skip message otherwise.
- [x] Keep search, update discovery, and all write operations out of native CI smoke.

## Decisions

- CI does not install Scoop or App Installer solely for smoke coverage; doing
  so would test bootstrap behavior and mutable external state rather than the
  application contract.
- Ignored tests remain opt-in locally. Only the Windows workflow invokes them
  with an exact test name.
- When `choco` is present, its inventory must be non-empty because the hosted
  image is expected to use Chocolatey for package management. Runner image
  changes must not prevent the remaining native manager contracts from running.

## Verification

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --locked --jobs 1`
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- Windows target checks compile all three native smoke tests.

## Follow-up

After the CI run records a successful native Chocolatey inventory, proceed to
the next cross-platform development manager rather than adding another Windows
manager immediately.
