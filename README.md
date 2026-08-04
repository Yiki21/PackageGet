# updater

A cross-platform desktop GUI built with Rust and `iced` for viewing, searching, installing, and updating packages across multiple package managers.

## Overview

`updater` brings package management workflows from different tools into a single desktop interface.

You can use it to:

- Detect which package managers are available on the current system
- View available package updates in one place
- Search for and install packages across package managers
- Browse installed packages and uninstall them
- Manage system packages and development tools without switching between commands such as `apt`, `dnf`, `cargo`, `npm`, and `flatpak`

It is especially useful when:

- You use multiple package managers and want a central place to manage updates
- You prefer not to switch frequently between different command-line tools
- You want a GUI for batch installation, removal, and update operations

Currently supported package managers:

- System packages: `apt`, `dnf`, `pacman`, `zypper`, and Windows `winget`
- Windows applications: `scoop` and `chocolatey`
- Applications: `flatpak`, `snap`, and `homebrew`
- Development tools: `cargo`, `go`, `npm`, `pnpm`, `pipx`, `uv tool`, `.NET global tools`, RubyGems, Composer Global, and Nix profiles

The built-in catalog is filtered by platform. Linux adds the native system/application managers and all development managers. Windows uses `winget`, `scoop`, `chocolatey`, and the development managers except Nix. macOS uses `homebrew` plus the development managers, including Nix. Windows and macOS manager contracts are tested on native CI runners. The `1.0.0` packaging pipeline builds unsigned Linux, Windows, and macOS artifacts with a shared checksum manifest.

Nix is deliberately not auto-enabled: choose one user profile from Settings first. Its initial contract supports installed packages and explicit install/update/uninstall operations while preserving flake identity. It does not advertise update inventory or package search, because `nix profile` has no read-only list-updates command or profile-scoped catalog.

## Features

- Automatically detects package managers available in the current environment
- Shows update counts and detailed package lists for each package manager
- Updates selected packages in batches
- Lists installed packages with search, sorting, and batch removal support
- Searches for packages across package managers and installs them in batches
- Manages enabled package managers from the settings page
- Supports custom executable paths for package managers
- Supports a custom binary installation directory for Go packages
- Supports one explicitly selected current-user Nix profile on Linux and macOS
- Saves configuration in the user configuration directory for use across restarts

## Build requirements

All platforms require the stable Rust toolchain and a native C/C++ build toolchain. The following additional packages are Linux build requirements:

- `cargo`
- `mold`, which is used by default for Linux builds
- A C/C++ build toolchain, such as `gcc` or `clang`
- `pkg-config`
- OpenSSL development libraries
- Wayland or X11 development libraries, plus `libxkbcommon`
- `pkexec`, usually provided by `polkit`, to install, remove, or update system packages

Install the required dependencies on common distributions:

```bash
# Debian / Ubuntu
sudo apt update
sudo apt install -y build-essential mold pkg-config libssl-dev libwayland-dev libx11-dev libx11-xcb-dev libxkbcommon-dev libxkbcommon-x11-dev policykit-1
```

```bash
# Fedora
sudo dnf install -y gcc gcc-c++ mold pkgconf-pkg-config openssl-devel wayland-devel libX11-devel libxkbcommon-devel libxkbcommon-x11-devel polkit
```

```bash
# Arch Linux
sudo pacman -S --needed base-devel mold pkgconf openssl wayland libx11 libxkbcommon libxkbcommon-x11 polkit
```

## Running for development

Run the application from a Wayland or X11 desktop session:

```bash
cargo run -p updater
```

## Installation

### Option 1: Install a release package

If the repository has published a release for your platform, download one of these artifacts:

- Linux native packages: `.deb`, `.rpm`, or Arch Linux `.pkg.tar.zst`
- Linux portable packages: glibc/musl `.tar.gz` or glibc `.AppImage`, for x86_64 and aarch64
- Windows x86_64: portable `.zip` or per-user setup `.exe`
- macOS arm64/x86_64: `.app.zip` or `.dmg`

GitHub Actions builds these packages automatically. Windows and macOS artifacts are currently unsigned: SmartScreen or Gatekeeper may block or warn on first launch. Verify the downloaded file against `SHA256SUMS`; do not treat the absence of a warning as signature verification.

Install an Arch Linux release package with:

```bash
sudo pacman -U ./updater-*.pkg.tar.zst
```

After installing a release package, launch Updater from your desktop application menu.

To run a portable glibc or musl archive:

```bash
tar -xzf updater-*-linux-*-glibc.tar.gz
cd updater-*-linux-*-glibc
./updater
```

The musl archive is built and validated on Alpine Linux. It still uses the host Wayland/X11, font, graphics, and desktop runtime libraries, so it is not a fully static binary and is not intended for glibc distributions.

To run an AppImage:

```bash
chmod +x updater-*-linux-*.AppImage
./updater-*-linux-*.AppImage
```

Portable archives and AppImages do not install the fixed-path Polkit helper or policy. Read-only workflows and user-scoped managers work directly; use a DEB, RPM, or Arch package when privileged APT, DNF, Pacman, or Zypper writes are required. A Flatpak distribution is outside the 1.0 scope because preserving the host package-manager CLI contract would require host-spawn permissions and a separate sandbox capability profile.

### Option 2: Build from source

Install the build requirements listed above, then run:

```bash
cargo build --release -p updater --locked
```

After the build completes, run the binary directly:

```bash
./target/release/updater
```

To install it in your local binary directory:

```bash
install -Dm755 target/release/updater ~/.local/bin/updater
```

Make sure `~/.local/bin` is included in your `PATH`.

## Usage

Start the application with:

```bash
updater
```

If it is not installed in your `PATH`, run the built binary directly:

```bash
./target/release/updater
```

On first launch, the application automatically detects package managers available in the current environment. The basic workflow is:

1. Open the updates page to view available updates from each package manager.
2. Select packages and run a batch update.
3. Open the installed packages page to browse, search, sort, or remove packages in batches.
4. Open the search page to find and install new packages across package managers.
5. Open the settings page to enable or disable package managers, configure custom executable paths, and select a Nix user profile when needed.

Additional notes:

- System package changes run through Updater's restricted helper and request authorization through `pkexec`; Updater never reads or stores the administrator password
- Configuration is stored in `updater/config.json` under the user configuration directory
- See [Configuration](docs/configuration.md) for the file schema and reset instructions
- If a package manager is not detected, you can specify its executable path manually from the settings page

## System package authorization

Native DEB, RPM, and Arch release packages install `/usr/lib/updater/updater-system-helper` and four `com.ayi.updater.*` Polkit actions. Portable tar archives and AppImages do not install these system files. The actions provide the Updater icon, vendor, operation-specific description, and localized authentication message. The active desktop Polkit agent still owns the dialog layout, colors, typography, password controls, and authentication itself.

The helper accepts only `install`, `update`, `remove`, and metadata `refresh` requests for APT, DNF, Pacman, and Zypper. It validates package identifiers and directly executes fixed system binaries without a shell. Custom executable paths remain available for detection and read-only queries, but are deliberately not used by privileged system package operations.

Running the unpackaged GUI directly supports all read-only workflows. To exercise privileged system package changes from a source build, install the release helper, policy, and icon as root first; installing a generated release package is the recommended way to keep these assets consistent.

## Release status

`1.0.0` is an unsigned cross-platform release. It includes Linux native packages, glibc/musl portable archives, AppImages, Windows x86_64 portable/setup packages, and macOS arm64/x86_64 app archives and DMGs. Verify every download against the matching `SHA256SUMS`; Windows SmartScreen and macOS Gatekeeper may warn because signing and notarization are intentionally outside the 1.0 policy. See [RELEASE_NOTES.md](RELEASE_NOTES.md) for platform limitations and compatibility notes.
