# Iteration 005：DNF 直接迁移与 Progress Parity

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 DNF 迁移为 `updater-managers` 中第二个直接实现，复用已有 command/error 边界，并无损迁移当前 DNF 下载与事务两阶段 progress 语义。现有 UI 和 Config V1 继续通过轻量 wrapper 工作，mixed registry 直接注册 APT 与 DNF。

## 实施计划

- [ ] 扩展共享 progress parser，支持 DNF step ratio、下载/事务两阶段区间和现有中英文 transaction marker，同时保持 APT percent 行为不变。
- [ ] 直接实现 DNF descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [ ] 保留 refresh/no-refresh、`pkexec`、批量 install/update/remove 和现有命令参数语义。
- [ ] 将 core 的旧 DNF 入口改为兼容 wrapper，删除旧 parser、command construction 和执行实现副本。
- [ ] 更新 mixed built-in 注册：APT、DNF 使用直接实现，其余 9 个 manager 继续使用 legacy adapter。
- [ ] 增加纯离线 fixture/parser、command construction、progress phase、conversion 与 registration contract tests。
- [ ] 在本机 DNF 可用时执行只读 availability/listing smoke test，不运行安装、升级、删除或 privileged transaction。
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

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| 待提交 | 完成 Iteration 004 并建立 Iteration 005 计划 | 文档检查 |

## 验证记录

本轮实施后持续填写。

## 遗留项 / 下一轮

本轮完成后填写。
