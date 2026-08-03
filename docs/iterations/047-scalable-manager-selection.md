# Iteration 047：可扩展的 Package Manager 选择体验

- 日期：2026-08-03
- 状态：已完成
- ROADMAP阶段：阶段7 Package Manager 生态扩展的 UI 基线
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

随着 manager 数量增加，主页和 Settings 不能继续依赖平铺、难以扫描的复选框列表。本轮统一 manager 的视觉身份、分类、搜索和批量选择语义，为后续新增 manager 保留稳定的 UI 容量。

## 实现范围

- Finding、Updates 和 Installed 共用可折叠 source picker，折叠态显示已选 manager Logo 与数量，展开态按 System、Application、Development 和 Other 分类。
- source picker 支持名称、ID 和描述搜索，并显示 availability、加载状态和包数量；`Select shown`与`Clear shown`只影响当前筛选结果，保留隐藏 manager 的选择和页面状态。
- Settings 使用同一 manager Logo、分类和搜索语义，同时覆盖已配置和可添加的 manager；未知或没有品牌资产的第三方 manager 使用稳定 initials fallback。
- Logo 容器使用固定尺寸，避免异步状态、长名称或窄窗口造成布局跳动；第三方品牌资产不改变 manager identity 或业务逻辑。
- Simple Icons 的 CC0 来源和商标边界记录在`THIRD_PARTY_NOTICES.md`，并随DEB、RPM、Arch、portable tar、AppImage、Windows和macOS产物分发。

## 非目标

- 不在本轮新增 manager、修改配置schema或提升版本号。
- Flatpak仍只作为受支持的 package manager，不新增Flatpak应用分发格式。
- Logo只用于识别对应生态，不表示项目与品牌方存在关联或背书。

## 验收

- UI新增3项单元测试，覆盖Finding、Updates和Installed筛选后批量清除不会破坏隐藏manager状态；完整workspace tests通过。
- workspace fmt、check、test、clippy和build已按ROADMAP要求串行通过；release metadata、Shell语法及portable tar真实清单验证通过。
- Gamescope headless Wayland仍触发已知的wgpu `ERROR_SURFACE_LOST_KHR`；强制Xwayland后应用在700×600目标下稳定运行至受控超时，但compositor截图是黑帧，因此不把它记录成视觉验收。
- [CI run 30805679742](https://github.com/Yiki21/PackageGet/actions/runs/30805679742)已通过Linux fmt/check/test/clippy/build、Windows全部离线contracts和macOS原生workspace check。
- [Package run 30805679816](https://github.com/Yiki21/PackageGet/actions/runs/30805679816)已通过DEB/RPM（amd64/arm64）、Arch、glibc/musl portable、AppImage、Windows ZIP/installer、macOS Intel/arm64构建与产物校验；bundle checksums已生成。
