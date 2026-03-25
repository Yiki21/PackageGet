# updater

一个基于 Rust 和 `iced` 的桌面 GUI 更新工具，用来统一查看、搜索、安装和更新不同包管理器中的软件包。

目前支持的包管理器：

- 系统包：`apt`、`dnf`、`pacman`、`zypper`
- 应用/开发工具包：`flatpak`、`homebrew`、`cargo`、`go`、`npm`、`pnpm`

## 功能列表

- 自动检测当前环境中可用的包管理器
- 统一查看各包管理器的可更新包数量和详细列表
- 批量更新选中的软件包
- 查看已安装软件包，并支持搜索、排序和批量卸载
- 跨包管理器搜索软件包，并支持批量安装
- 在设置页管理已启用的包管理器
- 为包管理器指定自定义可执行文件路径
- 为 Go 包安装位置指定自定义二进制目录
- 将配置保存到用户配置目录，重启后继续使用

## 构建依赖

- Rust 工具链：项目当前使用 [`nightly`](./rust-toolchain.toml)
- `cargo`
- C/C++ 构建工具链：如 `gcc` 或 `clang`
- `pkg-config`
- OpenSSL 开发库
- Linux 桌面相关库：通常至少需要 `wayland` 和 `libxkbcommon`
- 如果需要对系统包执行安装、卸载、更新，还需要 `pkexec`（通常来自 `polkit`）

常见发行版可直接安装：

```bash
# Debian / Ubuntu
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev libwayland-dev libxkbcommon-dev policykit-1
```

```bash
# Fedora
sudo dnf install -y gcc gcc-c++ pkgconf-pkg-config openssl-devel wayland-devel libxkbcommon-devel polkit
```

```bash
# Arch Linux
sudo pacman -S --needed base-devel pkgconf openssl wayland libxkbcommon polkit
```

## 开发运行

```bash
cargo run -p ui
```

## 安装说明

从源码构建：

```bash
cargo build --release -p ui
```

构建完成后可直接运行：

```bash
./target/release/ui
```

如果希望安装到本地命令目录：

```bash
install -Dm755 target/release/ui ~/.local/bin/updater
```

然后确保 `~/.local/bin` 在你的 `PATH` 中。

## 说明

- 启动时会自动检测当前环境可用的包管理器
- 配置会保存到用户配置目录中的 `updater/config.json`
- 系统包管理器相关操作会调用 `pkexec` 获取权限
