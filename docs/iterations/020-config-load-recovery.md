# Iteration 020：Config Load 可见恢复

- 日期：2026-07-30
- 状态：已完成
- ROADMAP阶段：阶段3严格配置加载的用户可恢复边界
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

当严格Config loader拒绝缺失字段、未知顶层字段、重复manager、非法settings或损坏JSON时，应用必须显示可操作的恢复界面，而不是只记录日志并停在默认页面状态。用户可以Retry、打开配置目录手工检查，或在二次确认后重新检测manager并原子覆盖当前配置。

## 实施计划

- [x] 审计App启动状态、Config存储路径、现有桌面URL opener和恢复页面所需主题能力。
- [x] 让`Config`公开当前配置文件路径，并让load/reload/save共用同一解析入口。
- [x] 将App启动配置状态表达为Loading、Failed、ConfirmReset、Resetting和Ready互斥状态。
- [x] Config失败时用恢复页替换正常工作区，显示bounded/wrapped错误详情与Retry、Open Config Folder、Reset Configuration操作。
- [x] Retry继续使用严格loader；打开目录失败必须反馈；Reset必须二次确认，确认前不得写磁盘。
- [x] Reset重新检测当前manager并通过现有原子save覆盖`config.json`；失败继续停留在恢复页，成功才启动package data初始化。
- [x] 补充配置路径和App恢复状态转换测试。
- [x] 更新ROADMAP、配置文档与本记录，并串行通过完整workspace门禁。

## 行为约束

- 不增加旧schema迁移、版本判别、字段容错或自动修复；Retry与正常启动使用完全相同的严格loader。
- load失败时不启动installed/updates初始化，不让默认空Config伪装成成功加载。
- Open Config Folder只创建/打开解析出的配置目录，不修改现有`config.json`。
- Reset是明确的破坏性操作：仅在确认后用当前检测结果和默认应用设置原子替换配置；不隐式删除、重命名或备份原文件。
- Reset取消后返回原始load error；reset/save失败显示新的recovery error并允许继续操作。
- startup recovery只属于App状态机，不引入service/controller层；单次UI布局保持在`App::view`状态分支中。

## 验收标准

- `ConfigLoaded(Err)`进入可见Failed状态，错误文字可换行且正常页面操作不可见。
- Retry进入Loading，并在成功后切换Ready和启动初始化；再次失败返回Failed。
- Open Config Folder结果失败时在恢复页显示，成功不改变load error。
- Reset先进入ConfirmReset；Cancel和Escape不写配置并返回Failed。
- Confirm进入Resetting；成功应用新Config，失败返回Failed且不进入正常工作区。
- 完整串行质量门禁通过，无warning。

## 进度日志

### 2026-07-30

- Iteration 019已完成Settings executable验证，并把Config load恢复冻结为`0.3.0-beta.1`前的第一项硬门槛。
- 当前`App::ConfigLoaded(Err)`只写error log并返回空Task；`pm_config`仍是默认空值，用户没有Retry、配置目录或reset入口。
- `Config::load/read_from_path/save_to_path`已经提供严格schema和原子替换，本轮直接复用，不改存储格式。
- 现有`open_http_url`已实现`gio open`/`xdg-open`顺序和错误传播；本轮只抽取两类桌面目标真正共享的opener边界。
- `ConfigLoadState`直接放在App reducer中表达互斥状态；恢复页直接留在`App::view`，没有新增controller或只调用一次的view helper。
- Retry与首次加载复用同一个`load_config_task`；URL与目录复用同一个`open_desktop_target`，两处抽取都有两个真实调用方。
- Reset确认后调用现有manager检测和原子`Config::save`；reset失败保留原始load error，并单独显示recovery error。
- 完整workspace串行门禁通过：179项测试成功、14项真实环境测试显式ignored、0失败，format/check/clippy/build均通过。
- 发布检查点继续保持：本轮消除Config恢复硬阻塞，但异步stale result、写操作冻结确认和Linux artifact验收仍未完成；最早在Iteration 023通过后发布`0.3.0-beta.1` Linux preview。
- 用户已在宿主桌面完成人工验证，确认恢复页面可正常显示；Iteration 023仍保留Wayland/X11、打开目录和旧配置恢复矩阵的发布级验收。

## Git提交

- `f3fed93 docs: plan config load recovery`
- `7998229 feat: add visible config load recovery`
- `e05bcd9 docs: record config recovery progress`

## 验证记录

- `cargo test -p updater app::tests --locked --jobs 1 -- --test-threads=1`：7 passed。
- `cargo test -p updater_core storage::tests --locked --jobs 1 -- --test-threads=1`：10 passed，其他测试目标按filter为0。
- `cargo clippy -p updater_core -p updater --all-targets --locked --jobs 1 -- -D warnings`：通过，无warning。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过，无warning。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：通过，179项成功、14项显式ignored、0失败。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。
- 用户人工验收：宿主桌面上的恢复页面显示正常。

## 遗留项 / 下一轮

- Iteration 021：为初始化、Finding、Updates和Installed加入request generation并拒绝晚到结果。
- Activity时间戳仍按阶段3补齐，但不阻塞Linux beta。
