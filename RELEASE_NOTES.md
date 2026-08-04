# Updater 1.2.0

`1.2.0` is an unsigned cross-platform feature release of Updater. It adds native Gentoo Portage and Void Linux XBPS package management, real container-backed read-only validation, and optional privileged system integration for portable Linux archives while keeping the existing 17-asset bundle and one `SHA256SUMS` manifest.

## Highlights

- Adds a direct Gentoo Portage system manager for installed packages, read-only update discovery, search, install, update, and dependency-aware removal. Installed targets retain `category/package:SLOT` and repository identity.
- Filters Portage update plans to real ebuild or binary-package version transitions. Rebuilds, new dependencies, and new SLOTs are not presented as ordinary updates.
- Adds a direct Void Linux XBPS system manager using `xbps-query` inventory/search and the official six-field `xbps-install --update --dry-run` transaction format.
- Extends the restricted Polkit helper with fixed `/usr/bin/emerge`, `/usr/bin/xbps-install`, and `/usr/bin/xbps-remove` plans plus manager-specific input validation.
- Runs ignored manager tests against pinned official Gentoo and Void Linux container images in Linux CI, covering real availability, inventory, count, scope, origin, and exact-version queries.
- Includes the matching helper, policy, and icon in portable Linux tar archives behind an explicit root-run installer; extraction alone never modifies the host.

## Release assets

- Linux: Debian `.deb`, RPM `.rpm`, Arch `.pkg.tar.zst`, glibc/musl `.tar.gz`, and glibc `.AppImage` for x86_64/aarch64 as applicable.
- Windows: x86_64 portable `.zip` and per-user setup `.exe`.
- macOS: arm64/x86_64 `.app.zip` and `.dmg`.
- Verify every downloaded asset against the matching `SHA256SUMS` file from this release.

## Compatibility and limits

- Existing configuration and Activity history remain readable. Portage and XBPS are added to the Linux catalog and remain disabled when their required command families are unavailable.
- Portage requires both `emerge` and `qlist` from `portage-utils`. Update discovery follows the configured world plan and excludes masked search results; USE/profile/repository configuration remains owned by Portage.
- XBPS requires `xbps-query`, `xbps-install`, and `xbps-remove` from one command directory. Repository/key management, holds, orphan cleanup, and alternative selection remain outside this manager contract.
- Portable tar users must explicitly run `system-integration/install.sh` as root before privileged system-manager writes work. AppImages do not include the installer and remain read-only for APT, DNF, Pacman, Zypper, Portage, and XBPS.
- Windows and macOS artifacts are unsigned. SmartScreen or Gatekeeper may warn on first launch; signing and notarization are intentionally outside the 1.0 policy.
- Flatpak distribution is outside 1.0 because the application depends on host package-manager CLIs and privileged authorization boundaries.

## Previous release

`1.1.0` remains available under its original immutable tag and release assets. This release does not rewrite that history.
