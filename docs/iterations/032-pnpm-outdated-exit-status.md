# Iteration 032：pnpm 初始化退出码修复

- 日期：2026-08-02
- 状态：已完成
- ROADMAP阶段：阶段4/5——便携manager真实CLI合同与初始化可靠性
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

修复pnpm存在可用更新时Updater初始化将合法结果误判为命令失败的问题，让已安装RPM与`cargo run`使用真实pnpm时都能完成updates初始化。

## 根因与范围

- 本机`pnpm 11.5.1 outdated --global --format json`在发现更新时输出合法JSON并以状态1退出；当前production复用只接受状态0的`run_success`，因此稳定误判失败。
- `pnpm list --global --depth 0 --json --long`在同一环境以状态0成功，installed路径、asdf shim发现和global root解析不是本次根因。
- 本轮只修正outdated退出码合同：状态0继续接受合法JSON；状态1仅在解析出非空更新集合时接受；其他状态与状态1空集合继续作为命令失败。
- 不改变pnpm installed、search、write、registry identity、路径containment或link只读规则。

## 实施计划

- [x] 使用本机asdf pnpm复现真实list/outdated输出与退出码。
- [x] 修正pnpm outdated状态1更新语义。
- [x] 增加状态1非空成功、状态0空集合成功、状态1空集合失败与其他非零状态失败的离线回归合同。
- [x] 串行通过完整本地质量门禁。
- [x] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

## 验收标准

- 本机真实pnpm更新结果不再被初始化误判为命令失败。
- `PackageUpdate`仍保留准确current/latest版本、user scope、registry origin与typed target。
- 非预期退出码和无更新证据的状态1不会被吞掉。
- format/check/test/clippy/build及三平台CI无warning。

## 进度日志

### 2026-08-02

- 应用菜单明确执行`/usr/bin/updater`，RPM数据库版本为`0.2.4-1`；用户确认当前`cargo run`同样复现，排除仅旧RPM二进制导致的问题。
- 本机真实命令返回两项更新：`playwright 1.62.0 -> 1.62.1`、`@volcengine/cli 1.0.52 -> 1.1.0`，stdout为合法JSON、stderr为空、退出状态为1。
- 同环境installed list返回状态0及合法global root/dependency JSON，根因收敛到outdated退出状态解释。
- 修复后真实主机pnpm只读smoke通过availability、installed/count、outdated与search；将PATH收窄为桌面会话的系统目录后再次通过，确认asdf shim discovery与退出码解释均正常。

## Git提交

- `6132882 fix: accept pnpm outdated update status`

## 验证记录

- `cargo fmt --all -- --check`通过。
- `cargo check --workspace --all-targets --locked --jobs 1`通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`通过：217 passed，14 ignored。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`通过。
- `cargo build --workspace --locked --jobs 1`通过。
- `cargo test -p updater-managers --test pnpm_contract --locked --jobs 1 -- --test-threads=1`通过：9 passed，1 ignored。
- `cargo test -p updater-managers --test pnpm_contract host_pnpm_read_only_smoke_is_explicitly_opt_in --locked --jobs 1 -- --ignored --exact --test-threads=1`通过：真实pnpm只读链路1 passed；精简桌面PATH下复验同样通过。
- GitHub Actions CI run `30734000047`通过：Linux完整质量门禁2m04s、Windows x86_64 workspace及manager原生离线合同2m32s、macOS arm64 workspace及Homebrew合同1m47s。

## 遗留项 / 下一轮

- 修复完成后继续ROADMAP中的pipx Windows原生准入。
- 当前安装的RPM仍为`0.2.4-1`；新修复需要后续Linux prerelease构建与安装才能进入应用菜单版本。
