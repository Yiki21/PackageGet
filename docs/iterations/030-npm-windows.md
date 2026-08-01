# Iteration 030：npm Windows 原生准入

- 日期：2026-08-01
- 状态：已完成
- ROADMAP阶段：阶段4——按实际平台能力注册可移植管理器
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

沿用Cargo与Go的Windows原生准入标准，验证npm global root、scoped package路径、registry identity和写操作在Windows上的真实行为，将npm纳入Windows内置目录。

## 范围决策

- 保持现有npm parser与typed target合同；production已经要求CLI package path精确等于global root下的预期路径，并在目录计量前验证canonical containment。
- 顶层symlink/junction继续只读且不计量，目录遍历不跟随被识别为symlink的内部条目；本轮不声称管理npm link来源。
- Windows合同使用真实临时绝对路径、转义后的Windows JSON path与受控`npm.cmd`，覆盖availability、installed、outdated、search、typed update和uninstall。
- registry URL与scoped package identity保持跨平台一致，写操作继续以独立argv传递，不拼接shell命令。
- pnpm与pipx继续延后到各自Windows原生合同完成后再调整平台声明。

## 实施计划

- [x] 审计npm global root、installed path、size、outdated、search与write边界。
- [x] npm descriptor与内置目录新增Windows能力。
- [x] 增加Windows原生npm离线合同并接入portable CI。
- [x] 串行通过本地完整质量门禁和Windows GNU交叉检查。
- [x] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

## 验收标准

- `builtin_managers_for(Platform::Windows)`稳定返回Winget、Cargo、Go和npm。
- Windows global root与`@scope/tool` JSON path通过精确路径及canonical containment验证。
- installed size、outdated target、search已安装版本和registry origin保持准确。
- Windows原生合同冻结`npm install -g @scope/tool@2.0.0`与`npm uninstall -g @scope/tool`参数。
- format/check/test/clippy/build、Windows GNU check和三平台GitHub Actions无warning。

## 进度日志

### 2026-08-01

- Iteration 029完成Go Windows原生准入，GitHub Actions run `30692627630`及文档收口run `30692753373`均通过。
- 审计确认npm production路径未使用Unix专属API；global root、CLI package path与canonical containment合同可直接由Windows临时目录验证。
- 首次Windows原生运行发现fake CLI的嵌套batch条件会在未命中的`--depth=0`参数上触发`cmd.exe`解析错误，production尚未进入JSON解析。
- fake CLI改为按首参数使用label分派，避开带`=`参数的嵌套条件歧义；最终原生runner通过Windows JSON path、size、outdated、search与typed write合同。

## Git提交

- `f174006 feat: admit npm on Windows`
- `41a02db test: make npm Windows fixture cmd-safe`

## 验证记录

- `cargo fmt --all -- --check`通过。
- `cargo check --workspace --all-targets --locked --jobs 1`通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`通过：217 passed，14 ignored。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`通过。
- `cargo build --workspace --locked --jobs 1`通过。
- `cargo check --workspace --target x86_64-pc-windows-gnu --locked --jobs 1`通过。
- `cargo check -p updater-managers --test npm_contract --target x86_64-pc-windows-gnu --locked --jobs 1`通过且无warning。
- `cargo test -p updater-managers --test npm_contract --locked --jobs 1 -- --test-threads=1`通过：9 passed，1 ignored。
- GitHub Actions CI run `30693412188`通过：Linux完整质量门禁2m11s、Windows x86_64 workspace及manager lib/Cargo/Go/npm原生离线合同2m37s、macOS arm64 workspace及Homebrew合同2m06s。

## 遗留项 / 下一轮

- 按原生准入标准继续处理pnpm和pipx。
- npm link/junction来源保持只读，后续若要管理需新增明确source identity与跨平台reparse-point合同。
- Windows/macOS真实GUI smoke与安装包交付仍由阶段4后续迭代和阶段6承担。
