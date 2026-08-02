# Iteration 040：Nix profile manager

## 状态

进行中

## 目标

接入 ROADMAP 下一优先级 Nix profile manager。首批只管理用户明确选择的单一 profile，覆盖 installed、install、update 和 uninstall，并在 package origin 中保留 profile、元素名、original flake URL、locked flake URL、flake attribute 与 outputs identity。

## 范围

- [ ] 新增 `builtin:nix-profile` direct manager，仅注册 Linux 与 macOS。
- [ ] 通过 manager-owned `settings.profile` 保存一个绝对 profile 路径；缺失、空值、相对路径与已知 system profile 必须拒绝。
- [ ] Settings 提供 Nix profile 选择与清除入口；自动检测不能生成缺少 profile 的配置。
- [ ] installed 只解析 `nix profile list --json --profile <path>` 的 version 1/2/3 manifest schema，并保留 source identity。
- [ ] install target 必须携带显式 installable identity；update/uninstall target 必须与当前 profile 中同名元素的完整 origin 一致。
- [ ] update 只允许拥有未锁定 original flake reference 的元素；store-path-only 与 locked-only 元素仍可列出和卸载，但不能伪装成可更新。
- [ ] 写操作按 target 串行调用 `nix profile install|upgrade|remove --profile <path>`，并复用 bounded progress/error contract。
- [ ] 不广告 `Updates` 或 `Search`：Nix profile 没有只读 update inventory，`nix search` 也没有单一 profile catalog 语义。
- [ ] 离线 parser、配置、命令构造、伪造 target 与 Linux/macOS 原生 fake CLI 合同全部通过。
- [ ] 本地串行 workspace 门禁、Windows GNU compile check、GitHub Actions Linux/Windows/macOS CI 全部通过。

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

- 待实现。

## 提交

- 待补充。

## 验证

- 待补充。

## 遗留项 / 下一轮

- 待本轮完成后评估 ROADMAP 中剩余候选；需要多 profile 时先扩展 manager instance identity。
