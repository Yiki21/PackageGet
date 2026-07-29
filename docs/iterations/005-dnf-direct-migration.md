# Iteration 005：DNF 直接迁移与 Progress Parity

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 DNF 迁移为 `updater-managers` 中第二个直接实现，复用已有 command/error 边界，并无损迁移当前 DNF 下载与事务两阶段 progress 语义。现有 UI 和 Config V1 继续通过轻量 wrapper 工作，mixed registry 直接注册 APT 与 DNF。

## 实施计划

- [x] 扩展共享 progress parser，支持 DNF step ratio、下载/事务两阶段区间和现有中英文 transaction marker，同时保持 APT percent 行为不变。
- [x] 直接实现 DNF descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [x] 保留 refresh/no-refresh、`pkexec`、批量 install/update/remove 和现有命令参数语义。
- [x] 将 core 的旧 DNF 入口改为兼容 wrapper，删除旧 parser、command construction 和执行实现副本。
- [x] 更新 mixed built-in 注册：APT、DNF 使用直接实现，其余 9 个 manager 继续使用 legacy adapter。
- [x] 增加纯离线 fixture/parser、command construction、progress phase、conversion 与 registration contract tests。
- [x] 在本机 DNF 可用时执行只读 availability/listing smoke test，不运行安装、升级、删除或 privileged transaction。
- [ ] 串行通过 format、check、test、clippy、build，并由 GitHub Actions 复验。

## 非目标

- 本轮不迁移 Pacman、Zypper、Flatpak、Homebrew 或语言工具 manager。
- 本轮不修改 UI identity、Config V2 或 manager settings 页面。
- 本轮不改变 DNF 的 stop-on-failure、刷新选择或提权模型。
- 本轮不把本机只读 smoke test 误报为完整安装/升级事务验收。

## 设计约束

- DNF 实现位于根目录平铺 crate 的 `managers/src/dnf.rs`，不新增通用容器目录或单文件 feature folder。
- shared progress 只抽取 APT/DNF 已共同需要的行为；DNF phase state 使用明确类型，不用散落字符串状态。
- 所有 parser 和 command tests 默认离线，不能访问仓库 metadata 网络，也不能修改 RPM 数据库。
- legacy wrapper 只负责 Config V1、model、progress 与 error 转换，具体命令逻辑只能存在于 `updater-managers`。
- manifest 继续使用宽 semver line；精确依赖图由提交的 `Cargo.lock` 固定。

## 进度日志

### 2026-07-29

- Iteration 004 已完成，APT 已成为首个直接 manager，legacy UI 与 mixed registry 均复用同一命令实现。
- 确定 DNF 为下一迁移对象，用于验证 shared command layer 和两阶段 progress parser 对复杂系统 manager 的适用性。
- `updater-managers` 已增加平铺的 `dnf.rs`，直接实现完整 object-safe manager contract，并继续使用宽依赖声明与锁文件固定依赖图。
- shared progress 现以 `DnfPhase::{Download, Transaction}` 表达阶段状态，保留中英文 transaction marker、step ratio reset 检测及 `0.00..0.60` / `0.60..0.99` 映射。
- DNF refresh/no-refresh、`pkexec`、批量 install/upgrade/remove 与 `--skip-unavailable` 命令语义已由离线测试锁定。
- `core/src/pm/dnf.rs` 已收缩为 Config V1、model、progress 与 typed error 转换层，旧 DNF command/parser/execute 副本已删除。
- mixed built-in registry 现在直接注册 APT 与 DNF，并继续为其余 9 个 manager 注册 legacy adapter；两类 direct duplicate contract 均有测试覆盖。
- 根据本机编辑器反馈，将仓库 toolchain override 与 GitHub Actions 统一改为不写死 patch 版本的 `stable` channel，并在本地声明 `rust-analyzer` 组件。
- direct DNF integration contract 已覆盖 descriptor/elevation、离线 missing executable、空批次边界事件及错配 config/target 拒绝路径。
- 本机 DNF5/RPM 只读 smoke 已通过 direct manager 的 availability、installed 与 count 路径；未执行 refresh、search、check-upgrade、`pkexec` 或写事务。
- 全工作区已在 `stable` toolchain 下串行通过 format、all-targets check、确定性测试、clippy 与 build；等待 GitHub Actions 对相同门禁复验。

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| `57565e4` | 完成 Iteration 004 并建立 Iteration 005 计划 | 文档检查 |
| `66193a5` | 实现直接 DNF manager 与两阶段 progress parser | `cargo test -p updater-managers --jobs 1 -- --test-threads=1`；`cargo clippy -p updater-managers --all-targets --jobs 1 -- -D warnings` |
| `5476e99` | 将 legacy DNF 路由到直接实现并更新 mixed registry | `cargo test -p updater_core --jobs 1 -- --test-threads=1`；`cargo clippy -p updater_core --all-targets --jobs 1 -- -D warnings` |
| `7d9caaa` | 改用 stable channel 并补齐本地 rust-analyzer component | `rustup show active-toolchain`；`rust-analyzer --version`；`cargo check -p updater-managers --jobs 1` |
| `4a8af32` | 增加 direct DNF 离线 integration contracts | `cargo test -p updater-managers --all-targets --jobs 1 -- --test-threads=1`；`cargo clippy -p updater-managers --all-targets --jobs 1 -- -D warnings` |
| `c2f1087` | 增加默认 ignored 的本机 DNF 只读 smoke test | `cargo test -p updater-managers --test dnf_contract local_dnf_availability_and_installed_listing_smoke --jobs 1 -- --ignored --test-threads=1` |
| 待提交 | 记录 Iteration 005 全工作区本地门禁 | format、check、116 passed / 12 ignored、clippy、build |

## 验证记录

- `updater-managers`：17 个单元测试、4 个 APT integration contract tests 通过。
- `cargo check -p updater-managers --jobs 1` 通过。
- `cargo clippy -p updater-managers --all-targets --jobs 1 -- -D warnings` 通过。
- `updater_core`：68 项测试通过，11 项依赖本机软件或网络的测试保持 ignored。
- `cargo check -p updater_core --jobs 1` 通过。
- `cargo clippy -p updater_core --all-targets --jobs 1 -- -D warnings` 通过。
- 仓库 override 已解析为本机 `stable` toolchain，`rust-analyzer` 可从该 toolchain 正常启动；CI 与 package workflow 不再固定 patch 版本。
- direct DNF offline contract：4 项通过；本机只读 DNF smoke：1 项通过。
- `cargo fmt --all -- --check` 通过。
- `cargo check --workspace --all-targets --locked --jobs 1` 通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：116 项通过、12 项 ignored、0 失败。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings` 通过。
- `cargo build --workspace --locked --jobs 1` 通过。

## 遗留项 / 下一轮

本轮完成后填写。
