# Iteration 021：异步读取 Request Generation

- 日期：2026-07-30
- 状态：进行中
- ROADMAP阶段：阶段5异步结果新鲜度与阶段3发布前可靠性
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

初始化、Finding search、Updates refresh和Installed refresh必须只接受当前请求的结果。配置保存、package operation reload、重复刷新或同一manager的新请求开始后，旧Task可以自然结束，但其晚到消息不得覆盖新数据、写入旧错误、清除当前loading状态或提前完成Update All预检。

## 实施计划

- [x] 记录Iteration 020恢复页的用户人工验收，并审计所有异步读取Task与结果消息。
- [x] App为每次`reload_package_data`分配generation，init progress/item/finished消息全部携带并校验该generation。
- [x] Finding、Updates和Installed为每个活动manager请求记录request ID；新请求覆盖同一manager的旧ID，不阻断其他manager并行读取。
- [x] reload清空全部页面读取请求与Update All预检状态，同时保持现有`ReloadReason`页面上下文语义。
- [x] Install成功后的重复搜索在reload状态重置后重新发起，并获得新的request ID。
- [x] 复用现有Installed load task，删除App中重复的manager lookup/config lookup/installed调用。
- [x] 增加旧init generation、旧search结果、旧updates结果和旧installed结果的reducer测试。
- [ ] 更新ROADMAP和本记录，并串行通过完整workspace门禁。

## 行为约束

- generation/request ID只判定结果新鲜度，不承诺取消底层命令或网络请求；旧Task允许结束，其消息必须成为无副作用的no-op。
- 不建立通用request manager、service/controller、泛型tracker或跨页面trait；三个页面直接在现有Info状态中维护`manager -> request ID`。
- 同一manager的新请求采用last-request-wins；不同manager仍可并行并分别完成。
- stale结果不得从活动请求map中删除当前ID，否则UI会错误结束spinner并允许重复操作。
- package write progress/result不纳入本轮generation；它们由单一active operation与CancellationToken管理，取消语义属于Iteration 022。
- Config load、Settings save、homepage opener和Activity persistence不纳入本轮；它们没有与本轮相同的可重入读取覆盖路径。
- 保留active page、sort、search text和`ReloadReason::preserves_page_context`既有规则；只清理与旧数据快照绑定的结果、选择、loading和预检状态。

## 验收标准

- reload A的任意init消息在reload B开始后不能修改B的计数、错误、日志、progress或finished状态。
- 同一manager的request A晚于request B返回时，A不能覆盖B，也不能移除B的活动request ID。
- 被取消选择或被reload清除的Finding请求即使返回，也不能重新插入结果或错误。
- Update All预检只由当前request ID的结果推进；stale结果不能生成确认计划。
- Install成功后的重复搜索仍使用上一次已执行query，不会因reload invalidation丢失。
- 完整串行质量门禁通过，无warning。

## 进度日志

### 2026-07-30

- 用户已在宿主桌面验证Iteration 020恢复页面，本轮开始前已将该验收写回对应记录。
- 当前init消息只携带manager/result，任意旧`Init*Finished`都能结束最新loading状态；App随后发起的full Installed load也没有请求身份。
- Finding、Updates和Installed结果只按`ManagerId`路由；重复刷新、取消选择或reload后，旧结果仍能覆盖cache/error并清除新spinner。
- Updates的Update All预检使用独立refreshing集合；配置reload若发生在预检期间，旧集合和晚到结果可能阻塞或提前生成计划。
- 本轮采用App reload generation加页面per-manager request ID，不增加统一调度层。
- App的六类init消息现在先比对当前reload generation；旧item/progress/finished均为无副作用no-op，匹配的Installed finished才会启动full list读取。
- 三个页面的loading状态直接改为`HashMap<ManagerId, u64>`，同一个结构同时表达spinner与当前request ID，没有额外tracker或同步风险。
- reload清空活动request map和Update All预检；Install follow-up改为在reload同步状态重置后发送`RepeatLastSearch`，再登记新ID并执行上次query。
- App初始化完成后的Installed full load复用页面`start_load`入口，删除重复的registry、config和runtime调用实现。
- 新增7项reducer测试，覆盖旧init generation、同manager旧结果、取消选择后的晚到搜索、旧Update All预检结果、reload invalidation与重复搜索新ID。

## Git提交

- `0a69834 docs: record config recovery visual acceptance`
- `c5f9e47 docs: plan request generation iteration`
- `a0a9d38 fix: reject stale package data results`

## 验证记录

- `cargo check -p updater --all-targets --locked --jobs 1`：通过，无warning。
- `cargo test -p updater --locked --jobs 1 -- --test-threads=1`：39项成功、0失败。
- `cargo clippy -p updater --all-targets --locked --jobs 1 -- -D warnings`：通过，无warning。
- 完整workspace串行门禁待执行。

## 遗留项 / 下一轮

- Iteration 022：Discover install与selected Updates冻结计划确认，以及准确的协作取消文案。
- Iteration 023：Linux Wayland/X11、clean/旧配置恢复矩阵和release artifact验收；通过后发布`0.3.0-beta.1`。
- Activity时间戳仍按阶段3补齐，但不阻塞Linux beta。
