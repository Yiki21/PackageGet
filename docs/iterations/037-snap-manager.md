# Iteration 037：Linux Snap manager

- 日期：2026-08-02
- 状态：进行中
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
- [ ] 实现直接SnapManager并接入Linux built-in catalog。
- [ ] 增加Linux离线CLI合同、严格parser与target拒绝测试。
- [ ] 通过本地完整质量门禁和可用环境中的真实只读Snap smoke。
- [ ] GitHub Actions在Linux原生runner全部通过。

## 验收标准

- descriptor只在Linux广告已经实现的六项能力，并明确写操作需要snapd授权。
- installed与updates保留snap name、revision/version、system scope、tracking channel、confinement和refresh状态。
- search保留stable channel、版本、publisher、summary与安装所需confinement。
- install/update/uninstall argv完全由严格校验的typed target生成，且程序路径前不出现`pkexec`或Updater system helper。
- malformed表格、重复identity、未知notes/origin、错manager/scope和版本固定target被严格拒绝。
- format/check/test/clippy/build和Linux原生CI无warning。

## 进度日志

### 2026-08-02

- Iteration 036完成Linux 0.3.0-beta.2发布，下一优先级进入Snap manager。
- snapd 2.76源码确认`list`、`refresh --list`和`find`表格合同；官方授权文档确认状态变更由snapd REST API使用peer credentials与Polkit action `io.snapcraft.snapd.manage`控制。

## Git提交

- 待实现后补充。

## 验证记录

- 待实现后补充。

## 遗留项 / 下一轮

- 本轮不改变snapd refresh timer、proxy、store或system-wide refresh policy。
- 下一轮按ROADMAP进入RubyGems与Composer Global，或先处理本轮验证发现的Snap合同缺口。
