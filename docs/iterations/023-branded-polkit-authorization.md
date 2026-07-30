# Iteration 023：品牌化 Polkit 授权链

- 日期：2026-07-30
- 状态：已完成
- ROADMAP阶段：阶段4 Linux系统manager提权与阶段6 Linux打包资产
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

APT、DNF、Pacman和Zypper不再让`pkexec`直接执行可配置的manager路径，而是统一调用安装在固定路径的最小特权helper。Updater提供自有Polkit actions，让桌面authentication agent展示Updater图标、动作说明和本地化认证消息，同时保持密码输入完全由系统agent负责。

## 实施计划

- [x] 新增固定路径`/usr/libexec/updater-system-helper`，只接受install/update/remove/refresh、四个系统manager和经过严格校验的package名称。
- [x] helper只用固定绝对路径直接`exec`系统manager，不经过shell，不接受任意executable、flag、本地package路径或环境注入。
- [x] 新增按动作区分的`com.ayi.updater.*` Polkit actions，配置Updater图标、英文/简体中文description与message。
- [x] APT、DNF、Pacman、Zypper的写操作和需要提权的metadata refresh全部切换到helper；只读命令继续尊重Settings中的custom executable。
- [x] `.deb`与`.rpm`包含helper、policy和显式Polkit运行时依赖。
- [x] 增加helper参数白名单、option/path injection拒绝、四个manager命令映射以及manager调用契约测试。
- [x] 验证policy XML、软件包内容和完整workspace串行门禁。
- [x] 经用户授权覆盖安装当前`updater-0.2.4-1`后，由Hyprpolkitagent完成真实认证窗口视觉验收。

## 安全边界

- Updater和helper都不读取、保存或转发管理员密码；authentication agent仍由桌面会话提供。
- helper不信任`pkexec`传入参数。可保留授权时仍只允许预定义package操作，不能转换成任意root命令。
- system manager的custom executable只影响可用性检查和只读查询；root写操作固定使用发行版标准路径。
- package名称必须是非空ASCII标识符，拒绝前导`-`、路径分隔符、空白、shell元字符、过长名称和过大批次。
- policy对active session使用`auth_admin_keep`减少同一批操作的重复认证；inactive/other session仍要求管理员认证。
- `.policy`只能定制动作名称、认证消息、vendor和图标。窗口布局、圆角、颜色、字体及密码控件由Hyprpolkitagent/KDE/GNOME等当前Polkit agent控制；本轮不替换全局agent。

## 验收标准

- 四个system manager生成`pkexec /usr/libexec/updater-system-helper <action> <manager> ...`，参数不再包含custom executable路径。
- policy通过helper固定路径和`argv1`选择install/update/remove/refresh动作，并显示Updater品牌元数据。
- helper对所有允许组合生成与当前行为等价的固定命令；非法manager/action/package不启动任何子进程。
- helper使用直接`exec`保留package manager原始stdout/stderr和退出状态。
- `.deb`/`.rpm`中的helper为`0755`，policy为`0644`，安装位置与policy annotation完全一致。
- 完整串行质量门禁通过，无warning。

## 进度日志

### 2026-07-30

- 本机实际认证agent为Hyprpolkitagent；本地`pkexec(1)`确认自有action可设置description、message、icon和defaults，并可用`org.freedesktop.policykit.exec.argv1`按helper首参数选择动作。
- 审计发现当前四个manager会把Settings中的custom executable直接交给`pkexec`。新helper不能延续该契约，否则可配置路径会成为任意root程序入口。
- APT、DNF、Pacman和Zypper除install/update/remove外，分别还有需要提权的metadata refresh命令，必须一并切换。
- 本机使用`cargo-generate-rpm 0.21.0`真实打包时确认`-p updater`会查找不存在的`updater/Cargo.toml`；该参数要求workspace成员目录，workflow已改回`-p ui`。
- 新增`updater-system-helper`二进制：action/manager/package解析是可单测纯函数，最终执行清空环境、固定工作目录和绝对程序路径，并用`exec`保留原命令输出与退出状态。
- shared manager command只保留一个跨四个manager复用的`system_helper_command`；固定`/usr/bin/pkexec`与helper路径，没有新增service、controller、trait或只调用一次的wrapper。
- 四个Polkit action按首参数区分install/update/remove/refresh，均提供Updater icon、英文/简体中文description/message和active-session `auth_admin_keep`。
- APT、DNF、Pacman与Zypper的写操作及提权refresh已切换；单元测试明确证明custom executable不会出现在root命令参数中。
- `.deb`与`.rpm`已包含helper和policy；package dump确认RPM内为`root:root`、helper `0755`、policy `0644`，DEB提取后的两个文件hash与源码完全一致。
- package workflow固定已验证的`cargo-deb 3.7.0`和`cargo-generate-rpm 0.21.0`，RPM selector修正为工具要求的workspace目录`ui`。
- 完整workspace串行门禁通过：198项测试成功、14项真实环境测试显式ignored、0失败，format/check/clippy/build均通过。
- 当前主机已安装本轮生成的`updater-0.2.4-1`；RPM校验无改动，helper、policy、desktop entry、图标和文档均由软件包持有。
- `pkaction`读取到Updater vendor/icon、按动作区分的description/message、固定helper路径与`argv1`约束；真实Hyprpolkitagent窗口显示了Updater的系统更新认证消息。
- 实机同时确认Hyprpolkitagent 0.1.3将窗口标题固定为`Hyprland Polkit Agent`。该标题不读取Polkit action，保持由全局桌面agent管理，不在Updater中替换或修改agent。

## Git提交

- `5ada47b docs: plan branded polkit authorization`
- `191130e feat: add restricted polkit system helper`
- `61386a4 docs: record polkit authorization validation`

## 验证记录

- `cargo test -p updater --bin updater-system-helper --locked --jobs 1 -- --test-threads=1`：5项成功，覆盖16种固定命令组合、package参数拒绝与policy XML action contract。
- `cargo test -p updater-managers --lib --locked --jobs 1 -- --test-threads=1`：43项成功，四个system manager命令契约通过。
- `xmllint --dropdtd assets/linux/com.ayi.updater.policy | xmllint --noout --dtdvalid /usr/share/polkit-1/policyconfig-1.dtd -`：通过本机Polkit DTD验证。
- `cargo build --release -p updater --locked --jobs 1`：通过，同时生成GUI与helper x86_64 ELF。
- `cargo generate-rpm -p ui`：生成`target/generate-rpm/updater-0.2.4-1.x86_64.rpm`；SHA-256为`7c2a2399a8738a9c5a3ff3aedcc130e9096c14e8045882b7a4213897c63cc5c2`。
- `cargo deb -p updater --no-build --no-strip --locked`：生成`target/debian/updater_0.2.4-1_amd64.deb`；SHA-256为`97d5722873a5b6c98e639ee122c991e72b2a2567481e67bc08d75c5ba397bd41`。Fedora主机无法解析Debian `$auto` shared-library package名称，Ubuntu CI仍需完成该项原生验证；显式`policykit-1`依赖与包内容已确认。
- `rpmlint`：helper/policy路径、权限、依赖和summary无问题；仍报告`cargo-generate-rpm`未生成BuildHost/Changelog tag及GUI无man page，留给Iteration 024发布硬化评估。
- `rpm -V updater`：通过，已安装文件与RPM数据库记录一致。
- `pkaction --action-id com.ayi.updater.update-system-packages --verbose`：确认Updater品牌元数据、`auth_admin_keep`和helper action绑定均已生效。
- Hyprpolkitagent真实窗口：显示`Authentication is required to update system packages with Updater`，密码输入和认证按钮仍由系统agent提供。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过，无warning。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：198项成功、14项ignored、0失败。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。

## 遗留项 / 下一轮

- Iteration 024继续Linux Wayland/X11、配置矩阵和beta发布物收口；Updater不接管或定制全局Polkit authentication agent。
