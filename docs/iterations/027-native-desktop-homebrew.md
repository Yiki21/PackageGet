# Iteration 027：原生桌面能力与 macOS Homebrew 验证

- 日期：2026-08-01
- 状态：已完成
- ROADMAP阶段：阶段4——跨平台原生桌面能力与macOS Homebrew
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

在Iteration 026完成Winget后，收敛阶段4剩余的自动化缺口：URL与目录打开不再硬编码Linux opener；Windows executable discovery遵循PATHEXT；macOS原生CI执行现有Homebrew离线读写契约，而不只做workspace编译。

## 范围决策

- Linux继续按顺序尝试`gio open`和`xdg-open`。
- macOS使用系统`open`命令；Windows对URL使用`rundll32.exe url.dll,FileProtocolHandler`，对目录使用`explorer.exe`，所有target均作为独立参数传递，不经过shell字符串拼接。
- Windows命令发现按PATHEXT顺序尝试无扩展名及`.COM`、`.EXE`、`.BAT`、`.CMD`等候选；显式带扩展名命令不重复追加。
- Homebrew保持现有formula/cask/tap identity与非交互写边界；本轮只将其离线合同放到macOS原生runner执行，不声称已完成真实主机读操作或GUI人工验收。
- 不在本轮改动native notification、文件选择器实现或发布物；这些需要真实窗口/系统交互smoke和阶段6打包验证。

## 实施计划

- [x] 增加typed desktop open target与按平台生成的直接命令spec，并覆盖URL/目录参数测试。
- [x] Windows executable discovery支持PATHEXT且保持Unix executable-bit语义不变。
- [x] 冻结macOS Apple Silicon `/opt/homebrew/bin`与Intel `/usr/local/bin`搜索路径。
- [x] Windows CI运行manager lib离线测试，macOS CI运行Homebrew contract测试。
- [x] 串行通过Linux完整质量门禁、Windows GNU check和GitHub Actions三平台原生复验。

## 验收标准

- Windows/macOS不再尝试`gio`或`xdg-open`；HTTP(S) URL与已验证目录通过各自平台命令直接传参。
- Windows PATH目录中仅存在`winget.exe`等PATHEXT候选时可被解析，PATHEXT大小写与空条目不造成重复或错误候选。
- Linux现有manager discovery与desktop opener回归测试不变。
- macOS原生runner通过Homebrew availability、installed、updates、search和typed write的fake CLI合同。
- 完整format/check/test/clippy/build、Windows GNU check和原生CI无warning。

## 进度日志

### 2026-08-01

- Iteration 026已完成Winget首轮原生契约，GitHub Actions run `30688164727`和文档收口run `30688323478`均通过。
- 审计发现`ui/src/content/shared.rs`仍只尝试Linux desktop opener；`managers/src/command.rs`路径扫描也未显式实现Windows PATHEXT候选。
- 现有Homebrew离线contract使用Unix fake CLI，可直接提升到macOS原生CI执行，真实host smoke继续保持ignored。
- URL与目录打开已按`Platform`生成拥有参数的直接命令：Linux `gio`/`xdg-open`、macOS `open`、Windows `rundll32`/`explorer`。
- executable discovery新增PATHEXT候选去重、显式扩展名保护和Windows Apps/npm/Scoop/Chocolatey常见目录；Linux/macOS Homebrew固定前缀改为目标平台选择。
- portable CI的Windows job改为运行全部manager lib离线测试，macOS job新增Homebrew integration contract。

## Git提交

- `d8d5b79 feat: add native desktop capability paths`

## 验证记录

- `cargo fmt --all -- --check`通过。
- `cargo check --workspace --all-targets --locked --jobs 1`通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`通过：216 passed，14 ignored。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`通过。
- `cargo build --workspace --locked --jobs 1`通过。
- `cargo check --workspace --target x86_64-pc-windows-gnu --locked --jobs 1`通过。
- `cargo test -p updater-managers --test homebrew_contract --locked --jobs 1 -- --test-threads=1`通过：8 passed，1 ignored。
- GitHub Actions CI run `30690411248`通过：Linux完整质量门禁2m05s、Windows x86_64 workspace与全部manager lib离线测试1m54s、macOS arm64 workspace与Homebrew contract 2m05s。

## 遗留项 / 下一轮

- Windows/macOS真实窗口启动、system theme、剪贴板、rfd文件/目录选择器、URL/目录实际打开和native notification仍需受控原生GUI smoke。
- 阶段6再交付Windows安装包及macOS `.app`/`.dmg`、图标、bundle/version metadata与unsigned限制说明。
