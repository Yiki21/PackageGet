# Updater 1.0.0

`1.0.0` is an unsigned cross-platform release of Updater. It combines Linux native and portable formats with Windows x86_64 and macOS arm64/x86_64 packages, using one release bundle and one `SHA256SUMS` manifest.

## Highlights

- Supports Linux APT, DNF, Pacman, Zypper, Flatpak, Snap, Cargo, Go, npm, pnpm, pipx, uv, .NET global tools, RubyGems, Composer Global and Nix profiles.
- Supports Windows `winget` and macOS Homebrew through the same typed manager registry.
- Activity Center records start/finish time, package scope and per-manager success, failure, cancellation or not-started results.
- Cancellation waits for the active manager process to exit before reporting the final cancelled state.
- Linux native packages install the restricted Polkit helper and policy; portable archives and AppImages preserve user-scoped and read-only workflows without installing system authorization files.

## Release assets

- Linux: Debian `.deb`, RPM `.rpm`, Arch `.pkg.tar.zst`, glibc/musl `.tar.gz`, and glibc `.AppImage` for x86_64/aarch64 as applicable.
- Windows: x86_64 portable `.zip` and per-user setup `.exe`.
- macOS: arm64/x86_64 `.app.zip` and `.dmg`.
- Verify every downloaded asset against the matching `SHA256SUMS` file from this release.

## Compatibility and limits

- Configuration and Activity history keep the direct schemas introduced during the beta line. Older Activity records without timestamps, scope or manager outcomes remain readable with explicit unknown defaults.
- Windows and macOS artifacts are unsigned. SmartScreen or Gatekeeper may warn on first launch; signing and notarization are intentionally outside the 1.0 policy.
- Flatpak distribution is outside 1.0 because the application depends on host package-manager CLIs and privileged authorization boundaries.
- Portable archives and AppImages do not install the fixed-path Polkit helper. Use a native Linux package for privileged APT, DNF, Pacman or Zypper writes.

## Historical preview

`0.3.0-beta.3` was an unsigned Linux-only preview. Its release notes and assets remain available under the original tag; this release does not rewrite beta history.
