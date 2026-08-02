# Iteration 035：.NET global tools manager

- 日期：2026-08-02
- 状态：进行中
- ROADMAP阶段：阶段7——扩展Package Manager生态
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

接入ROADMAP第二优先级`.NET global tools`，首批覆盖installed、updates、search、install、update和uninstall；只管理current-user global scope，并在Linux、Windows与macOS原生runner冻结CLI与写操作合同。

## 范围决策

- installed只解析`dotnet tool list --global --format json`；local tool manifest与任意`--tool-path`不进入同一inventory。
- updates对每个已安装package执行`dotnet package search <id> --exact-match --format json`，继承当前目录的NuGet.Config层级并从返回的稳定版本元数据中选择最新版本；不执行真实update探测。
- search使用tool-only的`dotnet tool search`，不把普通NuGet library误报为可安装工具。
- typed target使用`.NET global tool` origin与`global:<package-id>` reference；所有写操作显式携带`--global`，install允许可选精确版本，update/uninstall拒绝版本固定target。
- availability必须真实执行`dotnet --version`，因此只有shim而没有当前可用SDK时保持unavailable。

## 实施计划

- [x] 核对当前.NET CLI与官方文档的global list、NuGet exact search和write合同。
- [ ] 实现直接DotnetToolManager并接入三平台built-in catalog。
- [ ] 增加Linux/Windows/macOS可运行的离线CLI合同。
- [ ] 通过本地完整质量门禁、Windows GNU交叉检查与真实只读dotnet smoke。
- [ ] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

## 验收标准

- descriptor在三平台只广告已经实现的六项能力。
- installed与updates保留package ID、installed/latest版本、user scope和typed origin；命令别名不被误作package identity。
- search只返回.NET tool结果，并保留NuGet package ID、最新版本和描述。
- `dotnet tool install/update/uninstall <id> --global`参数由typed target冻结，只有install可增加`--version <version>`。
- JSON schema、重复identity、malformed package/version/source和异常搜索表格被严格拒绝。
- format/check/test/clippy/build、Windows GNU check和三平台原生CI无warning。

## 进度日志

### 2026-08-02

- Iteration 034完成`uv tool` manager，GitHub Actions run `30736297578`在Linux、Windows和macOS原生runner全部通过。
- 本机.NET SDK 10.0.302确认`tool list --global --format json`输出versioned JSON，`package search --exact-match --format json`按NuGet.Config源返回全部版本；`tool search`保持tool-only但不提供JSON格式。

## Git提交

- 待记录。

## 验证记录

- 待记录。

## 遗留项 / 下一轮

- local manifest、任意`--tool-path`和多manager-instance identity不在本轮范围。
- private authenticated NuGet source由dotnet CLI及用户NuGet.Config负责；Updater不读取或持有源凭证。
