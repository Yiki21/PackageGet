# Iteration 026：Winget Manager 首轮原生契约

- 日期：2026-08-01
- 状态：进行中
- ROADMAP阶段：阶段4——Windows首批Winget manager
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

在Iteration 025的Windows原生编译与目标registry基线上，新增首个Windows manager：Winget。首轮覆盖availability、installed、updates、search、install、update和uninstall；所有读写目标冻结为官方Package Identifier与source，非交互参数、协议解析和HRESULT错误分类均有离线测试保护。

## 官方CLI审计结论

- `winget list`、`search`和`upgrade`当前官方文档没有通用JSON输出；不能实现或测试不存在的`--output json`。
- `winget export --output <file> --include-versions`使用官方packages JSON schema，适合作为可管理installed inventory的主路径；无法匹配source的普通Windows应用不会被伪装成Winget package。
- search与upgrade继续输出表格；解析必须从纯横线separator推导列范围，不依赖英语表头，分别冻结ID、version/available和source。
- install/upgrade/uninstall使用`--id <identifier> --exact`，存在source时追加`--source <name>`；所有source操作接受source agreement并禁用交互，install/upgrade同时接受package agreement。
- Winget官方HRESULT包含no applications found、requires admin、authentication cancelled、install in progress/no network/reboot required等稳定代码；优先按退出码分类，再保留有界命令诊断。

## 实施计划

- [x] 新增`WingetManager` descriptor和Windows catalog注册，保留Linux/macOS现有catalog不变。
- [x] 使用官方export JSON schema解析PackageIdentifier、Version、Scope和SourceDetails，临时文件在成功或失败后均清理。
- [x] 为search与updates增加separator/header-padding离线表格fixture，覆盖带空格名称、可选Match列和Unicode列宽。
- [x] 冻结install/update/uninstall命令参数，拒绝跨manager target、缺失source identity、非法scope和不支持的版本重放。
- [x] 将Winget HRESULT映射到typed `ManagerErrorKind`，覆盖permission、network、busy、reboot required、cancelled、protocol/no-match。
- [ ] Windows原生CI运行Winget纯离线单元测试；真实Windows只读smoke保持显式ignored，不执行真实安装、升级或卸载。
- [x] 串行通过Linux完整质量门禁和Windows GNU check，再由GitHub Actions复验Linux/Windows/macOS。

## 行为约束

- 不把`winget list`中无法匹配source的任意ARP应用纳入可升级或可重放installed inventory。
- 不按显示名称执行写操作；Package Identifier和source必须从读取结果冻结，legacy无source target只允许用户显式输入ID的install场景。
- 不使用`--force`、`--ignore-security-hash`、`--override`或自动提升权限；installer自身请求UAC时由Windows处理。
- 不批量调用`upgrade --all`；core按manager group串行，Winget内部逐个冻结target执行并报告部分进度。
- 不新增只复用一次的parser wrapper；JSON、table、target和HRESULT边界各自保持直接、可测试。

## 验收标准

- Windows catalog包含`builtin:winget`，Linux/macOS catalog结果不回归。
- installed JSON、search table和updates table fixture均能保留ID、版本、source和scope；协议缺列或空关键字段返回Protocol错误。
- 所有写命令包含`--id`、`--exact`、`--disable-interactivity`和agreement参数，source identity存在时精确重放。
- no-match读操作返回空集合；reboot/admin/network/busy/cancel HRESULT返回对应typed error。
- Linux完整format/check/test/clippy/build、Windows GNU check和原生三平台CI通过，无warning。

## 进度日志

### 2026-08-01

- Iteration 025已由GitHub Actions run `30686922393`完成Linux、Windows x86_64和macOS arm64原生编译基线。
- 官方winget-cli文档与packages schema审计完成，确认只有export提供installed JSON；list/search/upgrade仍需解析表格输出。
- UI确认计划和core执行入口已从包名字符串改为完整`PackageTarget`，确保Winget以及现有Homebrew/Flatpak的scope和origin在确认后不丢失。
- 新增`WingetManager`、Windows catalog注册、export JSON解析、Unicode display-width表格解析、逐目标写命令和官方HRESULT分类。
- Windows CI新增Winget纯离线lib测试；真实Windows availability smoke保持ignored。

## Git提交

- 待实现提交后回填。

## 验证记录

- `cargo fmt --all -- --check`通过。
- `cargo check --workspace --all-targets --locked --jobs 1`通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`通过：213 passed，14 ignored。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`通过。
- `cargo build --workspace --locked --jobs 1`通过。
- `cargo check --workspace --target x86_64-pc-windows-gnu --locked --jobs 1`通过。
- GitHub Actions原生Linux/Windows/macOS复验待提交后运行。

## 遗留项 / 下一轮

- 本轮不交付Windows安装包、图标、版本资源或签名；属于阶段6发布物迭代。
- Windows真实只读smoke和窗口启动需要原生环境人工/受控验证后记录。
