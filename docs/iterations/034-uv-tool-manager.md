# Iteration 034：uv tool manager

- 日期：2026-08-02
- 状态：进行中
- ROADMAP阶段：阶段7——扩展Package Manager生态
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

接入ROADMAP第一优先级`uv tool`，首批覆盖installed、updates、install、update和uninstall；通过`uv tool list --outdated`执行只读更新探测，并在Linux、Windows与macOS原生runner冻结CLI、路径和写操作合同。

## 范围决策

- installed使用`uv tool dir`与`uv tool list --show-paths`，只接受tool root的canonical直接子目录并严格计量普通文件；symlink不跟随。
- updates只解析`uv tool list --outdated --show-paths`中显式的installed/latest版本，不用真实upgrade探测。
- typed target使用`uv tool` origin与`tool:<name>` reference；install允许可选精确版本，update/uninstall拒绝版本固定target。
- 首轮不广告search：当前uv CLI没有能同时尊重用户Python index配置的安全只读搜索合同，不以硬编码PyPI覆盖私有源语义。

## 实施计划

- [x] 核对当前uv CLI与官方文档的tool list/outdated/upgrade合同。
- [x] 实现直接UvManager并接入三平台built-in catalog。
- [x] 增加Linux/Windows/macOS可运行的离线CLI与文件系统合同。
- [ ] 通过本地完整质量门禁、Windows GNU交叉检查与真实只读uv smoke。
- [ ] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

## 验收标准

- descriptor只广告已实现的五项能力，Search保持关闭。
- installed与updates保留tool name、installed/latest版本、user scope、typed origin与严格目录size。
- `uv tool install name==version`、`uv tool upgrade name`与`uv tool uninstall name`参数由typed target冻结。
- escaping/symlink tool环境被拒绝，平台fixture不访问真实网络或修改runner的uv环境。
- format/check/test/clippy/build、Windows GNU check和三平台原生CI无warning。

## 进度日志

### 2026-08-02

- Iteration 033完成pipx Windows准入；随后移除CI全局`--test-threads=1`，仅对实际共享fixture的Cargo合约做文件内串行，run `30735554809`在默认CI并行度下三平台通过。
- 本机`uv 0.11.32`确认installed header为`name vVERSION (path)`，outdated header增加`[latest: VERSION]`；官方文档确认upgrade保留原安装约束。

## Git提交

- 待记录。

## 验证记录

- 待记录。

## 遗留项 / 下一轮

- 只有出现尊重uv用户配置、私有index与凭证语义的只读查询合同后才增加Search。
- uv的额外requirements、extras、Python版本与复杂source identity后续按独立typed metadata增量处理，不在首轮把display文本伪装成完整来源模型。
