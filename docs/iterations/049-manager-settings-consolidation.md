# Iteration 049：Package Manager 配置职责收敛

- 日期：2026-08-03
- 状态：已完成
- ROADMAP阶段：阶段7 Package Manager 生态扩展的管理体验收敛
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

Settings 不再同时承担应用偏好与 Package Manager 管理。所有 manager 的发现、添加、移除、可执行路径、manager-specific 配置、健康检查和保存入口统一进入 Package Managers 页面；Settings 仅保留外观、通知及后续应用级配置。

## 实现范围

- Package Managers 页面复用唯一的 Settings draft/save 状态，不复制第二套配置模型或持久化流程。
- Health 模块收敛为健康摘要、只读 availability 扫描、取消和诊断报告，不再维护重复的 manager 列表、搜索和筛选。
- 现有已配置 manager、可添加 manager、PATH 检测、Unload、自定义 executable、Go binary dir 与 Nix profile 配置整体迁入 Package Managers 页面。
- Settings 页面只显示 Appearance、Notifications 和应用配置保存状态。
- Managers 与 Settings 视为同一配置工作区，两者之间可直接切换；从任一页面离开工作区时，共享 draft 的未保存修改必须经过 Save/Discard/Cancel 确认。
- Package Managers 页面继续使用较大的 manager 名称、状态和配置文字，避免回退到难以阅读的 11–12px 信息层级。
- manager-owned draft 变化会取消并清空旧健康扫描；Appearance 与 Notifications 等应用偏好变化不会错误地使健康状态失效。

## 非目标

- 不改变 Config schema、manager registry 或写操作权限模型。
- 不引入独立 manager 配置文件，也不把 Health availability scan 当成安装、仓库刷新或自动修复。
- 不在本轮拆分外观与 manager draft 的持久化文件；两页仍保存同一个原子 Config snapshot。

## 验收

- 新增导航测试，覆盖配置页面之间可直接切换，以及从 Managers 离开时未保存修改会被拦截。
- 新增状态测试，覆盖 manager draft 变化使健康扫描失效，以及应用偏好变化保留健康扫描。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --all-features --locked --jobs 1`：通过。
- `cargo test --workspace --all-features --locked --jobs 1 -- --test-threads=1`：通过；其中 updater UI 67 项测试通过。
- `cargo clippy --workspace --all-targets --all-features --locked --jobs 1 -- -D warnings`：通过。
- 当前会话没有暴露隔离 Agent Workspace GUI 工具，且不使用宿主真实桌面代替隔离验收；本轮未宣称完成截图级视觉验收。
