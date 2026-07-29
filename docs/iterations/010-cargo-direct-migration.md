# Iteration 010：Cargo 直接迁移与 Registry/Local Source Identity

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 Cargo 迁移为 `updater-managers` 中第七个直接实现，保留 `cargo install --list` 的 installed/binary metadata 与 crates.io search/update 能力，同时明确区分 registry 安装和 local/path source。网络失败、HTTP 状态、JSON 协议与不可更新的 local source 不能再被静默转换成空 metadata 或跳过的 update。

## 实施计划

- [ ] 审计当前 stable Cargo 的 `install --list`、install/uninstall argv、registry/path/git source 表达，以及 crates.io API 的 search/detail schema、HTTP 状态和 rate-limit headers。
- [ ] 直接实现 Cargo descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [ ] 将 installed parser 收敛为 validated source identity，保留 crate name、version、binary 列表和 registry/local source；malformed header/continuation 不静默污染相邻 crate。
- [ ] direct registry target 使用明确 origin/reference；local/path/git package 只读展示，不伪装成可从 crates.io 更新的 registry package。
- [ ] installed metadata 与 updates 使用 typed crates.io response structures、统一 User-Agent 和 timeout；HTTP/network/protocol/rate-limit failure 返回 typed error并保留 bounded detail。
- [ ] metadata 请求使用固定上限的串行 batch 或 bounded concurrency，避免旧实现逐包吞错和不受控 fan-out。
- [ ] search 使用结构化 query encoding 和 typed response，保留 crates.io registry origin、installed state、description 与 homepage/repository fallback；非成功 HTTP 状态不能伪造空结果。
- [ ] write 根据冻结的 registry target 构造 install/force-update/uninstall argv；明确 version pin、local source 与 legacy Unknown target 的兼容边界。
- [ ] 保留 `CARGO_INSTALL_ROOT`/`CARGO_HOME` binary-size 发现，并对缺失 binary、重复 binary 和 filesystem error 使用稳定、可测试的降级语义。
- [ ] 将 core Cargo 收缩为 Config V1、model、progress 与 typed error wrapper，并更新 mixed registry。
- [ ] 增加 install-list fixtures、registry/local collision、binary size、HTTP mock、timeout/status/schema、search/update、command construction、conversion、registration 与 public API contracts。
- [ ] 在当前宿主执行显式 opt-in 只读 Cargo smoke；真实 crates.io 查询保持单独 opt-in，不执行 install/update/uninstall。
- [ ] 串行通过 workspace format、check、test、clippy 与 build完整门禁，并由 GitHub Actions 复验。

## 审计重点

- 核实 `cargo install --list` 对 registry、`--path`、`--git`、多 binary 和多个安装版本的真实文本，不用“header 多出第三个 token”简单等价为 local source。
- 核实 crates.io detail 中 stable/latest version 的选择规则，不能把 prerelease 意外当成 stable update。
- 明确 crates.io 404、429、5xx、invalid JSON、timeout 与 offline 的 `ManagerErrorKind` 映射；更新扫描不能逐包吞掉这些失败。
- 测试 HTTP endpoint 通过 manager-private test setting 或内部依赖注入覆盖，生产默认仍固定到 crates.io HTTPS。
- 检查 Cargo executable 与 install root 的自定义配置是否应独立表达，避免把 executable path 当作 `CARGO_HOME`。

## Identity 与兼容边界

- registry crate、local path 和 git source 是不同 identity；具体 canonical grammar 在真实 `cargo install --list` 审计后冻结。
- `PackageInfo.name` 保留 crate name；source/registry/path 放入 `PackageOrigin`，direct target 使用 `PackageScope::User`。
- direct updates 只生成可安全重放的 registry targets；local/path/git installed package不因同名 crates.io crate而显示错误 update。
- Unknown 只用于 Config V1/UI 的旧短名称命令；direct read/search/update 不产生 Unknown。
- version pin 不得被静默忽略；是否支持精确 `--version` 由本轮命令审计和 fixtures 决定。

## 非目标

- 本轮不迁移 Go、npm、pnpm 或 pipx。
- 本轮不实现 Cargo workspace dependency 更新、`cargo update` 或 Rust toolchain 更新。
- 本轮不修改 Config V2、UI identity 或 manager settings 页面。
- 本轮不执行真实 `cargo install`、force reinstall 或 uninstall。
- 本轮不写死 Cargo、crates.io API client 或依赖 crate 的最低 minor/patch版本。

## 设计约束

- Cargo 实现位于根目录平铺 crate 的 `managers/src/cargo.rs`，不新增通用 `crates/` 或 manager 分组目录。
- HTTP/JSON 使用 reqwest、serde 与 URL/query APIs，不手写 percent encoding 或用 `serde_json::Value` 穿透协议边界。
- 默认 tests 使用本地 mock HTTP 与 fake Cargo executable，完全离线；宿主/network smoke 显式 opt-in。
- 网络并发必须有固定上限，命令和 HTTP timeout 都返回 typed error；不持锁跨 await。
- toolchain 与 CI 继续跟随 `stable` channel；manifest 使用宽 semver line，精确依赖图由提交的 `Cargo.lock` 固定。

## 进度日志

### 2026-07-29

- Iteration 009 已完成 direct Homebrew、formula/cask/tap identity、timeout child termination、Linuxbrew 只读 smoke、本地完整门禁与 GitHub Actions 复验。
- 初步代码审阅确认旧 Cargo parser 能识别简单 local path marker，但 updates/metadata 会逐包吞掉 crates.io failure，search 手写 percent encoding并将非成功 HTTP 状态转换为空结果。
- 下一步先审计真实 stable Cargo 输出与 crates.io typed schema，再冻结 registry/local/git identity 和 HTTP error contract。

## Git 提交

本轮实施后逐项记录。

## 验证记录

本轮实施后逐项记录。

## 遗留项 / 下一轮

本轮完成后填写。
