# Iteration 024：Linux Beta 发布硬化

- 日期：2026-07-30
- 状态：进行中
- ROADMAP阶段：阶段4 Linux X11启动与阶段6 Linux发布物
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将Iteration 022后的Linux版本收敛为可验证的`0.3.0-beta.1`候选：同一二进制支持Wayland和X11，clean与旧schema配置均有明确启动结果，本机生成可安装的`.deb`/`.rpm`和校验和，CI保留amd64/arm64矩阵并能随tag发布完整资产。

## 实施计划

- [ ] 为Iced启用Wayland/X11双后端，删除强制Wayland退出，并统一reverse-DNS application ID与desktop entry。
- [ ] 将workspace和软件包版本更新为`0.3.0-beta.1`，补充Linux preview release notes与README限制说明。
- [ ] 在package workflow中校验版本一致性、汇总四种架构产物并生成`SHA256SUMS`。
- [ ] 使用隔离的`XDG_CONFIG_HOME`验证clean config、当前schema和旧schema拒绝/恢复入口，不修改用户真实配置。
- [ ] 本机构建x86_64 `.deb`/`.rpm`，检查包内容、元数据、安装依赖和SHA-256校验和。
- [ ] 验证Wayland/X11启动；隐藏workspace不可用时使用可复现的隔离display/container证据，不接触真实系统package写操作。
- [ ] 串行通过完整workspace质量门禁，并记录arm64验证仍需由原生GitHub Actions runner完成还是已经通过。

## 行为约束

- X11与Wayland共用同一个应用入口，不增加backend wrapper、launcher helper或只使用一次的检测函数。
- application ID固定为`com.ayi.updater`；desktop entry的`StartupWMClass`与窗口identity一致。
- clean配置测试只允许在临时`XDG_CONFIG_HOME`中自动检测并写入配置；旧schema测试必须保留原文件并进入可见恢复状态。
- 本轮不执行install/update/remove等真实系统package写操作。
- 当前提权窗口由`pkexec`选择的桌面Polkit authentication agent绘制。发布硬化不收集密码、不自绘认证对话框，也不引入未经独立安全设计的特权helper。
- `0.3.0-beta.1`明确标记为unsigned Linux preview；Windows/macOS和stable完成标准不随本轮提前宣称完成。

## 验收标准

- 未设置Wayland环境变量时应用不再主动退出；Wayland和X11 feature均编译进Linux二进制。
- desktop entry、窗口application ID、Cargo package version和release notes一致。
- clean config可创建当前schema；当前schema可加载；旧schema被严格拒绝且文件内容不被自动改写。
- x86_64 `.deb`与`.rpm`可生成，包内二进制、desktop entry、图标和文档路径正确；校验和可由`sha256sum -c`复验。
- GitHub Actions仍在原生amd64/arm64 runner上分别生成`.deb`与`.rpm`，release job输出四个包和`SHA256SUMS`。
- 完整串行质量门禁通过，无warning。

## 进度日志

### 2026-07-30

- 审计确认GUI目前同时存在代码级Wayland环境检查与仅启用Iced `wayland` feature两层限制；X11尚未进入二进制。
- 当前窗口`application_id`和desktop `StartupWMClass`均为`updater`，尚未使用ROADMAP约定的`com.ayi.updater`。
- package workflow已有amd64/arm64原生runner矩阵和正确的`cargo generate-rpm -p updater`命令，但release job尚未生成checksums。
- 本机会话未暴露`agent-workspace-linux` workspace工具；后续GUI证据将明确区分编译、隔离display启动和用户已完成的恢复页视觉验收。
- Iteration 023先交付Updater自有Polkit action与最小特权helper；本轮在该授权链通过后继续发布收口。

## Git提交

- 待记录。

## 验证记录

- 待记录。

## 遗留项 / 下一轮

- 待本轮完成后确定。
