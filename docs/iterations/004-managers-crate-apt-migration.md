# Iteration 004：Managers Crate 与 APT 直接迁移

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

在 workspace 根目录新增平铺的 `managers/` crate，建立不依赖 Iced 的内置 manager 实现边界，并将 APT 从 legacy 静态 trait 迁移为直接实现 `updater_manager_api::PackageManager` 的首个 manager。现有 UI 与 Config V1 继续工作，APT 命令逻辑只能保留一份。

## 实施计划

- [x] 新增根目录 `managers/`（package `updater-managers`），统一使用 workspace dependency 与宽 semver manifest 约束。
- [x] 抽取 APT 所需的 executable resolution、command execution、bounded progress 与 typed error 工具，工具层不依赖 core 或 UI。
- [x] 直接实现 APT descriptor、availability、installed/count、updates、search 与统一 execute。
- [x] 将 core 的旧 APT 入口改为兼容 wrapper，复用新实现并完成 Config V1、model 与 progress 的反向转换，删除旧 APT 命令实现副本。
- [x] 提供混合 built-in 注册：APT 使用直接实现，其余 10 个 manager 暂时继续使用 legacy adapter，保持 stable ID 不变。
- [x] 增加纯离线 parser、command construction、conversion、registration 与 progress contract tests。
- [ ] 串行通过 format、check、test、clippy、build，并由 GitHub Actions 复验。

## 非目标

- 本轮不迁移 DNF、Pacman、Zypper、Flatpak、Homebrew 或语言工具 manager。
- 本轮不让 UI 改用 `ManagerId`，也不迁移 Config V2。
- 本轮不改变 APT refresh、`pkexec` 提权、批量执行或 stop-on-failure 语义。
- 本轮不新增运行时动态插件加载。

## 设计约束

- workspace crate 继续平铺为 `ui/`、`core/`、`manager-api/`、`managers/`；仅在 `managers/src/` 内按共享工具与具体 manager 建立有意义的模块层级。
- `updater-managers` 的具体命令实现只依赖公共 API 与通用运行库，不能依赖 Iced。
- APT 的 legacy wrapper 只做兼容转换，不复制 parser、command arguments 或执行流程。
- 所有测试默认离线，不执行 `apt update/install/remove`，不触发 `pkexec`，不修改系统包数据库。
- manifest 不写死类似 `3.27.0` 的最低 minor/patch；精确解析结果继续由 `Cargo.lock` 固定。

## 进度日志

### 2026-07-29

- Iteration 003 已完成，11 个 built-in 已具备 stable identity，并可通过 legacy adapter 注册到对象安全 registry。
- 确定首个直接迁移对象为 APT，以系统 manager 的批处理、提权和 refresh 路径验证新 crate 边界。
- 确定 `managers/` 使用 workspace 根目录平铺结构，不恢复通用 `crates/` 容器目录。
- 新增平铺的 `updater-managers` workspace crate；初始边界只依赖公共 API 与通用运行库，不依赖 core 或 Iced。
- 新增共享 command/progress 工具：executable resolution、5 秒 availability timeout、有界 64 行 channel、单行 2 KiB 与 20 行错误尾部。
- APT 已直接实现对象安全公共 trait，保留 batch、`pkexec`、refresh 与 `--only-upgrade` 语义，并提供迁移期 progress bridge。
- core 的旧 APT 模块缩减为兼容转换层，读取与写入均调用直接实现；原 parser、命令参数和进程逻辑已删除。
- 新增混合 `register_builtin_managers`：APT 注册直接实例，其余 10 个 stable ID 继续注册 legacy adapter。
- 补齐 managers public contract、缺失 executable、空执行、target/config 防串用、parser、command、bounded progress、typed error、legacy conversion 与 mixed registry 离线测试。
- workspace 完整本地门禁通过；全量容器复验交由 GitHub Actions，不在本机使用 `act` 重复构建相同 Rust 依赖图。

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| `b52d48b` | 完成 Iteration 003 并建立 Iteration 004 计划 | 文档检查 |
| `454a59a` | 建立平铺的 `updater-managers` crate 与 workspace 边界 | managers check、manifest review |
| `ddc9f94` | 实现共享 command/progress 工具与直接 APT manager | managers unit test、clippy |
| `1e0b649` | 让 core legacy APT 入口复用直接实现并加入混合注册 | core check、focused test |
| `84301bf` | 补齐直接 APT、legacy bridge 与 mixed registry 离线 contract tests | focused test、clippy |
| 待提交 | 记录 Iteration 004 完整本地验证 | format、check、test、clippy、build |

## 验证记录

- `cargo check -p updater-managers --all-targets --locked --jobs 1`：通过。
- `cargo test -p updater-managers --all-targets --locked --jobs 1 -- --test-threads=1`：2 个离线测试通过。
- `cargo clippy -p updater-managers --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo test -p updater_core --lib --locked --jobs 1 -- --test-threads=1`：62 个测试通过，12 个环境测试 ignored。
- `cargo clippy -p updater_core --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo check -p updater --all-targets --locked --jobs 1`：legacy UI 路径通过。
- `cargo test -p updater-managers --all-targets --locked --jobs 1 -- --test-threads=1`（contract checkpoint）：14 个纯离线测试通过。
- `cargo test -p updater_core --lib --locked --jobs 1 -- --test-threads=1`（bridge checkpoint）：66 个测试通过，12 个环境测试 ignored。
- `cargo test -p updater_core --test builtin_registry --locked --jobs 1 -- --test-threads=1`：2 个 mixed registry 测试通过。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 --quiet -- --test-threads=1`：108 个测试通过，13 个环境测试 ignored。
- `cargo build --workspace --locked --jobs 1 --quiet`：通过。

## 遗留项 / 下一轮

本轮完成后填写。
