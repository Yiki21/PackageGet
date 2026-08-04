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

- [ ] 新增直接`BunManager`实现、三平台catalog注册与Bun品牌标识。
- [ ] 添加installed/outdated解析、空global root、scoped identity、重复/畸形输出和写命令argv离线测试。
- [ ] 在Windows与macOS原生CI执行同一Bun contract；真实宿主Bun smoke保持显式ignored且只读。
- [ ] 更新README、第三方notice、ROADMAP与迭代索引。
- [ ] 通过本地串行format/check/test/clippy/build门禁和原生CI。

## 非目标

- Bun CLI自身升级、cache清理、trusted dependencies、global directory自定义UI或项目依赖管理。
- 把file/git/workspace global spec伪装成稳定registry package identity。
- 为了统一Discover页面而新增不可靠的package search capability。

## 验证计划

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --locked --jobs 1`
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`
- `cargo build --workspace --locked --jobs 1`
- Linux本机真实Bun只读smoke；GitHub Actions原生Windows/macOS Bun离线contract。
