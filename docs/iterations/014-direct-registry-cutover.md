# Iteration 014：Direct Registry Cutover 与 Legacy Adapter 清理

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——完成direct built-in切换并清理过渡层
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

全部内置manager已经迁入`updater-managers`并由对象安全API直接注册，但core仍公开`LegacyPackageManagerAdapter`与`register_legacy_managers`，注册文件中也保留永远不会命中的legacy fallback循环。本轮移除这条已失去生产调用方的过渡路径，冻结单一direct built-in catalog/registration契约，为后续Config V2和UI `ManagerId`迁移提供稳定入口。

## 实施计划

- [ ] 审计`LegacyPackageManagerAdapter`、`register_legacy_managers`、built-in registration、Config V1 wrapper与外部fake manager测试的当前调用图。
- [ ] 冻结全部built-in ID、descriptor顺序、平台集合与capability，确保cutover不改变registry可见契约。
- [ ] 将direct built-in集合收敛为单一catalog/registration入口，避免core手写direct注册后再遍历legacy fallback。
- [ ] 删除`LegacyPackageManagerAdapter`、`register_legacy_managers`及其仅服务过渡层的转换、错误和progress代码。
- [ ] 保留Config V1和当前UI所需的静态wrapper，不在本轮同时迁移配置格式或页面state key。
- [ ] 增加catalog完整性、稳定顺序、duplicate ID、capability与平台过滤契约；外部trait-object manager测试继续通过。
- [ ] 审计并删除因adapter退出而无调用方的依赖、imports、tests与dead code，不引入新的crate分组目录。
- [ ] 更新manager authoring/ROADMAP相关文档，明确第三方manager走公共API显式注册，不再以legacy enum adapter接入。
- [ ] 串行通过workspace format、check、test、clippy与build完整门禁，并由GitHub Actions复验。

## 边界决策

- 本轮只删除registry侧legacy adapter，不删除Config V1、`PackageManagerType`、旧UI模型或每个manager的Config/model/progress兼容wrapper。
- direct built-in catalog必须返回`Arc<dyn PackageManager>`或等价对象安全集合；core继续拥有duplicate detection和registry ordering。
- built-in descriptor是稳定公共契约；cutover不得更改ID、display name、category、platform、capability或authorization。
- 目标平台不适用manager的注册/可见策略按现有行为冻结；若发现ROADMAP与当前实现不一致，先用contract记录，再单独修复平台过滤。
- 不引入运行时动态库、Rust ABI插件加载、branch或PR流程。
- 不写死Rust、Python或任何依赖的最低minor/patch版本；toolchain与CI继续跟随stable channel。

## 非目标

- 本轮不实施Config V2持久化、backup/restore或V1到V2原子迁移。
- 本轮不把UI selection、HashMap/HashSet key从`PackageManagerType`迁为`ManagerId`。
- 本轮不删除静态manager wrapper或closed enum dispatcher；这些必须与Config/UI identity迁移协调完成。
- 本轮不新增Winget、Windows/macOS发布物或修改Iced UI。
- 本轮不执行任何真实包管理器写操作。

## 验证方案

- 单元/契约：built-in catalog包含全部且仅包含稳定ID，顺序确定，重复注册仍返回typed duplicate error。
- 回归：Config V1 wrapper、UI编译、external fake manager注册、全部manager离线contract保持通过。
- 代码健康：adapter专用符号、fallback循环和dead dependencies不可残留。
- 本地门禁逐条串行并使用单job/单测试线程；真实manager测试保持ignored/read-only。
- GitHub Actions复验每个实现与文档收口检查点。

## 进度日志

### 2026-07-29

- Iteration 013完成后，全部内置manager均已有`updater-managers` direct implementation。
- 当前`register_builtin_managers`先逐个注册全部direct manager，随后仍遍历`ALL_PACKAGE_MANAGERS`执行一个实际上不会命中的legacy fallback。
- 调用图初查约有323处`PackageManagerType`相关引用，分布在Config V1、UI页面state、静态wrapper和adapter；本轮只处理已无生产必要的adapter路径。

## Git 提交

- Iteration 014计划检查点：本次提交。

## 验证记录

待实施后填写。

## 遗留项 / 下一轮

本轮完成后填写。
