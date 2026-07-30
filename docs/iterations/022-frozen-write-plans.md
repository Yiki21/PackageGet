# Iteration 022：写操作冻结计划与协作取消文案

- 日期：2026-07-30
- 状态：进行中
- ROADMAP阶段：阶段5写操作透明度与Linux beta发布前可靠性
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

Discover安装和Updates已选项更新必须先从当前页面数据与selection生成冻结的manager/package计划，明确展示package名称、数量、来源和已知提权要求，再由用户确认执行。取消入口必须准确说明它只会阻止下一个manager启动，已经运行的manager命令会自然结束。

## 实施计划

- [ ] 在现有`content/workflows.rs`中增加Finding和Updates共同使用的轻量冻结计划，不增加controller、service、trait或通用状态机。
- [ ] Finding将直接安装改为prepare/confirm/cancel三步；确认后只消费冻结计划，不重新读取live selection。
- [ ] Updates用一个pending update状态统一selected update与既有Update All确认；两者确认后都只消费冻结计划。
- [ ] 两个确认区展示每个manager的package名称/数量，并按`ManagerDescriptor::authorization`标明required或may request authorization。
- [ ] pending确认期间冻结会改变计划范围的选择、刷新和搜索入口；Escape优先取消确认。
- [ ] active operation取消按钮和status文案明确为“当前manager完成后停止”；不声称终止当前命令或系统事务。
- [ ] 增加计划冻结、空计划拒绝、确认取消和取消状态文案测试。
- [ ] 更新ROADMAP和本记录，并串行通过完整workspace门禁。

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

## Git提交

- 待记录。

## 验证记录

- 待执行。

## 遗留项 / 下一轮

- Iteration 023：Linux Wayland/X11、clean/旧配置恢复矩阵和release artifact验收；通过后发布`0.3.0-beta.1`。
- 真正的process lifecycle取消仍按ROADMAP阶段5单独实施，不纳入Linux beta前的文案修正。
