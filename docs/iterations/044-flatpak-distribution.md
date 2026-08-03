# Iteration 044：Flatpak 分发与宿主 Flatpak 桥接

- 日期：2026-08-03
- 状态：进行中
- ROADMAP阶段：阶段6跨平台发布物
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

生成x86_64与aarch64 Flatpak bundle，并让沙箱版本只通过显式宿主桥接管理Flatpak应用。不能让Flatpak包静默注册无法工作的系统或语言manager，也不能把任意自定义executable透传给宿主。

## 范围决策

- 使用当前Flathub同时提供双架构构建的GNOME Platform/SDK 50作为runtime基线，manifest固定app ID `com.ayi.updater`和stable branch。
- sandbox仅注册`builtin:flatpak`；APT、DNF、Pacman、Zypper、Snap、Homebrew及语言工具manager不在Flatpak capability profile中出现。
- Flatpak CLI通过官方`flatpak-spawn --host`执行，宿主程序固定为`/usr/bin/flatpak`，并清理环境后只恢复Flatpak用户安装和桌面授权所需的最小变量。
- sandbox内拒绝`ManagerConfig.executable`覆盖，避免将用户配置路径变成任意宿主命令入口；所有Flatpak参数继续由typed target、scope、remote和完整ref契约生成。
- manifest不声明`--filesystem=host`或`--filesystem=host-os`；显示、GPU、网络和`org.freedesktop.Flatpak` session D-Bus是首轮唯一额外权限。
- `org.freedesktop.Flatpak`允许host spawn，属于高敏感权限。它是当前CLI架构保持宿主Flatpak功能的显式能力，不描述为严格沙箱隔离；后续若接入专用宿主服务，应收窄到应用自有D-Bus接口。

## 实施计划

- [x] 核对官方manifest、sandbox permission与`flatpak-spawn --host`契约，并确认GNOME 50双架构runtime可用。
- [ ] 增加sandbox-aware Flatpak command bridge和纯函数离线契约测试。
- [ ] 让运行时catalog在Flatpak sandbox中只注册Flatpak manager，同时保留按Platform查询的确定性。
- [ ] 增加Flatpak manifest、可复现bundle脚本和x86_64/aarch64 Package矩阵。
- [ ] 扩展bundle严格计数、checksum与tag Release资产清单。
- [ ] 更新README与ROADMAP，明确权限、能力profile、安装方式和限制。
- [ ] 通过本地Rust/manifest/script门禁、远端CI、Package jobs和下载后bundle复核。

## 验收标准

- Flatpak sandbox中的manager catalog只含`builtin:flatpak`，原生Linux catalog保持不变。
- sandbox桥接命令以`flatpak-spawn --host --watch-bus --clear-env`开始，固定调用`/usr/bin/flatpak`，并拒绝custom executable。
- 两个`.flatpak`均可导入临时repository，metadata中包含预期runtime、command、desktop文件、AppStream metadata和图标。
- bundle job严格计数2个Flatpak资产，并为总计19个发布物生成同一`SHA256SUMS`。
- README明确该包需要host-spawn权限、只管理Flatpak，并且不包含原生包的Polkit helper。

## 进度日志

### 2026-08-03

- 官方命令文档确认`flatpak-spawn --host`在沙箱内运行宿主命令，并要求访问`org.freedesktop.Flatpak`D-Bus接口。
- 官方manifest文档确认runtime、runtime-version、SDK、command与finish-args边界；临时隔离的Flathub metadata查询确认GNOME Platform/SDK 50同时提供x86_64与aarch64 ref。
- 直接给予该D-Bus权限会允许host spawn，因此本轮只在产品代码中桥接固定Flatpak CLI并将权限风险公开；不会声称该权限本身能阻止被篡改进程调用其他宿主命令。

## 遗留项 / 下一轮

- 专用宿主agent和应用自有D-Bus协议可进一步收窄权限，但需要独立安装、版本协商、命令白名单与认证设计，不在本轮Flatpak bundle中伪装完成。
