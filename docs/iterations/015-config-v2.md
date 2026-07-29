# Iteration 015：Config V2 直接切换

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 3——配置、identity与manager设置迁移
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将磁盘配置直接切换为带`format_version`的Config V2，以`ManagerId`和公共`ManagerConfig`保存manager identity、executable与typed JSON settings。Config V1不再读取、迁移或备份；未知第三方manager配置仍原样保留，不能因当前registry缺失而删除。

## 实施计划

- [x] 审计现有Config、默认值、load/save调用方、Settings draft/baseline和direct manager wrapper。
- [x] 冻结Config V2-only schema：`format_version: 2`、`managers: Vec<ManagerConfig>`、appearance与notifications；明确duplicate ID和unsupported version策略。
- [ ] 删除Config V1字段与`PackageManagerConfig`，缺失或不支持的`format_version`直接返回typed config error。
- [ ] detection直接生成稳定built-in `ManagerId`配置，不再建立system/app持久化分组。
- [ ] 将Go bin目录直接保存到`builtin:go`的manager settings，不保留顶层`go_bin_dir`。
- [ ] 保留未知V2 `ManagerId`、executable和settings；当前registry是否存在只影响运行时状态，不影响round-trip。
- [ ] 实现testable path-based load/save API，生产`ProjectDirs`入口只负责解析路径并委托。
- [ ] 实现同目录temporary file、flush/sync与atomic rename；写失败不得破坏原文件。
- [ ] reload复用同一V2 typed load路径；missing version、unsupported version和malformed document返回可见typed error，不自动覆盖。
- [ ] 将Settings和direct wrapper直接改用`ManagerConfig`，不提供Config V1兼容查询或投影层；本轮不迁移页面selection key。
- [ ] 增加V2 round-trip、unknown manager、Go settings、duplicate、missing/unsupported version、malformed和atomic replacement contracts。
- [ ] 更新manager authoring与配置文档，说明settings ownership、未知manager保留和schema兼容策略。
- [ ] 串行通过workspace format、check、test、clippy与build完整门禁，并由GitHub Actions复验。

## Schema与迁移决策

- `format_version`是配置文件协议版本，不是Rust/Python/依赖的最低minor版本约束。
- V2磁盘模型直接复用`updater_manager_api::ManagerConfig`；不再序列化`PackageManagerType` variant。
- manager settings由对应manager拥有schema，core只保留JSON对象并在调用manager时传递；日志和错误不得泄露credential值。
- 缺少`format_version`的文档不再视为可迁移输入，直接拒绝加载；损坏JSON不得被默认配置覆盖。
- 非V2版本必须拒绝加载并保留原文件；不向未知schema写回。
- duplicate `ManagerId`默认视为protocol/config错误，避免同一ID的settings与executable选择不确定。
- appearance与notifications保持现有默认和序列化语义。

## 非目标

- 本轮不把Finding/Updates/Installed/Settings的HashMap、HashSet或selection key迁为`ManagerId`。
- 本轮不删除`PackageManagerType`、静态manager trait或`core/src/pm/*`wrapper；这些仅作为当前运行时调用面，不再参与配置序列化。
- 本轮不实现缺失manager的完整UI状态或配置恢复页面；先提供core typed状态与错误。
- 本轮不改变manager detection优先级、执行顺序、平台catalog策略或任何package manager命令。
- 本轮不执行真实install、update、uninstall或配置目录以外的系统修改。
- 本轮不写死Rust或依赖的最低minor/patch版本。

## 验证方案

- 所有storage测试使用temporary directory和fixture，不读写用户真实`config.json`。
- V2 unknown manager round-trip断言JSON settings和顺序不丢失。
- atomic replacement contracts断言失败前后原文件内容不被截断。
- UI现有Settings draft/baseline测试继续通过，并直接操作V2 manager配置。
- 完整workspace门禁逐条串行，使用单job与单测试线程；GitHub Actions复验最终HEAD。

## 进度日志

### 2026-07-29

- Iteration 014已完成direct catalog cutover，registry不再依赖legacy enum adapter。
- 当前`core/src/storage.rs`直接序列化Config V1，save使用单次`tokio::fs::write`，没有format version、backup或atomic rename。
- 当前UI Settings直接编辑`system_manager`、`app_managers`与`go_bin_dir`，本轮将其改为直接编辑V2 `managers`，不建立中间兼容投影。
- 初查调用面主要集中在`core/src/storage.rs`、`ui/src/content/setting.rs`和各direct wrapper的Config V1转换。
- 用户确认不需要历史遗留设计：本轮改为Config V2-only，不实现V1读取、迁移、恢复备份或兼容投影。
- V2校验边界确定为版本、duplicate manager ID和settings JSON object；未知但合法的manager ID不丢弃。

## Git 提交

- Iteration 015计划检查点：本次提交。

## 验证记录

待实施后填写。

## 遗留项 / 下一轮

本轮完成后填写。
