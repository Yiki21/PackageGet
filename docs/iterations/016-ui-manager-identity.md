# Iteration 016：UI ManagerId Identity Cutover

- 日期：2026-07-30
- 状态：已完成
- ROADMAP阶段：阶段3——配置、UI identity与manager设置迁移
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将UI内部所有manager identity、HashMap/HashSet key、message payload、selection key、progress和operation outcome从闭合`PackageManagerType`切换为稳定`ManagerId`。App使用direct built-in registry提供descriptor metadata；未注册manager不得回退显示为DNF。现有静态core执行入口只在任务执行边界临时解析built-in ID，本轮不把命令引擎重写混入UI迁移。

## 实施计划

- [x] 审计App、Finding、Installed、Updates、Settings、shared、workflows、activity与status中的`PackageManagerType`状态和fallback调用面。
- [x] 建立UI manager catalog/context：注册direct built-ins，按`ManagerId`解析descriptor、display name、description、category、capability和当前runtime支持状态。
- [x] 将shared `PackageSelectionKey`、keyboard navigation、inspector和manager filter组件改为`ManagerId`，删除DNF display fallback。
- [x] 将Finding的selected/searching/results/errors/messages/inspector/install progress改为`ManagerId`。
- [x] 将Installed的selected/loading/results/errors/messages/inspector/remove progress改为`ManagerId`。
- [x] 将Updates的selected/loading/results/errors/messages/update plan/progress/retry state改为`ManagerId`。
- [x] 将workflows的grouping、batch progress、operation outcome和failure identity改为`ManagerId`；只在调用旧core执行函数前解析built-in runtime type。
- [x] 将App初始化任务、reload reconciliation、progress logs与Content路由改为`ManagerId`。
- [x] 将Settings detection、availability、path selection和manager渲染key改为`ManagerId`；unknown configured manager保持可见且保存不丢失。
- [x] 将Activity/Status消费的manager identity改为`ManagerId`，display统一通过catalog解析或稳定ID fallback。
- [x] 增加catalog metadata、unknown manager fallback、selection reconciliation、page state和operation grouping contracts。
- [x] 更新ROADMAP/manager authoring与本轮进度，记录仍存在的静态core执行边界。
- [x] 串行通过workspace format、check、test、clippy与build完整门禁，并由GitHub Actions复验。

## Identity决策

- `ManagerId`是UI状态、消息和集合的唯一manager key；display name不能充当identity。
- descriptor是name、description、category、platform、capability与authorization的唯一metadata来源。
- unknown/missing manager显示其稳定ID和明确missing状态，绝不使用任意built-in作为fallback。
- `PackageManagerType`只允许出现在现有core兼容执行边界和该边界的测试中，不得重新进入UI page state。
- selection reconciliation按`ManagerId`处理，配置刷新时保留仍配置的unknown manager ID。
- manager group顺序必须由配置顺序或catalog稳定顺序决定，不能依赖HashMap迭代顺序。

## 非目标

- 本轮不删除core中的`PackageManagerType`、静态trait、旧UI package model或`core/src/pm/*`wrapper。
- 本轮不把所有read/write workflow直接重写为`ManagerRegistry::manager_for`；该执行引擎cutover单独迭代。
- 本轮不实现Config load恢复页面、缺失第三方manager安装机制或运行时动态插件。
- 本轮不改变任何package manager命令、parser、提权、批处理、取消或网络语义。
- 本轮不执行真实install、update、uninstall或配置目录以外的系统修改。
- 本轮不写死Rust或依赖的最低minor/patch版本。

## 验证方案

- 编译期`rg`契约确认UI page state、message payload与selection key不再使用`PackageManagerType`。
- unknown manager fixture断言显示稳定ID、不会显示DNF、配置round-trip不丢失。
- Finding/Installed/Updates选择与retry contracts按`ManagerId`保持现有交互语义。
- batch grouping、partial failure和activity/status断言保留正确manager identity。
- 所有测试保持离线，不调用真实package manager写操作。
- 完整workspace门禁逐条串行，使用单job与单测试线程；GitHub Actions复验最终HEAD。

## 进度日志

### 2026-07-30

- Iteration 015已完成唯一Config schema直接切换，本机配置和CI均已验证。
- 当前UI共有大量`PackageManagerType`状态引用，集中在三个package页面、App初始化、shared selection/inspector、workflows和Settings。
- Finding、Installed与Updates当前在找不到source时使用`PackageManagerType::Dnf`作为display fallback，本轮必须删除。
- direct registry已有稳定descriptor排序和capability检查，可作为UI metadata source；静态core执行边界继续作为本轮非目标。
- 新增UI `ManagerCatalog`，从direct built-in registry缓存descriptor；unknown ID显示稳定ID，平台注册状态从descriptor解析。
- Finding、Installed、Updates、Settings、shared、workflows、App init、Activity和Status的manager identity已统一切换为`ManagerId`。
- Config中的unknown manager不会再被`filter_map(PackageManagerType::from_manager_id)`丢弃；Settings保持可见并保留原配置，初始化/read task返回带原ID的明确错误。
- Finding、Installed和Updates已删除DNF fallback；旧core上报的progress manager不再覆盖当前group的`ManagerId`。
- Activity failure identity已切换为`ManagerId`；后续Iteration 017按用户要求删除了version与旧display-name兼容层，只保留单一当前schema。
- UI package测试25项通过；完整workspace门禁等待文档检查点提交后执行。

## Git提交

- `fc893e0 docs: plan ui manager identity cutover`
- `112b02e refactor(ui): use stable manager identities`
- `b4d878c docs: record ui identity cutover progress`
- `91076d1 refactor(ui): satisfy identity quality gates`
- `56ef880 docs: record ui identity local validation`

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo check -p updater --jobs 1`：通过，无warning。
- `cargo test -p updater --jobs 1 -- --test-threads=1`：通过，25项测试全部成功。
- 静态identity检查：UI中的`PackageManagerType`只剩App/Finding/Installed/Updates/workflows最终旧core执行边界；无DNF fallback、unknown-ID `filter_map`或manager state集合残留。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：通过，211项成功，14项显式ignored，0失败。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。
- GitHub Actions CI run `30509352724`：成功，format/check/test/clippy/build job全部通过，耗时4分23秒。

## 遗留项 / 下一轮

- `PackageManagerType`与旧静态core read/write API仍存在；下一轮应将执行引擎改为registry/capability驱动，再删除UI中的最终ID转换边界。
- Config load error仍只写日志；需要单独实现可见恢复页面。
- Activity记录尚无时间戳，Settings executable path尚未完成通用validation/reset交互。
