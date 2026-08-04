# Iteration 064：启动、定向刷新与 RubyGems 可靠性

- 日期：2026-08-05
- 状态：已完成
- ROADMAP 阶段：阶段 5/7 工作流可靠性与 Package Manager 生态

## 目标

消除首屏启动时无条件执行全部 package manager inventory/update 扫描的等待，
修复 source picker 滚动条与行尾选择框重叠，并让 package operation 完成后的
缓存刷新严格限定到本次成功执行过的 manager。同时补上 RubyGems 在输出安装
错误却返回成功退出码时的失败识别。

## 实施范围

- [x] 启动只加载配置、Activity 与桌面主题；首次进入 Updates 或 Installed 时
  才分别启动对应初始化任务。
- [x] package operation 从 `OperationOutcome.manager_outcomes` 提取
  `Succeeded` manager，仅刷新这些 manager 的 installed/updates 缓存并保留
  其他 manager 的缓存、选择及 Discover 搜索结果。
- [x] 防止仍在运行的初始化结果覆盖较新的定向刷新结果。
- [x] RubyGems 写命令除检查退出状态外，也识别规范的 `ERROR:` 输出；即使
  `gem update` 返回 0，仍向执行引擎报告带原始错误行的结构化失败。
- [x] source picker 使用嵌入式滚动条布局，为行尾选择框保留固定间距。
- [x] 添加启动懒加载、成功/未执行 manager 刷新隔离及 RubyGems 假成功退出
  合同测试。

## 验证

- [x] `cargo test -p updater-managers --test rubygems_contract --locked --jobs 1 -- --test-threads=1`
- [x] `cargo test -p updater --bin updater --locked --jobs 1 -- --test-threads=1`
- [x] 隔离 headless Gamescope 1200x800 启动执行跟踪：首屏无 package manager
  子进程。
- [x] 隔离 headless Gamescope 1200x800 与 700x800 source picker 截图：滚动条
  与行尾选择框不重叠。
- [x] `cargo fmt --all -- --check`
- [x] `cargo check --workspace --all-targets --locked --jobs 1`
- [x] `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- [x] `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- [x] `cargo build --workspace --locked --jobs 1`
- [x] `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked --jobs 1`

## 边界

- Updates/Installed 的首次打开仍会执行该页面需要的真实扫描；本轮优化的是首屏
  可用时间，不缓存或伪造 package manager 结果。
- 配置中的 manager 发生变化时仍执行完整 package-data reload，确保新增、移除
  和 capability 变化同步到两个数据页。
- 本轮不执行真实 RubyGems 写事务；回归测试使用离线 fake CLI 精确复现
  “stderr 输出 `ERROR:`、进程退出 0”的 RubyGems 行为。
