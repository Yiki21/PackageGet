# Iteration 029：Go Windows 原生准入

- 日期：2026-08-01
- 状态：进行中
- ROADMAP阶段：阶段4——按实际平台能力注册可移植管理器
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

沿用Iteration 028建立的原生准入标准，修复Go在Windows上混淆逻辑package identity与`.exe`实际文件名的问题，将Go纳入Windows内置目录，并在Windows runner执行完整离线合同。

## 范围决策

- Windows `GOBIN`中的`tool.exe`对外稳定表示为`tool`，module与install package继续来自`go version -m -json`，不从文件名猜测。
- inventory内部同时保留逻辑名和实际文件名；update/install使用逻辑target与typed origin，uninstall在匹配build-info后删除实际`.exe`。
- 卸载继续要求无版本target、唯一inventory匹配、typed origin一致、普通文件和canonical `GOBIN` containment，不放宽现有安全边界。
- Windows合同使用受控`go.cmd`和`tool.exe`，覆盖availability、inventory、updates、typed install与实际卸载，不访问网络或真实Go环境。
- npm、pnpm与pipx继续延后到各自Windows原生合同完成后再调整平台声明。

## 实施计划

- [x] 审计Go inventory、build-info、legacy update和uninstall路径。
- [x] 分离Windows逻辑二进制名与实际`.exe`文件名。
- [x] Go descriptor与内置目录新增Windows能力。
- [x] 增加Windows原生Go离线合同并接入portable CI。
- [ ] 串行通过本地完整质量门禁和Windows GNU交叉检查。
- [ ] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

## 验收标准

- `builtin_managers_for(Platform::Windows)`稳定返回Winget、Cargo和Go。
- `tool.exe` inventory对外名称为`tool`，size和module/package origin保持准确。
- update target、typed install和legacy identity不携带`.exe`后缀。
- uninstall通过逻辑名唯一定位build-info，并只删除匹配的实际`tool.exe`。
- format/check/test/clippy/build、Windows GNU check和三平台GitHub Actions无warning。

## 进度日志

### 2026-08-01

- Iteration 028完成Cargo Windows原生准入，GitHub Actions run `30691476536`及文档收口run `30691592153`均通过。
- 审计确认Go调用`go version -m -json`时已持有实际路径，但此前直接把`file_name()`同时用于UI identity和uninstall路径，导致Windows暴露`tool.exe`并破坏逻辑target语义。

## Git提交

- 待记录。

## 验证记录

- 待执行。

## 遗留项 / 下一轮

- 按原生准入标准继续处理npm、pnpm和pipx。
- Windows/macOS真实GUI smoke与安装包交付仍由阶段4后续迭代和阶段6承担。
