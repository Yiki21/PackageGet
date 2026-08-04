# Iteration 058: Package Manager Platform Add Filtering

- 日期：2026-08-04
- 状态：已完成
- ROADMAP阶段：阶段7——扩展 Package Manager生态

## 目标

避免 Package Managers 页面把当前平台不支持的 manager 当成可添加项，同时保留用户已有的跨平台配置，避免配置迁移时被静默删除。

## 实施

- 可添加 manager 列表复用 `ManagerCatalog::supports_current_platform`，在展示 Select Path/Add 入口前过滤不支持的平台。
- `AddDetectedManager` 在 Settings 状态更新层增加同一平台检查，防止过期消息或非 UI 调用绕过列表过滤。
- 已配置的 unsupported manager 不从 draft 中移除，仍可显示状态并允许用户显式卸载；本轮不改变 unknown manager 的保留策略。

## 验证

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --locked --jobs 1`
- `cargo test -p updater --bin updater --locked --jobs 1 -- --test-threads=1`
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`

新增回归测试 `unsupported_manager_cannot_be_added_to_the_draft`，覆盖状态层拒绝 unsupported manager 的行为。
