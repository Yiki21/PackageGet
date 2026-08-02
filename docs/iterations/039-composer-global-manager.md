# Iteration 039：Composer Global manager

- 日期：2026-08-02
- 状态：已完成
- ROADMAP阶段：阶段7——扩展Package Manager生态
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

接入ROADMAP下一优先级Composer Global manager，覆盖installed、updates、search、install、update和uninstall；只管理当前`$COMPOSER_HOME/composer.json`中`require`声明的直接全局依赖，不把项目、`require-dev`或传递依赖伪装成独立工具。

## 范围决策

- 通过`composer global config home --absolute`发现当前Composer实际使用的绝对home；后续每个读写命令都显式绑定同一`COMPOSER_HOME`，避免验证路径与执行路径漂移。
- installed交叉验证global `composer.json`的`require`对象与`composer global show --direct --format=json`结构化输出；只接纳合法`vendor/package`且确实直接声明、已经安装的包。
- updates使用`composer global outdated --direct --format=json`的`version`、`latest`和`latest-status`，并再次与直接`require`集合求交；不执行真实update探测。
- search使用`composer global search --format=json`，保留当前global Composer配置的repositories与认证语义，不硬编码Packagist。
- install使用`composer global require`；update只对单个直接依赖执行`composer global update <name> --with-dependencies`；uninstall使用`composer global remove <name>`。
- typed origin以结构化JSON冻结绝对home、package name和原始constraint；每次写操作重新发现home并拒绝错manager、scope、origin、home或非直接依赖目标。

## 实施计划

- [x] 核对Composer 2.10.0 CLI帮助、真实空global home输出与上游Show/Search命令JSON合同。
- [x] 实现直接ComposerGlobalManager并接入Linux、Windows、macOS built-in catalog。
- [x] 增加三平台离线CLI合同、严格JSON parser、direct dependency与home ownership测试。
- [x] 通过本地完整质量门禁、Windows GNU交叉检查与真实只读Composer smoke。
- [x] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

## 验收标准

- descriptor在三平台只广告已经实现的六项能力，scope固定为current-user global且不要求提权。
- installed只展示`composer.json.require`与Composer已安装结构化输出的交集，并保留name、version、constraint与绝对home。
- updates只为已安装的直接require生成，search尊重global repositories；空global home返回空列表而不是协议错误。
- install/update/uninstall只在重新发现并绑定的绝对home执行；update/uninstall拒绝传递依赖、`require-dev`、伪造home与过期origin。
- malformed home、composer.json、show/outdated/search输出、重复identity、不安全name/constraint与错manager/scope/origin被严格拒绝。
- format/check/test/clippy/build、Windows GNU check与三平台原生CI无warning。

## 进度日志

### 2026-08-02

- Iteration 038完成RubyGems manager，按ROADMAP进入Composer Global直接依赖身份合同。
- 本机Composer 2.10.0确认global home可通过`global config home --absolute`发现，空global inventory/outdated返回JSON空数组；官方ShowCommand源码确认`--direct`按root requires过滤且JSON可提供`direct-dependency`、`version`、`latest`与`latest-status`。
- 完成`builtin:composer-global`六项能力与三平台catalog接入；每次读写重新发现绝对home，后续命令绑定`COMPOSER_HOME`并移除可重定向manifest的`COMPOSER`环境变量。
- 官方ShowCommand源码复核确认非空show/outdated结果为`{"installed":[...]}`，空结果才为`[]`；parser冻结这两种合同并拒绝非空顶层数组、重复identity和字段漂移。
- `--direct`实际合并`require`与`require-dev`；实现同时解析两张根依赖表，只展示/更新`require`，显式跳过合法`require-dev`，但拒绝不属于任一根依赖集合的伪造条目。
- typed origin以JSON冻结home、package和constraint；install只接受search origin，update/uninstall要求当前`composer.json.require`仍存在且constraint未漂移。
- 首轮CI `30747158635`中Linux与macOS通过，Windows仅因CMD `%*`保留`"--format=json"`语法引号导致日志断言失败；生产读取、搜索和写命令均已成功。
- 本机Wine复现同一Windows断言，日志归一化去除命令行语法引号后整份Windows合同4项通过、1项忽略；最终CI `30747439412`全绿：Linux 3m14s、Windows 3m00s、macOS 58s。

## Git提交

- `3ba1b24 docs: plan Composer Global manager iteration`
- `2f86501 feat: add Composer Global manager`
- `294d899 test: normalize Composer Windows argv logging`

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：251项通过，20项按环境要求忽略。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --all-targets --locked --jobs 1`：通过。
- `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked --jobs 1`：通过；仅打印既有Unix-only测试在Windows target下的cfg import warning，Composer无新增warning。
- `cargo test -p updater-managers --test composer_contract --locked --jobs 1 -- --test-threads=1`：4项通过，1项忽略。
- `cargo test -p updater-managers --lib composer --locked --jobs 1 -- --test-threads=1`：5项通过。
- `cargo test -p updater-managers --test composer_contract --target x86_64-pc-windows-gnu --locked --jobs 1 --no-run`：Windows GNU合同编译通过。
- Wine执行Windows GNU Composer合同：4项通过，1项忽略。
- Composer真实只读smoke：1项通过，验证本机availability、global home、inventory/count、outdated与search，不执行写操作。
- GitHub Actions CI `30747439412`：Linux、Windows、macOS全部通过。

## 遗留项 / 下一轮

- 本轮不管理当前项目依赖、`require-dev`、Composer plugins生命周期、传递依赖或多个global home实例。
- 下一轮按ROADMAP进入显式单一用户profile的Nix profile评估与实现。
