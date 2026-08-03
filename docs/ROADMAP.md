## 当前 updater 已具备较完整的 Linux/Wayland 多包管理器工作流，但下一轮迭代不能只做局部 UI 润色：

- 界面在 Dark/High Contrast、窄窗口和键盘可见性方面仍有明显完善空间；安装/更新确认、配置恢复、异步结果新鲜度和取消语义也需要继续增强。
- 工作区目前只有 ui、core 两个 crate，core/src/lib.rs 同时承担领域模型、配置、硬编码枚举分发和所有内置 PackageManager，现有 PackageManager trait
  是静态关联函数集合，不能作为 dyn PackageManager 注册，也无法让外部 crate 真正接入。
- GUI 在 ui/src/main.rs 强制检查 Wayland，ui/Cargo.toml 仅启用 Iced Wayland feature；系统管理器、pkexec、打包和 CI 都是 Linux 专用。
- Cargo manifest 里的普通版本字符串是 caret 兼容范围，并非 = 精确锁死；真正固定可复现依赖图的是 Cargo.lock。manifest 对 1.x 以上 crate
  使用主版本兼容线，对 0.x crate 保留兼容 minor 线，不写死类似 3.27.0 的最低 patch/minor；同时保留并提交 lockfile，而不是删除锁文件或使用不受控的通配版本。

目标是分阶段完成：先稳定依赖与扩展边界，再迁移内置管理器和平台层，随后交付 X11、Windows、macOS 以及统一的视觉/功能改进。每个阶段都保持可编译、可回滚，避免同时重写 UI、执行引擎和所有管理器。

已确认的产品决策

- 平台支持分阶段交付：首轮保留 Linux Wayland，新增 Linux X11、Windows、macOS GUI；Windows 首批接入 Winget，macOS 首批迁移并验证现有
  Homebrew。Chocolatey、Scoop 与 macOS MAS 后续通过同一接口添加；softwareupdate 待 manager-level transaction、pending state 与 reboot-required 模型落地后接入。
- PackageManager 首轮采用编译时扩展：第三方 crate 依赖公共 API、实现 trait 并显式注册；不直接加载 Rust 动态库，也不在本轮承诺免重编译的运行时插件。
- 依赖按风险分组升级到实施时最新稳定版：Iced/窗口后端单独迁移，其他依赖分组更新；继续提交 Cargo.lock 保证发布可复现。
- 1.0采用明确的未签名发布政策：Linux .deb/.rpm/Arch `.pkg.tar.zst`、Windows 便携包与安装包、macOS .app/.dmg均允许unsigned；签名、Apple signing/notarization及证书secrets不作为1.0门禁。所有发布页面必须标注限制，并要求用同一release的`SHA256SUMS`校验下载；严格资产清单与公开CI记录提供构建provenance。
- 所有 PackageManager 写操作仍按 manager group 串行执行，不并发执行安装、更新或卸载。

## 推荐实施方案

### 阶段 1：建立可复现的现代依赖与跨平台构建基线

1.  在根 Cargo.toml 集中全部直接依赖及版本，子 crate 改用 workspace = true；消除 core/Cargo.toml 中重复的 Tokio/Serde 等独立版本声明。
2.  制定依赖策略：

- manifest 对 1.x 以上 crate 使用主版本线、对 0.x crate 使用兼容 minor 线，并采用 Cargo 默认 caret 语义；不使用 \*，非必要不使用 =x.y.z；
- 依次更新基础库、网络/序列化库、平台集成库，最后单独升级 Iced 及其窗口/渲染依赖；
- 每组更新后审阅 Cargo.lock、重复依赖、MSRV/API 变化并串行验证；
- 为 Cargo 和 GitHub Actions 增加 Dependabot 分组更新，防止版本再次长期滞后。

3.  将工具链迁移到验证过的 stable Rust：删除 .cargo/config.toml 的全局 -Zthreads 和 Linux 目标中的 -Zshare-generics；保留仅作用于 Linux GNU 目标的 clang/mold
    设置。rust-toolchain.toml 与 CI 使用同一 stable channel，不写死 patch 版本；本地工具链包含 rustfmt、clippy、rust-analyzer。
4.  单独迁移最新稳定 Iced：重点适配 ui/src/shortcut.rs 的自定义 Widget 实现、ui/src/main.rs 的 application/window API、ui/src/app.rs 的 Task/订阅 API以及
    ui/src/theme.rs 的样式 API。
5.  新增独立质量 CI（format、check、确定性单元测试、clippy），把需要真实网络或本机包管理器的测试标为显式 integration/ignored；RPM使用`cargo-generate-rpm`
    要求的workspace成员目录`ui`，不能把Cargo package名`updater`误作路径。

关键文件：Cargo.toml、Cargo.lock、core/Cargo.toml、ui/Cargo.toml、rust-toolchain.toml、.cargo/config.toml、.github/workflows/package.yml、ui/src/shortcut.rs

### 阶段 2：拆分 crate，并建立真正可扩展的 PackageManager API

工作区调整为四个职责清晰、不过度碎片化的 crate：

- manager-api（package updater-manager-api）：无 Iced、无具体命令实现的公共扩展契约。
- core（updater_core）：registry、配置/迁移、检测策略、操作计划和串行执行引擎。
- managers（package updater-managers）：所有内置 APT/DNF/Pacman/Zypper/Flatpak/Homebrew/Cargo/Go/npm/pnpm/pipx/Winget 实现及共享命令工具。
- ui（updater）：Iced 展示、桌面集成和最终发行包。

公共接口

1.  在 updater-manager-api 定义：

- 可序列化、带命名空间的 ManagerId（如 builtin:apt、org.example:manager）；
- ManagerDescriptor、平台集合、manager category、capabilities、授权提示和稳定显示元数据；
- ManagerConfig（ID、自定义 executable path、manager 私有 serde_json::Value settings）；
- PackageInfo、PackageUpdate、PackageTarget、scope/origin 等跨管理器通用字段；
- PackageAction、ProgressEvent、ManagerAvailability、结构化 ManagerError；
- 对象安全的 PackageManager: Send + Sync 和非 Iced 的 ProgressSink。

2.  trait 全部使用实例方法；异步动态分发继续使用 async-trait。核心方法覆盖 descriptor、availability、installed/count、updates、search 和统一 execute(action,
    packages, progress)。不在 public trait 中保留静态必需方法、泛型回调或关联异步类型，以确保可存入 Arc<dyn PackageManager>。
3.  PackageManager 每次收到一个 manager 的完整 package group，由实现决定单命令批处理还是内部逐包串行；updater_core 仍严格按 manager group 顺序执行，并保留
    stop-on-first-failure与partial success。Iteration 044起，公共progress契约提供向后兼容的取消查询；内置manager在运行命令期间响应取消并等待底层进程树退出，第三方manager未接入时仍在下一组前响应。
4.  在 updater_core 实现确定性 ManagerRegistry：显式注册 Arc<dyn PackageManager>、拒绝重复/非法 ID、按 descriptor 稳定排序、按 capability
    在调用前拒绝不支持的操作。
5.  updater-managers::builtin_managers 提供对象安全的内置实现catalog，updater_core::register_builtin_managers负责注册与duplicate检查；后续使用
    cfg(target_os)只编译/提供适用平台的manager。引擎不再硬编码pkexec，提权由具体manager自己负责。
6.  提供 docs/manager-authoring.md 和一个独立 fake/sample manager 集成测试，演示第三方 crate 如何声明 ID、capability、实现 trait、注册并被 engine
    调用。首轮不提供 Iced 专用“插件设置页面”接口。

迁移结果（Iteration 018）

1.  所有built-in实现、共享命令工具和contract test均已迁入`updater-managers`，core不再包含具体manager适配器。
2.  `updater_core::execute_package_groups`负责跨manager串行、首次失败停止、部分结果和组间协作取消；Iced channel与message映射留在UI。
3.  UI读取、搜索、刷新、检测和写操作均通过同一个`ManagerRegistry`解析`ManagerId`并检查capability。
4.  `PackageManagerType`、`define_package_managers!`、宏生成的match dispatcher、旧静态`PackageManager` trait和`core/src/pm/`已删除。

关键文件：`manager-api/src/lib.rs`、`managers/src/`、`core/src/registry.rs`、`core/src/execution.rs`、`ui/src/content/workflows.rs`、
`docs/manager-authoring.md`。

### 阶段 3：迁移配置、UI identity 和 manager 设置

1.  直接替换Config schema，将manager identity从序列化Rust enum改为ManagerId；磁盘只保留
    managers: Vec<ManagerConfig> 和对应 manager settings，不保留 system_manager、app_managers、go_bin_dir 等旧字段或兼容投影。
2.  Config loader拒绝缺少必需字段、未知顶层字段、重复manager ID或损坏的config.json，不增加版本判别或迁移路径；保存使用同目录临时文件和原子替换。未注册的第三方manager
    配置不删除，而是保留为 disabled/missing 状态。
3.  将 UI 中用 PackageManagerType 作为 HashMap/HashSet/selection key 的地方逐页迁为 ManagerId，名称、说明、平台和 capability 全部通过 registry/catalog
    解析；移除 Finding/Updates/Installed 中 DNF 作为 fallback display manager 的假设。
4.  Settings 继续保留现有 draft/baseline 隔离和 dirty-navigation prompt，但改为 registry 驱动：

- 显示当前平台可用、不可用和缺失的 manager；
- 支持编辑/验证/重置已配置 executable path；
- 支持多个配置 manager，不再把“system/app”类别当成执行策略；
- manager 特有 settings 先由 built-in ID 对应的受控编辑器处理，公共 API 不依赖 Iced。

5.  为 Config load error 增加可见启动恢复界面，提供 Retry、打开配置目录、经确认后重新检测/重置配置；不再只在 ui/src/app.rs::ConfigLoaded 里写日志后停住。
6.  Activity history直接记录ManagerId与后续时间戳，不保留版本字段或旧display-name兼容路径，并保留现有上限和隐私脱敏。

当前进度（Iteration 024）：

- UI page state、message payload、selection key、progress、operation outcome和Activity failure identity已切换为`ManagerId`；Finding、Installed、Updates中的DNF display fallback已删除。
- UI catalog持有共享的direct built-in registry，descriptor作为名称、说明、category、platform与capability metadata来源；读取、搜索、刷新、检测和写操作均通过该registry执行。unknown configured manager显示稳定ID并在Settings draft/save/reload中保留。
- Settings已支持配置manager的executable选择、更换和恢复`$PATH`；保存前复用对应direct manager的availability契约校验当前平台上的自定义路径，失败不写盘、不更新baseline。异步保存结果只应用启动时的Config快照，较新draft继续保持dirty。
- Activity使用无版本字段的单一当前schema，failure直接保存`ManagerId`；旧history不读取或迁移，时间戳仍未加入。
- Config严格加载失败会进入独立恢复状态，提供Retry、打开配置目录和经确认后的原子reset；失败前后都不会让默认空Config进入正常工作区，也不增加旧schema迁移或自动修复。
- 初始化Task使用App reload generation；Finding、Updates和Installed使用per-manager request ID。reload或同一manager新请求开始后，晚到结果不会覆盖cache/error、清除当前spinner或推进旧Update All预检。
- Discover install与selected Updates现在先冻结按manager稳定排序的package计划，再展示名称、数量和descriptor提权提示并确认执行；确认后不重新读取live selection或cache。Update All沿用同一pending update状态，既有刷新和失败source排除不变。
- active operation停止入口明确为当前manager完成后停止；请求后status保持active并说明后续manager不会启动。底层仍只在manager group之间检查取消，不伪装成已终止当前系统事务。
- `PackageManagerType`、宏dispatcher、旧静态trait和core legacy manager适配器已删除；Activity时间戳仍是后续迭代。

发布检查点（Iteration 024完成后）：

1. 当前`main`仍不能直接作为stable发布：`Build-v0.2.4`之后的四crate拆分、Config/Activity schema直接切换和执行引擎替换规模较大，且阶段4至阶段6的跨平台与完整artifact目标尚未完成。
2. Linux beta前的功能可靠性缺口已收敛：Iteration 020完成Config load可见恢复，Iteration 021拒绝晚到读取结果，Iteration 022补齐Discover install与selected Updates冻结确认并准确表达manager边界取消语义。
3. Iteration 023先交付品牌化Polkit授权链：APT、DNF、Pacman与Zypper通过固定最小特权helper执行写操作，Linux包携带按动作区分的Updater policy metadata；密码和认证UI仍完全属于桌面Polkit agent。
4. Iteration 024已完成Linux release hardening：Wayland/X11、clean/旧配置恢复、本地与GitHub Actions原生amd64/arm64五包矩阵、checksums和完整质量门禁均已通过；`Build-v0.3.0-beta.1`已发布为unsigned Linux prerelease，`Build-v0.2.4`继续保持Latest stable。
5. 完整stable仍以阶段4、阶段5和阶段6的跨平台功能、可靠性、artifact与文档标准为准；Windows/macOS产物不存在时，不把Linux preview描述成ROADMAP目标已完成。

复用：core/src/storage.rs 的 ProjectDirs 路径；ui/src/content/setting.rs 的
draft/baseline、sync_from_config、is_dirty；ui/src/content.rs::ReloadReason::preserves_page_context；ui/src/activity.rs 的 retention/redaction。

### 阶段 4：交付 Linux X11、Windows 和 macOS 首轮原生支持

GUI/桌面层

1.  删除 ui/src/main.rs 的强制 Wayland 环境变量退出逻辑；Linux 同时启用 Iced Wayland 和 X11 backend，Windows/macOS 使用各自 native backend。Linux 专属
    features 放入 target-specific dependency 配置，不能泄漏到非 Linux 构建。
2.  使用稳定 reverse-DNS application ID com.ayi.updater，并逐平台验证 system theme、字体、窗口最小尺寸、剪贴板、文件/目录选择器、URL 打开和 native
    notification；不可用能力用 capability/target gate 明确降级，而不是让应用启动失败。
3.  扩展 executable discovery：Windows 使用正常 PATH/PATHEXT 语义并覆盖 Winget/Scoop/Chocolatey/npm 等标准位置；macOS 覆盖 Apple Silicon 与 Intel Homebrew
    路径；保留 Linux 现有规则。

首批 manager

1.  Linux 按 target 注册 APT/DNF/Pacman/Zypper/Flatpak 等现有实现；具体系统manager通过Updater自有Polkit action和固定最小特权helper使用pkexec，不将可配置executable路径直接提升为root。
2.  Windows 新增 Winget manager，覆盖
    availability、installed、updates、search、install/update/uninstall，优先使用官方机器可读输出；所有表格/JSON解析均使用离线 fixture
    测试，显式处理非交互、source agreement、installer/reboot/elevation 结果。
3.  macOS 迁移并原生验证现有 Homebrew manager（Apple Silicon/Intel）；本阶段不把 Homebrew 描述成 macOS 系统更新，也不承诺 softwareupdate/MAS。
4.  Cargo、Go、npm、pnpm、pipx 等便携 manager 按实际平台能力注册；Flatpak 等不适用 manager 不出现在目标平台默认检测结果中。
5.  扩展错误分类为跨平台 typed error：command missing、permission/elevation、network、lock/busy、timeout、installer reboot required、parse/protocol
    failure；UI 通过通用 error kind 给出平台相关建议。

关键文件：ui/src/main.rs、ui/Cargo.toml、managers/src/\*\*、原 core/src/pm/common.rs/error.rs、registry builtin registration。

当前进度（Iteration 025已完成）：

- 公共`Platform::current()`已统一目标解析；built-in catalog和registry按descriptor声明的平台过滤，不再在Windows/macOS runtime无条件注册Linux manager。
- Linux保持现有11个manager；macOS首批注册Homebrew与Cargo、Go、npm、pnpm、pipx；Windows在Winget实现前保持空catalog，不用空壳能力伪装支持。
- Linux `com.ayi.updater`窗口identity改为target-gated设置，Windows GNU目标的workspace库与二进制check已通过；macOS由原生arm64 GitHub runner验证，Linux不伪装Apple SDK交叉编译结果。
- CI保留Linux完整质量门禁，并增加Windows x86_64与macOS arm64原生workspace compile job；run `30686922393`三项job均已通过。下一轮进入Winget独立迭代。

### 阶段 5：统一视觉与核心用户工作流

该阶段放在 Iced 和 manager identity 迁移之后，避免同一批 UI 文件重复重构。

美观、响应式与可访问性

1.  在 ui/src/theme.rs 增加 typed PageAccent 及 Light/Dark/High Contrast 下解析后的 foreground/soft tokens；SharedUi::page_header、summary、manager
    section、sidebar 全部消费语义 token，不再把 Light palette 原色直接用于深色文本。
2.  让 sidebar 当前项真正使用页面 identity accent，同时保留背景/字重等非颜色选中线索；统一首页命名为 Discover。
3.  将现有 LayoutMode 作为 presentation signal 传入 Content::view：

- Wide 保留当前横向布局；
- Medium 使用明确的两行 toolbar；
- Narrow 将 search、source、sort、selection actions 顺序堆叠，任何功能都不因窄屏消失。

4.  Narrow sidebar 使用 Iced stack/opaque backdrop 做真正 overlay sheet，不再与最小 640px 内容并排争抢 200px；支持 backdrop、Close、Escape
    和成功导航关闭，并保留 Settings dirty prompt 语义。
5.  在 setting.rs 的共享 icon_button 内按 semantic.on_primary/disabled foreground 统一 tint currentColor SVG；用细 outline/divider 加强 toolbar、group
    和长列表层级，不引入重阴影和卡片堆叠。
6.  Activity Center 改为 overlay/drawer，窄高度下压缩常驻快捷键提示，让 package list 和 status panel 不互相挤压。
7.  升级 Iced 后检查正式 focus/accessibility API：用 toolkit 支持的 focus/semantic API实现键盘焦点和 accessible labels，不用 hover 状态伪造 focus ring。

功能、透明度与可靠性

1.  为 Discover install 和 selected Updates 复用 updates.rs::UpdatePlan、remove confirmation 和 collect_selected_package_groups 模式：冻结 manager/package
    scope，展示需要提权的 manager、package 名称/数量后再确认执行。
2.  给 Finding search、Updates/Installed refresh 和初始化任务加入 generation/request ID，丢弃晚到的旧结果；保持 ReloadReason 的页面上下文规则。
3.  失败计划区分 completed/failed/unattempted manager groups，提供重新扫描后仅重试失败/未执行部分的入口，不盲目重放过期 selection。
4.  在 package model/inspector/确认页展示 manager scope、origin 与安装目标；Go/pipx 的 Discover 明确标注 exact identifier lookup，避免伪装成目录搜索。
5.  Activity record 增加时间、请求 scope、per-manager outcome 和有界脱敏诊断摘要；状态面板继续保留 partial success 和 bounded log。
6.  最后单独处理取消语义：重构 core 命令执行为可追踪 child/process-group lifecycle，状态区分 cancellation requested、terminating、cancelled after
    exit、completed。只有底层进程确认结束后才释放 active operation 和写入 Cancelled；绝不因 UI task abort 就声称系统事务已停止。

关键文件：ui/src/theme.rs、ui/src/content/shared.rs、ui/src/sidebar.rs、ui/src/app.rs、ui/src/content/{finding,updates,installed,setting,workflows}.rs、ui/s
rc/status_panel.rs、ui/src/activity.rs、manager API package model、command executor。

### 阶段 6：跨平台发布物、文档和后续 manager

1.  保留并验证 Linux amd64/arm64 .deb/.rpm与Arch x86_64 `.pkg.tar.zst`，同时打包Polkit policy与固定特权helper并验证X11/Wayland desktop entry；修正 RPM package 选择器。
2.  通过通用 Rust 二进制打包工具配置 Windows x86_64 便携 .zip 与安装包，增加 .ico、版本信息和 application identity。
3.  生成 macOS Apple Silicon/Intel .app 和 .dmg，增加 .icns、Info.plist、bundle ID、最低系统版本说明；首轮 artifact 和 README 明确标注
    unsigned/Gatekeeper/SmartScreen 限制。
4.  .github/workflows/package.yml 增加 Windows/macOS runner、架构化 artifact 名称、checksums 和 release 汇总；签名/notarization job
    预留清晰输入，但在没有证书/secrets 时不伪装成已签名发布。
5.  更新 README 和 manager-authoring 文档，按平台列出实际支持的 manager、提权模型、安装方式和限制。
6.  后续独立增量通过同一 registry 接入 Chocolatey、Scoop 与 macOS MAS；softwareupdate 在阶段 7 要求的 manager-level transaction、pending state 与 reboot-required 模型落地后接入。若将来需要免重编译插件，另建带版本协商、权限、超时和隔离的子进程 JSON/Wasm 协议，不直接加载不稳定 Rust ABI 动态库。
7.  扩展Linux分发格式：提供x86_64/aarch64 glibc portable tar与AppImage；musl只作为经过Alpine原生验证的runtime-specific tar，不宣传为全静态通用GUI。Flatpak分发不进入1.0范围；若未来重新立项，仍必须先解决宿主CLI桥接或受限capability profile，不能让沙箱静默移除核心manager能力。

当前进度（Iteration 042已完成）：

- 保留Linux amd64/arm64 DEB/RPM与Arch x86_64五包产线，新增Windows x86_64 portable ZIP和per-user Inno Setup installer。
- 新增macOS arm64与Intel原生runner产线；每个架构从同一bundle ID、版本、Mach-O与ICNS验证后的`.app`生成`.app.zip`和DMG。
- Windows嵌入ICO与VERSIONINFO；macOS bundle包含`Info.plist`、`com.ayi.updater` identity、最低系统版本和应用资源。
- bundle job严格汇总5个Linux、2个Windows、4个macOS产物，为全部11项生成同一`SHA256SUMS`；Package run `30782732075`的所有原生构建、artifact结构验证和bundle校验均已通过。
- Arch Cargo cache已绕开`makepkg --cleanbuild`的源码目录清理边界，exact-key命中后job由20分30秒降到8分50秒；release build与完整release tests仍保留。
- 当前公开`0.3.0-beta.3`仍是Linux-only prerelease；跨平台产物将在下一beta或RC tag首次公开。Windows/macOS产物仍是unsigned preview，不描述为已签名或已notarize。

当前进度（Iteration 043已完成）：

- portable格式的能力边界已冻结并写入用户文档：tar/AppImage不携带已安装的Polkit系统集成，系统manager特权写操作仍推荐使用DEB/RPM/Arch原生包。
- 已交付x86_64/aarch64 glibc tar、Alpine/musl runtime tar与AppImage共6项；所有musl产物均通过Alpine原生构建、loader和动态依赖验证。
- bundle严格汇总既有11项与新增6项，为17个发布资产生成统一`SHA256SUMS`；Package run `30785115485`全部成功，两个musl Cargo cache也已成功保存。
- Flatpak分发已明确移出1.0范围，避免为一种发布格式引入host-spawn权限、独立capability profile和新的安全架构；现有Flatpak manager本身不受影响。

当前进度（Iteration 044已完成）：

- 取消已从manager group边界扩展到内置manager正在运行的命令；Unix process group与Windows process tree均等待底层进程退出后才报告`cancelled`。
- UI区分cancellation requested、terminating和最终结果；取消不再伪装成普通failure，1.0 unsigned发布政策、SHA256SUMS校验和CI provenance已写入发布文档。

当前进度（Iteration 045已完成）：

- Activity record现在记录RFC3339 UTC毫秒开始/完成时间、aggregate scope和有序per-manager outcome；成功、失败、取消与未启动manager均可区分。
- 旧Activity JSON通过serde defaults继续读取，新字段缺失时显示历史时间不可用及mixed/unknown scope；既有有界保留、ManagerId identity和redaction规则保持不变。
- 1.0剩余发布门禁收敛为真实平台smoke证据：Linux Wayland/X11、Windows和macOS的安装、启动、升级、卸载，以及各平台unsigned限制提示。

当前进度（Iteration 046已完成）：

- 用户已完成发布资产的真实安装验证，确认可以进入1.0发布；签名和notarization按既定政策不作为门禁。
- workspace、DEB/RPM、Arch、README、release notes和metadata preflight统一升为`1.0.0`；tag Release将公开17个资产及同一`SHA256SUMS`。

当前进度（Iteration 047已完成）：

- Finding、Updates和Installed已改用共享的可折叠source picker；已选Logo预览、分类、搜索、availability、加载状态和包数量在三页保持一致。
- Settings以同一Logo和分类呈现已配置及可添加的manager，统一搜索名称、ID和描述；未知第三方manager使用稳定initials fallback。
- 筛选后的`Select shown`和`Clear shown`只修改当前可见manager，隐藏manager的选择、结果和inspector状态保持不变。
- Simple Icons来源、CC0许可和商标边界进入第三方notice；DEB、RPM、Arch、portable tar、AppImage、Windows和macOS包均携带该文件。
- CI run `30805679742`已通过Linux、Windows和macOS质量矩阵；Package run `30805679816`已通过17项跨平台产物构建、notice校验和统一checksums bundle。

### 阶段 7：扩展 Package Manager 生态

本阶段在跨平台构建、发布和真实 CLI 验证基线稳定后实施。新增 manager 只广告已经实现并通过 fixture、命令构造和真实只读 smoke test 验证的 capability；不能为了统一界面伪造 manager 原本不存在的搜索、更新或卸载语义。

1.  第一优先级新增 `uv tool`，首批覆盖 installed、updates、install、update 和 uninstall，使用 `uv tool list --outdated` 获取更新；只有在搜索能够尊重用户配置的 Python index 时才广告 search，不硬编码 PyPI 结果覆盖私有源语义。实现优先复用 pipx 已验证的包模型和测试边界，但不共享只使用一次的解析包装。
2.  新增 `.NET global tools`，首批只管理 current-user global scope，使用 `dotnet tool list --global --format json` 读取已安装工具，并通过 NuGet 版本元数据发现更新，不能用真实 update 命令充当只读探测；同时覆盖 search、install、update 和 uninstall。local tool manifest 与任意 `--tool-path` 暂不混入同一列表；availability 必须实际执行 CLI，正确处理 asdf 等 shim 存在但当前未选择 SDK 的情况。
3.  新增 Linux `Snap` application manager，覆盖 installed、updates、search、install、update 和 uninstall，并保留 channel、confinement 与自动刷新状态。所有需要授权的写操作必须通过 snapd 的明确授权路径或扩展后的固定 Polkit helper 执行，不能把用户配置的可执行路径直接提升为 root。
4.  随后新增 RubyGems 与 Composer Global。RubyGems 必须区分 user/system `GEM_HOME` 与多版本安装；Composer 只管理 `$COMPOSER_HOME` 中的直接全局依赖并优先解析结构化输出，不能把项目依赖或传递依赖伪装成独立全局工具。
5.  在上述 manager 稳定后评估 `Nix profile`。首批只支持明确配置的单一用户 profile，并保留 flake/source identity；多 profile 支持必须先引入独立 manager instance identity，不能通过重复 `ManagerId` 绕过现有 Config 唯一性约束。
6.  较低优先级候选包括 Bun Global、Krew、LuaRocks，以及面向新增发行版构建目标的 APK、XBPS、Portage、eopkg 和 swupd。`paru`/`yay` 只能作为 Pacman/AUR 的显式替代后端评估，不能与 Pacman 默认同时展示重复包；安装流程必须保留 PKGBUILD 审阅和交互安全边界。
7.  以下工具在领域模型扩展前不作为普通 PackageManager 接入：Conda/Mamba/Micromamba 需要 environment identity 与多实例配置；asdf/mise/rustup 需要独立 runtime/toolchain manager 模型；rpm-ostree 与 macOS softwareupdate 需要 manager-level transaction、pending deployment 和 reboot-required 状态。AppImage 在没有统一可信的搜索、安装和更新协议前不列为 manager。

验证方案

自动化测试

- updater-manager-api：外部 fake manager 的对象安全实现、capability、progress、typed errors。
- updater_core：registry重复ID/稳定排序、Config schema校验、未知ID保留、原子替换、串行manager ordering、stop-on-failure、partial success、在下一组前取消。
- updater-managers：每个 manager 的离线 parser fixture、命令构造、平台注册；真实网络/本机 package manager 测试保持 opt-in。
- updater：breakpoint/layout、dirty Settings、stale request rejection、确认计划冻结、Activity retention/redaction、status outcome、shortcut capture。
- CI target matrix 至少覆盖 Linux Wayland/X11 编译、Windows x86_64、macOS arm64/x86_64；平台 smoke test
  只启动窗口/加载配置并受控退出，不执行真实安装、更新或卸载。

本地串行命令

受机器资源限制，所有 Rust 命令必须逐条等待结束，使用单 job/单测试线程；不得并行 build/test/run，也不得在构建时同时启动应用。每个阶段按以下顺序执行适用项：

     cargo fmt --all -- --check
     cargo check --workspace --all-targets --locked --jobs 1
     cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1
     cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings
     cargo build --workspace --locked --jobs 1

     依赖升级阶段先完成一次受控 update 并提交新 Cargo.lock，之后恢复 --locked 验证。跨平台 target 的 check/build 也逐个平台串行执行。

     手动端到端矩阵

     1. Linux：原生 Wayland 与 X11，各检查启动、主题、文件选择、通知、manager 检测；不并行启动两个实例。
     2. UI：Light/Dark/High Contrast × Wide/Medium/Narrow，检查 page accents、toolbar、sidebar sheet、inspector、Activity/status、键盘操作和 Settings dirty
     prompt。
     3. Windows：Winget 检测、只读 search/list/update scan、非管理员与需要提权场景、便携包和安装包启动。
     4. macOS：Apple Silicon/Intel 构建产物启动、Homebrew 检测及只读流程、.app/.dmg 资源与未签名提示。
     5. 操作确认：使用受控测试 manager 驱动 install/update/remove、partial failure、retry、stale response 和 cancellation
     lifecycle；真实系统事务只在明确授权的隔离环境中验证。

     完成标准

     - 四 crate 边界落地，UI/core 不再依赖闭合 PackageManagerType match；示例第三方 crate 可仅依赖 updater-manager-api 实现并编译时注册。
     - 直接依赖已按组升级到实施时最新稳定发行线，版本集中、lockfile 可复现、stable Rust 与质量 CI 通过。
     - 同一代码库可在 Wayland、X11、Windows、macOS 启动；首批平台 manager 为 Linux 现有集合、Windows Winget、macOS Homebrew。
     - Linux/Windows/macOS 未签名发布物均由 CI 生成并明确标注限制。
     - 主题、窄窗口和操作透明度改进完成；Settings/配置恢复、stale result、操作确认具备测试保护。
     - 取消状态只在底层进程真实结束后报告完成，所有写操作仍按 manager group 串行。
