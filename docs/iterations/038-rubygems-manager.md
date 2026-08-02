# Iteration 038：RubyGems manager

- 日期：2026-08-02
- 状态：进行中
- ROADMAP阶段：阶段7——扩展Package Manager生态
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

接入ROADMAP下一优先级RubyGems manager，覆盖installed、updates、search、install、update和uninstall；严格区分`GEM_HOME`/user gem home/repository与同一gem的多个已安装版本。

## 范围决策

- 通过`gem environment home`、`user_gemhome`与`path`发现当前Ruby实际可见的repository；每个repository使用隔离的`GEM_HOME`/`GEM_PATH`执行本地读取，避免RubyGems跨路径合并同名同版本。
- installed解析`gem list --local --details --all`的官方location合同；typed origin使用结构化JSON冻结repository、精确installed version与default-gem状态。
- updates在每个repository内执行只读`gem outdated`，只为该repository中最新的已安装版本生成更新；不执行真实update探测。
- search使用用户当前RubyGems source配置执行`gem search <query> --remote --all`；install默认写入当前`GEM_HOME`，不硬编码rubygems.org。
- update使用`gem update <name> --install-dir <repo>`并保留RubyGems的多版本行为；uninstall按origin精确版本执行并拒绝default gem、依赖确认和伪造repository。
- 所有命令移除`RUBYGEMS_GEMDEPS`，避免从当前目录自动发现并执行项目依赖文件。

## 实施计划

- [x] 核对RubyGems 4.0.10 CLI帮助与上游源码中的environment、list details、outdated、search和write合同。
- [ ] 实现直接RubyGemsManager并接入Linux、Windows、macOS built-in catalog。
- [ ] 增加三平台离线CLI合同、严格parser、repository ownership与多版本拒绝测试。
- [ ] 通过本地完整质量门禁、Windows GNU交叉检查与真实只读RubyGems smoke。
- [ ] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

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

## Git提交

- 待实现后补充。

## 验证记录

- 待实现后补充。

## 遗留项 / 下一轮

- 本轮不管理Bundler项目依赖、Gemfile、vendor目录或Ruby runtime版本。
- 下一轮按ROADMAP进入Composer Global的直接依赖与`COMPOSER_HOME`身份合同。
