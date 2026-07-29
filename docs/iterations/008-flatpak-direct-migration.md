# Iteration 008：Flatpak 直接迁移与 User/System Scope Parity

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 Flatpak 迁移为 `updater-managers` 中第五个直接实现，并修复旧实现长期忽略的 user/system installation、完整 ref 与 remote identity 问题。installed、updates、search 与 write target 必须以 scope/ref/origin 保持一致；现有 UI 与 Config V1 继续通过兼容 wrapper 工作。

## 实施计划

- [ ] 直接实现 Flatpak descriptor、availability、current version、installed/count、updates、search 与统一 execute。
- [ ] installed 使用 machine-readable columns 同时读取 user/system scope、完整 ref 与 origin，并保留同 ID 多 installation/branch 实例。
- [ ] 删除不可用的 `flatpak info --show-version/--show-branch` 路径，current version 改为从完整 installed listing 精确匹配。
- [ ] 分别查询 system/user app updates；`refresh=false` 使用 `--cached`，并按 `(scope, normalized ref)` 匹配 installed state。
- [ ] 分别查询 system/user search catalog，将 remote 写入 `PackageOrigin`，确保 install target 能无交互选择 remote/ref。
- [ ] write 根据 target scope/ref/origin 构造 `--system|--user --noninteractive` 命令，不使用 `pkexec`；Project scope 明确返回 Unsupported。
- [ ] 为 legacy `PackageScope::Unknown` 保留旧命令默认行为并加测试，作为 Config V1/UI 迁移完成前的受控兼容路径。
- [ ] 扩展 Flatpak busy/network/permission/cancelled typed error classifier，不吞掉 repository 或 system-helper failure。
- [ ] 将 core Flatpak 收缩为 Config V1、model、progress 与 error wrapper，并更新 mixed registry。
- [ ] 增加 scope/ref/origin、NBSP size、new-build update、命令构造、conversion、registration 与 public API contracts。
- [ ] 在当前宿主机执行 opt-in 只读 smoke，验证 user/system installed、count 与 cached app updates；search 保持单独 opt-in。
- [ ] 串行通过 format、check、test、clippy、build，并由 GitHub Actions 复验。

## 目标命令契约

- availability：`flatpak --version`。
- installed：`flatpak list --app --columns=application,name,version,branch,size,origin,installation,ref`。
- updates 分别执行：
  - `flatpak remote-ls --system --updates --app [--cached] --columns=application,ref,branch,version,commit,origin`
  - `flatpak remote-ls --user --updates --app [--cached] --columns=application,ref,branch,version,commit,origin`
- search 分别执行：
  - `flatpak search --system --columns=application,name,description,version,branch,remotes QUERY`
  - `flatpak search --user --columns=application,name,description,version,branch,remotes QUERY`
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
- 本轮不引入新的 package identity 公共类型或修改 Config V2/UI identity。
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

## Git 提交

本轮实施后逐项记录。

## 验证记录

本轮实施后逐项记录。

## 遗留项 / 下一轮

本轮完成后填写。
