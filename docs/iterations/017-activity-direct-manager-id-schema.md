# Iteration 017：Activity Direct ManagerId Schema

- 日期：2026-07-30
- 状态：进行中
- ROADMAP阶段：阶段3——配置、UI identity与manager设置迁移
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

删除Activity history的版本化兼容层，只保留一个当前schema。failure identity直接持久化为`ManagerId`，不保存legacy display name，不读取或迁移旧`activity-v1.json`。

## 实施计划

- [x] 将`ActivityRecord.failed_manager_id`与legacy `failed_manager: Option<String>`合并为`failed_manager: Option<ManagerId>`。
- [x] 删除`ActivityRecord.version`、version过滤和v1兼容测试，当前schema拒绝多余旧字段。
- [x] 将当前history文件改为`activity.json`，不迁移、不读取、不删除旧`activity-v1.json`。
- [x] 更新Activity summary、构造器与测试，验证当前schema round-trip和ManagerId validation。
- [x] 更新ROADMAP、manager identity文档与Iteration 016遗留表述，删除v1/v2兼容承诺。
- [ ] 串行通过workspace format、check、test、clippy与build完整门禁，并由GitHub Actions复验。

## Schema决策

- Activity只有一个当前schema，不包含`version`、`format_version`或V2命名。
- manager identity只保存validated `ManagerId`；display name只在渲染时通过catalog解析。
- 旧history文件不属于当前schema输入，不提供自动迁移或fallback读取。
- 加载损坏或非当前schema的`activity.json`时沿用现有安全行为：返回空history，不重写输入文件。

## 非目标

- 本轮不迁移或删除用户目录中的旧Activity文件。
- 本轮不改变Activity retention、error redaction、notification或operation outcome语义。
- 本轮不加入时间戳；时间戳仍作为后续独立字段设计。
- 本轮不改package manager执行引擎。

## 进度日志

### 2026-07-30

- 用户明确不需要Activity旧版本兼容，version 1/2双读和legacy display-name字段应删除。
- Iteration 016已完成UI `ManagerId` identity cutover，本轮仅收紧Activity磁盘schema。
- `ActivityRecord`已删除`version`与字符串failure fallback，当前failure字段直接是`Option<ManagerId>`。
- 当前schema启用`deny_unknown_fields`，带version字段的旧结构会被拒绝；加载失败沿用空history安全行为。
- history路径已切换为`activity.json`，旧`activity-v1.json`不会被读取、迁移或删除。
- UI package check与26项单元测试通过，完整workspace门禁等待文档检查点后执行。

## Git提交

- `4606964 docs: plan direct activity schema`
- `807dc63 refactor(activity): use direct manager identity schema`

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo check -p updater --locked --jobs 1`：通过，无warning。
- `cargo test -p updater --locked --jobs 1 -- --test-threads=1`：通过，26项测试全部成功。
- 完整workspace check/test/clippy/build与GitHub Actions：待最终验证。

## 遗留项 / 下一轮

- Activity时间戳仍未加入；应与后续Activity排序/展示需求一起设计。
- 旧`activity-v1.json`可能仍留在用户数据目录，但当前应用不会读取或修改它。
