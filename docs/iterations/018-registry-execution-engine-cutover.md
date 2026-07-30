# Iteration 018：Registry Execution Engine Cutover

- 日期：2026-07-30
- 状态：已完成
- ROADMAP阶段：阶段2收尾与阶段3执行边界迁移
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将UI最后保留的legacy执行边界切换为`ManagerRegistry`和对象安全的`PackageManager` API，使读取、搜索、刷新与写操作都直接使用`ManagerId`。切换完成后删除`PackageManagerType`、宏生成的match dispatcher、旧静态trait及仅为兼容层存在的core manager适配器。

## 实施计划

- [x] 提交已通过workspace检查的代码精简：内联单次复用helper、删除空namespace封装，并保持现有行为不变。
- [x] 盘点direct manager API与UI任务边界，明确读取、刷新、搜索、进度、失败和取消语义。
- [x] 在core提供registry驱动的领域执行入口；分组写操作保持manager间串行、首次失败停止和组间取消检查。
- [x] 让UI共享同一个`ManagerRegistry`，catalog与执行任务不再各自构造或依赖closed enum。
- [x] 将初始化、Finding、Installed、Updates与Settings检测任务切换到direct API模型和capability检查。
- [x] 删除legacy enum、宏dispatcher、静态trait、`core/src/pm/`兼容模块及无用依赖和测试。
- [x] 更新ROADMAP与manager authoring文档，记录新的执行路径和剩余阶段3工作。
- [x] 串行通过workspace format、check、test、clippy与build完整门禁。

## 行为约束

- manager identity始终使用validated `ManagerId`，显示信息仅从registry/catalog解析。
- registry lookup与capability检查在执行前完成；unknown、missing或unsupported manager返回明确错误。
- 同一manager的一组写操作保持既有顺序，不并发执行多个manager的系统修改。
- 取消只在manager组之间生效；不伪装成可以中断已启动的外部命令。
- 写操作保留逐包进度与部分成功结果；首次失败后不启动后续manager组。
- 本轮不改变用户配置schema、Activity schema或具体manager命令语义。

## 精简原则

- 只抽取真正跨调用点复用，或需要独立单元测试的逻辑。
- 单次调用的薄helper直接内联；不因函数体较长而机械拆分。
- 数据转换优先使用iterator与不可变值；副作用和状态更新保持边界清晰。
- 不用新type包装已经由`ManagerId`、registry和manager API表达的概念。

## 验收标准

- UI执行路径中不存在`PackageManagerType`转换。
- workspace中不存在legacy closed enum、宏dispatcher与旧静态`PackageManager` trait。
- built-in manager读取和写入均由同一个registry实例解析并检查capability。
- engine测试覆盖稳定顺序、unsupported capability、首次失败停止、部分结果与组间取消。
- 完整串行质量门禁通过，无warning。

## 进度日志

### 2026-07-30

- Iteration 017已完成Activity direct `ManagerId` schema；ROADMAP下一项是registry执行引擎切换与legacy执行边界删除。
- 行为保持的代码精简已提交：删除25个private helper与3个public helper，空`SharedUi` namespace改为直接模块函数，净减少281行。
- 新增`core/src/execution.rs`，只承载跨manager复用且需要独立测试的顺序、失败、部分结果、progress与取消语义；读取操作直接调用registry manager，不增加service wrapper。
- `ManagerCatalog`现在持有共享`Arc<ManagerRegistry>`；Config首次检测、Settings availability、初始化计数、Finding search、Installed list、Updates scan和三类写操作均使用该实例。
- UI模型直接使用manager API的`PackageInfo`与`PackageUpdate`；写操作直接使用`PackageAction`、`OperationProgress`与`OperationOutcome`。
- 取消不再abort Iced future或提前伪造Activity记录；token只在当前manager完成后的下一组边界生效，最终结果由core统一生成。
- 已删除`PackageManagerType`、`define_package_managers!`、旧静态trait、`core/src/pm/`的12个兼容文件和core的8个无用运行时依赖。
- 新增4项engine测试，覆盖输入顺序、unsupported capability、部分失败停止和组间取消。
- 完整串行门禁通过；workspace共171项测试成功、14项真实环境测试显式ignored、0失败。删除的41项测试属于legacy core适配器重复覆盖，direct manager contract测试继续保留。

## Git提交

- `55fd98f docs: plan registry execution engine cutover`
- `eb16b3f refactor: simplify core and ui flows`
- `54eb1f0 refactor: cut over to registry execution engine`
- `81bc5d4 docs: record registry execution cutover progress`

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过，无warning。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：通过，171项成功，14项显式ignored，0失败。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。
- workspace检索确认不存在`PackageManagerType`、`define_package_managers!`、旧静态`PackageManager` trait和UI legacy execution类型。

## 遗留项 / 下一轮

- 下一轮优先完成Settings executable path的保存前验证、错误反馈与显式重置，并保持draft/baseline隔离。
- Config load error可见恢复界面仍未实现，排在Settings路径闭环之后。
- Activity时间戳仍未加入。
