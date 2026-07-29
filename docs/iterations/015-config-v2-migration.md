# Iteration 015：Config V2 Schema 与 V1 无损迁移

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 3——配置、identity与manager设置迁移
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将磁盘配置从Rust enum驱动的Config V1迁移为带`format_version`的Config V2，以`ManagerId`和公共`ManagerConfig`保存manager identity、executable与typed JSON settings。加载现有`config.json`时必须无损迁移并保留恢复备份；未知第三方manager配置必须原样保留，不能因当前registry缺失而删除。

## 实施计划

- [ ] 审计Config V1 JSON、默认值、load/reload/save调用方、Settings draft/baseline与真实本机配置；只读检查，不输出敏感settings内容。
- [ ] 冻结Config V2 schema：`format_version`、`managers: Vec<ManagerConfig>`、appearance与notifications；明确duplicate ID、未知字段和future version策略。
- [ ] 建立V1/V2 typed deserialize边界，不使用`serde_json::Value`猜测整个文档版本；缺失version只进入V1 parser。
- [ ] 将V1 `system_manager`与`app_managers`映射为稳定built-in `ManagerId`，保留custom executable path。
- [ ] 将V1 `go_bin_dir`迁入`builtin:go`的manager settings；未配置Go时不得凭空创建enabled manager。
- [ ] 保留未知V2 `ManagerId`、executable和settings；当前registry是否存在只影响运行时状态，不影响round-trip。
- [ ] 实现testable path-based load/save API，生产`ProjectDirs`入口只负责解析路径并委托。
- [ ] 实现同目录temporary file、flush/sync与atomic rename；V1首次迁移前创建可恢复backup，写失败不得破坏原文件。
- [ ] reload复用同一typed load/migration路径；future format version和malformed document返回可见typed error，不自动覆盖。
- [ ] 提供Config V1兼容查询/投影边界，使当前UI与静态wrapper继续编译；本轮不迁移页面selection key。
- [ ] 增加V1 fixture、V2 round-trip、unknown manager、Go settings、duplicate、future version、malformed、backup和atomic failure contracts。
- [ ] 更新manager authoring与配置文档，说明settings ownership、未知manager保留和schema兼容策略。
- [ ] 串行通过workspace format、check、test、clippy与build完整门禁，并由GitHub Actions复验。

## Schema与迁移决策

- `format_version`是配置文件协议版本，不是Rust/Python/依赖的最低minor版本约束。
- V2磁盘模型直接复用`updater_manager_api::ManagerConfig`；不再序列化`PackageManagerType` variant。
- manager settings由对应manager拥有schema，core只保留JSON对象并在调用manager时传递；日志和错误不得泄露credential值。
- V1没有`format_version`；只有完整通过V1 typed schema后才能迁移，不能把任意损坏JSON当作旧配置修复。
- future version必须拒绝加载并保留原文件；不向未知schema写回。
- duplicate `ManagerId`默认视为protocol/config错误，避免同一ID的settings与executable选择不确定。
- migration backup与原文件位于同一配置目录，命名固定且不覆盖一个仍可恢复的原始V1 backup。
- appearance与notifications保持现有默认和序列化语义。

## 非目标

- 本轮不把Finding/Updates/Installed/Settings的HashMap、HashSet或selection key迁为`ManagerId`。
- 本轮不删除`PackageManagerType`、静态manager trait、Config V1 UI projection或`core/src/pm/*`wrapper。
- 本轮不实现缺失manager的完整UI状态或配置恢复页面；先提供core typed状态与错误。
- 本轮不改变manager detection优先级、执行顺序、平台catalog策略或任何package manager命令。
- 本轮不执行真实install、update、uninstall或配置目录以外的系统修改。
- 本轮不写死Rust或依赖的最低minor/patch版本。

## 验证方案

- 所有storage测试使用temporary directory和fixture，不读写用户真实`config.json`。
- V1 migration断言ID、path、Go settings、appearance与notifications逐字段保持。
- V2 unknown manager round-trip断言JSON settings和顺序不丢失。
- backup/atomic contracts断言失败前后原文件内容与恢复文件状态。
- UI现有Settings draft/baseline测试继续通过，证明兼容投影未改变交互语义。
- 完整workspace门禁逐条串行，使用单job与单测试线程；GitHub Actions复验最终HEAD。

## 进度日志

### 2026-07-29

- Iteration 014已完成direct catalog cutover，registry不再依赖legacy enum adapter。
- 当前`core/src/storage.rs`直接序列化Config V1，save使用单次`tokio::fs::write`，没有format version、backup或atomic rename。
- 当前UI Settings直接编辑`system_manager`、`app_managers`与`go_bin_dir`，因此本轮需要明确兼容投影，避免把UI identity迁移混入storage改造。
- 初查调用面主要集中在`core/src/storage.rs`、`ui/src/content/setting.rs`和各direct wrapper的Config V1转换。

## Git 提交

- Iteration 015计划检查点：本次提交。

## 验证记录

待实施后填写。

## 遗留项 / 下一轮

本轮完成后填写。
