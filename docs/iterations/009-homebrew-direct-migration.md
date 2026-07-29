# Iteration 009：Homebrew 直接迁移与 Formula/Cask Identity

- 日期：2026-07-29
- 状态：已完成
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 Homebrew 迁移为 `updater-managers` 中第六个直接实现，覆盖 Linuxbrew 与 macOS Homebrew 的只读和写命令契约。迁移必须区分 formula 与 cask，保留 tap/origin 和安装 identity，避免旧实现仅凭名称、易碎文本与静默 fallback 混合两类包。现有 UI 与 Config V1 继续通过 compatibility wrapper 工作。

## 实施计划

- [x] 审计当前 Homebrew stable CLI 的 availability、installed、outdated、search、formula/cask info、refresh 与写命令，优先选择稳定 JSON 输出并固定离线 fixtures。
- [x] 直接实现 Homebrew descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [x] 为 formula 与 cask 建立明确的 private identity，并映射到 `PackageScope`、`PackageOrigin` 与完整 write target；同名 formula/cask 不得静默去重。
- [x] installed 解析保留 formula/cask 的版本、description、homepage、tap 与安装状态；失败时不得以不完整文本结果伪装成功。
- [x] updates 保留 `refresh=false` 的 no-auto-update 语义；`refresh=true` 的 `brew update` 设有明确 timeout，并区分 timeout、network、permission 与 repository failure。
- [x] search 先发现候选 identity，再以有界、确定性的 metadata 查询补齐 formula/cask 类型和 installed state；命令失败不返回伪造空列表。
- [x] write 根据冻结的 formula/cask target 构造 install/upgrade/uninstall argv；Config V1 的 `PackageScope::Unknown` 保留旧名称命令作为受控兼容路径。
- [x] 保持 manager group 内写操作串行，所有 target 在执行前完成校验；progress 与错误使用 managers crate 的 bounded shared runner。
- [x] 将 core Homebrew 收缩为 Config V1、model、progress 与 typed error wrapper，并更新 mixed registry。
- [x] 增加 JSON/text fixtures、formula/cask collision、tap/origin、refresh/no-refresh、timeout、command construction、conversion、registration 与 public API contracts。
- [x] 在可用的 Linuxbrew 宿主或容器执行显式 opt-in 只读 smoke；macOS 命令差异以 fixture/CI 可验证边界记录，不执行真实写事务。
- [x] 串行通过 workspace format、check、test、clippy 与 build 完整门禁。
- [x] 由 GitHub Actions 复验相同的 locked 单 job 门禁。

## 审计重点

- 确认 `brew info --json=v2 --installed` 对 formula/cask 的真实字段、空数组与多版本安装表现。
- 确认 outdated 是否有足够稳定的 JSON 输出表达 formula/cask、current version 与 available version；不继续依赖 `NAME (OLD) < NEW` 的单一文本形态，除非有明确兼容层与 fixtures。
- 确认 search 如何无歧义区分 formula 与 cask，以及 tap-qualified name 在 install/upgrade/uninstall 中的行为。
- 确认 `HOMEBREW_NO_AUTO_UPDATE=1`、`HOMEBREW_NO_ANALYTICS=1` 等 command-local 环境不会改变调用者全局环境。
- 确认 Linuxbrew 与 Apple Silicon/Intel Homebrew executable discovery 已由 shared resolver 覆盖，不在 manager 内复制平台路径表。

## Identity 与兼容边界

- direct target 必须能区分 formula 和 cask；具体编码在完成真实 CLI/JSON 审计后冻结，不用 package name 猜测类型。
- `PackageOrigin.name` 优先保存 tap/source，`PackageOrigin.reference` 保存可重放且无歧义的 package identity。
- direct installed/update/search 不产生 `PackageScope::Unknown`；Unknown 仅用于 Config V1/UI 的名称兼容命令。
- 同名 formula/cask、不同 tap 或多个 installed versions 不得在 parser 中静默覆盖。
- Homebrew 不被描述为 macOS 系统更新；其 category 保持 Application/Development 边界中的现有产品定位，平台 metadata 按实际支持声明。

## 非目标

- 本轮不迁移 Cargo、Go、npm、pnpm 或 pipx。
- 本轮不实现 Winget，也不展开阶段 4 的完整 Windows/macOS GUI 支持。
- 本轮不修改 Config V2、UI identity 或 manager settings 页面。
- 本轮不执行真实 install、upgrade、uninstall 或 tap mutation。
- 本轮不写死某个 Homebrew 最低 minor/patch 版本；只依赖经 fixtures 与 smoke 验证的能力，并对缺失能力返回 typed error。

## 设计约束

- Homebrew 实现位于根目录平铺 crate 的 `managers/src/homebrew.rs`，不新增通用 `crates/` 或 manager 分组目录。
- JSON 使用 `serde`/`serde_json` typed structures 解析，不用 ad hoc 字符串切割读取结构化数据。
- timeout、command-local environment 与 progress 只扩展真实共享边界；Homebrew 专属协议不能污染其他 manager。
- 默认测试完全离线；宿主或容器 smoke 只读且显式 opt-in。
- toolchain 与 CI 继续跟随 `stable` channel；manifest 使用宽 semver line，精确依赖图由提交的 `Cargo.lock` 固定。

## 进度日志

### 2026-07-29

- Iteration 008 已完成 direct Flatpak、scope/ref/origin parity、宿主只读 smoke、本地完整门禁与 GitHub Actions 复验。
- 初步代码审阅确认旧 Homebrew installed 已使用部分 JSON，但 outdated/search/current-version 仍依赖文本输出，formula/cask identity 未贯穿 read/write target。
- 本轮先完成真实 CLI/JSON 审计，再冻结 direct contract；不会先写死 Homebrew minor 版本或未经验证的字段假设。
- 本机 `/home/linuxbrew/.linuxbrew/bin/brew` 只读审计确认 `info --json=v2 --installed` 返回独立 `formulae`/`casks` 数组；formula 包含 `full_name`、`tap`、多条 `installed.version/time`，cask 包含 `full_token`、`tap`、`version` 与 `installed`。
- `outdated --json=v2` 的本机命令实现为 formula/cask 统一输出 `name`、`installed_versions`、`current_version`、`pinned` 与 `pinned_version`，不再需要解析 `NAME (OLD) < NEW`/`!=` 文本。
- 非 TTY 的无类型 `brew search QUERY` 只输出名称和空行，无法可靠判断 formula/cask；direct search 将分别执行 `brew search --formula QUERY` 与 `brew search --cask QUERY`，再分类型批量调用 `info --json=v2` 补齐 metadata。
- direct reference 冻结为 `formula:FULL_NAME` 或 `cask:FULL_TOKEN`，`PackageScope::User` 表示当前 Homebrew prefix 的用户安装，tap 保存在 `PackageOrigin.name`；同名跨类型结果不能去重。
- scoped write 使用 `brew install|upgrade|uninstall --formula|--cask` 和冻结的 full identity；Unknown compatibility target 保留旧的 `brew COMMAND NAME` argv。
- read/write command-local 环境设置 no-auto-update、no-analytics、no-ask、no-install-cleanup 与 C locale；显式 refresh 的 `brew update` 不设置 no-auto-update，并延续 180 秒 update/90 秒 read timeout。
- `managers/src/homebrew.rs` 已使用 typed serde structures 解析 installed/outdated/info JSON；direct installed/search/update target 统一输出短 name、User scope、tap origin 与 canonical typed reference。
- formula 多 keg 作为一条 write identity 保留全部 display versions；legacy singular current version 优先 `linked_keg`，没有 linked keg 时使用最后一个 installed version。
- search metadata 以 32 个候选为固定上限串行分批；formula/cask discovery、metadata 与 no-match 分支均有 public fake executable contracts。
- shared Tokio command 增加 `kill_on_drop(true)`；fake slow child contract 验证 100ms timeout 后 PID 已终止，避免 `brew update` 超时后在后台继续运行。
- resolver 补齐 Intel macOS `/usr/local/bin`，并保留 Linuxbrew 与 Apple Silicon `/opt/homebrew/bin` contracts。
- scoped write 校验 User scope、typed kind、tap、短 name、canonical reference 和不支持的 version pin；Unknown target 只保留裸名称 argv，同时通过 command-local no-auto-update/no-analytics/no-ask 避免隐式 refresh 与交互阻塞。
- core Homebrew 已删除 JSON/text parser、timeout 与命令副本，只保留 Config V1、legacy model/progress 和 typed error 转换；mixed registry 当前为六个 direct manager、五个 legacy adapter。
- 本机 Linuxbrew opt-in smoke 已通过 availability、installed/count parity、typed current version 与 `updates(false)`；未执行 refresh、search 或任何写事务。
- 首轮 GitHub Actions 在 `refresh_is_explicit_and_precedes_inventory_and_outdated` 失败：runner 父环境已设置 `HOMEBREW_NO_AUTO_UPDATE`，仅在 refresh command 中“不添加”变量无法保证显式 update 语义。
- `CommandSpec` 已增加 command-local `env_remove`，`brew update` 主动删除继承的 no-auto-update；本地通过 `HOMEBREW_NO_AUTO_UPDATE=1` 精确复现 CI 环境并验证 refresh/update/inventory/outdated 顺序。

## Git 提交

- `0ee3cbf docs: record Homebrew command audit`
- `6903f89 feat: add typed Homebrew manager`
- `bc0292e refactor: route Homebrew through direct manager`
- `8f88ebb docs: record Homebrew migration progress`
- `a3ab2a2 docs: record Homebrew workspace validation`
- `7de00dc fix: make Homebrew refresh environment deterministic`

## 验证记录

- `brew --version`：本机 Homebrew 可用；版本仅作为本次审计证据，不写入最低版本约束。
- `brew info --json=v2 --installed`：只读成功，76 个 formula、2 个 cask；观察到 formula 同时保留两个 installed versions。
- `brew outdated --json=v2`：只读成功，本机当前 formula/cask outdated 数组均为空。
- `brew search --formula ripgrep` 与 `brew search --cask ripgrep`：只读成功，确认必须分类型查询。
- `cargo check -p updater-managers --jobs 1`：通过。
- `cargo test -p updater-managers --jobs 1 -- --test-threads=1`：68 passed，6 ignored。
- `cargo test -p updater-managers --test homebrew_contract --jobs 1 -- --test-threads=1`：8 passed，1 ignored。
- `cargo test -p updater-managers --test homebrew_contract host_homebrew_read_only_smoke --jobs 1 -- --ignored --exact --test-threads=1 --nocapture`：1 passed。
- `cargo clippy -p updater-managers --all-targets --jobs 1 -- -D warnings`：通过。
- `cargo check -p updater_core --jobs 1`：通过。
- `cargo test -p updater_core --lib --jobs 1 -- --test-threads=1`：72 passed，5 ignored。
- `cargo test -p updater_core --test builtin_registry --jobs 1 -- --test-threads=1`：7 passed。
- `cargo clippy -p updater_core --all-targets --jobs 1 -- -D warnings`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：通过。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。
- GitHub Actions runs `30440505954`、`30440689740`、`30440752308`：在 deterministic tests 的 Homebrew refresh environment contract 失败，根因已定位并修复。
- `HOMEBREW_NO_AUTO_UPDATE=1 cargo test -p updater-managers --test homebrew_contract refresh_is_explicit_and_precedes_inventory_and_outdated --jobs 1 -- --exact --test-threads=1`：修复后通过。
- 修复后重新串行执行 workspace format、check、test、clippy 与 build：全部通过。
- GitHub Actions CI run `30441088683`：通过，耗时 3 分 30 秒；format、check、deterministic tests、clippy 与 build 全部成功。

## 遗留项 / 下一轮

- 下一轮进入 [Iteration 010：Cargo 直接迁移与 Registry/Local Source Identity](010-cargo-direct-migration.md)。
- Config V1/UI 仍只能保存 Homebrew 短 name，formula/cask/tap identity 的完整保留留待阶段 3；direct registry 路径已经输出 typed reference。
- Homebrew search 真实宿主 smoke 未纳入默认验收，因为本机存在会触发 repository failure 的 untrusted tap；离线 public contracts 已覆盖 formula/cask discovery、metadata 和错误传播。
