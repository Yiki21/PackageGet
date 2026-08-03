# Updater 0.3.0-beta.3

`0.3.0-beta.3` is an unsigned Linux preview that adds four manager contracts while keeping package identity, scope, source, and write operations explicit.

## Highlights

- Adds Linux Snap inventory, update discovery, search, install, update, and uninstall while preserving channel, confinement, revision, and refresh state.
- Adds RubyGems management with explicit repository, `GEM_HOME`, installed-version, default-gem, and user/system scope boundaries.
- Adds Composer Global management for direct runtime requirements in the active Composer home; project, development, and transitive dependencies remain outside its inventory.
- Adds one explicitly selected current-user Nix profile on Linux and macOS, preserving profile element, original/locked flake, attribute, outputs, and store-path identity.
- Filters Updates and Search sources by advertised capability, so managers without a truthful read-only contract are omitted instead of reported as failures.
- Verifies the expanded catalog and write argv on native Linux, Windows, and macOS CI runners.
- Release assets include `.deb`, `.rpm`, and Arch Linux `.pkg.tar.zst` packages plus `SHA256SUMS`.

## Compatibility

- Configuration and activity history continue to use the direct schemas introduced in `0.3.0-beta.1`; unsupported legacy configuration opens the recovery flow without rewriting the original file.
- System package writes require the helper and Polkit policy installed by a release package. Running only the GUI binary supports read-only workflows.
- Snap writes use the explicit snapd authorization path; custom executable paths are not promoted into a privileged command.
- RubyGems preserves multiple installed versions and does not uninstall default gems as ordinary user packages.
- Composer Global manages only direct `require` entries in one active Composer home.
- Nix is not auto-enabled. Select one absolute user profile in Settings first; system/default profiles and duplicate manager IDs are rejected.
- Nix does not advertise update inventory or search because `nix profile` has no read-only list-updates command or profile-scoped catalog.

## Preview Limits

- Packages are unsigned. Verify downloads with the release `SHA256SUMS` file before installation.
- Windows and macOS artifacts are not part of this preview.
- Arch Linux artifacts target x86_64. Debian and RPM workflows also build native arm64/aarch64 packages.
- Cancellation stops before the next manager group; it does not terminate an active package-manager transaction.
- The desktop Polkit authentication agent owns password controls, dialog styling, and the top-level authorization window title.

## 1.0 Release Policy

The 1.0 release is intentionally unsigned on Linux, Windows, and macOS. Code signing, Apple notarization, and signing credentials are not 1.0 release gates. Every published asset must be listed in the release bundle and checked against the matching `SHA256SUMS` file; the public CI run and asset manifest are the release provenance. Windows and macOS downloads will continue to show an explicit unsigned warning.

The beta3 notes above describe the historical Linux-only public preview. Cross-platform portable assets and active-command cancellation are being completed in the 1.0 roadmap and are not retroactively claimed as part of that tag.
