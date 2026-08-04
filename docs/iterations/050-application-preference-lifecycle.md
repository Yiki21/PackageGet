# Iteration 050：应用偏好与 Manager 状态生命周期隔离

- 日期：2026-08-04
- 状态：已完成
- ROADMAP阶段：阶段7 Package Manager 管理体验收敛

## 本轮目标

继续落实 Settings 与 Package Managers 的职责边界：Appearance 和 Notifications 是应用级偏好，不应因为保存或丢弃而重载 manager 数据、取消 Health 检查或清空已有健康结果。

## 实现范围

- `Content` 在应用配置保存成功后比较保存前后的 `managers` 快照。
- 只有 manager 配置变化才返回 `ReloadPackageData(ConfigurationChanged)`。
- 丢弃 pending configuration 时，仅当 managers draft 发生变化才 invalidate `ManagerHealthInfo`。
- Appearance/Notifications 仍共享同一份 Config 原子保存流程，但不再制造 manager-side 副作用。

## 验收

- 新增测试：保存应用偏好不触发 package-data reload。
- 新增测试：丢弃应用偏好时保留正在进行的 Health scan。
- `cargo fmt --all`：通过。
- `cargo test -p updater --bin updater --locked --jobs 1 -- --test-threads=1`：通过，69 项。
- 完整 workspace check、tests 与 Clippy 门禁随后执行并记录结果。

## 下一轮

按阶段 7 进入下一个 manager 候选或 manager 管理页的真实 GUI smoke；新增 manager 必须先冻结命令、identity、scope 和平台契约，再实现 UI 广告。
