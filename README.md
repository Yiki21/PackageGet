# updater

一个基于 Rust 和 `iced` 的桌面 GUI 更新工具，用来统一查看、搜索、安装和更新不同包管理器中的软件包。

## 这个软件是干嘛的

`updater` 的目标是把不同包管理器分散的操作统一到一个桌面界面里。

你可以用它：

- 查看当前机器里哪些包管理器可用
- 统一查看有哪些软件包可以更新
- 统一搜索软件包并安装
- 查看已安装软件包并卸载
- 在一个界面里处理系统包和开发工具包，而不是分别记 `apt`、`dnf`、`cargo`、`npm`、`flatpak` 等不同命令

它更适合下面这类场景：

- 你同时使用多个包管理器，想集中管理更新
- 你不想频繁切换不同命令行工具
- 你希望用 GUI 做批量安装、卸载、更新操作

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
- `mold`（Linux 构建默认使用）
- C/C++ 构建工具链：如 `gcc` 或 `clang`
- `pkg-config`
- OpenSSL 开发库
- Linux 桌面相关库：通常至少需要 `wayland` 和 `libxkbcommon`
- 如果需要对系统包执行安装、卸载、更新，还需要 `pkexec`（通常来自 `polkit`）

常见发行版可直接安装：

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

## 开发运行

```bash
cargo run -p updater
```

## 安装方式

### 方式一：下载发布包安装

如果仓库已经发布了 Release，直接下载对应系统的安装包或可执行文件即可：

- macOS：下载 `dmg`
- Windows：下载 `exe`
- Linux：下载 `deb` 或 `rpm`

这些产物由 GitHub Actions 自动打包生成，适合普通用户直接安装使用。

### 方式二：从源码构建安装

先准备好上面的构建依赖，然后执行：

```bash
cargo build --release -p updater --locked
```

构建完成后可直接运行：

```bash
./target/release/updater
```

如果希望安装到本地命令目录：

```bash
install -Dm755 target/release/updater ~/.local/bin/updater
```

然后确保 `~/.local/bin` 在你的 `PATH` 中。

## 怎么用

启动程序：

```bash
updater
```

如果你没有把它安装到 `PATH` 中，也可以直接运行：

```bash
./target/release/updater
```

首次启动后，程序会自动检测当前环境中可用的包管理器。基础使用流程如下：

1. 在更新页查看每个包管理器的可更新包。
2. 勾选需要更新的软件包，执行批量更新。
3. 在已安装页查看、搜索、排序，或批量卸载已安装软件。
4. 在搜索页跨包管理器查找新软件并安装。
5. 在设置页启用或禁用包管理器，并自定义可执行文件路径。

补充说明：

- 涉及系统包管理器的安装、卸载、更新时，程序会通过 `pkexec` 请求权限
- 配置会保存到用户配置目录中的 `updater/config.json`
- 如果某个包管理器没有被检测到，可以在设置页手动指定它的可执行文件路径
