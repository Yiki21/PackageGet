# Updater 0.3.0-beta.1

`0.3.0-beta.1` is an unsigned Linux preview focused on the package-manager execution core and native Linux delivery.

## Highlights

- Direct manager registry and stable manager IDs replace the legacy dispatcher path.
- Discover installs and selected updates use frozen, reviewable execution plans before any write begins.
- Late asynchronous results no longer overwrite newer searches, refreshes, or reloads.
- Configuration load failures enter a visible recovery screen instead of silently continuing with defaults.
- APT, DNF, Pacman, and Zypper writes use Updater Polkit actions and a fixed, restricted privileged helper.
- One Linux binary supports both Wayland and X11 with the `com.ayi.updater` application identity.
- Release assets include `.deb`, `.rpm`, and Arch Linux `.pkg.tar.zst` packages plus `SHA256SUMS`.

## Compatibility

- Configuration and activity history use their current schemas directly. Unsupported legacy configuration opens the recovery flow without rewriting the original file.
- Legacy activity history is not migrated; Updater starts a new current-format history instead.
- System package writes require the helper and Polkit policy installed by a release package. Running only the GUI binary supports read-only workflows.

## Preview Limits

- Packages are unsigned. Verify downloads with the release `SHA256SUMS` file before installation.
- Windows and macOS artifacts are not part of this preview.
- Arch Linux artifacts target x86_64. Debian and RPM workflows also build native arm64/aarch64 packages.
- Cancellation stops before the next manager group; it does not terminate an active package-manager transaction.
- The desktop Polkit authentication agent owns password controls, dialog styling, and the top-level authorization window title.
