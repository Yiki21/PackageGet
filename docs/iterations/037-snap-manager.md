# Iteration 037：Linux Snap manager

- 日期：2026-08-02
- 状态：已完成
- ROADMAP阶段：阶段7——扩展Package Manager生态
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

接入ROADMAP第三优先级Linux `Snap` application manager，覆盖installed、updates、search、install、update和uninstall；冻结channel、confinement、refresh状态与snapd原生Polkit授权边界。

## 范围决策

- installed解析`LC_ALL=C snap list`，保留tracking channel、confinement以及held/disabled自动刷新状态。
- updates先取得installed typed identity，再解析只读`snap refresh --list`；不执行真实refresh探测。
- search解析`snap find <query>`并把stable channel与classic/devmode confinement冻结进typed target。
- install根据typed target显式恢复`--channel`与必要的`--classic`/`--devmode`；update只执行`snap refresh <name>`，保留现有tracking channel与confinement；uninstall保留snapd默认snapshot行为。
- 写操作直接调用配置的`snap`客户端，由snapd通过`io.snapcraft.snapd.manage` Polkit action授权；Updater不以root身份启动用户配置的可执行文件。
- availability实际执行`snap version`；本轮只在Linux catalog注册。

## 实施计划

- [x] 核对snapd 2.76 CLI源码与官方文档中的list、refresh、find、write和Polkit合同。
- [x] 实现直接SnapManager并接入Linux built-in catalog。
- [x] 增加Linux离线CLI合同、严格parser与target拒绝测试。
- [x] 通过本地完整质量门禁；本机没有snap/snapd，真实只读smoke保持显式opt-in且未运行。
- [x] GitHub Actions在Linux原生runner执行独立Snap合同并通过，Windows/macOS portable check同步通过。

## 验收标准

- descriptor只在Linux广告已经实现的六项能力，并明确写操作需要snapd授权。
- installed与updates保留snap name、revision/version、system scope、tracking channel、confinement和refresh状态。
- search保留stable channel、版本、publisher、summary与安装所需confinement。
- install/update/uninstall argv完全由严格校验的typed target生成，且程序路径前不出现`pkexec`或Updater system helper。
- malformed表格、重复identity、冲突confinement、非法origin、错manager/scope和版本固定target被严格拒绝；格式合法的新note token完整保留，避免随snapd扩展静默丢状态。
- format/check/test/clippy/build和Linux原生CI无warning。

## 进度日志

### 2026-08-02

- Iteration 036完成Linux 0.3.0-beta.2发布，下一优先级进入Snap manager。
- snapd 2.76源码确认`list`、`refresh --list`和`find`表格合同；官方授权文档确认状态变更由snapd REST API使用peer credentials与Polkit action `io.snapcraft.snapd.manage`控制。
- 直接SnapManager进入Linux catalog，installed/update/search保留system scope、channel、confinement、refresh与原始notes；local Snap保持可见但不能伪装成store install target。
- 首次CI run `30742548500`三平台通过，但Snap专项step被放在仅含Windows/macOS的portable matrix中而始终跳过；随后移入Linux quality job，避免只依赖workspace全测试的隐式覆盖。
- 最终GitHub Actions run `30742663619`在Linux明确执行Snap专项合同，并与Windows、macOS全部通过。

## Git提交

- `b541d3c docs: plan Snap manager iteration`
- `0eae27c feat: add Linux Snap manager`
- `abbdbd8 ci: run Snap contract on Linux`

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：233项通过，17项忽略。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。
- `cargo check --workspace --target x86_64-pc-windows-gnu --locked --jobs 1`：通过。
- `cargo test -p updater-managers --test snap_contract --locked --jobs 1 -- --test-threads=1`：3项通过，1项真实宿主smoke忽略。
- `cargo test -p updater-managers --lib snap --locked --jobs 1 -- --test-threads=1`：4项Snap parser/identity单元测试通过。
- 本机未安装`snap`，因此没有执行`host_snap_read_only_smoke_is_explicitly_opt_in`；未进行任何真实Snap写操作。
- GitHub Actions run `30742663619`：Linux 2m55s、Windows 2m38s、macOS 1m21s，全部通过；Linux独立Snap offline contract step通过。

## 遗留项 / 下一轮

- 本轮不改变snapd refresh timer、proxy、store或system-wide refresh policy。
- 下一轮按ROADMAP进入RubyGems与Composer Global，或先处理本轮验证发现的Snap合同缺口。
