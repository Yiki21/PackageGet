# Iteration 009：Homebrew 直接迁移与 Formula/Cask Identity

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 Homebrew 迁移为 `updater-managers` 中第六个直接实现，覆盖 Linuxbrew 与 macOS Homebrew 的只读和写命令契约。迁移必须区分 formula 与 cask，保留 tap/origin 和安装 identity，避免旧实现仅凭名称、易碎文本与静默 fallback 混合两类包。现有 UI 与 Config V1 继续通过 compatibility wrapper 工作。

## 实施计划

- [x] 审计当前 Homebrew stable CLI 的 availability、installed、outdated、search、formula/cask info、refresh 与写命令，优先选择稳定 JSON 输出并固定离线 fixtures。
- [ ] 直接实现 Homebrew descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [ ] 为 formula 与 cask 建立明确的 private identity，并映射到 `PackageScope`、`PackageOrigin` 与完整 write target；同名 formula/cask 不得静默去重。
- [ ] installed 解析保留 formula/cask 的版本、description、homepage、tap 与安装状态；失败时不得以不完整文本结果伪装成功。
- [ ] updates 保留 `refresh=false` 的 no-auto-update 语义；`refresh=true` 的 `brew update` 设有明确 timeout，并区分 timeout、network、permission 与 repository failure。
- [ ] search 先发现候选 identity，再以有界、确定性的 metadata 查询补齐 formula/cask 类型和 installed state；命令失败不返回伪造空列表。
- [ ] write 根据冻结的 formula/cask target 构造 install/upgrade/uninstall argv；Config V1 的 `PackageScope::Unknown` 保留旧名称命令作为受控兼容路径。
- [ ] 保持 manager group 内写操作串行，所有 target 在执行前完成校验；progress 与错误使用 managers crate 的 bounded shared runner。
- [ ] 将 core Homebrew 收缩为 Config V1、model、progress 与 typed error wrapper，并更新 mixed registry。
- [ ] 增加 JSON/text fixtures、formula/cask collision、tap/origin、refresh/no-refresh、timeout、command construction、conversion、registration 与 public API contracts。
- [ ] 在可用的 Linuxbrew 宿主或容器执行显式 opt-in 只读 smoke；macOS 命令差异以 fixture/CI 可验证边界记录，不执行真实写事务。
- [ ] 串行通过 workspace format、check、test、clippy 与 build 完整门禁，并由 GitHub Actions 复验。

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
- read/write command-local 环境设置 `HOMEBREW_NO_AUTO_UPDATE=1` 与 `HOMEBREW_NO_ANALYTICS=1`；显式 refresh 的 `brew update` 不设置 no-auto-update，并延续 180 秒 update/90 秒 outdated timeout。

## Git 提交

本轮实施后逐项记录。

## 验证记录

- `brew --version`：本机 Homebrew 可用；版本仅作为本次审计证据，不写入最低版本约束。
- `brew info --json=v2 --installed`：只读成功，76 个 formula、2 个 cask；观察到 formula 同时保留两个 installed versions。
- `brew outdated --json=v2`：只读成功，本机当前 formula/cask outdated 数组均为空。
- `brew search --formula ripgrep` 与 `brew search --cask ripgrep`：只读成功，确认必须分类型查询。

## 遗留项 / 下一轮

本轮完成后填写。
