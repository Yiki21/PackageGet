# Updater 1.2.1

`1.2.1` is an unsigned cross-platform reliability release of Updater. It reduces startup work, scopes package and health refreshes to the managers that actually changed, improves partial-result visibility, and fixes RubyGems error detection while preserving the existing 17-product-asset bundle and one `SHA256SUMS` manifest.

## Highlights

- Defers installed-package and update scans until their pages are first opened, so the initial screen loads without launching every configured package manager.
- Refreshes package data only for managers whose operations succeeded. Partial failures, cancellations, and managers that were not executed retain their existing caches and selections.
- Shows completed manager sections and manager-specific errors progressively while other update sources are still loading, without reporting a premature empty state.
- Reloads Installed and Updates data only for managers that were added, removed, or materially reconfigured; unchanged managers keep their caches, errors, selections, and in-flight work.
- Applies the same per-manager invalidation to Package Manager health checks. Changed managers become unchecked while unaffected health results and scans remain valid.
- Treats canonical RubyGems `ERROR:` output as a failed write even when `gem` exits with status 0, preserving the original error line in the structured failure.
- Fixes source-picker scrollbar overlap and keeps update actions usable at narrow window widths.

## Release assets

- Linux: Debian `.deb`, RPM `.rpm`, Arch `.pkg.tar.zst`, glibc/musl `.tar.gz`, and glibc `.AppImage` for x86_64/aarch64 as applicable.
- Windows: x86_64 portable `.zip` and per-user setup `.exe`.
- macOS: arm64/x86_64 `.app.zip` and `.dmg`.
- Verify every downloaded asset against the matching `SHA256SUMS` file from this release.

## Compatibility and limits

- Existing configuration and Activity history remain readable. This patch adds no Package Manager, capability, or persistent configuration schema.
- Package data and health results remain session-local. The first visit to Installed, Updates, or Health still performs the required real read-only scans.
- A manager configuration change invalidates only that manager. Search results for an affected manager are cleared rather than replaying a previous query against new settings.
- Health results have no background refresh or time-to-live policy. Once every enabled manager has a current result, `Recheck all` explicitly performs a full read-only scan.
- RubyGems operations that previously looked successful solely because of a zero exit status can now surface as failures when the CLI reports `ERROR:`.
- Windows and macOS artifacts are unsigned. SmartScreen or Gatekeeper may warn on first launch; signing and notarization are intentionally outside the 1.0 policy.
- Flatpak distribution is outside 1.0 because the application depends on host package-manager CLIs and privileged authorization boundaries.

## Previous release

`1.2.0` remains available under its original immutable tag and release assets. This release does not rewrite that history.
