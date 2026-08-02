# Iteration 039：Composer Global manager

- 日期：2026-08-02
- 状态：进行中
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
- [ ] 实现直接ComposerGlobalManager并接入Linux、Windows、macOS built-in catalog。
- [ ] 增加三平台离线CLI合同、严格JSON parser、direct dependency与home ownership测试。
- [ ] 通过本地完整质量门禁、Windows GNU交叉检查与真实只读Composer smoke。
- [ ] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

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

## Git提交

- 待实现后补充。

## 验证记录

- 待实现后补充。

## 遗留项 / 下一轮

- 本轮不管理当前项目依赖、`require-dev`、Composer plugins生命周期、传递依赖或多个global home实例。
- 下一轮按ROADMAP进入显式单一用户profile的Nix profile评估与实现。
