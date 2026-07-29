# Iteration 002：Manager API 与 Registry 基础

- 日期：2026-07-29
- 状态：已完成
- ROADMAP 阶段：阶段 2——拆分 crate，并建立真正可扩展的 PackageManager API
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

交付阶段 2 的第一个可回滚切片：新增不依赖 Iced 和具体命令实现的 `updater-manager-api` crate，并在 `updater_core` 中建立确定性的对象安全 registry。现有 `PackageManagerType`、内置 manager 和 UI 行为保持不变，后续迭代再通过 adapter 渐进迁移。

## 实施计划

- [x] 定义可验证、可序列化、带命名空间的 `ManagerId`。
- [x] 定义 descriptor、platform、category、capabilities、授权提示与稳定显示元数据。
- [x] 定义 manager config、package/update/target、action、progress、availability 与结构化错误模型。
- [x] 定义对象安全的异步 `PackageManager: Send + Sync` 和非 Iced `ProgressSink`。
- [x] 在 core 中实现确定性 `ManagerRegistry`：显式注册、拒绝重复/非法 ID、稳定排序和 capability gate。
- [x] 增加公共 API 单元测试、core registry 测试和外部 fake manager 集成测试。
- [x] 串行通过 format、check、test、clippy、build，并由 GitHub Actions 复验。

## 非目标

- 本轮不迁移 `core/src/pm/*` 的现有 manager 实现。
- 本轮不删除 `PackageManagerType`、宏 dispatcher 或旧静态 trait。
- 本轮不修改 UI identity、Config V2、平台注册逻辑或运行时插件协议。

## 设计约束

- 公共 crate 不依赖 Iced、Tokio runtime 或具体包管理器命令。
- trait 必须可存入 `Arc<dyn PackageManager>`，不暴露泛型回调、静态必需方法或关联异步类型。
- manager ID 在构造边界完成验证，registry 不接收未验证字符串。
- manager group 执行顺序和现有写操作语义在本轮保持不变。

## 进度日志

### 2026-07-29

- Iteration 001 已完成，GitHub Actions 五项质量门槛全绿。
- 建立本轮计划，确定先交付 API 与 registry 基础，再迁移内置 manager。
- 新增 `updater-manager-api` workspace crate，公共契约不依赖 Iced、Tokio runtime 或具体命令实现。
- `ManagerId` 使用私有 newtype，在 parse、`FromStr`、`TryFrom` 和 serde 反序列化边界统一验证。
- descriptor、capability、平台、配置、package model、progress、availability 与 typed error 已落地。
- `PackageManager` 使用实例方法和 `async-trait`，可作为 `Arc<dyn PackageManager>` 使用；非支持方法返回结构化 Unsupported 错误。
- core 新增 `ManagerRegistry`，使用 `BTreeMap<ManagerId, Arc<dyn PackageManager>>` 显式注册实例。
- registry 拒绝重复 ID，按 category、display name、ID 稳定排序，并在返回 manager 前执行 capability gate。
- 新增外部集成测试 crate，实现 fake manager 并通过 `Arc<dyn PackageManager>` 完成 availability、search、execute 与 progress 调用。
- 集成测试覆盖重复 ID、unsupported capability 和 descriptor 稳定排序。
- workspace format、all-targets check 与 Clippy 已在本机串行通过；完整 test/build 交由 GitHub Actions 复验。
- GitHub Actions run 30424759721 在 2 分 33 秒内通过 format、check、test、clippy、build。

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| `581da60` | 完成 Iteration 001 并建立 Iteration 002 计划 | 文档检查 |
| `371956c` | 新增 `updater-manager-api` 公共扩展契约 | crate check、test、clippy |
| `2b4d815` | 在 core 中新增确定性 ManagerRegistry | core check、clippy |
| `d0e09e2` | 增加外部 fake manager registry contract test | focused test、clippy |
| `3bf3ff7` | 记录本地 workspace 验证结果 | format、check、clippy、GitHub Actions |
| 待提交 | 完成本轮报告并建立 Iteration 003 计划 | 文档检查 |

## 验证记录

- `cargo check -p updater-manager-api --all-targets --jobs 1`：通过。
- `cargo test -p updater-manager-api --all-targets --locked --jobs 1 -- --test-threads=1`：4 个测试通过。
- `cargo clippy -p updater-manager-api --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo check -p updater_core --all-targets --locked --jobs 1`：通过。
- `cargo clippy -p updater_core --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo test -p updater_core --test manager_registry --locked --jobs 1 -- --test-threads=1`：3 个测试通过。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- [GitHub Actions CI run 30424759721](https://github.com/Yiki21/PackageGet/actions/runs/30424759721)：完整五项门槛通过。

## 遗留项 / 下一轮

- 下一轮通过 adapter 将现有 `PackageManagerType` 接入新 registry，保持 UI、Config V1 和现有执行行为不变。
- 具体计划见 [Iteration 003](003-legacy-manager-adapter.md)。
