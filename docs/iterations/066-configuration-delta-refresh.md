# Iteration 066：配置差量刷新与缓存保留

- 日期：2026-08-05
- 状态：已完成
- ROADMAP 阶段：阶段 5/7 工作流可靠性与 Package Manager 生态

## 目标

消除 Package Manager 配置保存后的全量 Installed/Updates 重扫。应用按稳定
`ManagerId` 比较保存前后的完整 `ManagerConfig`，只让新增、移除或配置内容实际
变化的 manager 失效，并完整保留未变化来源的缓存、选择、错误与在途请求。

## 实施范围

- [x] 按 manager ID 计算新增、配置变化和移除集合；单纯调整顺序或保存应用偏好
  不触发 package-data reload。
- [x] 已打开过 Installed/Updates 时，只为新增或配置变化的 manager 重新加载；
  移除 manager 只清理本地状态，不启动 CLI。
- [x] 页面尚未初始化时保持懒加载，保存 manager 配置不提前扫描未访问页面。
- [x] 首次多 manager 扫描仍在运行时，只拒绝受影响 manager 的旧结果，未变化
  manager 继续完成并合并。
- [x] 未变化 manager 的 cache、source selection、package selection、错误和在途
  request 保持不变；受影响 manager 的冻结确认状态与搜索结果失效。
- [x] 添加差量计算、缓存隔离、移除零扫描、懒加载与在途结果隔离回归测试。
- [x] 完成本地串行门禁和 Linux、Windows、macOS 原生 CI。

## 验证

- [x] `cargo test -p updater --bin updater --locked --jobs 1 -- --test-threads=1`
  （84 项通过）。
- [x] `cargo fmt --all -- --check`
- [x] `cargo check --workspace --all-targets --locked --jobs 1`
- [x] `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- [x] `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- [x] `cargo build --workspace --locked --jobs 1`
- [x] `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked --jobs 1`
- [x] main CI run `30979270678`：Linux format/check/test/Clippy/build、Snap、
  Portage/XBPS 离线合同及 Gentoo/Void 原生只读 smoke，Windows x86_64
  workspace 与 manager 合同，以及 macOS arm64 workspace 与跨平台 manager
  合同全部通过。

## 结果

- 实现提交 `d647e82` 将配置保存事件改为携带按稳定 ID 计算的 manager 差量；
  未变化 manager 不再因其他 manager 的添加、移除或路径/私有设置变化而重扫。
- 配置保存不再增加全局 package-data generation。首次扫描中的未变化来源继续
  合并；受影响来源通过既有 per-manager request 与 refresh override 拒绝旧结果。
- 已初始化页面对新增/变化 manager 直接启动一次定向 Installed/Updates 加载；
  未初始化页面保持零请求，移除 manager 在所有情况下都只清理本地状态。

## 边界

- 本轮不持久化 package cache；应用重启后仍由页面首次打开触发真实扫描。
- Health 状态的按 manager 保留和定向重检在 Iteration 067 处理；本轮只负责
  package-data cache 与请求生命周期。
- manager 配置变化后不自动重放 Discover 搜索；只清理受影响来源的旧搜索结果。
