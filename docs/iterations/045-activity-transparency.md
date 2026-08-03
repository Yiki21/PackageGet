# Iteration 045：Activity 操作透明度

- 日期：2026-08-03
- 状态：已完成
- ROADMAP阶段：阶段5操作透明度与阶段6发布收口
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

让 Activity Center 能解释一次操作何时开始、何时结束、作用于哪个 scope，以及每个 manager 最终发生了什么。旧版 Activity JSON 没有这些字段时继续可读，并将缺失值明确显示为历史记录不可用或 mixed/unknown，而不是猜测当前值。

## 实施结果

- `OperationOutcome` 增加可序列化的 aggregate `scope` 与按执行顺序排列的 `manager_outcomes`。
- 每个 manager outcome 记录 `ManagerId`、scope、请求/完成数量、成功/失败/取消/未启动状态和脱敏错误摘要。
- 操作开始和完成时记录 RFC3339 UTC 毫秒时间戳；Activity Center 展示时间、scope、聚合结果和 manager 明细。
- 取消或首个失败会为后续未执行的 manager 记录 `not_started`，不再让用户把未尝试目标误认为成功或丢失。
- Activity record 的新字段使用 serde defaults；缺字段的旧 JSON 保留可读，既有 50 条上限、ManagerId failure identity 和错误脱敏规则不变。
- UI 直接依赖 workspace 的 `chrono`，lockfile 保持可复现。

## 验收标准

- 新完成的操作显示非空开始/完成时间、aggregate scope 和每个 manager 的状态与 package 计数。
- 同一操作可区分 succeeded、failed、cancelled 和 not_started；取消仍不设置 aggregate `failed_manager`。
- 旧 Activity JSON 缺失新字段时可加载为空时间、`Unknown` scope 和空 manager outcome，不触发崩溃或静默迁移。
- 详细错误继续执行路径脱敏，历史记录不写入本地绝对路径、token 或 password 内容。
- 通过 workspace 的 format、check、test、clippy 和 build 串行门禁。

## 遗留项 / 下一轮

- 在 Windows、macOS 以及 Linux Wayland/X11 完成真实安装、启动、升级、卸载 smoke，并保存可复核记录。
- 继续保持 1.0 unsigned 政策，但在发布说明和各平台安装文档中明确提示 Gatekeeper/SmartScreen 与 Polkit 限制。
- Activity 的真实截图/人工布局检查仍属于跨平台 RC 验收，不用单元测试替代。
