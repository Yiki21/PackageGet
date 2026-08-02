# Iteration 040：Nix profile manager

## 状态

已完成

## 目标

接入 ROADMAP 下一优先级 Nix profile manager。首批只管理用户明确选择的单一 profile，覆盖 installed、install、update 和 uninstall，并在 package origin 中保留 profile、元素名、original flake URL、locked flake URL、flake attribute 与 outputs identity。

## 范围

- [x] 新增 `builtin:nix-profile` direct manager，仅注册 Linux 与 macOS。
- [x] 通过 manager-owned `settings.profile` 保存一个绝对 profile 路径；缺失、空值、相对路径与已知 system profile 必须拒绝。
- [x] Settings 提供 Nix profile 选择与卸载入口；自动检测不能生成缺少 profile 的配置。
- [x] installed 只解析 `nix profile list --json --profile <path>` 的 version 1/2/3 manifest schema，并保留 source identity。
- [x] install target 必须携带显式 installable identity；update/uninstall target 必须与当前 profile 中同名元素的完整 origin 一致。
- [x] update 只允许拥有未锁定 original flake reference 的元素；store-path-only 与 locked-only 元素仍可列出和卸载，但不能伪装成可更新。
- [x] 写操作按 target 串行调用 `nix profile install|upgrade|remove --profile <path>`，并复用 bounded progress/error contract。
- [x] 不广告 `Updates` 或 `Search`：Nix profile 没有只读 update inventory，`nix search` 也没有单一 profile catalog 语义。
- [x] 离线 parser、配置、命令构造、伪造 target 与 Linux/macOS 原生 fake CLI 合同全部通过。
- [x] 本地串行 workspace 门禁、Windows GNU compile check、GitHub Actions Linux/Windows/macOS CI 全部通过。

## 非目标

- 多 profile、重复 `ManagerId` 或 manager instance identity。
- Nix channels、legacy `nix-env` profiles、NixOS system profile、Home Manager generations。
- 通过执行 `nix profile upgrade` 探测 updates，或把 locked flake/store path 元素当作可升级条目。
- 用 `nix search` 构造一个与当前 profile source 无关的全局目录。

## 证据基线

- Nix 2.32 官方手册定义 `nix profile list --json --profile <path>`，并把 Name、flake attribute、original URL、locked URL 和 store paths 作为 profile 元素身份。
- Nix 2.32 官方源码的 manifest version 1 使用 `uri`/`originalUri`，version 2/3 使用 `url`/`originalUrl`；当前写出 version 3 object-shaped `elements`。
- `nix profile upgrade` 会直接获取、求值并写入新 generation，没有 dry-run/list-updates 模式；locked flake reference 不会产生新版本。
- 当前开发宿主未安装 Nix，因此真实宿主 smoke 保持显式忽略；本轮以官方合同、原生 runner fake CLI 和 GitHub Actions 为准。

## 实现检查点

- `NixProfileManager`使用typed serde结构解析manifest v1-v3；v1兼容`originalUri`/`uri`和indexed elements，v2/v3保留named element与`originalUrl`/`url`。
- installed origin冻结profile、element、original/locked URL、attrPath、outputs、store paths与priority；update/remove开始前重读inventory并完整比对origin，拒绝跨profile、stale或forged target。
- `nix profile install`在当前Nix仍是`profile add`的兼容别名，因此同一argv同时覆盖旧版与2.32，不需要失败后重放写操作。
- Config与Settings要求用户选择一个绝对profile；已知system/default profile、相对路径、控制字符和非UTF-8 UI路径被拒绝。
- catalog按Linux/macOS注册Nix；更新页和搜索页现在统一按descriptor capability过滤source，Nix不产生unsupported初始化错误，`uv tool`也不再被误列为Search source。
- README、configuration与manager-authoring已同步三平台catalog、显式profile配置和能力限制。

## 提交

- `87eb32b docs: plan Nix profile manager iteration`
- `ce7f85e feat: add explicit Nix profile manager`
- `5ca2d27 docs: record Nix profile manager verification`

## 验证

- `cargo test -p updater-managers --test nix_profile_contract --jobs 1 -- --test-threads=1`：3项通过，1项真实宿主smoke忽略。
- `cargo test -p updater-managers --lib nix_profile --jobs 1 -- --test-threads=1`：3项通过。
- `cargo test --workspace --locked --jobs 1 -- --test-threads=1`：workspace全部通过；UI 49项、manager lib 74项，全部公开合同与doc tests无失败。
- `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings`：通过。
- `cargo check -p updater-managers --test nix_profile_contract --target x86_64-pc-windows-gnu --locked --jobs 1`：通过且零warning。
- `cargo check -p updater --target x86_64-pc-windows-gnu --locked --jobs 1`：通过。
- `cargo fmt --all -- --check`与`git diff --check`：通过。
- 当前开发宿主没有`nix`，未伪造真实宿主smoke结果；GitHub Actions原生CI结果见本轮最终提交。
- GitHub Actions CI `30749365137`：Linux、Windows、macOS全部通过；macOS与Windows均执行Nix profile contract，Linux通过完整format/check/test/clippy/build门禁。

## 遗留项 / 下一轮

- 需要多profile时先扩展manager instance identity，不能复制`builtin:nix-profile`绕过Config唯一性。
- ROADMAP阶段7后续候选需要新的领域模型；下一轮先做发布前状态与剩余风险审计，再决定是否进入新manager。
