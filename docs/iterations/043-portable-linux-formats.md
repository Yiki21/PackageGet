# Iteration 043：Portable Linux 与 AppImage 发布物

- 日期：2026-08-03
- 状态：已完成
- ROADMAP阶段：阶段6跨平台发布物
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

在现有DEB、RPM和Arch原生包之外增加可直接解压运行的Linux归档，并为glibc目标生成AppImage。musl产物必须在Alpine原生环境编译和验证，不把动态依赖Wayland/X11及系统图形栈的GUI描述为全静态通用二进制。

## 范围决策

- glibc portable tar分别覆盖x86_64与aarch64，使用Ubuntu 22.04 userspace作为兼容基线；归档包含主程序、LICENSE、README和便携包限制说明。
- AppImage分别覆盖x86_64与aarch64，从同一glibc release binary构建并验证AppDir入口、desktop metadata、图标和ELF架构。
- portable tar与AppImage不安装固定路径Polkit helper。用户级manager和全部只读流程可直接使用；APT、DNF、Pacman和Zypper的特权写操作需要另行安装原生DEB/RPM/Arch包所提供的helper与policy。
- musl只作为Alpine/musl runtime归档发布，不声称能在任意glibc发行版运行；只有Alpine x86_64/aarch64原生CI构建和动态依赖验证均通过后才进入release bundle。
- Flatpak作为分发格式与项目内的Flatpak manager是两个概念。当前应用依赖宿主package-manager CLI，直接放入沙箱会使大多数manager不可用；本轮只冻结后续要求，不生成残缺的`.flatpak`。
- Flatpak后续实现必须先提供显式、可测试的宿主命令桥接或受限capability profile，并准确展示沙箱内可用manager；不能仅通过宽泛filesystem权限假装具备宿主执行能力。

## 实施计划

- [x] 审计glibc/musl、AppImage、Flatpak与当前运行时及Polkit契约的兼容边界。
- [x] 增加可复现的portable tar与AppImage构建脚本。
- [x] 扩展Linux x86_64/aarch64 glibc原生构建矩阵。
- [x] 完成Alpine/musl原生编译探针，并根据结果接入或门控musl tar。
- [x] 扩展bundle计数、checksum和tag Release资产清单。
- [x] 更新README与ROADMAP，说明各格式能力和运行时限制。
- [x] 通过本地脚本门禁、远端CI、原生Package jobs和下载后checksum复核。

## 验收标准

- 每个portable tar解压后可从顶层直接执行`./updater`，归档路径、owner和mtime可复现。
- 每个AppImage包含有效AppRun、`com.ayi.updater.desktop`、512px图标与目标架构ELF，并通过AppImage自解包结构验证。
- musl产物若发布，文件名明确包含`musl`，CI在相同Alpine runtime中验证动态加载器与所需共享库；未通过时不进入bundle计数。
- bundle job使用按格式分组的严格计数，并对所有发布物生成同一`SHA256SUMS`。
- README不把portable格式描述为已安装Polkit系统集成，也不把计划中的Flatpak描述为已交付。

## 进度日志

### 2026-08-03

- 当前Package run已稳定交付5个Linux原生包、2个Windows产物和4个macOS产物；Iteration 042最终bundle与checksum均通过。
- linuxdeploy `continuous`在当前验证点同时提供x86_64和aarch64工具；x86_64临时AppDir探针成功部署现有release binary，确认desktop文件的绝对`Exec=/usr/bin/updater`不能用于AppImage，需要独立portable desktop metadata。
- 当前release binary的直接ELF依赖仅显示glibc、libm与libgcc；Wayland/X11及图形后端由运行时动态加载，因此AppImage仍需明确宿主桌面/驱动依赖，musl也不能被描述为全静态GUI。
- Flatpak manifest本身不足以保持产品能力：沙箱内`/usr`来自runtime，当前manager executable discovery与固定宿主Polkit链均不可直接工作，必须先设计宿主桥接或受限产品profile。
- 新增`package-portable.sh`和`package-appimage.sh`；glibc产物以Ubuntu 22.04为兼容基线，AppImage工具以固定SHA-256下载，tar归档统一owner、排序与mtime。
- Alpine 3.22/Rust 1.97容器中的`cargo check --workspace --all-targets --locked`通过；Package矩阵随后在x86_64与aarch64原生Alpine容器中完成release构建、musl loader和共享库验证。
- Package run `30784539268`首次完整生成6个portable产物，连同既有原生包共17项；bundle严格计数与`SHA256SUMS`均通过。下载完整artifact后，本地`sha256sum -c`对17项全部通过，两个tar顶层结构和x86_64 AppImage自解包结构也已复核。
- 首轮musl构建成功，但Docker以root写入缓存目录导致`actions/cache` post步骤无法归档。后续将owner归一化限制在`.cargo-musl`与`target-musl`两个专用目录；Package run `30785115485`全绿，并分别保存aarch64与x86_64 musl cache key。
- 本地门禁通过：`actionlint`、`bash -n`、ShellCheck、`desktop-file-validate`、AppStream metadata验证、release metadata验证与workflow diff检查。

## 遗留项 / 下一轮

- Flatpak宿主执行设计需要单独安全评审，覆盖命令白名单、参数透传、环境清理、取消和错误分类；通过前不发布Flatpak bundle。
- musl首次缓存命中和真实Alpine桌面启动仍留给下一次Package run及发布候选手工smoke；本轮已经验证缓存成功保存、ELF/loader依赖和归档结构。
