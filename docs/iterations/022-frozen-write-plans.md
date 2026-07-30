# Iteration 022：写操作冻结计划与协作取消文案

- 日期：2026-07-30
- 状态：已完成
- ROADMAP阶段：阶段5写操作透明度与Linux beta发布前可靠性
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

Discover安装和Updates已选项更新必须先从当前页面数据与selection生成冻结的manager/package计划，明确展示package名称、数量、来源和已知提权要求，再由用户确认执行。取消入口必须准确说明它只会阻止下一个manager启动，已经运行的manager命令会自然结束。

## 实施计划

- [x] 在现有`content/workflows.rs`中增加Finding和Updates共同使用的轻量冻结计划，不增加controller、service、trait或通用状态机。
- [x] Finding将直接安装改为prepare/confirm/cancel三步；确认后只消费冻结计划，不重新读取live selection。
- [x] Updates用一个pending update状态统一selected update与既有Update All确认；两者确认后都只消费冻结计划。
- [x] 两个确认区展示每个manager的package名称/数量，并按`ManagerDescriptor::authorization`标明required或may request authorization。
- [x] pending确认期间冻结会改变计划范围的选择、刷新和搜索入口；Escape优先取消确认。
- [x] active operation取消按钮和status文案明确为“当前manager完成后停止”；不声称终止当前命令或系统事务。
- [x] 增加计划冻结、空计划拒绝、确认取消和取消状态文案测试。
- [x] 更新ROADMAP和本记录，并串行通过完整workspace门禁。

## 行为约束

- 冻结范围是按manager稳定排序的package名称列表；确认与执行之间不得重新读取`selected_packages`或页面cache。
- pending确认期间不启动新的package读取或修改selection；取消确认后恢复正常交互。
- Update All仍先强制刷新全部配置source，再冻结成功source的计划并列出被排除的失败source。
- 提权提示只来自manager descriptor，UI不按manager ID硬编码系统manager名单。
- `CancellationToken`仍只在manager group之间检查。用户请求停止后，当前manager命令继续运行；完成后不再启动后续manager。
- 真正的child/process-group终止、terminating状态和底层退出确认仍属于ROADMAP阶段5后续独立迭代，不在本轮伪实现。
- 不因确认视图较长拆出只使用一次的helper；仅共享Finding和Updates都消费的冻结计划与计划明细视图。

## 验收标准

- Discover点击Install和Updates点击Update Selected时不立即执行写操作，而是显示冻结计划。
- 确认区逐manager显示package名称/数量，并准确区分required与may request authorization。
- 确认执行使用prepare时的manager/package列表；后续live selection/cache变化不改变该计划。
- 空或已失效的selection不会进入运行态，也不会触发panic。
- Update All现有刷新、失败source排除和失败source重试行为保持不变。
- Escape和Cancel只关闭pending确认；运行中的停止入口清楚表达“当前manager完成后停止”。
- 完整串行质量门禁通过，无warning。

## 进度日志

### 2026-07-30

- 审计确认Discover当前直接从live selection执行安装，selected Updates也直接执行；Update All已有冻结计划，但只显示模糊的系统source提权提示。
- `AuthorizationHint`已由manager descriptor提供，确认页可以直接显示真实metadata，不需要硬编码APT/DNF等ID。
- core执行器只在每个manager group开始前检查`CancellationToken`；当前UI的“Cancel Task”容易被理解为立即终止，必须改为真实协作取消语义。
- `PackageActionPlan`只保存稳定排序的manager groups和package名称；Finding与Updates分别持有pending plan，确认后直接move进现有串行执行器，不增加通用工作流层。
- Discover安装和selected Updates现在先显示确认区；共同的计划明细视图逐manager列出package名称/数量，并根据descriptor区分`Authorization required`与`May request authorization`。
- Updates的selected与Update All共享单一pending update状态；Update All的强制刷新、失败source排除和failed-source重试保持原行为。
- 确认期间会禁用改变计划范围的source、selection、search/refresh入口；Escape先取消确认，再关闭inspector。
- `Stop After Current Manager`触发后变为`Stop Requested`，status panel明确当前manager会完成、后续manager不会启动；core结果文案同步改为在另一manager启动前停止。
- 新增7项测试，覆盖Finding/Updates冻结计划、失效selection、Escape优先级和取消边界文案；未执行真实系统package写操作。
- 完整workspace串行门禁通过：193项测试成功、14项真实环境测试显式ignored、0失败，format/check/clippy/build均通过。
- 发布判断：本轮已清除Linux beta前的写操作确认与取消文案缺口；仍需Iteration 023完成Linux release hardening后再发布`0.3.0-beta.1`，当前不tag。

## Git提交

- `304d4ca docs: plan frozen write operations iteration`
- `f028fa7 feat: confirm frozen package write plans`

## 验证记录

- `cargo check -p updater --all-targets --locked --jobs 1`：通过，无warning。
- `cargo test -p updater --locked --jobs 1 -- --test-threads=1`：46项成功、0失败。
- `cargo clippy -p updater --all-targets --locked --jobs 1 -- -D warnings`：通过，无warning。
- `cargo test -p updater_core --test execution --locked --jobs 1 -- --test-threads=1`：4项成功、0失败。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过，无warning。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：通过，193项成功、14项显式ignored、0失败。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。

## 遗留项 / 下一轮

- Iteration 023：Linux Wayland/X11、clean/旧配置恢复矩阵和release artifact验收；通过后发布`0.3.0-beta.1`。
- 真正的process lifecycle取消仍按ROADMAP阶段5单独实施，不纳入Linux beta前的文案修正。
