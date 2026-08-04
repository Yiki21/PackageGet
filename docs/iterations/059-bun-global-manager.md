# Iteration 059：Bun Global manager

- 日期：2026-08-04
- 状态：进行中
- ROADMAP阶段：阶段7——扩展 Package Manager生态

## 目标

按ROADMAP较低优先级候选的既定顺序接入`Bun Global`，在不增加core/UI特例的前提下覆盖当前用户的全局JavaScript工具，并只广告能够通过官方CLI、离线fixture和原生runner验证的能力。

## 已确认契约

- manager ID为`builtin:bun`，category为Development，支持Linux、Windows和macOS，普通写操作不要求提权。
- availability执行`bun --version`，并复用共享的平台检查与可执行文件发现入口；标准`~/.bun/bin`及`BUN_INSTALL/bin`需要进入发现路径。
- installed执行`bun list --global --depth 0`，只接纳直接全局依赖的npm package name与semver；当前用户尚未创建Bun global manifest时，官方`MissingPackageJSON`状态映射为空inventory。
- updates执行只读`bun outdated --global`，解析`Package / Current / Update / Latest`表格并冻结明确的latest版本；不以真实update命令充当探测。
- install、update和uninstall分别使用`bun add --global`、`bun update --global`和`bun remove --global`，写操作前整组验证manager ID、user scope、package identity、origin和可选版本。
- typed origin使用`Bun global`与`package:<name>`；CLI未报告registry identity时不伪造npm registry来源。
- 首轮不广告Search：`bun pm view`只提供exact package metadata，没有可验证且尊重用户registry配置的目录搜索合同。

## 范围

- [x] 新增直接`BunManager`实现、三平台catalog注册与Bun品牌标识。
- [x] 添加installed/outdated解析、空global root、scoped identity、重复/畸形输出和写命令argv离线测试。
- [ ] 在Windows与macOS原生CI执行同一Bun contract；真实宿主Bun smoke保持显式ignored且只读。
- [x] 更新README、第三方notice、ROADMAP与迭代索引。
- [ ] 通过本地串行format/check/test/clippy/build门禁和原生CI。

## 实施结果

- `BunManager`直接实现`updater-manager-api`，只广告installed、updates、install、update和uninstall；不把`bun pm view`exact metadata伪装成Search。
- 共享availability入口先执行descriptor平台检查；Bun可执行文件发现覆盖`BUN_INSTALL/bin`和`~/.bun/bin`，catalog在Linux、Windows、macOS保持稳定顺序。
- installed严格解析官方global tree，updates严格解析`Package / Current / Update / Latest`表格；无global manifest或lockfile的官方空状态返回空inventory，其他退出码保留typed error。
- 写操作冻结manager ID、current-user scope、`Bun global`/`package:<name>` origin和semver target，并保留scoped package identity；version-pinned uninstall和file/git/workspace spec明确拒绝。
- Linux本机Bun 1.3.14在隔离global目录中复核了真实list/outdated输出；仓库fake fixture覆盖Unix、Windows batch和macOS runner可复用的同一argv合同。

## 非目标

- Bun CLI自身升级、cache清理、trusted dependencies、global directory自定义UI或项目依赖管理。
- 把file/git/workspace global spec伪装成稳定registry package identity。
- 为了统一Discover页面而新增不可靠的package search capability。

## 验证计划

- [x] `cargo fmt --all -- --check`
- [x] `cargo check --workspace --all-targets --locked --jobs 1`
- [x] `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- [x] `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- [x] `cargo build --workspace --locked --jobs 1`
- [x] Linux本机真实Bun只读smoke。
- [ ] GitHub Actions原生Windows/macOS Bun离线contract。

## 官方 CLI 依据

- [bun add --global](https://bun.com/docs/pm/cli/add#global)
- [bun outdated](https://bun.com/docs/pm/cli/outdated)
- [bun update](https://bun.com/docs/pm/cli/update)
- [bun remove](https://bun.com/docs/pm/cli/remove)
