# Iteration 024：Linux Beta 发布硬化

- 日期：2026-07-30
- 状态：本地实施完成，等待CI验证
- ROADMAP阶段：阶段4 Linux X11启动与阶段6 Linux发布物
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将Iteration 022后的Linux版本收敛为可验证的`0.3.0-beta.1`候选：同一二进制支持Wayland和X11，clean与旧schema配置均有明确启动结果，本机生成可安装的`.deb`、`.rpm`和Arch Linux`.pkg.tar.zst`，CI保留原生amd64/arm64矩阵并能随tag发布五个包与校验和。

## 实施计划

- [x] 为Iced启用Wayland/X11双后端，删除强制Wayland退出，并统一reverse-DNS application ID与desktop entry。
- [x] 将workspace和软件包版本更新为`0.3.0-beta.1`，补充Linux preview release notes与README限制说明。
- [x] 在package workflow中校验版本一致性、汇总五个Linux包并生成`SHA256SUMS`。
- [x] 使用隔离的`XDG_CONFIG_HOME`验证clean config、当前schema和旧schema拒绝/恢复入口，不修改用户真实配置。
- [x] 本机构建x86_64 `.deb`、`.rpm`和`.pkg.tar.zst`，检查包内容、元数据、安装依赖和SHA-256校验和。
- [x] 在当前Hyprland会话中分别强制Wayland与X11启动，不接触真实系统package写操作。
- [x] 串行通过完整workspace质量门禁；原生arm64/aarch64包仍明确留给GitHub Actions runner验证。

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
- x86_64 `.deb`、`.rpm`与`.pkg.tar.zst`可生成，包内二进制、helper、Polkit policy、desktop entry、图标和文档路径正确；校验和可由`sha256sum -c`复验。
- GitHub Actions仍在原生amd64/arm64 runner上分别生成`.deb`与`.rpm`，另由官方Arch容器生成x86_64包；release job输出五个包和`SHA256SUMS`。
- 完整串行质量门禁通过，无warning。

## 进度日志

### 2026-07-30

- 审计确认GUI目前同时存在代码级Wayland环境检查与仅启用Iced `wayland` feature两层限制；X11尚未进入二进制。
- 当前窗口`application_id`和desktop `StartupWMClass`均为`updater`，尚未使用ROADMAP约定的`com.ayi.updater`。
- package workflow已有amd64/arm64原生runner矩阵；Iteration 023真实打包已将RPM命令纠正为`cargo generate-rpm -p ui`，release job仍未生成checksums。
- 本机会话未暴露`agent-workspace-linux` workspace工具；后续GUI证据将明确区分编译、隔离display启动和用户已完成的恢复页视觉验收。
- Iteration 023先交付Updater自有Polkit action与最小特权helper；本轮在该授权链通过后继续发布收口。
- Linux构建已同时启用Iced `wayland`和`x11` feature，窗口与desktop entry统一使用`com.ayi.updater`；不增加backend wrapper或环境检测helper。
- workspace版本更新为`0.3.0-beta.1`；DEB/RPM使用`0.3.0~beta.1-1`，Arch使用按`vercmp`正确排在`0.3.0`之前的`0.3.0beta.1-1`。
- package workflow保留DEB/RPM原生amd64/arm64矩阵，新增官方Arch容器、版本断言、单主包与`namcap`错误检查、五包汇总、`SHA256SUMS`和beta release notes。
- Arch首次打包暴露makepkg默认LTO与bundled C链接冲突、默认debug额外产物和非标准ELF目录；最终`PKGBUILD`关闭makepkg的debug/LTO注入、保留Cargo Thin LTO，并将helper统一安装到`/usr/lib/updater/`。
- DEB/RPM/Arch均显式声明通过`dlopen`使用的Wayland/X11/XKB依赖、Polkit和hicolor图标主题；DEB不再依赖跨发行版构建时不可靠的`$auto`映射。
- 用户已完成旧schema恢复页视觉验收；本轮隔离clean config自动生成当前schema，随后同一配置在X11启动中成功加载。
- 本地所有验证完成后执行清理；`cargo clean`受容器子UID文件阻塞后，通过已确认范围的rootless Podman user namespace移除整个`target/`，从17 GiB降为不存在。

## Git提交

- `dbb7d55 feat: support Linux Wayland and X11`
- `a5005e4 feat: prepare portable 0.3.0 beta packages`
- `9a40625 build: add Arch Linux release package`
- `9d31c3a fix: declare package runtime dependencies`

## 验证记录

- `cargo fmt --all -- --check`通过。
- `cargo check --workspace --all-targets --locked --jobs 1`通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`通过：199 passed，14个需要真实网络、本机manager或外部环境的smoke test按约定ignored。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`通过。
- `cargo build --workspace --locked --jobs 1`与`cargo build --release --locked --jobs 1 -p updater`通过。
- Wayland实启窗口：class=`com.ayi.updater`、title=`Updater`、`xwayland=false`；日志确认`Using Wayland platform`。
- X11实启窗口：class=`com.ayi.updater`、title=`Updater`、`xwayland=true`；日志确认`Using X11 platform`。
- clean配置在`/tmp/tmp.gx3XNlB2Mw/config/updater/config.json`生成当前三字段schema；X11复用该文件成功启动，真实用户配置未被读取或改写。
- Arch官方容器`makepkg --cleanbuild --noconfirm`通过并只生成一个主包；包内helper位于`/usr/lib/updater/updater-system-helper`，`namcap`无error。五条unused dependency warning对应运行时`dlopen`或Polkit policy，依赖按实际契约保留。
- `packaging/arch/.SRCINFO`与官方`makepkg --printsrcinfo`完全一致；`bash -n`、YAML解析和`actionlint v1.7.7`通过。
- DEB：`updater_0.3.0~beta.1-1_amd64.deb`，SHA-256 `f5aa8431cad9765ef96c3169a0156b148f9fc1287cf4625cec7f62b7b888c36d`。
- RPM：`updater-0.3.0~beta.1-1.x86_64.rpm`，SHA-256 `42623bdfa2751a4b27db34ef3bd5e5ec464b25dd97443811471a25676c6a8`。
- Arch：`updater-0.3.0beta.1-1-x86_64.pkg.tar.zst`，SHA-256 `d5c522c400dba96e0ff94620d5e7e9cbdf4a513ce7febde7de0314ce23d15f4b`。
- 三个本地包的临时`SHA256SUMS`均由`sha256sum -c`复验为`OK`；随后按要求清理本地`target/`。

## 遗留项 / 下一轮

- 当前代码已达到`0.3.0-beta.1`本地候选标准，但还不是已验证发布：先将提交同步到远端，通过`workflow_dispatch`运行原生amd64/arm64五包矩阵与bundle job。
- 只有远端五个包和`SHA256SUMS`全部通过后才创建`Build-v0.3.0-beta.1` tag；tag workflow将其标记为prerelease并使用`RELEASE_NOTES.md`。
- 本轮仍是unsigned Linux preview。Windows/macOS产物、签名和ROADMAP stable完成标准继续留在后续阶段。
