# updater

A Wayland desktop GUI built with Rust and `iced` for viewing, searching, installing, and updating packages across multiple package managers.

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

- System packages: `apt`, `dnf`, `pacman`, `zypper`
- Applications and development tools: `flatpak`, `homebrew`, `cargo`, `go`, `npm`, `pnpm`, `pipx`

## Features

- Automatically detects package managers available in the current environment
- Shows update counts and detailed package lists for each package manager
- Updates selected packages in batches
- Lists installed packages with search, sorting, and batch removal support
- Searches for packages across package managers and installs them in batches
- Manages enabled package managers from the settings page
- Supports custom executable paths for package managers
- Supports a custom binary installation directory for Go packages
- Saves configuration in the user configuration directory for use across restarts

## Build requirements

- Rust toolchain: the project uses [`stable`](./rust-toolchain.toml) with the `rustfmt`, `clippy`, and `rust-analyzer` components
- `cargo`
- `mold`, which is used by default for Linux builds
- A C/C++ build toolchain, such as `gcc` or `clang`
- `pkg-config`
- OpenSSL development libraries
- A native Wayland desktop session, plus the `wayland` and `libxkbcommon` development libraries; the X11 backend is currently disabled
- `pkexec`, usually provided by `polkit`, to install, remove, or update system packages

Install the required dependencies on common distributions:

```bash
# Debian / Ubuntu
sudo apt update
sudo apt install -y build-essential mold pkg-config libssl-dev libwayland-dev libxkbcommon-dev policykit-1
```

```bash
# Fedora
sudo dnf install -y gcc gcc-c++ mold pkgconf-pkg-config openssl-devel wayland-devel libxkbcommon-devel polkit
```

```bash
# Arch Linux
sudo pacman -S --needed base-devel mold pkgconf openssl wayland libxkbcommon polkit
```

## Running for development

Run the application from a native Wayland session:

```bash
cargo run -p updater
```

## Installation

### Option 1: Install a release package

If the repository has published a release, download the Linux `deb` or `rpm` package for your distribution.

GitHub Actions builds these packages automatically. They are intended for users who want to install and run the application directly.

After installing a `deb` or `rpm` package, launch Updater from your desktop application menu.

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
5. Open the settings page to enable or disable package managers and configure custom executable paths.

Additional notes:

- Operations that install, remove, or update system packages request elevated privileges through `pkexec`
- Configuration is stored in `updater/config.json` under the user configuration directory
- If a package manager is not detected, you can specify its executable path manually from the settings page
