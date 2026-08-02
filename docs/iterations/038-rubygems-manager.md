# Iteration 038：RubyGems manager

- 日期：2026-08-02
- 状态：已完成
- ROADMAP阶段：阶段7——扩展Package Manager生态
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

接入ROADMAP下一优先级RubyGems manager，覆盖installed、updates、search、install、update和uninstall；严格区分`GEM_HOME`/user gem home/repository与同一gem的多个已安装版本。

## 范围决策

- 通过`gem environment home`、`user_gemhome`与`path`发现当前Ruby实际可见的repository；每个repository使用隔离的`GEM_HOME`/`GEM_PATH`执行本地读取，并按完整`GEM_PATH`allowlist过滤RubyGems仍会注入的default gem，避免跨路径合并同名同版本。
- installed解析`gem list --local --details --all`的官方location合同；typed origin使用结构化JSON冻结repository、精确installed version与default-gem状态。
- updates在每个repository内执行只读`gem outdated`，只为该repository中最新的已安装版本生成更新；不执行真实update探测。
- search使用用户当前RubyGems source配置执行`gem search <query> --remote --all`；install默认写入当前`GEM_HOME`，不硬编码rubygems.org。
- update使用`gem update <name> --install-dir <repo>`并保留RubyGems的多版本行为；uninstall按origin精确版本执行并拒绝default gem、依赖确认和伪造repository。
- 所有命令移除`RUBYGEMS_GEMDEPS`，避免从当前目录自动发现并执行项目依赖文件。

## 实施计划

- [x] 核对RubyGems 4.0.10 CLI帮助与上游源码中的environment、list details、outdated、search和write合同。
- [x] 实现直接RubyGemsManager并接入Linux、Windows、macOS built-in catalog。
- [x] 增加三平台离线CLI合同、严格parser、repository ownership与多版本拒绝测试。
- [x] 通过本地完整质量门禁、Windows GNU交叉检查与真实只读RubyGems smoke。
- [x] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

## 验收标准

- descriptor在三平台只广告已经实现的六项能力，并说明system repository可能需要授权。
- installed保留name、version、repository、user/system scope和default状态；跨repository重复identity与同repository多版本都可区分。
- updates只读比较每个repository的current/latest版本；search尊重用户RubyGems source配置。
- install/update/uninstall只接受当前Ruby环境声明的绝对repository；uninstall按精确origin version执行且default gem保持只读。
- malformed环境、details/outdated/search输出、重复identity、错manager/scope/origin与不安全name/version被严格拒绝。
- format/check/test/clippy/build、Windows GNU check与三平台原生CI无warning。

## 进度日志

### 2026-08-02

- Iteration 037完成Linux Snap manager，下一优先级进入RubyGems/Composer Global；本轮先单独完成RubyGems，避免混合两套global identity。
- 本机RubyGems 4.0.10确认`gem list --local --details`报告每个版本的`Installed at`；上游`Gem::QueryUtils`源码确认单repository与多版本location格式。
- 完成`builtin:rubygems`六项能力及三平台catalog接入；typed origin以JSON冻结repository、精确版本和default状态，每次写操作前重新读取当前Ruby环境并验证repository ownership。
- 真实只读smoke发现RubyGems即使在隔离`GEM_HOME`/`GEM_PATH`下仍会注入其他当前repository拥有的default gem；实现改为只接纳完整环境allowlist中的location，并仅在对应repository轮次收集。
- 首轮CI `30744992672`中Linux与macOS通过，Windows `.cmd` fixture的嵌套label分派失败；改为顶层参数分派后，第二轮CI `30745145936`的Windows RubyGems原生合同通过。
- 第二轮CI的Linux暴露既有Homebrew fixture并行改写导致`ETXTBSY`；加文件级异步锁后，第三轮CI `30745336190`确认Homebrew已通过，并继续暴露既有npm fixture的并行availability串扰。
- npm合同采用相同的文件级异步隔离后，最终CI `30745584077`全绿：Linux 3m06s、Windows 2m38s、macOS 45s，RubyGems在Windows/macOS原生runner均通过。

## Git提交

- `4bfc00f docs: plan RubyGems manager iteration`
- `d6de1a9 feat: add RubyGems manager`
- `f8ecf74 test: fix RubyGems Windows fixture dispatch`
- `9f6fc26 test: serialize Homebrew fixture contracts`
- `0a8318b test: serialize npm fixture contracts`

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：241项通过，18项按环境要求忽略。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --all-targets --locked --jobs 1`：通过。
- `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked --jobs 1`：通过。
- `cargo test -p updater-managers --test rubygems_contract --locked --jobs 1 -- --test-threads=1`：3项通过，1项忽略。
- `cargo test -p updater-managers --lib rubygems --locked --jobs 1 -- --test-threads=1`：5项通过。
- RubyGems真实只读smoke：1项通过，验证本机availability、installed与count，不执行写操作。
- `cargo test -p updater-managers --test homebrew_contract --locked --jobs 1`：8项通过，1项忽略；默认测试并行度下通过。
- `cargo test -p updater-managers --test npm_contract --locked --jobs 1`：9项通过，1项忽略；默认测试并行度下通过。
- `cargo test -p updater-managers --test npm_contract --target x86_64-pc-windows-gnu --locked --jobs 1 --no-run`：Windows GNU合同编译通过。
- GitHub Actions CI `30745584077`：Linux、Windows、macOS全部通过。

## 遗留项 / 下一轮

- 本轮不管理Bundler项目依赖、Gemfile、vendor目录或Ruby runtime版本。
- 下一轮按ROADMAP进入Composer Global的直接依赖与`COMPOSER_HOME`身份合同。
