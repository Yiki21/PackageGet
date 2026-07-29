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
| 002 | 进行中 | 阶段 2：Manager API 与 Registry 基础 | [002-manager-api-registry-foundation.md](002-manager-api-registry-foundation.md) |
