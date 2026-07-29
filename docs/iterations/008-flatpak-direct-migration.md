# Iteration 008：Flatpak 直接迁移与 User/System Scope Parity

- 日期：2026-07-29
- 状态：已完成
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 Flatpak 迁移为 `updater-managers` 中第五个直接实现，并修复旧实现长期忽略的 user/system installation、完整 ref 与 remote identity 问题。installed、updates、search 与 write target 必须以 scope/ref/origin 保持一致；现有 UI 与 Config V1 继续通过兼容 wrapper 工作。

## 实施计划

- [x] 直接实现 Flatpak descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [x] installed 使用 machine-readable columns 同时读取 user/system scope、完整 ref 与 origin，并保留同 ID 多 installation/branch 实例。
- [x] 删除不可用的 `flatpak info --show-version/--show-branch` 路径，current version 改为从完整 installed listing 精确匹配。
- [x] 分别查询 system/user app updates；`refresh=false` 使用 `--cached`，并按 `(scope, normalized ref)` 匹配 installed state。
- [x] 分别查询 system/user search catalog，将 remote 写入 `PackageOrigin`，确保 install target 能无交互选择 remote/ref。
- [x] write 根据 target scope/ref/origin 构造 `--system|--user --noninteractive` 命令，不使用 `pkexec`；Project scope 明确返回 Unsupported。
- [x] 为 legacy `PackageScope::Unknown` 保留旧命令默认行为并加测试，作为 Config V1/UI 迁移完成前的受控兼容路径。
- [x] 扩展 Flatpak busy/network/permission/cancelled typed error classifier，不吞掉 repository 或 system-helper failure。
- [x] 将 core Flatpak 收缩为 Config V1、model、progress 与 error wrapper，并更新 mixed registry。
- [x] 增加 scope/ref/origin、NBSP size、new-build update、命令构造、conversion、registration 与 public API contracts。
- [x] 在当前宿主机执行 opt-in 只读 smoke，验证 user/system installed、count 与 cached app updates；search 保持单独 opt-in。
- [x] 串行通过 workspace format、check、test、clippy 与 build 完整门禁。
- [x] 由 GitHub Actions 复验相同的 locked 单 job 门禁。

## 目标命令契约

- availability：`flatpak --version`。
- installed：`flatpak list --app --columns=application:f,name:f,version:f,branch:f,size,origin:f,installation:f,ref:f`。
- updates 分别执行：
  - `flatpak remote-ls --system --updates --app [--cached] --columns=application:f,ref:f,branch:f,version:f,commit:f,origin:f`
  - `flatpak remote-ls --user --updates --app [--cached] --columns=application:f,ref:f,branch:f,version:f,commit:f,origin:f`
- search 分别执行：
  - `flatpak --default-arch`
  - `flatpak search --system --columns=application:f,name:f,description:f,version:f,branch:f,remotes:f QUERY`
  - `flatpak search --user --columns=application:f,name:f,description:f,version:f,branch:f,remotes:f QUERY`
- write：
  - install：`flatpak install --system|--user -y --noninteractive REMOTE REF`
  - update：`flatpak update --system|--user -y --noninteractive REF`
  - uninstall：`flatpak uninstall --system|--user -y --noninteractive REF`

## Identity 与兼容边界

- installed/update identity 使用 `(PackageScope, normalized full ref)`，不能仅按 application ID 去重。
- `PackageOrigin.name` 保存 remote；`PackageOrigin.reference` 保存 normalized full ref，供后续写 target 冻结身份。
- direct read results 必须带明确 User/System scope；同一 application ID 的两个 installation 都必须保留。
- `PackageScope::Unknown` 仅用于旧 UI 兼容：保持无 `--user/--system` 的既有默认命令语义，并保留明确测试；新 direct target 不应产生 Unknown。
- named system installation 不能由现有 scope 无损表达；本轮遇到时返回 Unsupported/Protocol，不静默当作默认 system。

## 非目标

- 本轮不迁移 Homebrew 或语言工具 manager。
- 本轮不引入新的 package identity 公共类型或修改 Config schema/UI identity。
- 本轮不支持 Flatpak runtime 更新，installed/updates/search 均保持 app 语义。
- 本轮不在自动测试中安装、更新或卸载真实 Flatpak 应用。
- 本轮不为 named system installation 扩展 manager-api；后续有真实需求再设计。

## 设计约束

- Flatpak 实现位于根目录平铺 crate 的 `managers/src/flatpak.rs`，不新增通用 `crates/` 或 package-manager 分组目录。
- 不使用 `pkexec`；Flatpak 自己的 system helper/Polkit 负责 system transaction 授权。
- parser 与 command contracts 默认使用 fake executable 离线验证；真实宿主 smoke 只读且显式 opt-in。
- decimal `KB/MB/GB` 与 binary `KiB/MiB/GiB` 必须按各自单位换算，并支持 NBSP whitespace。
- toolchain 与 CI 继续跟随 `stable` channel；manifest 使用宽 semver line，精确依赖图由提交的 `Cargo.lock` 固定。

## 进度日志

### 2026-07-29

- Iteration 007 已完成 direct Zypper、locale/exit-code parity、Tumbleweed 无网络只读 smoke、本地完整门禁与 GitHub Actions 复验。
- Flatpak 只读审计确认旧 `info --show-version/--show-branch` 在当前 Flatpak 1.18.0 不可用，旧 updates 也会漏掉 user installation 并可能混入 runtime。
- 当前宿主机 `/usr/bin/flatpak` 为 1.18.0，同时存在 system 与 user app installations，可用于本轮高价值只读 smoke。
- 确定 scope/ref/origin 是本轮核心验收，不能只把旧 application-ID parser 搬到 managers crate。
- 实测确认 `list` 的 ref 是三段 `ID/ARCH/BRANCH`，`remote-ls` 的 ref 是四段 `app/ID/ARCH/BRANCH`；direct 实现统一存储并输出四段 canonical app ref，同时拒绝 runtime、段数错误和 application/ref 不一致。
- search 无 arch/ref columns，因此通过同一 executable 执行 `flatpak --default-arch` 后构造完整 ref；逗号分隔的 remotes 拆成独立 scope/ref/remote target。
- direct public contracts 已覆盖同 ID 的 system/user 实例、decimal/binary/NBSP size、cached updates、same-version new build、search 多 remote、scoped/legacy write argv、配置/target mismatch 与空执行边界。
- core Flatpak 已收缩为 Config V1、旧 model、progress 与 typed error wrapper；mixed registry 当前包含五个 direct manager 和六个 legacy adapter。
- opt-in 宿主只读 smoke 通过：availability、system/user installed、count 和 cached system/user app updates 均符合 direct contract。

## Git 提交

- `2d5f0fc feat: add scoped Flatpak manager`
- `a9055c2 refactor: route Flatpak through direct manager`
- `cce7599 docs: record Flatpak migration progress`
- `96efa83 docs: record Flatpak workspace validation`

## 验证记录

- `cargo check -p updater-managers --jobs 1`：通过。
- `cargo test -p updater-managers --lib --jobs 1 -- --test-threads=1`：34 passed。
- `cargo test -p updater-managers --test flatpak_contract --jobs 1 -- --test-threads=1`：6 passed，1 ignored。
- `cargo test -p updater-managers --test flatpak_contract host_flatpak_read_only_smoke --jobs 1 -- --ignored --exact --test-threads=1 --nocapture`：1 passed。
- `cargo clippy -p updater-managers --all-targets --jobs 1 -- -D warnings`：通过。
- `cargo check -p updater_core --jobs 1`：通过。
- `cargo test -p updater_core --lib --jobs 1 -- --test-threads=1`：69 passed，7 ignored。
- `cargo test -p updater_core --test builtin_registry --jobs 1 -- --test-threads=1`：6 passed。
- `cargo clippy -p updater_core --all-targets --jobs 1 -- -D warnings`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：通过。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。
- GitHub Actions CI run `30438378318`：通过，耗时 3 分 15 秒；format、check、deterministic tests、clippy 与 build 全部成功。

## 遗留项 / 下一轮

- 下一轮进入 [Iteration 009：Homebrew 直接迁移与 Formula/Cask Identity](009-homebrew-direct-migration.md)。
- 当时的Config与旧UI仍只能保存package name，无法完整保留Flatpak direct target的scope/ref/origin；该限制留待阶段3的Config/UI identity迁移解决。
- named system installation 继续显式 Unsupported，不静默映射成默认 system installation。
