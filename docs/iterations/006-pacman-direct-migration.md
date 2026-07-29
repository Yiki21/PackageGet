# Iteration 006：Pacman 直接迁移与 Arch Transaction Parity

- 日期：2026-07-29
- 状态：已完成
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 Pacman 迁移为 `updater-managers` 中第三个直接实现，复用现有 command/progress/error 边界，并保持 Arch Linux 当前 installed、search、update listing、refresh 与批量事务语义。现有 UI 和 Config V1 继续通过轻量 wrapper 工作，mixed registry 直接注册 APT、DNF 与 Pacman。

## 实施计划

- [x] 直接实现 Pacman descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [x] 保留自定义 executable、refresh/no-refresh、`pkexec` 与现有 `-Q`、`-Qq`、`-Qu`、`-Ss` 命令语义。
- [x] 保留 install/update 的 `-S --needed --noconfirm`、uninstall 的 `-R --noconfirm` 和批量 package group 行为。
- [x] 扩展 shared command error classifier，覆盖 Pacman transaction/database lock，同时不把普通 `pkexec` command failure 误判为 permission。
- [x] 将 core 的旧 Pacman 入口改为兼容 wrapper，删除旧 parser、command construction 和执行实现副本。
- [x] 更新 mixed built-in 注册：APT、DNF、Pacman 使用直接实现，其余 8 个 manager 继续使用 legacy adapter。
- [x] 增加纯离线 installed/update/search fixture、command construction、conversion 与 registration contract tests。
- [x] 宿主机验证结构化 availability，并在 Arch Linux 容器内执行 direct manager 的只读 availability/installed/count smoke test。
- [x] 串行通过 format、check、test、clippy、build，并由 GitHub Actions 复验。

## 非目标

- 本轮不迁移 Zypper、Flatpak、Homebrew 或语言工具 manager。
- 本轮不引入 `checkupdates`、`expac` 等额外 Arch 工具依赖。
- 本轮不重新设计现有 `pacman -Sy` refresh 或特定 package update 策略。
- 本轮不在宿主机或容器内运行 `pacman -Sy`、安装、升级、删除或任何 privileged smoke transaction。
- 本轮不修改 UI identity、Config V2 或 manager settings 页面。

## 设计约束

- Pacman 实现位于根目录平铺 crate 的 `managers/src/pacman.rs`，不新增通用 `crates/` 容器目录或单文件 feature folder。
- legacy wrapper 只负责 Config V1、model、progress 与 error 转换；Pacman 命令和 parser 只能存在于 `updater-managers`。
- shared progress 继续只提供现有通用 percent 行为；没有真实复用需求时不加入 Pacman 专属抽象。
- 所有 parser、command 和 contract tests 默认离线，不能刷新 sync database 或修改本机 package database。
- toolchain 与 CI 继续跟随 `stable` channel；manifest 使用宽 semver line，精确依赖图由提交的 `Cargo.lock` 固定。

## 进度日志

### 2026-07-29

- Iteration 005 已完成本地与 GitHub Actions 门禁，DNF 已成为第二个直接 manager，并验证了复杂两阶段 progress 迁移路径。
- 确定 Pacman 为下一迁移对象，用于继续收敛系统 manager 的 command/error/wrapper 模式，同时验证无需专属 progress state 的直接迁移。
- 宿主机未安装 Pacman；根据补充验收要求，真实只读验证将使用 Podman 的 Arch Linux 容器执行 direct API，而不是仅验证 CLI 或缺失状态。
- `updater-managers` 已增加平铺的 `pacman.rs`，直接实现完整 object-safe manager contract，并复用现有 bounded command progress。
- 自定义 executable、refresh/no-refresh、`-Q/-Qq/-Qu/-Ss`、批量 `-S/-R` 与 `--needed/--noconfirm` 参数已由离线测试锁定。
- shared command error classifier 已覆盖 `failed to init transaction`、`unable to lock database` 与 `could not lock database`。
- `core/src/pm/pacman.rs` 已收缩为 Config V1、model、progress 与 typed error 转换层，旧 Pacman command/parser/execute 副本已删除。
- mixed built-in registry 现在直接注册 APT、DNF 与 Pacman，并继续为其余 8 个 manager 注册 legacy adapter；Pacman duplicate contract 已补齐。
- direct Pacman integration contract 已覆盖 descriptor、elevation、缺失自定义 executable、空事务 progress boundary、错误 manager identity 与只读环境 smoke。
- 并行 parity 审计发现并修复了 Pacman `--version` logo 误识别、取消错误未分类，以及 core wrapper 丢失旧锁冲突/授权取消提示的问题。
- public API fake executable fixture 已直接覆盖 `--version`、`-Q`、`-Qq`、`-Qu` 与 `-Ss`，不再只依赖私有 parser 单元测试。
- 当前 Fedora 宿主机未安装 Pacman；显式 availability smoke 验证其返回 `CommandMissing { command: "pacman" }`。
- Podman 因本机 Docker Hub 失效凭据无法拉取镜像，因此在不改动登录配置的前提下改用 Docker；官方 `archlinux:base` 镜像（digest `sha256:3406a568f45d68f0bef35dc80b3eacec8bda59b0292b2e50d5932ba1667f20cf`）中的 direct API 只读 smoke 通过。
- 修复后的完整 workspace 本地门禁已串行通过；等待本轮最新提交的 GitHub Actions 复验后关闭 Iteration 006。

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| `03720cd` | 完成 Iteration 005 并建立 Iteration 006 计划 | 文档检查；GitHub Actions `30431775898` |
| `c45ec4e` | 将真实只读 smoke 扩展为 Podman Arch Linux direct API 验证 | 文档检查 |
| `c1fc617` | 实现直接 Pacman manager 与 lock error parity | `cargo test -p updater-managers --jobs 1 -- --test-threads=1`；`cargo clippy -p updater-managers --all-targets --jobs 1 -- -D warnings` |
| `65e8fc8` | 将 legacy Pacman 路由到直接实现并更新 mixed registry | `cargo test -p updater_core --jobs 1 -- --test-threads=1`；`cargo clippy -p updater_core --all-targets --jobs 1 -- -D warnings` |
| `746d111` | 增加 direct Pacman integration contracts 与环境 smoke | 4 个默认 contract tests；宿主 availability 1 项；Arch Docker smoke 1 项 |
| `c42d391` | 修复 availability version、cancel/lock error parity，并增加 public API fixtures | managers 34 项通过；core 71 项通过；Arch Docker smoke 1 项 |
| `48bb9e4` | 记录 Iteration 006 完整本地门禁 | workspace 128 项通过；完整 format/check/test/clippy/build |

## 验证记录

- `updater-managers`：21 个单元测试、8 个默认 integration contract tests 通过，1 个本机 DNF smoke 保持 ignored。
- `cargo check -p updater-managers --jobs 1` 通过。
- `cargo clippy -p updater-managers --all-targets --jobs 1 -- -D warnings` 通过。
- `updater_core`：70 项测试通过，11 项依赖本机软件或网络的测试保持 ignored。
- `cargo check -p updater_core --jobs 1` 通过。
- `cargo clippy -p updater_core --all-targets --jobs 1 -- -D warnings` 通过。
- `cargo test -p updater-managers --test pacman_contract --jobs 1 -- --test-threads=1`：5 项通过，2 项环境 smoke 保持 ignored。
- 宿主机显式运行 `local_pacman_availability_is_structured`：1 项通过。
- Docker `archlinux:base` 显式运行 `arch_container_pacman_read_only_smoke`：修复前后各 1 项通过，并断言版本行包含 `Pacman v`；未执行 refresh 或写事务。
- `cargo test -p updater_core --jobs 1 -- --test-threads=1`：71 项通过，11 项环境或网络测试保持 ignored。
- `cargo clippy -p updater-managers -p updater_core --all-targets --jobs 1 -- -D warnings` 通过。
- `cargo fmt --all -- --check` 通过。
- `cargo check --workspace --all-targets --locked --jobs 1` 通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：128 项通过，14 项环境或网络测试保持 ignored。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings` 通过。
- `cargo build --workspace --locked --jobs 1` 通过。
- GitHub Actions CI run `30433800326` 通过，耗时 3 分 13 秒；format、check、deterministic tests、clippy 与 build 全部成功。

## 遗留项 / 下一轮

- 下一轮进入 [Iteration 007：Zypper 直接迁移与 Exit-Code/Locale Parity](007-zypper-direct-migration.md)。
- 并行审计确认 Zypper 的主要迁移风险是 locale-sensitive 表格与专属退出码；本轮不顺带切换 XML 协议。
- Flatpak 审计确认 user/system scope、完整 ref 与 remote identity 是直接迁移前提，不能按单一 application ID 简单照搬；安排为 Iteration 008 独立处理。
