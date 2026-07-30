# Iteration 018：Registry Execution Engine Cutover

- 日期：2026-07-30
- 状态：进行中
- ROADMAP阶段：阶段2收尾与阶段3执行边界迁移
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将UI最后保留的legacy执行边界切换为`ManagerRegistry`和对象安全的`PackageManager` API，使读取、搜索、刷新与写操作都直接使用`ManagerId`。切换完成后删除`PackageManagerType`、宏生成的match dispatcher、旧静态trait及仅为兼容层存在的core manager适配器。

## 实施计划

- [ ] 提交已通过workspace检查的代码精简：内联单次复用helper、删除空namespace封装，并保持现有行为不变。
- [ ] 盘点direct manager API与UI任务边界，明确读取、刷新、搜索、进度、失败和取消语义。
- [ ] 在core提供registry驱动的领域执行入口；分组写操作保持manager间串行、首次失败停止和组间取消检查。
- [ ] 让UI共享同一个`ManagerRegistry`，catalog与执行任务不再各自构造或依赖closed enum。
- [ ] 将初始化、Finding、Installed、Updates与Settings检测任务切换到direct API模型和capability检查。
- [ ] 删除legacy enum、宏dispatcher、静态trait、`core/src/pm/`兼容模块及无用依赖和测试。
- [ ] 更新ROADMAP与manager authoring文档，记录新的执行路径和剩余阶段3工作。
- [ ] 串行通过workspace format、check、test、clippy与build完整门禁。

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
- 已完成一轮行为保持的代码精简，待作为本轮第一个独立检查点提交。

## Git提交

- 待记录。

## 遗留项 / 下一轮

- 待本轮完成后填写。
