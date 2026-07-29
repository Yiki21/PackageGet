# Iteration 012：npm/pnpm 直接迁移与 Global Package Identity

- 日期：2026-07-29
- 状态：已完成
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将npm与pnpm从共享legacy实现迁移为`updater-managers`中的直接实现，冻结global package name、scope、registry origin、installed prefix/store metadata与write target的契约。两者共享Node.js package identity，但CLI参数、JSON shape、退出码与global directory语义必须分别验证，不能用一个宽松`serde_json::Value` parser互相猜测。

## 实施计划

- [x] 分别审计当前stable npm与pnpm的availability、global root/prefix、installed、outdated、search及write命令；允许sub agent并行收集只读证据，集成改动保持线性提交。
- [x] 冻结`builtin:npm`与`builtin:pnpm` descriptor、平台、capabilities、User scope、registry origin和scoped package name grammar。
- [x] 为npm建立typed installed/outdated/search schema、退出码边界、deterministic ordering与malformed/empty response契约。
- [x] 为pnpm建立独立typed installed/outdated/search schema，明确object/array差异、global virtual store/path字段和非零退出行为。
- [x] 明确global package size只从validated package path读取；filesystem error、symlink和package path越界不能静默伪造size。
- [x] 实现refresh-free updates、installed/count/current version、search和统一execute；read/write命令均使用固定timeout且不持锁跨await。
- [x] 冻结npm `install/uninstall -g`与pnpm `add/remove -g`参数；update使用typed target和明确version，不把option-like package name注入argv。
- [x] scoped package如`@scope/name`保留完整identity；禁止把`@`版本分隔和scope前缀混淆，legacy Unknown target仅提供受控兼容。
- [x] 将core npm/pnpm收缩为Config V1、model/progress与typed error wrapper，mixed registry改为direct npm/pnpm且只保留pipx legacy adapter。
- [x] 增加fake npm/pnpm executables、offline JSON fixtures、exit-code、timeout、scope/version、command argv、size boundary、conversion和duplicate registration contracts。
- [x] 执行显式opt-in宿主只读smoke；不执行真实global install、update或remove，也不修改Node.js全局配置。
- [x] 串行通过workspace format、check、test、clippy与build完整门禁，并由GitHub Actions复验。

## Identity 与协议边界

- `PackageInfo.name`与`PackageTarget.name`使用完整registry package name，包括`@scope/name`；`PackageOrigin`必须区分npm registry identity与本机global scope。
- installed、outdated和search来自三个不同CLI协议；缺字段、未知shape或invalid UTF-8返回typed protocol error，不退化成空列表。
- npm/pnpm的“没有可用更新”退出行为必须按各自实测契约处理，不能把任意非零状态或任意可解析stdout当成功。
- pnpm store/virtual store不是可删除target；uninstall只能通过pnpm CLI执行。本轮不直接删除node_modules或store文件。
- search结果的latest metadata与installed version分离；未安装package不伪造`unknown`为真实版本。

## 非目标

- 本轮不迁移pipx，也不管理project-local `package.json`、workspace或lockfile依赖。
- 本轮不切换npm registry、不修改auth token、`.npmrc`、pnpm config或Corepack配置。
- 本轮不执行真实global package写操作。
- 本轮不修改Config V2、UI identity或manager settings页面。
- 本轮不写死Node.js、npm、pnpm或Rust依赖的最低minor/patch版本。

## 设计约束

- 实现保持平铺在`managers/src/`；只有确实被npm和pnpm共同消费的协议代码才提取为同层共享模块，不新增`crates/`或manager分组目录。
- 默认tests完全离线；宿主与live registry测试必须显式opt-in且只读。
- typed schema应按CLI分别定义；共享只限于validated package identity、目标构造和无歧义的通用字段。
- 所有Rust门禁使用`--jobs 1`，tests使用`--test-threads=1`；不得并发运行Cargo build/test。
- toolchain与CI跟随stable channel；manifest使用宽semver line，精确依赖图由`Cargo.lock`固定。

## 进度日志

### 2026-07-29

- Iteration 011已完成Cargo live schema hotfix与Go direct migration；本地完整门禁和GitHub Actions均通过。
- 当前legacy `core/src/pm/npm.rs`同时实现npm/pnpm，并使用宽泛`serde_json::Value`接受object/array；search JSON解析失败会被转换为空结果，协议错误边界需要收紧。
- 当前outdated逻辑在stdout可解析时可能忽略非零status；新实现必须分别冻结npm/pnpm的no-update与failure contract。
- 当前write已区分npm `install/uninstall -g`和pnpm `add/remove -g`，但target仍只有字符串；本轮补齐scoped name、origin、version与option-like input validation。
- 本机只读审计确认npm global inventory为单个root object加package-name keyed dependencies；pnpm为root array加dependencies map。两者都必须保留完整`@scope/name`，不能继续用宽泛object/array猜测同一schema。
- npm无更新为exit 0加`{}`，存在更新时为exit 1加严格outdated object，且同名项可能是one-or-many；只有这两种组合可视为成功。pnpm无更新为exit 0加`{}`，其他非零status不能因stdout可解析而放行。
- npm/pnpm search均返回array且是宽匹配；query必须使用各自明确的参数边界，invalid JSON不能转为空结果。不可达registry会阻塞，所有network read command必须使用kill-on-drop的固定timeout。
- installed path只作为size和local-link证据：npm path必须匹配global root下由完整package name推导的路径；pnpm path可包含global instance/virtual-store布局。path不得进入identity或write target，symlink/link package不得被跟随计算size。
- identity冻结为完整registry package name加User scope。installed origin标记manager global inventory，不武断声称registry；outdated/search origin记录当前配置registry，并使用`package:FULL_NAME` reference。
- write只接受validated registry package name和可选version/tag：拒绝空值、option-like name、额外slash、path/git/file/url/alias spec。scoped name的首个`@`属于scope，版本suffix只能在完整name之后构造。
- `managers/src/npm.rs`与`managers/src/pnpm.rs`已分别实现direct manager及独立typed schema；共享只使用既有command/progress API，没有新增manager分组目录或宽泛JSON抽象。
- npm updates冻结配置registry、完整package name与exact available version；pnpm同样不再把direct update降级为`@latest`。typed install允许validated version或dist-tag，legacy Unknown保留受控latest兼容。
- direct execute在发出Started和执行首个写命令前构造并验证整组commands，随后按package串行有界执行；invalid后续target不会造成前序partial write。
- core npm/pnpm已删除legacy command、directory traversal和parser副本，只保留Config V1、自定义executable、model/progress/error转换；mixed registry目前10个direct manager，仅pipx保留legacy adapter。
- 首次pnpm宿主smoke暴露真实global dependency path是symlink；实现已改为保留identity但不跟随计算size，普通目录仍执行canonical containment和fallible deterministic traversal，并增加离线symlink contract。
- npm与pnpm宿主只读smoke最终均通过availability、inventory/count、outdated与search；未执行任何global write或配置修改。
- npm/pnpm direct migration后的完整本地门禁已串行通过；format、locked workspace check、全部targets tests、workspace clippy `-D warnings`与build均无失败。
- GitHub Actions在direct实现提交与最终验证提交上均通过完整CI；Iteration 012可以关闭。

## Git 提交

- Iteration 012计划检查点：本次提交（`docs: complete Go iteration and plan npm pnpm`）。
- npm/pnpm CLI与identity审计检查点：`b6f705c docs: audit npm pnpm manager contracts`。
- npm/pnpm direct/core migration检查点：`aa8b393 feat: migrate npm pnpm to direct managers`。
- npm/pnpm迁移进度检查点：`6330408 docs: record npm pnpm migration progress`。
- npm/pnpm本地验证检查点：`3c826a1 docs: record npm pnpm migration validation`。

## 验证记录

- npm只读审计：`npm --version`、`node --version`、`npm prefix -g`、`npm root -g`、`npm config get registry`、global list/outdated/search均完成；版本仅为本次宿主证据，不写入最低版本约束。
- pnpm只读审计：`pnpm --version`、resolved global root/bin/store、registry、global list/outdated/search均完成；未执行install、update、remove或配置变更。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test -p updater-managers --test npm_contract --locked --jobs 1 -- --test-threads=1`：9 passed，1 ignored。
- `cargo test -p updater-managers --test pnpm_contract --locked --jobs 1 -- --test-threads=1`：9 passed，1 ignored。
- npm与pnpm host read-only ignored smoke分别显式运行：各1 passed。
- `cargo test -p updater-managers --lib --locked --jobs 1 -- --test-threads=1`：42 passed。
- `cargo test -p updater_core --lib --locked --jobs 1 -- --test-threads=1`：56 passed。
- `cargo test -p updater_core --test builtin_registry --locked --jobs 1 -- --test-threads=1`：11 passed。
- `cargo clippy -p updater-managers -p updater_core --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：全部通过；默认忽略需要宿主工具、容器或live network的显式opt-in smoke。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。
- GitHub Actions `30448205891`（npm/pnpm实现提交）：通过。
- GitHub Actions `30448448820`（最终验证提交）：通过，format、check、deterministic tests、clippy与build全部成功。

## 遗留项 / 下一轮

- npm/pnpm已成为第九和第十个direct manager；mixed registry只剩pipx使用legacy adapter。
- 下一轮进入 [Iteration 013：pipx直接迁移与Venv/Source Identity](013-pipx-direct-migration.md)。
