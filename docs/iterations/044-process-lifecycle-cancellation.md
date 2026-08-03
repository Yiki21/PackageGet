# Iteration 044：进程生命周期取消与 1.0 发布政策

- 日期：2026-08-03
- 状态：进行中
- ROADMAP阶段：阶段5操作透明度与阶段6发布收口
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

把取消从manager group之间的协作检查扩展到正在运行的内置package-manager命令。UI只有在底层进程树已经退出后才能报告取消完成；执行期间必须区分已请求和正在终止，不能因为future被丢弃就假定系统事务停止。

同时冻结1.0发布政策：Linux、Windows和macOS产物均允许未签名发布，签名、Apple notarization及证书secrets不作为1.0门禁。所有发布页面和安装文档必须明确标注unsigned，并要求使用同一release中的`SHA256SUMS`校验下载；CI构建记录和严格资产清单作为公开provenance，不把未配置签名默认为遗漏。

## 范围决策

- `ProgressSink`增加向后兼容的只读取消查询，第三方manager不实现时保持不可取消；core为每次执行注入实际token。
- 内置manager的统一命令runner在命令启动前和运行期间检查取消。Unix命令进入独立process group，Windows按PID终止完整process tree；runner等待底层child退出并回收输出reader后才返回`ManagerErrorKind::Cancelled`。
- core收到typed cancellation时返回取消结果，不把它包装成普通manager failure；已经成功完成的package与manager计数继续保留。
- UI将按钮和状态文案改为“Stop Operation”与“Stopping current manager...”，最终结果只在runner确认退出后出现。
- 已经通过Polkit进入root身份的Linux系统事务存在权限与包数据库一致性边界。runner会请求终止其process group并等待可观察child退出，但不承诺绕过内核权限强杀root事务；若无法确认退出，必须返回明确终止失败，不能报告Cancelled。
- 本轮不扩展Activity字段；完整时间、scope与per-manager outcome留在下一独立迭代，避免同时改变执行生命周期和持久化schema。

## 实施计划

- [x] 冻结unsigned 1.0发布政策与取消生命周期契约。
- [ ] 扩展公共取消查询并保持第三方`ProgressSink`源码兼容。
- [ ] 为内置runner增加Unix process group、Windows process tree终止和退出确认。
- [ ] 将全部内置manager写命令接入取消查询。
- [ ] 区分core取消结果与普通失败，并更新UI状态文案。
- [ ] 增加API、core、runner与UI回归测试。
- [ ] 通过本地串行门禁、远端原生平台CI与Package workflow。

## 验收标准

- 取消静默长运行命令时不依赖stdout/stderr产生新行，runner仍能发现请求。
- Unix测试确认child及其同process-group后代均退出；Windows实现使用系统process-tree终止语义并由原生CI编译验证。
- `OperationOutcome`能稳定区分success、cancelled和failed，取消不设置`failed_manager`。
- UI在请求后立即禁止重复点击，底层退出前显示正在终止，退出后才显示取消结果。
- 1.0文档明确说明全平台unsigned、校验步骤和provenance；CI中没有伪签名或空notarization结果。

## 遗留项 / 下一轮

- Activity补齐完整时间、scope和per-manager outcome，并提供schema迁移与redaction测试。
- 在Windows、macOS及Linux Wayland/X11真实环境完成安装、启动、升级、卸载smoke记录。
