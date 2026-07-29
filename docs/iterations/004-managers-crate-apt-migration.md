# Iteration 004：Managers Crate 与 APT 直接迁移

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

在 workspace 根目录新增平铺的 `managers/` crate，建立不依赖 Iced 的内置 manager 实现边界，并将 APT 从 legacy 静态 trait 迁移为直接实现 `updater_manager_api::PackageManager` 的首个 manager。现有 UI 与 Config V1 继续工作，APT 命令逻辑只能保留一份。

## 实施计划

- [x] 新增根目录 `managers/`（package `updater-managers`），统一使用 workspace dependency 与宽 semver manifest 约束。
- [ ] 抽取 APT 所需的 executable resolution、command execution、bounded progress 与 typed error 工具，工具层不依赖 core 或 UI。
- [ ] 直接实现 APT descriptor、availability、installed/count、updates、search 与统一 execute。
- [ ] 将 core 的旧 APT 入口改为兼容 wrapper，复用新实现并完成 Config V1、model 与 progress 的反向转换，删除旧 APT 命令实现副本。
- [ ] 提供混合 built-in 注册：APT 使用直接实现，其余 10 个 manager 暂时继续使用 legacy adapter，保持 stable ID 不变。
- [ ] 增加纯离线 parser、command construction、conversion、registration 与 progress contract tests。
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

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| `b52d48b` | 完成 Iteration 003 并建立 Iteration 004 计划 | 文档检查 |
| 待提交 | 建立平铺的 `updater-managers` crate 与 workspace 边界 | managers check、manifest review |

## 验证记录

本轮实施后持续填写。

## 遗留项 / 下一轮

本轮完成后填写。
