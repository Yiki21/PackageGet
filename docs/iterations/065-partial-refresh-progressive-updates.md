# Iteration 065：部分成功刷新与 Updates 渐进加载

- 日期：2026-08-05
- 状态：进行中
- ROADMAP 阶段：阶段 5/7 工作流可靠性与 Package Manager 生态

## 目标

补齐 package operation 部分成功时的定向刷新语义，并让 Updates 页在多个
package manager 首次初始化或显式刷新时先展示已经完成的来源，不再让一个慢来源
阻塞全部可用结果。

## 实施范围

- [x] 删除页面与 App 之间的整体 `reload` 布尔值；操作完成后统一从
  `OperationOutcome.manager_outcomes` 选择 `Succeeded` manager 刷新。
- [x] 失败、取消或未执行的 manager 保留原缓存和选择，不触发额外扫描。
- [x] Updates 同时识别首次初始化和显式刷新中的已选来源，展示剩余加载数量并
  保留已有 manager section。
- [x] 尚有来源加载时不提前展示“无更新”或“无搜索结果”。
- [x] 初始化或刷新未结束时拒绝重复 Refresh Selected、Refresh All 与 Update All。
- [x] 700px 窄窗口让 Search 与 Actions 稳定换行，Refresh All 不再被裁切，
  Update All 的禁用态与实际行为一致。
- [x] 添加部分失败与渐进加载状态回归测试。
- [x] 完成本地串行门禁和 GUI 冒烟。
- [ ] 原生 Linux、Windows、macOS CI 通过。

## 验证

- [x] `cargo test -p updater --bin updater --locked --jobs 1 -- --test-threads=1`
  （76 项通过）。
- [x] 隔离 headless Gamescope 1200x800：13 个已选来源中 5 个已发现更新，
  2 个仍在加载；`uv tool`、DNF 等已完成结果与剩余加载提示同时可见。
- [x] 隔离 headless Gamescope 700x800：Search/Actions 换行、Refresh 按钮、
  Update All 禁用态、渐进提示和 package list 均无裁切或重叠。
- [x] `cargo fmt --all -- --check`
- [x] `cargo check --workspace --all-targets --locked --jobs 1`
- [x] `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- [x] `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- [x] `cargo build --workspace --locked --jobs 1`
- [x] `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked --jobs 1`

## 边界

- 本轮不增加持久化 package cache；应用重启后的第一页扫描仍读取真实 manager
  状态。
- 本轮不改变只读初始化的并发模型，也不改变写操作按 manager group 串行执行的
  约束。
- 配置变化仍执行完整 package-data reload；按变更 manager 定向重载留给后续迭代。
