# Iteration 001：阶段 1 构建基线

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 1——建立可复现的现代依赖与跨平台构建基线
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

在不改动 PackageManager 扩展模型和产品行为的前提下，完成依赖集中、稳定 Rust 工具链、依赖升级、质量 CI 与 Linux 打包修复，为后续 crate 拆分建立可复现基线。

## 实施计划

- [x] 审计 workspace manifest、Cargo.lock、Rust 工具链、Cargo 配置、测试、CI 和打包流程。
- [ ] 将全部直接依赖集中到根 `Cargo.toml`，成员 crate 统一使用 workspace 继承。
- [ ] 删除 nightly 专用编译参数，固定到实施时确认的 stable Rust，并安装 rustfmt、clippy。
- [ ] 按基础库、网络/序列化、平台集成、Iced 的顺序升级依赖并审阅 lockfile。
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

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| 待提交 | 持久化 ROADMAP 与 Iteration 001 计划 | 文档检查 |

## 验证记录

尚未开始本轮最终验证。

## 遗留项 / 下一轮

本轮完成后填写。
