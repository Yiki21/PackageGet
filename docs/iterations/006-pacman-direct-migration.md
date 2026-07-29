# Iteration 006：Pacman 直接迁移与 Arch Transaction Parity

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 Pacman 迁移为 `updater-managers` 中第三个直接实现，复用现有 command/progress/error 边界，并保持 Arch Linux 当前 installed、search、update listing、refresh 与批量事务语义。现有 UI 和 Config V1 继续通过轻量 wrapper 工作，mixed registry 直接注册 APT、DNF 与 Pacman。

## 实施计划

- [ ] 直接实现 Pacman descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [ ] 保留自定义 executable、refresh/no-refresh、`pkexec` 与现有 `-Q`、`-Qq`、`-Qu`、`-Ss` 命令语义。
- [ ] 保留 install/update 的 `-S --needed --noconfirm`、uninstall 的 `-R --noconfirm` 和批量 package group 行为。
- [ ] 扩展 shared command error classifier，覆盖 Pacman transaction/database lock，同时不把普通 `pkexec` command failure 误判为 permission。
- [ ] 将 core 的旧 Pacman 入口改为兼容 wrapper，删除旧 parser、command construction 和执行实现副本。
- [ ] 更新 mixed built-in 注册：APT、DNF、Pacman 使用直接实现，其余 8 个 manager 继续使用 legacy adapter。
- [ ] 增加纯离线 installed/update/search fixture、command construction、conversion 与 registration contract tests。
- [ ] 宿主机验证结构化 availability，并在 Podman Arch Linux 容器内执行 direct manager 的只读 availability/installed/count smoke test。
- [ ] 串行通过 format、check、test、clippy、build，并由 GitHub Actions 复验。

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

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| `03720cd` | 完成 Iteration 005 并建立 Iteration 006 计划 | 文档检查；GitHub Actions `30431775898` |
| 待提交 | 将真实只读 smoke 扩展为 Podman Arch Linux direct API 验证 | 文档检查 |

## 验证记录

本轮实施后持续填写。

## 遗留项 / 下一轮

本轮完成后填写。
