# Iteration 019：Settings Executable Validation 与发布检查点

- 日期：2026-07-30
- 状态：已完成
- ROADMAP阶段：阶段3 manager设置闭环与发布准备评估
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

完成Settings中自定义package manager executable path的编辑闭环：保存前必须通过对应manager的真实availability检查，失败时保留draft并显示具体manager错误；用户可以显式恢复使用`$PATH`。同时修正异步保存结果应用当前draft的竞态，并基于现有CI、tag和artifact能力给出可执行的版本发布门槛。

## 实施计划

- [x] 审计Settings draft/baseline、manager availability契约、tag历史和打包流水线。
- [x] 保存前只校验当前平台支持、已注册且配置了自定义executable的manager，复用manager自己的version/platform/settings检查。
- [x] 在对应manager项显示校验失败，并提供更换路径与恢复`$PATH`操作。
- [x] 保存结果携带启动保存时的Config快照，避免异步完成后错误提交较新的未保存draft。
- [x] 保留未注册manager配置；缺失实现不能导致用户修改其他设置时丢失配置或无法保存。
- [x] 补充路径重置、失败保留draft和保存快照的聚焦测试。
- [x] 更新ROADMAP和本记录，冻结Linux preview与完整跨平台正式版的不同门槛。
- [x] 串行通过workspace format、check、test、clippy与build完整门禁。

## 行为约束

- 自定义路径校验调用对应`PackageManager::availability`，不在UI或core复制文件类型、执行权限和version命令规则。
- 只有显式自定义路径参与保存阻断；使用`$PATH`的manager允许保存，运行时availability继续反映宿主环境变化。
- 当前平台不支持或未注册的manager无法执行真实校验，但其配置必须原样保留，不能因当前build缺少实现而阻断保存。
- 校验失败不写磁盘、不更新baseline、不触发package data reload，并将错误关联到具体`ManagerId`。
- 保存成功只应用启动该次保存时的Config快照；保存期间发生的新编辑仍保持dirty。
- 不为单次调用逻辑创建helper；availability状态格式化继续留在唯一展示点，测试直接覆盖reducer状态转换。

## 发布检查点

当前版本为`0.2.4`，tag命名为`Build-v*`。现有package workflow只构建Linux amd64/arm64的`.deb`与`.rpm`，尚无Windows/macOS artifact、checksums或签名流程。

- Linux preview候选：Iteration 020完成Config load可见恢复，021完成关键异步请求的generation/stale-result拒绝，022补齐所有写操作的冻结计划确认和真实取消文案，023只做Linux Wayland/X11、旧配置恢复与tag artifact验收。Iteration 023全部通过后发布`0.3.0-beta.1`。
- 完整正式版：不能早于阶段4和阶段6的跨平台实现与artifact矩阵完成；Windows/macOS尚未实现时不得按ROADMAP中的跨平台产品目标发布为stable。
- 本轮结束时重新按测试、恢复能力、平台矩阵和artifact证据评估候选版本号，不因workflow能够生成Linux包就直接打tag。

## 验收标准

- 无效、不可执行或version check失败的自定义路径不能写入config文件。
- Settings在manager行显示具体失败原因；重置后draft使用`$PATH`且旧错误消失。
- unknown configured manager在编辑其他设置并保存后仍完整保留。
- 保存过程中修改draft时，成功结果只更新已保存快照，较新编辑仍为unsaved状态。
- 完整串行质量门禁通过，无warning。

## 进度日志

### 2026-07-30

- Iteration 018已完成registry execution engine切换，阶段3下一项是Settings executable path完整验证与重置。
- 审计确认11个direct manager统一通过共享command availability检查自定义路径的存在性、普通文件/执行权限和5秒version command；Settings当前保存仅执行`Config::save`，未调用该契约。
- 当前tag为`Build-v0.1.0`至`Build-v0.2.4`；package workflow只上传Linux `.deb/.rpm`，与ROADMAP的完整跨平台发布标准仍有明确差距。
- Settings保存任务现在只收集当前平台支持且显式配置executable的direct manager，串行调用availability；失败结果写回对应manager状态，但不写config文件或推进baseline。
- 所有配置manager行现在均可选择/更换executable；自定义路径可显式恢复为`$PATH`，重置时清除旧availability和保存错误。
- 保存结果携带Config快照；异步期间的新draft不会被吸收到baseline，旧路径availability也不会覆盖已变化的manager draft。
- 新增3项聚焦测试，连同原有Settings测试共7项通过，覆盖路径重置、验证失败和保存快照/stale status。
- 隔离GUI验证尝试在Gamescope headless中启动应用，但wgpu因headless Wayland surface返回`ERROR_SURFACE_LOST_KHR`；未连接宿主桌面，GUI实机检查保留为Iteration 023发布门禁。
- 发布判断冻结为：当前`main`不直接发stable；Iteration 023门禁通过后可发`0.3.0-beta.1` Linux preview，完整stable继续等待ROADMAP阶段4/5/6。
- 完整workspace串行门禁通过：174项测试成功、14项真实环境测试显式ignored、0失败，check/clippy/build均无warning。

## Git提交

- `455759a docs: plan settings executable validation`
- `11ca059 feat: validate manager executable settings`
- `bfdc115 docs: record settings validation progress`

## 验证记录

- `cargo test -p updater content::setting::tests --locked --jobs 1 -- --test-threads=1`：通过，7项成功、0失败。
- `cargo check -p updater --all-targets --locked --jobs 1`：通过。
- `cargo clippy -p updater --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build -p updater --locked --jobs 1`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过，无warning。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：通过，174项成功、14项显式ignored、0失败。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。

## 遗留项 / 下一轮

- Iteration 020：Config load error可见恢复界面，是Linux preview候选前的第一项硬门槛。
- Iteration 021：跨页面request generation与stale-result拒绝。
- Iteration 022：Discover install/selected Updates确认计划与真实取消文案。
- Iteration 023：Linux Wayland/X11、配置恢复和发布artifact验收；通过后发布`0.3.0-beta.1`。
- Activity时间戳不阻塞Linux beta，但仍按阶段3补齐；Windows/macOS与完整stable继续按阶段4/5/6推进。
