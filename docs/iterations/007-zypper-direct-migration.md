# Iteration 007：Zypper 直接迁移与 Exit-Code/Locale Parity

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 Zypper 迁移为 `updater-managers` 中第四个直接实现，保持现有 RPM installed metadata、search、update listing、refresh 与批量事务语义，同时显式处理 Zypper 的 locale-sensitive 表格输出和专属退出码。现有 UI 与 Config V1 继续通过轻量 wrapper 工作，mixed registry 直接注册 APT、DNF、Pacman 与 Zypper。

## 实施计划

- [x] 直接实现 Zypper descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [x] 保留自定义 Zypper executable、RPM query metadata、refresh/no-explicit-refresh 和批量 install/update/remove 命令语义。
- [x] 为 Zypper 表格命令固定 `LC_ALL=C`，避免用户桌面 locale 改变 header 与 parser contract。
- [x] 增加 Zypper 专属退出码映射：permission、busy、reboot required、search no-match、cancelled、network/incomplete result 与其他事务失败。
- [x] 将 core 的旧 Zypper 入口收缩为 Config V1、model、progress 与 typed error compatibility wrapper。
- [x] 更新 mixed built-in registry：四个 system manager 使用直接实现，其余 7 个 manager 继续使用 legacy adapter。
- [x] 增加离线 RPM/table fixtures、command construction、exit-code、conversion、registration 与 public API contract tests。
- [x] 在 openSUSE Tumbleweed 容器中执行 direct API 的无网络、只读 availability/installed/count/current-version smoke。
- [ ] 串行通过 format、check、test、clippy、build，并由 GitHub Actions 复验。

## 现有命令契约

- availability：`zypper --version`。
- current version：`rpm -q --queryformat "%{VERSION}-%{RELEASE}" PACKAGE`。
- installed：`rpm -qa --queryformat` 输出 name、version-release、summary、size、install time 与 URL。
- count：`rpm -qa`，失败时回退完整 installed listing。
- search：`zypper --non-interactive search --details QUERY`。
- updates：可选 `pkexec ZYPPER --non-interactive refresh`，随后 `ZYPPER --non-interactive list-updates`。
- install/update/remove：`pkexec ZYPPER --non-interactive COMMAND -y PACKAGES...`。

## 专属退出码边界

- `5`：`Permission`。
- `7`：`Busy`。
- `102`：`RebootRequired`。
- `104`：search 无匹配；仅 search 将其转换为空结果。
- `105`：`Cancelled`。
- `106`：`Network` 或不完整 repository result，不能静默接受部分结果。
- `103`、`107`：保留明确 detail 的 `Other`，避免掩盖需要重启 manager 或 RPM script failure。

## 非目标

- 本轮不迁移 Flatpak、Homebrew 或语言工具 manager。
- 本轮不从文本表格切换到 Zypper XML 输出，也不新增 XML parser 依赖。
- 本轮不改变既有 `--non-interactive`、`-y`、refresh 或 manager-group 串行策略。
- 本轮不在宿主机或容器内执行 refresh、search、update listing 或任何写事务 smoke。
- 本轮不修改 Config V2、UI identity 或 manager settings 页面。

## 设计约束

- Zypper 实现位于根目录平铺 crate 的 `managers/src/zypper.rs`，不新增通用 `crates/` 或 package-manager 分组目录。
- command-local environment 只抽象真实复用边界；Zypper 专属退出码不能污染其他 manager 的通用 classifier。
- legacy wrapper 只保留 Config V1、model、progress 与 error 转换；Zypper 命令和 parser 只能存在于 `updater-managers`。
- parser、command 与 contract tests 默认离线；容器 smoke 使用 `--network none` 和只读 workspace mount。
- toolchain 与 CI 继续跟随 `stable` channel；manifest 使用宽 semver line，精确依赖图由提交的 `Cargo.lock` 固定。

## 进度日志

### 2026-07-29

- Iteration 006 已完成 direct Pacman、public API fixtures、Arch 容器只读 smoke、本地完整门禁与 GitHub Actions 复验。
- Zypper 只读审计已完成，确认现有命令、parser、RPM metadata 和 wrapper 边界；没有在审计期间编辑代码或并行运行 Rust 构建。
- 确定本轮优先锁定 `LC_ALL=C` 与专属退出码，而不是扩大迁移面切换 XML 协议。
- 容器验收目标为 `registry.opensuse.org/opensuse/tumbleweed:latest`，只执行 availability、installed、count 与 current-version direct API。
- `updater-managers` 已新增平铺 `zypper.rs`，直接实现完整 object-safe manager contract，并继续复用 bounded percent progress。
- shared command boundary 已增加 command-local environment，且仅 search/list-updates 设置 `LC_ALL=C`；refresh/write 不依赖 locale-sensitive parser。
- shared progress runner 已允许 manager 注入 status mapper；Zypper 5/7/102/105/106 映射到 typed error，103/104/107 保留 `Other` detail，未知状态继续回退通用 stderr classifier。
- search 仅将退出码 104 解释为无匹配；106 等失败即使带有 partial stdout 也不会被解析成成功结果。
- `core/src/pm/zypper.rs` 已删除旧 command construction、RPM/table parser 与执行副本，只保留 Config V1、model、progress 和 typed error 转换。
- mixed built-in registry 现在直接注册 APT、DNF、Pacman、Zypper；其余 7 个 manager 继续使用 legacy adapter，并增加 direct Zypper duplicate contract。
- public API fake executable contracts 已实际验证 `LC_ALL=C` 子进程环境、reordered table、duplicate first-wins、104 search no-match、106 partial-result rejection，以及完整 status/fallback error matrix。
- Podman `registry.opensuse.org/opensuse/tumbleweed:latest`（digest `sha256:cb29ab2b3c1a47859ac491f105319ed03b6334121ef815c1bab3de0825178f11`）在 rootfs/workspace 只读且 `--network none` 下通过 direct API smoke。

## Git 提交

| 提交 | 内容 | 验证 |
| --- | --- | --- |
| `a48fb14` | 实现 direct Zypper、command-local locale 与专属退出码 | managers 41 项通过、3 项 ignored；check/clippy 通过 |
| `80d20f0` | 将 legacy Zypper 路由到 direct manager 并更新 mixed registry | core 74 项通过、11 项 ignored；check/clippy 通过 |
| `f234fe7` | 增加 direct Zypper public contracts 与 Tumbleweed smoke | 7 个默认 contract tests；Podman smoke 1 项 |

## 验证记录

- `cargo check -p updater-managers --jobs 1` 通过。
- `cargo test -p updater-managers --jobs 1 -- --test-threads=1`：41 项通过，3 项环境 smoke 保持 ignored。
- `cargo clippy -p updater-managers --all-targets --jobs 1 -- -D warnings` 通过。
- `cargo check -p updater_core --jobs 1` 通过。
- `cargo test -p updater_core --jobs 1 -- --test-threads=1`：74 项通过，11 项环境或网络测试保持 ignored。
- `cargo clippy -p updater_core --all-targets --jobs 1 -- -D warnings` 通过。
- `cargo test -p updater-managers --test zypper_contract --jobs 1 -- --test-threads=1`：7 项通过，1 项容器 smoke 保持 ignored。
- Podman Tumbleweed 显式运行 `tumbleweed_container_zypper_read_only_smoke`：1 项通过；容器无网络且没有执行 refresh、search、updates 或写事务。

## 遗留项 / 下一轮

本轮完成后填写；已知后续候选为 Flatpak scope/ref/origin parity。
