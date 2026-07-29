# Iteration 011：Go 直接迁移与 Module/Binary Identity

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将 Go 迁移为 `updater-managers` 中第八个直接实现，明确区分 executable binary name、Go module/package install path 与 version query identity。旧实现中遍历 GOBIN、解析 `go version -m`、查询 module versions 时的逐项吞错必须收敛为稳定、typed 且可测试的契约。

## 实施计划

- [x] 审计当前 stable Go 的 `go env GOBIN/GOPATH`、`go version -m`、`go list -m -versions` 与 `go install PACKAGE_PATH@version` 输出和错误行为。
- [x] 冻结 module/package/binary identity、`PackageOrigin` reference、User scope 与 legacy Unknown compatibility grammar。
- [x] 直接实现 descriptor、availability、current version、installed/count、updates、exact module search 与统一 execute。
- [x] 使用 validated build-info parser 保留 module path、package path、binary path/version；malformed 或缺失 metadata 不静默伪造 registry package。
- [x] GOBIN 解析优先 manager setting，再使用 `go env GOBIN`，随后使用第一个 GOPATH/bin；空值、非 UTF-8、多 GOPATH 与 filesystem errors 返回稳定 typed result。
- [x] installed directory traversal 使用确定性排序、普通文件边界和有界 metadata probing；单个 binary probe失败不能被无条件吞掉。
- [x] updates 只查询具有可重放 module identity 的 binary，固定串行上限，明确 devel/pseudo/prerelease/replace metadata 的比较边界。
- [x] exact module search 使用 `go list -m -versions MODULE`，命令失败、empty/no-version 与 malformed output不伪造成通用目录搜索。
- [x] write 冻结 `go install PACKAGE_PATH@VERSION|latest` 与 command-local `GOBIN`；uninstall 只删除已验证属于 configured GOBIN 的 binary target，防止 path traversal/任意文件删除。
- [x] 将 core Go 收缩为 Config V1、`go_bin_dir` setting、model、progress 与 typed error wrapper，并更新 mixed registry。
- [x] 增加 fake Go、temporary GOBIN、build-info/versions fixtures、module/binary collision、path safety、command/env、conversion、registration 与 public contracts。
- [x] 执行显式 opt-in 宿主只读 smoke；不执行真实 install、update 或 binary removal。
- [ ] 串行通过 workspace format、check、test、clippy 与 build完整门禁，并由 GitHub Actions 复验。

## Identity 与安全边界

- `PackageInfo.name` 保留用户可识别的 binary name，module/package path 放入 typed `PackageOrigin.reference`；direct update target必须能恢复准确 module install path。
- 同一 module 可能产生多个 binary，同名 binary 也可能来自不同 module；不得只用末段名称覆盖 identity。
- `PackageScope::User` 表示 configured/resolved GOBIN；Unknown 只用于 Config V1/UI 的旧短名称兼容。
- uninstall target必须先验证 binary basename、GOBIN containment、symlink/目录边界；本轮不提供任意路径删除。
- Go exact module lookup不是 crates.io/npm 风格的目录搜索，UI 展示语义留待阶段 3/5调整。

## 非目标

- 本轮不迁移 npm、pnpm 或 pipx。
- 本轮不管理 Go toolchain、`go.mod` dependency 或 workspace module updates。
- 本轮不修改 Config V2、UI identity 或 manager settings 页面。
- 本轮不执行真实 `go install` 或删除宿主 binary。
- 本轮不写死 Go 或依赖 crate 的最低 minor/patch版本。

## 设计约束

- Go 实现位于平铺的 `managers/src/go.rs`，不新增 `crates/` 或 manager 分组目录。
- 命令输出使用 typed line/parser structures；不以正则或字符串容错隐藏 protocol error。
- 默认 tests 完全离线，使用 fake Go executable 与 temporary GOBIN；宿主 smoke 显式 opt-in且只读。
- 所有 probe/update/write 串行且有固定 timeout；不持锁跨 await，不修改调用者全局环境。
- toolchain 与 CI 跟随 stable channel；manifest 使用宽 semver line，精确依赖图由 `Cargo.lock` 固定。

## 进度日志

### 2026-07-29

- Iteration 010 已完成 direct Cargo、source identity、typed crates.io client、宿主只读 smoke、本地完整门禁；等待 GitHub Actions 最终复验。
- 初步代码审阅确认旧 Go 会吞掉单个 `go version -m` 与 version query failure，GOBIN directory read/entry/file-type error也被转换为空或跳过。
- 旧 uninstall 通过短 name 拼接 GOBIN 后直接删除文件；本轮必须先冻结 typed target并验证 containment与文件边界。
- 开始 Go 审计时收到 Cargo 实际回归：`bluetui` detail endpoint 返回 `cargo registry response is invalid`。live response确认 schema 同时包含 `id` 和 `name`，而 Cargo typed struct 将 `id` alias 到 `name`，Serde 因 duplicate field拒绝官方响应。
- Cargo hotfix 已移除错误 alias，mock fixtures 改为官方 `name` 字段；HTTP client 将 body transport读取与 JSON protocol解析拆开，并对幂等 GET 的瞬时 body failure执行一次100ms有界重试。
- `bluetui` exact detail live smoke、Cargo离线 contracts、workspace check与全部默认 tests均通过；修复不把 malformed JSON重试成成功，也不吞掉非成功 HTTP status。
- 本机 Go 1.26.4 的 `go env -json` 返回非默认 GOBIN `/home/ayi/.asdf/installs/golang/1.26.4/bin` 与 GOPATH `/home/ayi/go`；direct resolver必须尊重 Go 自己解析后的 GOBIN，不能假设 `$GOPATH/bin`。
- `go version -m -json ABSOLUTE_BINARY` 已验证为稳定 typed boundary：顶层 `Path` 是 main package install path，`Main.Path/Main.Version` 是 module identity/version，`Main.Replace` 表示 replacement；本轮不再用 regex解析 text output。
- identity 冻结为 binary display name + `PackageOrigin.name=MODULE_PATH` + `PackageOrigin.reference=package:PACKAGE_PATH` + User scope。`cmd/stringer` 类 package path与module path不同，二者必须同时保留。
- `go list -m -versions -json MODULE` 返回 typed module path与版本列表；latest update查询使用 `go list -m -json MODULE@latest` 的 `Version`，避免仅取文本最后一个 token。devel、replace、missing module version不产生可重放的registry update。
- write 冻结为 command-local `GOBIN=RESOLVED_BIN_DIR go install PACKAGE_PATH@VERSION|latest`；scoped uninstall只允许validated binary basename且必须位于resolved GOBIN内，拒绝路径分隔符、symlink与目录。
- `managers/src/go.rs` 已完成 direct implementation，read path统一使用 `go env -json`、`go version -m -json`、`go list -m ... -json` typed schema，不再保留 regex/text parser。
- installed inventory只枚举GOBIN直接 regular files并排序；明确的 `not a Go executable` 可跳过，其他command/timeout/JSON/filesystem failure均传播typed error。
- updates使用 `MODULE@latest` typed response与 semver 1.x比较，只在available大于installed时产生update；devel与replacement build只读展示，不产生可能降级或错误来源的update。
- direct target使用binary display name、module origin与 `package:PACKAGE_PATH` reference；legacy binary update会先通过inventory唯一解析真实package path，不再生成错误的 `go install BINARY@latest`。
- uninstall在删除前重新解析当前inventory、核对typed origin、basename、regular file、canonical GOBIN parent与symlink边界；Unknown短名同样不能绕过inventory。
- core Go已删除regex、directory traversal与command副本，只保留Config V1、`go_bin_dir` setting、legacy model/progress和typed error转换；mixed registry当前为8个direct manager、3个legacy adapter。
- 本机opt-in smoke已通过Go availability、真实GOBIN的gopls/gup/kind build-info与installed/count parity，未执行network query或任何写操作。

## Git 提交

- Cargo回归修复检查点：`3d9be46 fix: decode crates.io package metadata`。
- Go CLI/identity审计检查点：`787fd0d docs: audit Go manager contracts`。
- Go direct/core migration检查点：`4d92ba2 feat: migrate Go to direct manager`。

## 验证记录

- `cargo test -p updater-managers --test cargo_contract --jobs 1 -- --test-threads=1`：9 passed，2 ignored。
- `cargo test -p updater-managers --test cargo_contract crates_io_bluetui_detail_read_only_smoke --jobs 1 -- --ignored --exact --test-threads=1 --nocapture`：1 passed。
- `cargo clippy -p updater-managers --all-targets --jobs 1 -- -D warnings`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：通过。
- `go version go1.26.4 linux/amd64`；版本仅为本次审计证据，不写入最低版本约束。
- `go env GOBIN GOPATH GOMODCACHE GOPROXY`：只读成功。
- `go version -m -json`：对本机 gopls/gup/kind只读成功，确认 package/module/version schema。
- `go list -m -versions -json github.com/nao1215/gup` 与 `go list -m -json github.com/nao1215/gup@latest`：只读成功。
- `cargo test -p updater-managers --test go_contract --jobs 1 -- --test-threads=1`：7 passed，1 ignored。
- `cargo test -p updater-managers --lib --jobs 1 -- --test-threads=1`：41 passed。
- `cargo test -p updater-managers --test go_contract host_go_read_only_smoke_is_explicitly_opt_in --jobs 1 -- --ignored --exact --test-threads=1 --nocapture`：1 passed。
- `cargo test -p updater_core --lib --jobs 1 -- --test-threads=1`：61 passed。
- `cargo test -p updater_core --test builtin_registry --jobs 1 -- --test-threads=1`：9 passed。
- `cargo clippy -p updater-managers -p updater_core --all-targets --jobs 1 -- -D warnings`：通过。

## 遗留项 / 下一轮

本轮完成后填写。
