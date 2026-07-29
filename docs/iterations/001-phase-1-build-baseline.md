# Iteration 001：阶段 1 构建基线

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 1——建立可复现的现代依赖与跨平台构建基线
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

在不改动 PackageManager 扩展模型和产品行为的前提下，完成依赖集中、稳定 Rust 工具链、依赖升级、质量 CI 与 Linux 打包修复，为后续 crate 拆分建立可复现基线。

## 实施计划

- [x] 审计 workspace manifest、Cargo.lock、Rust 工具链、Cargo 配置、测试、CI 和打包流程。
- [x] 将全部直接依赖集中到根 `Cargo.toml`，成员 crate 统一使用 workspace 继承。
- [x] 删除 nightly 专用编译参数，固定到实施时确认的 stable Rust，并安装 rustfmt、clippy。
- [x] 按基础库、网络/序列化、平台集成、Iced 的顺序升级依赖并审阅 lockfile。
- [ ] 将依赖真实网络或本机包管理器的测试明确标记为 ignored，保持默认测试确定性。
- [ ] 新增质量 CI 和分组 Dependabot，修正 RPM workflow 的 Cargo package 名称。
- [ ] 串行通过 format、check、test、clippy、build 五项质量门槛。

## 已确认基线

- 本机最新 stable 工具链为 Rust 1.97.1，官方 stable channel manifest 日期为 2026-07-16。
- Iced 已使用当前稳定版 0.14.0，本轮重点是集中声明和 stable 编译兼容，不需要跨大版本迁移。
- 当前 `.cargo/config.toml` 仍包含 `-Zthreads` 和 `-Zshare-generics`，因此只能使用 nightly。
- 当前 RPM workflow 使用目录名 `ui`，正确的 Cargo package 名称为 `updater`。
- 当前部分测试会访问 crates.io 或调用本机 DNF、Flatpak、Homebrew，需要从默认确定性测试中隔离。

## 进度日志

### 2026-07-29

- 完成仓库与 ROADMAP 基线审计。
- 确认采用 `docs/iterations/NNN-*.md` 持久化每轮计划和进度。
- 确认采用单人 `main` 线性提交工作流，不创建额外分支或 PR。
- 将 ui、core 的全部直接依赖集中到根 workspace manifest。
- 将工具链固定为 Rust 1.97.1，并删除 `-Zthreads`、`-Zshare-generics`。
- stable 下的格式检查与 workspace all-targets check 已通过。
- manifest 不写死类似 `3.27.0` 的最低 patch/minor：1.x 以上使用主版本线，0.x 保留兼容 minor 线，实际版本由 lockfile 固定。
- 完成基础库组更新：anyhow、async-trait、chrono、env_logger、futures、log、regex、tempfile、thiserror，并审阅新增的传递依赖。
- 完成异步/网络/序列化组更新：Tokio 1.53.1、Reqwest 0.13.4、Serde 1.0.229、serde_json 1.0.151；URL 已在当前兼容线最新版。
- 完成平台集成组更新：Mimalloc 0.1.52；Notify、RFD、Directories 已在当前稳定兼容线最新版。
- 确认 Iced 0.14.0 已是当前稳定版，现有 API 在 Rust 1.97.1 下无需迁移即可通过 all-targets check。

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| `6aaaf4e` | 持久化 ROADMAP 与 Iteration 001 计划 | 文档检查 |
| `d8f0444` | 集中 workspace 依赖并迁移到 stable Rust | `cargo fmt`、`cargo check` |
| `1285137` | 采用宽松 semver 声明并更新基础依赖组 | `cargo check` |
| `4014565` | 更新异步、网络与序列化依赖组 | `cargo check` |
| 待提交 | 更新并审阅平台集成依赖组 | `cargo check`、`cargo tree` |

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- 基础依赖组更新后再次执行 workspace all-targets check：通过。
- 异步/网络/序列化依赖组更新后再次执行 workspace all-targets check：通过。
- 平台集成依赖组更新后再次执行 workspace all-targets check：通过；直接依赖树已审阅。

## 遗留项 / 下一轮

本轮完成后填写。
