# Iteration 023：品牌化 Polkit 授权链

- 日期：2026-07-30
- 状态：进行中
- ROADMAP阶段：阶段4 Linux系统manager提权与阶段6 Linux打包资产
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

APT、DNF、Pacman和Zypper不再让`pkexec`直接执行可配置的manager路径，而是统一调用安装在固定路径的最小特权helper。Updater提供自有Polkit actions，让桌面authentication agent展示Updater图标、动作说明和本地化认证消息，同时保持密码输入完全由系统agent负责。

## 实施计划

- [ ] 新增固定路径`/usr/libexec/updater-system-helper`，只接受install/update/remove/refresh、四个系统manager和经过严格校验的package名称。
- [ ] helper只用固定绝对路径直接`exec`系统manager，不经过shell，不接受任意executable、flag、本地package路径或环境注入。
- [ ] 新增按动作区分的`com.ayi.updater.*` Polkit actions，配置Updater图标、英文/简体中文description与message。
- [ ] APT、DNF、Pacman、Zypper的写操作和需要提权的metadata refresh全部切换到helper；只读命令继续尊重Settings中的custom executable。
- [ ] `.deb`与`.rpm`包含helper、policy和显式Polkit运行时依赖。
- [ ] 增加helper参数白名单、option/path injection拒绝、四个manager命令映射以及manager调用契约测试。
- [ ] 验证policy XML、软件包内容和完整workspace串行门禁；实际认证窗口由安装后的桌面agent做视觉验收。

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

## Git提交

- 待记录。

## 验证记录

- 待记录。

## 遗留项 / 下一轮

- Iteration 024继续Linux Wayland/X11、配置矩阵和beta发布物收口。
