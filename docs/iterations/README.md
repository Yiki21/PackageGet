# Updater Iteration Log

本目录用于持久化 `docs/ROADMAP.md` 的每一轮实施计划和进度。

## 工作流

1. 每轮开始时新增一个独立的迭代文件，写明目标、范围、计划和验收标准。
2. 实施过程中持续更新任务状态、关键决策、验证结果和 Git 提交。
3. 每个可验证的进度点形成一个小而清晰的 commit。
4. 当前为单人开发，直接在 `main` 上保持线性历史，不额外创建分支或 PR。
5. 迭代完成后记录遗留项，并给出下一轮计划入口。

## 迭代索引

| 迭代 | 状态 | 主题 | 文件 |
| --- | --- | --- | --- |
| 001 | 已完成 | 阶段 1：可复现的现代依赖与跨平台构建基线 | [001-phase-1-build-baseline.md](001-phase-1-build-baseline.md) |
| 002 | 已完成 | 阶段 2：Manager API 与 Registry 基础 | [002-manager-api-registry-foundation.md](002-manager-api-registry-foundation.md) |
| 003 | 已完成 | 阶段 2：Legacy Manager Adapter | [003-legacy-manager-adapter.md](003-legacy-manager-adapter.md) |
| 004 | 已完成 | 阶段 2：Managers Crate 与 APT 直接迁移 | [004-managers-crate-apt-migration.md](004-managers-crate-apt-migration.md) |
| 005 | 已完成 | 阶段 2：DNF 直接迁移与 Progress Parity | [005-dnf-direct-migration.md](005-dnf-direct-migration.md) |
| 006 | 已完成 | 阶段 2：Pacman 直接迁移与 Arch Transaction Parity | [006-pacman-direct-migration.md](006-pacman-direct-migration.md) |
| 007 | 已完成 | 阶段 2：Zypper 直接迁移与 Exit-Code/Locale Parity | [007-zypper-direct-migration.md](007-zypper-direct-migration.md) |
| 008 | 已完成 | 阶段 2：Flatpak 直接迁移与 User/System Scope Parity | [008-flatpak-direct-migration.md](008-flatpak-direct-migration.md) |
| 009 | 已完成 | 阶段 2：Homebrew 直接迁移与 Formula/Cask Identity | [009-homebrew-direct-migration.md](009-homebrew-direct-migration.md) |
| 010 | 已完成 | 阶段 2：Cargo 直接迁移与 Registry/Local Source Identity | [010-cargo-direct-migration.md](010-cargo-direct-migration.md) |
| 011 | 已完成 | 阶段 2：Go 直接迁移与 Module/Binary Identity | [011-go-direct-migration.md](011-go-direct-migration.md) |
| 012 | 已完成 | 阶段 2：npm/pnpm 直接迁移与 Global Package Identity | [012-node-global-direct-migration.md](012-node-global-direct-migration.md) |
| 013 | 已完成 | 阶段 2：pipx 直接迁移与 Venv/Source Identity | [013-pipx-direct-migration.md](013-pipx-direct-migration.md) |
| 014 | 已完成 | 阶段 2：Direct Registry Cutover 与 Legacy Adapter 清理 | [014-direct-registry-cutover.md](014-direct-registry-cutover.md) |
| 015 | 已完成 | 阶段 3：Config 直接切换 | [015-config-direct-cutover.md](015-config-direct-cutover.md) |
| 016 | 已完成 | 阶段 3：UI ManagerId Identity Cutover | [016-ui-manager-identity.md](016-ui-manager-identity.md) |
| 017 | 已完成 | 阶段 3：Activity Direct ManagerId Schema | [017-activity-direct-manager-id-schema.md](017-activity-direct-manager-id-schema.md) |
| 018 | 已完成 | 阶段 2/3：Registry Execution Engine Cutover | [018-registry-execution-engine-cutover.md](018-registry-execution-engine-cutover.md) |
| 019 | 已完成 | 阶段 3：Settings Executable Validation 与发布检查点 | [019-settings-executable-validation.md](019-settings-executable-validation.md) |
| 020 | 已完成 | 阶段 3：Config Load 可见恢复 | [020-config-load-recovery.md](020-config-load-recovery.md) |
| 021 | 已完成 | 阶段 5：异步读取 Request Generation | [021-request-generation.md](021-request-generation.md) |
| 022 | 已完成 | 阶段 5：写操作冻结计划与协作取消文案 | [022-frozen-write-plans.md](022-frozen-write-plans.md) |
| 023 | 已完成 | 阶段 4/6：品牌化 Polkit 授权链 | [023-branded-polkit-authorization.md](023-branded-polkit-authorization.md) |
| 024 | 已完成 | 阶段 4/6：Linux Beta 发布硬化 | [024-linux-beta-release-hardening.md](024-linux-beta-release-hardening.md) |
| 025 | 已完成 | 阶段 4：跨平台编译与目标 Registry 基线 | [025-cross-platform-compile-registry.md](025-cross-platform-compile-registry.md) |
| 026 | 已完成 | 阶段 4：Winget Manager 首轮原生契约 | [026-winget-manager.md](026-winget-manager.md) |
| 027 | 已完成 | 阶段 4：原生桌面能力与 macOS Homebrew 验证 | [027-native-desktop-homebrew.md](027-native-desktop-homebrew.md) |
| 028 | 已完成 | 阶段 4：Cargo Windows 原生准入 | [028-cargo-windows.md](028-cargo-windows.md) |
| 029 | 已完成 | 阶段 4：Go Windows 原生准入 | [029-go-windows.md](029-go-windows.md) |
| 030 | 已完成 | 阶段 4：npm Windows 原生准入 | [030-npm-windows.md](030-npm-windows.md) |
| 031 | 已完成 | 阶段 4：pnpm Windows 原生准入 | [031-pnpm-windows.md](031-pnpm-windows.md) |
| 032 | 已完成 | 阶段 4/5：pnpm 初始化退出码修复 | [032-pnpm-outdated-exit-status.md](032-pnpm-outdated-exit-status.md) |
| 033 | 已完成 | 阶段 4：pipx Windows 原生准入 | [033-pipx-windows.md](033-pipx-windows.md) |
| 034 | 进行中 | 阶段 7：uv tool manager | [034-uv-tool-manager.md](034-uv-tool-manager.md) |
