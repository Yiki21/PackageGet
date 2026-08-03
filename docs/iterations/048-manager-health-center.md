# Iteration 048：Package Manager 管理页

- 日期：2026-08-03
- 状态：已完成
- ROADMAP阶段：阶段7 Package Manager 生态扩展的运行时可观测性基线
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

随着 Package Manager 数量增加，用户需要一个集中、易扫描的管理入口，而不是在 Settings 或单个页面中拼接零散的 availability 状态。本轮建立应用级健康状态，并让 Sidebar、Package Managers 管理页和 Settings 共享同一份结果。

## 实现范围

- 新增 Package Managers 管理页，只扫描当前配置的 manager，并按 Healthy、Degraded、Unavailable、Error、Unchecked 分类。
- 健康扫描沿用 manager registry 的 availability contract，按配置顺序串行执行；扫描期间支持 cooperative cancel，generation 会拒绝过期结果。
- Degraded 由已有 Installed/Updates 初始化或加载错误派生；健康检查不安装包、不刷新仓库、不执行自动修复。
- 管理页提供名称/ID搜索、状态筛选、进度、Recheck、Cancel、Copy report 和 Configure 入口；诊断报告沿用 Activity 的路径、凭据和敏感字段脱敏规则。
- 管理页只展示 manager identity、状态、版本、可执行摘要和最后检查时间；capability、authorization 等详细配置不再与 Settings 重复。
- Sidebar 显示 Health 入口，并在检查中或存在问题时显示 badge；Settings 对已配置 manager 回退显示管理页的最近结果。
- 新增 Health 图标及 Lucide ISC 来源声明；自动解析 executable 明确显示为`System PATH`，不伪造实际路径。

## 非目标

- 不在本轮增加 manager、改变 Config schema 或引入后台定时扫描。
- 不执行写操作、仓库刷新、权限提升或自动修复。
- 不把“可执行文件来自 PATH”误报成可重现的 canonical executable path。

## 验收

- Health stale generation、取消 token、运行时 degraded 和诊断脱敏均有单元测试；Sidebar badge 优先级有单元测试。
- `cargo check -p updater --all-targets --all-features --locked --jobs 1`、定向 Health/Sidebar tests、完整 workspace tests、clippy、build 和 release metadata 已验证。
- [CI run 30808354940](https://github.com/Yiki21/PackageGet/actions/runs/30808354940)已通过Linux fmt/check/test/clippy/build、Windows离线契约矩阵和macOS arm64 workspace check。
- [Package run 30808354332](https://github.com/Yiki21/PackageGet/actions/runs/30808354332)已通过17项跨平台产物构建、Health 图标 notice 校验和统一 checksums bundle。
- 本地 GUI 仍遵守既定约束：不把 headless Wayland compositor 的黑帧或 wgpu surface failure 记作视觉验收；需要视觉证据时使用隔离 X11 路径并明确记录限制。

## 后续

- 继续补充 manager-specific health detail 与平台 smoke evidence，但不改变管理页的只读检查边界。
