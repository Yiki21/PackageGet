# Updater 0.3.0-beta.2

`0.3.0-beta.2` is an unsigned Linux preview that expands the built-in manager catalog and freezes portable manager behavior on native Linux, Windows, and macOS runners.

## Highlights

- Adds `uv tool` management for installed tools, update discovery, install, update, and uninstall.
- Adds current-user `.NET global tools` management with JSON inventory, NuGet metadata update discovery, tool-only search, and typed write targets.
- Adds the first complete Winget manager contract for Windows source builds.
- Admits Cargo, Go, npm, pnpm, and pipx on Windows with native command and filesystem fixtures; Homebrew remains natively verified on macOS.
- Keeps manager package identity, source, scope, path containment, and write arguments strict across platforms.
- Fixes pnpm's no-update exit-status handling and serializes only fixture contracts that share executable state while retaining normal CI parallelism.
- Release assets include `.deb`, `.rpm`, and Arch Linux `.pkg.tar.zst` packages plus `SHA256SUMS`.

## Compatibility

- Configuration and activity history continue to use the direct schemas introduced in `0.3.0-beta.1`; unsupported legacy configuration opens the recovery flow without rewriting the original file.
- System package writes require the helper and Polkit policy installed by a release package. Running only the GUI binary supports read-only workflows.
- `uv tool` does not advertise search because the current CLI lacks a safe search contract that preserves configured private-index semantics.
- `.NET global tools` manages only current-user global scope. Local manifests and arbitrary `--tool-path` installations remain separate.

## Preview Limits

- Packages are unsigned. Verify downloads with the release `SHA256SUMS` file before installation.
- Windows and macOS artifacts are not part of this preview.
- Arch Linux artifacts target x86_64. Debian and RPM workflows also build native arm64/aarch64 packages.
- Cancellation stops before the next manager group; it does not terminate an active package-manager transaction.
- The desktop Polkit authentication agent owns password controls, dialog styling, and the top-level authorization window title.
