# Iteration 067：Health 差量失效与定向重检

- 日期：2026-08-05
- 状态：进行中
- ROADMAP 阶段：阶段 5/7 工作流可靠性与 Package Manager 生态

## 目标

让 Package Managers 健康状态和 Iteration 066 的 package-data 差量刷新保持同一
生命周期：保存、添加、移除或修改单个 manager 时，只让受影响 manager 进入
Unchecked，未变化 manager 的健康结果、扫描进度和在途检查继续有效。

## 实施范围

- [x] 按稳定 `ManagerId` 保留未变化 manager 的 availability 记录，并清理移除或
  配置变化 manager 的旧记录。
- [x] 配置变更与活动健康扫描重叠时，仅隔离受影响 manager 的排队/晚到结果；不
  取消未变化 manager 的只读检查。
- [x] 首次打开 Health 或显式检查时，只扫描尚无当前记录的 manager；所有 manager
  均有记录时，Recheck 才恢复全量扫描。
- [x] 为差量保留、活动扫描隔离、定向扫描与全量 Recheck 添加回归测试。
- [ ] 完成本地串行门禁和 Linux、Windows、macOS 原生 CI。

## 验证

- [x] `cargo test -p updater --bin updater --locked --jobs 1 -- --test-threads=1`
  （91 项通过）。
- [x] `cargo fmt --all -- --check`
- [x] `cargo check --workspace --all-targets --locked --jobs 1`
- [x] `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- [x] `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- [x] `cargo build --workspace --locked --jobs 1`
- [x] `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked --jobs 1`
- [ ] main CI 的 Linux、Windows x86_64 与 macOS arm64 原生 job 全部通过。

## 边界

- 本轮不持久化 Health 结果；应用重启后仍按页面访问触发真实 availability 检查。
- 当前正在执行的 manager 命令不被 UI 任务强制终止；配置变化只阻止其结果写回，
  与既有取消和进程生命周期语义一致。
- 本轮不增加健康结果 TTL 或后台定时刷新策略。

## 下一轮入口

- 评估 Health 结果的显式刷新 scope 与有限 TTL，避免长期显示历史 availability，
  同时保持按 manager 的低开销重检。
