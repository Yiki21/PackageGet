# Iteration 028：Cargo Windows 原生准入

- 日期：2026-08-01
- 状态：进行中
- ROADMAP阶段：阶段4——按实际平台能力注册可移植管理器
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

在Windows已具备Winget和PATHEXT命令发现基线后，将生产实现已经具备跨平台路径语义的Cargo纳入Windows内置目录，并用Windows原生runner执行离线读写契约，建立其他可移植管理器后续准入的验证标准。

## 范围决策

- Cargo新增Windows平台声明，Windows内置目录按系统管理器优先顺序提供Winget和Cargo。
- Windows契约使用受控`cargo.cmd`与`rg.exe`夹具，覆盖availability、`cargo install --list`解析、安装目录体积以及install/update/uninstall参数，不访问真实registry或修改runner工具链。
- Cargo命令继续使用结构化参数与`CARGO_TERM_COLOR=never`，不通过业务代码拼接shell命令。
- Go暂不在Windows注册：现有清单以`GOBIN`文件名作为package identity，需先明确`.exe`后缀与module identity的映射。
- npm、pnpm和pipx暂不在Windows注册：先补各自Windows原生fake CLI读写契约，再调整平台声明。

## 实施计划

- [x] 审计Cargo、Go、npm、pnpm和pipx的Windows路径、身份与测试边界。
- [x] Cargo descriptor和内置目录新增Windows能力。
- [x] 增加Windows原生Cargo离线契约并接入portable CI。
- [ ] 串行通过本地完整质量门禁和Windows GNU交叉检查。
- [ ] GitHub Actions在Linux、Windows和macOS原生runner全部通过。

## 验收标准

- `builtin_managers_for(Platform::Windows)`稳定返回`builtin:winget`、`builtin:cargo`。
- Cargo descriptor明确包含Linux、Windows和macOS，其他可移植manager的平台声明不被扩大。
- Windows原生测试真实执行`.cmd`夹具，解析`rg.exe`且从配置的install root计算体积。
- Windows原生测试冻结`cargo install --version`、`cargo install --force`和`cargo uninstall`参数边界。
- format/check/test/clippy/build、Windows GNU check和三平台GitHub Actions无warning。

## 进度日志

### 2026-08-01

- Iteration 027已完成Windows PATHEXT、跨平台desktop opener和macOS Homebrew原生合同，GitHub Actions run `30690411248`及文档收口run `30690500893`均通过。
- 审计确认Cargo生产路径已支持`.exe`二进制体积查询和结构化命令执行，当前缺口集中在平台声明与Windows原生合同。
- 审计确认Go会直接使用`GOBIN`文件名作为package name，不能在未处理`.exe`身份前直接放开；Node与pipx管理器也缺Windows原生读写合同。

## Git提交

- 待记录。

## 验证记录

- 待执行。

## 遗留项 / 下一轮

- 按相同准入标准逐个处理Go、npm、pnpm和pipx，不以跨平台编译通过替代原生行为合同。
- Windows/macOS真实GUI smoke与安装包交付仍由阶段4后续迭代和阶段6承担。
