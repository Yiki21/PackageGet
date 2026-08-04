# Updater 1.1.0

`1.1.0` is an unsigned cross-platform feature release of Updater. It expands package-manager administration and adds native Scoop, Chocolatey, and Bun Global contracts while preserving the 17-asset release bundle and one `SHA256SUMS` manifest.

## Highlights

- Adds a dedicated Package Managers surface for enabling sources, editing manager-owned settings, checking availability and runtime health, and copying redacted diagnostics without mixing those states into application preferences.
- Adds direct Windows Scoop and Chocolatey managers with typed source, scope, origin, update, search, and write contracts. Hosted Windows CI exercises offline fixtures and available read-only native inventory probes.
- Adds Bun Global on Linux, Windows, and macOS for current-user installed packages, read-only update discovery, install, update, and uninstall. Search remains unadvertised because Bun has no registry-aware directory search contract.
- Adds scalable manager selection, platform-aware add choices, and on-demand package details with retry support where a manager exposes a low-side-effect detail query.
- Consolidates manager availability, capability validation, execution outcomes, and transient executable-busy handling behind the shared typed contracts.

## Release assets

- Linux: Debian `.deb`, RPM `.rpm`, Arch `.pkg.tar.zst`, glibc/musl `.tar.gz`, and glibc `.AppImage` for x86_64/aarch64 as applicable.
- Windows: x86_64 portable `.zip` and per-user setup `.exe`.
- macOS: arm64/x86_64 `.app.zip` and `.dmg`.
- Verify every downloaded asset against the matching `SHA256SUMS` file from this release.

## Compatibility and limits

- Existing configuration and Activity history remain readable. Unknown manager records and manager-owned settings continue to round-trip without being rewritten into application preferences.
- Chocolatey represents machine-wide packages and requires elevation for writes; Scoop and Bun retain their own user/global scope rules instead of sharing a synthetic Windows scope.
- Bun Global accepts registry package identities and semantic versions only. File, Git, workspace, version-pinned uninstall, and fabricated search targets remain outside its first contract.
- Windows and macOS artifacts are unsigned. SmartScreen or Gatekeeper may warn on first launch; signing and notarization are intentionally outside the 1.0 policy.
- Flatpak distribution is outside 1.0 because the application depends on host package-manager CLIs and privileged authorization boundaries.
- Portable archives and AppImages do not install the fixed-path Polkit helper. Use a native Linux package for privileged APT, DNF, Pacman or Zypper writes.

## Previous release

`1.0.0` remains available under its original immutable tag and release assets. This release does not rewrite that history.
