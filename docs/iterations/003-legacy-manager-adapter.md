# Iteration 003：Legacy Manager Adapter

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——渐进迁移现有 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

为现有 `PackageManagerType` 增加稳定 built-in identity 和对象安全 adapter，将全部现有 manager 显式注册到 `ManagerRegistry`。当前 UI、Config V1、旧静态 trait 和内置 manager 模块保持工作，不改变用户行为。

## 实施计划

- [ ] 为每个 `PackageManagerType` 定义唯一稳定 ID、descriptor、platform、category、capabilities 与授权提示。
- [ ] 实现 `ManagerConfig` 到现有 Config V1 的兼容桥接，包括 executable path 和 Go 私有设置。
- [ ] 实现 `LegacyPackageManagerAdapter`，映射 availability、installed/count、updates、search 和 execute。
- [ ] 将旧 package model、progress event 和 `CoreError` 转换为新公共 API 类型。
- [ ] 提供 `register_legacy_managers`，通过新 registry 注册全部现有 built-in adapter。
- [ ] 增加纯离线 identity、注册、转换、capability 与 progress contract tests。
- [ ] 串行通过 format、check、test、clippy、build，并由 GitHub Actions 复验。

## 非目标

- 本轮不移动 `core/src/pm/*` 到 `updater-managers` crate。
- 本轮不删除旧宏 dispatcher、静态 trait 或 `PackageManagerType`。
- 本轮不让 UI 改用 `ManagerId`，也不迁移 Config V2。
- 本轮不改变写操作的 manager group 串行与停止语义。

## 设计约束

- adapter 只做兼容转换，不复制具体 manager 命令实现。
- built-in ID 必须稳定、唯一，并统一使用 `builtin:<name>`。
- 任何不支持的操作在 registry capability gate 处拒绝，不依赖字符串错误。
- 测试不得访问网络或执行本机包管理器事务。

## 进度日志

### 2026-07-29

- Iteration 002 已完成，对象安全公共 API、registry 和外部 fake manager contract 已通过 CI。
- 建立本轮 adapter 迁移计划。

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| 待提交 | 完成 Iteration 002 并建立 Iteration 003 计划 | 文档检查 |

## 验证记录

尚未开始本轮代码验证。

## 遗留项 / 下一轮

本轮完成后填写。
