# Iteration 025：跨平台编译与目标 Registry 基线

- 日期：2026-08-01
- 状态：已完成
- ROADMAP阶段：阶段4——Windows/macOS原生支持前置基线
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

在不提前实现Winget、跨平台发布物或声称实机验收的前提下，让同一workspace具备明确的目标平台边界：窗口配置可在Windows/macOS编译，当前构建只注册descriptor声明支持的manager，CI在原生Windows和macOS runner持续检查完整workspace。

## 实施计划

- [x] 在公共manager API中提供当前目标平台解析，删除各manager和UI重复的target判断。
- [x] 保留完整built-in catalog构造顺序，同时提供按明确`Platform`过滤的catalog和registry入口。
- [x] 当前GUI构建只注册适用manager：Linux保留现有11项，macOS注册Homebrew与便携开发manager，Windows在Winget落地前保持空catalog。
- [x] 将Linux窗口application ID设置限制在Linux目标，修复非Linux system helper编译警告。
- [x] CI增加原生Windows/macOS workspace库与二进制`cargo check`，不在Linux伪装macOS交叉编译成功；Unix-only测试fixture继续由Linux完整门禁覆盖。
- [x] 串行通过Linux完整质量门禁，并记录Windows/macOS原生CI仍需远端复验。

## 行为约束

- 不通过空壳manager或虚假capability让Windows看似已有包管理功能；Winget留给独立迭代。
- 不删除unsupported manager的已保存配置；本轮只改变当前构建的runtime registry，继续保留未知ID配置恢复契约。
- 不把`cfg`分支包装成只使用一次的helper；只有当前平台解析和catalog过滤作为真实复用边界。
- macOS检查必须由GitHub Actions原生runner执行；Linux主机缺少Apple SDK和clang工具链时，只记录环境边界。

## 验收标准

- `Platform::current()`在Linux、Windows和macOS返回稳定typed value，其他目标返回`None`。
- `builtin_managers_for(Platform::Linux)`保持现有稳定顺序；macOS只包含descriptor明确支持的manager；Windows在Winget前为空。
- `register_builtin_managers`只注册当前目标支持的manager，显式平台入口仍可离线测试所有目标的过滤结果。
- Windows目标不再因Iced `PlatformSpecific::application_id`或helper import失败。
- Linux完整format/check/test/clippy/build通过；Windows/macOS原生CI check通过后再关闭本轮。

## 进度日志

### 2026-08-01

- Windows `x86_64-pc-windows-gnu`初始检查已编译到UI，确认阻塞为Iced Windows `PlatformSpecific`不存在`application_id`字段；同时system helper存在非Linux未使用`Command`警告。
- macOS `aarch64-apple-darwin`从Linux交叉检查被`aws-lc-sys`需要的Apple编译器参数阻断，确认应使用原生`macos` runner而非降低依赖或伪造本地通过。
- `Platform::current()`成为manager API中的唯一目标解析入口；APT、DNF、Pacman、Zypper、Flatpak、Homebrew和UI删除重复判断。
- `builtin_managers_for`与`register_builtin_managers_for`按descriptor过滤；Linux完整catalog保持11项产品顺序，macOS包含Homebrew、Cargo、Go、npm、pnpm和pipx，Windows在Winget实现前为空。
- Windows GNU目标的workspace库和二进制`cargo check`无warning通过；Linux窗口继续设置`com.ayi.updater`，Windows/macOS使用Iced各自的默认native platform settings。
- CI保留Linux全目标check/test/clippy/build，并新增`windows-latest`和官方arm64 `macos-15`原生workspace check；run `30686922393`三项原生job均通过。

## Git提交

- `6e6509e feat: establish cross-platform registry baseline`

## 验证记录

- `cargo fmt --all -- --check`通过。
- `cargo check --workspace --all-targets --locked --jobs 1`通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`通过：202 passed，14个真实网络、本机manager或外部环境smoke test按约定ignored。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`通过。
- `cargo build --workspace --locked --jobs 1`通过。
- `cargo check --workspace --locked --jobs 1 --target x86_64-pc-windows-gnu`通过，无warning。
- `ruby`/Psych解析`.github/workflows/ci.yml`通过；本机未安装`actionlint`。
- Linux主机上的macOS交叉检查未通过，失败停在`aws-lc-sys`调用非Apple `cc`时不识别`-arch`和`-mmacosx-version-min`；该结果用于确认原生runner边界，不记为产品源码失败或macOS通过。
- GitHub Actions run `30686922393`通过：macOS arm64 workspace check耗时1分44秒，Linux完整format/check/test/clippy/build耗时2分10秒，Windows x86_64 workspace check耗时3分34秒。

## 遗留项 / 下一轮

- 下一轮实现Winget manager的fixture、命令构造和typed result契约。
- macOS Homebrew真实只读验证、Windows/macOS窗口启动与发布物仍属于阶段4/6后续迭代。
