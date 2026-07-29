# Iteration 003：Legacy Manager Adapter

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——渐进迁移现有 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

为现有 `PackageManagerType` 增加稳定 built-in identity 和对象安全 adapter，将全部现有 manager 显式注册到 `ManagerRegistry`。当前 UI、Config V1、旧静态 trait 和内置 manager 模块保持工作，不改变用户行为。

## 实施计划

- [x] 为每个 `PackageManagerType` 定义唯一稳定 ID、descriptor、platform、category、capabilities 与授权提示。
- [x] 实现 `ManagerConfig` 到现有 Config V1 的兼容桥接，包括 executable path 和 Go 私有设置。
- [x] 实现 `LegacyPackageManagerAdapter`，映射 availability、installed/count、updates、search 和 execute。
- [x] 将旧 package model、progress event 和 `CoreError` 转换为新公共 API 类型。
- [x] 提供 `register_legacy_managers`，通过新 registry 注册全部现有 built-in adapter。
- [x] 增加纯离线 identity、注册、转换、capability 与 progress contract tests。
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
- 为 11 个现有 manager 增加唯一 `builtin:*` ID 和 descriptor，系统 manager 声明授权提示。
- 平台 metadata 当前按已实现能力保守声明：Linux managers、Homebrew 与便携开发 manager 的 macOS 支持；Windows 留待平台层完成后开放。
- 公共 `PackageInfo` 补齐旧模型已有的 size 与 install date，避免 adapter 迁移丢失 UI 元数据。
- 按仓库的扁平 workspace 结构将公共 crate 放在根目录 `manager-api/`，不增加无实际分组意义的 `crates/` 层。
- 新增对象安全 legacy adapter，所有读取和写入继续委托现有命令实现，不复制 manager 逻辑。
- 新 `ManagerConfig` 在调用边界校验 stable ID，并桥接 custom executable；Go 的 `go_bin_dir` 从 typed JSON settings 解析。
- availability、package/update metadata、write progress 与 `CoreError` 已转换为公共 API 的结构化模型和 typed error kind。
- `register_legacy_managers` 可将现有 11 个 built-in adapter 显式注册到 `ManagerRegistry`。
- 新增 11 个纯离线 adapter 测试，覆盖 system/app/Go 配置桥接、ID 防串用、metadata、availability、typed errors、progress、完整注册和空操作执行。

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| `0bade6c` | 完成 Iteration 002 并建立 Iteration 003 计划 | 文档检查 |
| `ad798ec` | 建立 built-in identity、descriptor 与完整 package metadata | unit test、clippy |
| `6b31606` | 将 `manager-api` 扁平放置在 workspace 根目录 | format、check、focused test |
| `dd0f718` | 实现 Config V1 桥接、legacy adapter、类型转换与 built-in 注册 | core check、clippy |
| 待提交 | 补齐 adapter 配置、转换、注册、错误与 progress 离线测试 | focused test、clippy |

## 验证记录

- `cargo test -p updater_core --lib --locked --jobs 1 -- --test-threads=1`：54 个测试通过，12 个环境测试 ignored。
- `cargo test -p updater_core --lib --locked --jobs 1 -- --test-threads=1`（adapter checkpoint）：65 个测试通过，12 个环境测试 ignored。
- `cargo test -p updater-manager-api --all-targets --locked --jobs 1 -- --test-threads=1`：4 个测试通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：扁平化 workspace 路径后通过。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。

## 遗留项 / 下一轮

本轮完成后填写。
