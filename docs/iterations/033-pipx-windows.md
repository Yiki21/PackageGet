# Iteration 033：pipx Windows 原生准入

- 日期：2026-08-02
- 状态：已完成
- ROADMAP阶段：阶段4——按实际平台能力注册可移植管理器
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

沿用Cargo、Go、npm与pnpm的Windows原生准入标准，验证pipx的PIPX_HOME、venv目录、distribution/venv identity、PyPI只读探测和写操作在Windows上的真实行为，将pipx纳入Windows内置目录。

## 范围决策

- 保持现有pipx list parser、source分类与typed target合同；production继续要求`PIPX_HOME/venvs`及每个直接子venv是可canonicalize的普通目录。
- Windows合同使用真实临时绝对路径与受控`pipx.cmd`，覆盖availability、installed size、PyPI updates、exact search、typed install/update/uninstall。
- install继续使用distribution identity与可选`==version`；update/uninstall继续使用venv identity。非PyPI、editable与pinned来源的既有只读规则不变。
- PyPI请求只访问测试进程内的loopback HTTP fixture，不访问真实网络；写操作只记录argv，不修改runner的pipx环境。

## 实施计划

- [x] 审计PIPX_HOME、venv containment、list schema、PyPI读取与write边界。
- [x] pipx descriptor与内置目录新增Windows能力。
- [x] 增加Windows原生pipx离线合同并接入portable CI。
- [x] 串行通过本地完整质量门禁和Windows GNU交叉检查。
- [x] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

## 验收标准

- `builtin_managers_for(Platform::Windows)`稳定返回Winget、Cargo、Go、npm、pnpm和pipx。
- Windows `PIPX_HOME/venvs/tool-env`通过绝对路径、canonical direct-child与严格目录计量验证。
- installed、updates与search保持venv、distribution、版本、user scope和PyPI origin准确。
- Windows原生合同冻结`pipx install example-tool==2.1.0`、`pipx upgrade tool-env`与`pipx uninstall tool-env`参数。
- format/check/test/clippy/build、Windows GNU check和三平台GitHub Actions无warning。

## 进度日志

### 2026-08-02

- Iteration 032完成pnpm真实初始化退出码修复，最终GitHub Actions run `30734098003`通过。
- 审计确认pipx production路径未使用Unix专属API；Windows特有风险集中在`.cmd`执行、反斜杠绝对目录、venv direct-child containment及包含`==`的install参数。
- 首次Windows原生运行已通过availability、inventory、PyPI updates/search与三类write执行，仅fixture使用原始`%*`记录到Rust为含`==`参数添加的cmd安全引号；改用`%~1/%~2`记录cmd解析后的逻辑argv，production命令构造不变。
- GitHub Actions run `30735094534`最终在Linux、Windows和macOS全部通过；Windows原生runner验证了`pipx.cmd`、Windows `PIPX_HOME`、loopback PyPI读取及install/update/uninstall写合同。

## Git提交

- `59b2ea8 feat: admit pipx on Windows`
- `14d24d2 test: normalize pipx Windows batch argv`

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：217项通过，14项忽略。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。
- `cargo check --workspace --target x86_64-pc-windows-gnu --locked --jobs 1`：通过。
- `cargo check -p updater-managers --test pipx_contract --target x86_64-pc-windows-gnu --locked --jobs 1`：通过。
- `cargo test -p updater-managers --test pipx_contract --locked --jobs 1 -- --test-threads=1`：9项通过，2项Windows专属测试在Linux忽略。
- GitHub Actions run `30735094534`：Linux 2m20s、Windows 2m38s、macOS 1m47s，全部通过。

## 遗留项 / 下一轮

- Windows/macOS真实GUI smoke与安装包交付仍由阶段4后续迭代和阶段6承担。
- 阶段4首批可移植manager完成Windows准入后，下一轮重新按ROADMAP剩余完成标准选择GUI smoke、发布物或阶段5工作流增量。
