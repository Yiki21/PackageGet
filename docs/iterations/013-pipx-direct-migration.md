# Iteration 013：pipx 直接迁移与 Venv/Source Identity

- 日期：2026-07-29
- 状态：进行中
- ROADMAP 阶段：阶段 2——逐个迁移内置 PackageManager
- 开发方式：直接在 `main` 上形成小步、线性的 Git 提交

## 本轮目标

将pipx迁移为`updater-managers`中的最后一个直接built-in实现，明确区分venv name、Python distribution name、`package_or_url`来源与PyPI query identity。旧实现对缺失metadata、PyPI request和filesystem size错误存在跳过或吞错，本轮将其收敛为typed、可重放且可测试的契约。

## 实施计划

- [ ] 审计当前stable pipx的availability、`list --json`、environment paths、upgrade/install/uninstall参数与exit behavior；只读检查本机真实venv inventory。
- [ ] 冻结`builtin:pipx` descriptor、Linux/macOS平台、六项capabilities、User scope与legacy Unknown兼容边界。
- [ ] 冻结venv name、distribution name、normalized PyPI name和`package_or_url`来源identity；同名、case、hyphen/underscore/dot不能无条件折叠。
- [ ] 建立typed `pipx list --json` schema，缺失main package/name/version、duplicate identity与unknown shape不伪造`unknown` package。
- [ ] 解析并验证`PIPX_HOME`与venvs root；size traversal不跟随symlink，path escape、permission和filesystem errors不静默转`None`。
- [ ] 建立typed PyPI JSON client与结构化URL；只将明确404映射为exact lookup无结果，network/status/body/protocol failure必须传播。
- [ ] updates仅查询可重放PyPI distribution；git/path/url/editable来源只读，不因PyPI同名package生成错误update。
- [ ] search明确为PyPI exact identifier lookup，保留canonical distribution name、installed/Not Installed语义、homepage与typed registry origin。
- [ ] 冻结write target：upgrade/uninstall使用validated venv identity，registry install使用distribution加可选exact version；拒绝option/path/url/spec注入。
- [ ] 实现availability、installed/count/current version、updates/search与统一execute，所有read/write固定timeout且整组target预验证后串行执行。
- [ ] 将core pipx收缩为Config V1、model/progress与typed error wrapper；built-in registry全部使用direct managers，不再注册任何legacy adapter。
- [ ] 增加fake pipx、mock PyPI、temporary venv root、source/identity collision、404/status/malformed body、path safety、argv、conversion和registration contracts。
- [ ] 执行显式opt-in宿主与PyPI只读smoke；不执行真实pipx install、upgrade、uninstall或environment修改。
- [ ] 串行通过workspace format、check、test、clippy与build完整门禁，并由GitHub Actions复验。

## Identity 与安全边界

- venv name是pipx upgrade/uninstall target；distribution name是PyPI query与registry install identity。二者可能相同，但不能由实现假定永远相同。
- `package_or_url`必须分类为PyPI distribution、git、URL、path或unknown source；non-registry source不得借同名PyPI metadata伪装为可更新registry package。
- PyPI名称比较遵守Python distribution normalization语义，但保留CLI与registry返回的canonical display name；ambiguous collision返回protocol error。
- `PackageOrigin.reference`使用typed grammar同时保存source kind和必要的venv/distribution identity；不得把本机venv path作为write target。
- uninstall始终调用pipx CLI；本轮不直接删除venv、shared libraries或bin links。

## 非目标

- 本轮不管理project virtualenv、requirements、pip dependency或Python interpreter升级。
- 本轮不修改pipx/PyPI index、proxy、credentials或environment配置。
- 本轮不执行真实pipx写操作。
- 本轮不删除Config V1或UI仍使用的`PackageManagerType`兼容层；legacy adapter/closed dispatcher清理在全部direct built-in稳定后单独迭代。
- 本轮不写死Python、pipx或Rust依赖的最低minor/patch版本。

## 设计约束

- 实现保持平铺在`managers/src/pipx.rs`，不新增`crates/`或manager分组目录。
- 默认tests完全离线，使用fake pipx和本地mock HTTP；宿主/live PyPI smoke必须显式opt-in且只读。
- CLI、filesystem和HTTP分别使用typed boundary，不通过`serde_json::Value`或逐项`let Ok(...) else { continue }`隐藏错误。
- metadata读取默认串行或固定有界；不持锁跨await，不修改调用者全局环境。
- 所有Rust门禁使用`--jobs 1`，tests使用`--test-threads=1`；toolchain与CI跟随stable channel，manifest不写死最低minor/patch版本。

## 进度日志

### 2026-07-29

- Iteration 012已完成npm/pnpm direct migration、真实宿主只读smoke、本地完整门禁与GitHub Actions复验。
- 当前legacy pipx会并发请求PyPI并逐项吞掉失败，search把所有request error都转换为空结果，updates也会跳过单包error；本轮需要区分404、network、status与protocol。
- 当前`pipx list --json`在缺失main package时跳过、缺失name/version时回退venv name/`unknown`；direct schema必须明确required identity，避免伪造registry package。
- 当前write直接使用display package name，而pipx upgrade/uninstall实际target是venv name；本轮必须冻结venv与distribution双identity。
- 本机只读初检确认pipx可用，`PIPX_HOME`与`PIPX_BIN_DIR`可解析，真实inventory包含多个venv；版本只作为审计证据，不进入最低版本约束。

## Git 提交

- Iteration 013计划检查点：本次提交（`docs: complete npm pnpm iteration and plan pipx`）。

## 验证记录

- pipx初步只读检查：availability、environment path和`list --json`成功；未执行install、upgrade或uninstall。

## 遗留项 / 下一轮

本轮完成后填写。
