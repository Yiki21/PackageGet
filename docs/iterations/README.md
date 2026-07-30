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
