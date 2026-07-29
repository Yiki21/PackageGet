# Iteration 015：Config 直接切换

- 日期：2026-07-29
- 状态：已完成
- ROADMAP 阶段：阶段 3——配置、identity与manager设置迁移
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将磁盘配置直接切换为以`ManagerId`和公共`ManagerConfig`保存manager identity、executable与typed JSON settings的唯一schema。不保留旧字段、版本判别、迁移、备份或兼容投影；未知第三方manager配置仍原样保留，不能因当前registry缺失而删除。

## 实施计划

- [x] 审计现有Config、默认值、load/save调用方、Settings draft/baseline和direct manager wrapper。
- [x] 冻结唯一Config schema：`managers: Vec<ManagerConfig>`、appearance与notifications；明确duplicate ID、未知顶层字段和malformed document策略。
- [x] 删除`system_manager`、`app_managers`、顶层`go_bin_dir`与`PackageManagerConfig`，不提供旧格式读取或转换。
- [x] detection直接生成稳定built-in `ManagerId`配置，不再建立system/app持久化分组。
- [x] 将Go bin目录直接保存到`builtin:go`的manager settings。
- [x] 保留未知`ManagerId`、executable和settings；当前registry是否存在只影响运行时状态，不影响round-trip。
- [x] 实现testable path-based load/save API，生产`ProjectDirs`入口只负责解析路径并委托。
- [x] 实现同目录temporary file、flush/sync与atomic rename；写失败不得破坏原文件。
- [x] reload复用同一typed load路径；missing required field、unknown top-level field和malformed document返回typed config error且不自动覆盖。
- [x] 将Settings和direct wrapper直接改用`ManagerConfig`，不提供旧Config兼容查询或投影层；本轮不迁移页面selection key。
- [x] 增加round-trip、unknown manager、Go settings、duplicate、missing/unknown field、malformed和atomic replacement contracts。
- [x] 更新manager authoring与配置文档，说明settings ownership、未知manager保留和schema失败策略。
- [x] 串行通过workspace format、check、test、clippy与build完整门禁，并由GitHub Actions复验。

## Schema决策

- 配置只有一个当前schema，不增加版本字段或版本化类型。
- 磁盘模型直接复用`updater_manager_api::ManagerConfig`；不再序列化`PackageManagerType` variant。
- manager settings由对应manager拥有schema，core只保留JSON object并在调用manager时传递；日志和错误不得泄露credential值。
- 缺少必需字段、未知顶层字段或损坏JSON直接拒绝加载，且不得用默认配置覆盖原文件。
- duplicate `ManagerId`视为config错误，避免同一ID的settings与executable选择不确定。
- appearance与notifications保持现有默认和序列化语义。

## 非目标

- 本轮不把Finding/Updates/Installed的HashMap、HashSet或selection key迁为`ManagerId`。
- 本轮不删除`PackageManagerType`、静态manager trait或`core/src/pm/*`wrapper；这些仅作为当前运行时调用面，不再参与配置序列化。
- 本轮不实现缺失manager的完整UI状态或配置恢复页面；先提供core typed状态与错误。
- 本轮不改变manager detection优先级、执行顺序、平台catalog策略或任何package manager命令。
- 本轮不执行真实install、update、uninstall或配置目录以外的系统修改。
- 本轮不写死Rust或依赖的最低minor/patch版本。

## 验证方案

- 所有storage测试使用temporary directory和fixture，不读写用户真实`config.json`。
- unknown manager round-trip断言JSON settings和顺序不丢失。
- atomic replacement contracts断言失败前后原文件内容不被截断。
- UI现有Settings draft/baseline测试继续通过，并直接操作`ManagerConfig`。
- 完整workspace门禁逐条串行，使用单job与单测试线程；GitHub Actions复验最终HEAD。

## 进度日志

### 2026-07-29

- Iteration 014已完成direct catalog cutover，registry不再依赖legacy enum adapter。
- 初查确认旧storage直接序列化`system_manager`、`app_managers`与`go_bin_dir`，save使用单次`tokio::fs::write`。
- 用户确认不需要历史遗留设计或版本化命名；本轮采用唯一Config schema。
- 校验边界确定为required/unknown fields、duplicate manager ID和settings JSON object；未知但合法的manager ID不丢弃。
- `core/src/storage.rs`已切换为`managers: Vec<ManagerConfig>`，并提供可测试的path API及原子替换。
- Settings draft/baseline、内置manager executable和Go settings已直接接入新Config；磁盘不再序列化`PackageManagerType`。
- 初次定向验证通过：`updater_core` 53项单元测试、`updater` 19项binary单元测试全部通过；删除版本字段后将重新执行。
- 本机`~/.config/updater/config.json`已按当前schema手工转换并通过结构校验；人工回退副本为`config.json.manual-backup-20260729`，两者权限均为`0600`。
- 完整本地门禁已通过。
- GitHub Actions run `30459148407`已在提交`7d522a6`上通过全部CI门禁。

## Git 提交

- Iteration 015初始计划检查点：`eacc84c`。
- Config直接切换实现：`e3888e4`。
- 现行Config测试命名与Clippy清理：`62be3e8`。
- 首次完整推送检查点：`7d522a6`。

## 验证记录

- `cargo check --workspace --all-targets --jobs 1`：删除版本字段后复验通过。
- `cargo test -p updater_core --lib --jobs 1 -- --test-threads=1`：删除版本字段后复验53 passed。
- `cargo test -p updater --bin updater --jobs 1 -- --test-threads=1`：删除版本字段后复验19 passed。
- `cargo fmt --all -- --check`：通过。
- `cargo test --workspace --all-targets --jobs 1 -- --test-threads=1`：通过；宿主与网络smoke按设计保持ignored。
- `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --jobs 1`：通过。
- GitHub Actions `30459148407`（format、check、test、clippy、build）：通过。

## 遗留项 / 下一轮

- 下一轮将Finding、Updates、Installed与Settings的页面state/selection key从`PackageManagerType`迁为`ManagerId`。
- manager名称、说明、category、platform与capability改由registry/catalog descriptor解析，并移除硬编码fallback display manager。
- 缺失第三方manager的完整Settings状态与Config load恢复界面继续按ROADMAP阶段3拆分实施。
