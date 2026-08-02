# Iteration 041：Linux 0.3.0-beta.3 发布

- 日期：2026-08-02
- 状态：已完成
- ROADMAP阶段：阶段6 Linux发布物与阶段7 manager增量交付
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将Iteration 037至040的Snap、RubyGems、Composer Global与Nix profile实现发布为`0.3.0-beta.3` unsigned Linux prerelease；继续产出两个DEB、两个RPM、一个Arch包与`SHA256SUMS`，不提前宣称Windows/macOS安装包或stable完成。

## 范围决策

- `Build-v0.3.0-beta.2`已存在且早于当前21个提交，不能移动或复用；本轮使用新版本与新tag `Build-v0.3.0-beta.3`。
- workspace、DEB/RPM、Arch、README与release notes统一更新到`0.3.0-beta.3`。
- Package workflow增加统一metadata preflight，在构建前验证tag、Cargo workspace、RPM、Arch、README与release notes版本一致。
- Arch optional dependencies同步本轮新增manager；DEB/RPM不添加跨发行版名称不稳定的弱依赖。
- tag只在版本提交的最终Linux/Windows/macOS CI通过后创建。
- 发布后从公开GitHub Release重新下载全部资产并执行`sha256sum -c SHA256SUMS`，不能只信任workflow内部artifact。

## 实施计划

- [x] 更新版本、Arch metadata、README与release notes。
- [x] 增加并验证Package metadata preflight。
- [x] 通过本地串行质量门禁、版本一致性和本地打包检查。
- [x] 推送版本提交并等待最终三平台CI通过。
- [x] 创建并推送`Build-v0.3.0-beta.3` annotated tag。
- [x] 验证五个package job、bundle、release与公开资产checksum。
- [x] 完成迭代记录并推送最终文档提交。

## 验收标准

- Cargo workspace四个package版本均为`0.3.0-beta.3`。
- DEB/RPM内部版本为`0.3.0~beta.3-1`，Arch版本为`0.3.0beta.3-1`。
- GitHub Release标记为prerelease，`Build-v0.2.4`继续保持Latest stable。
- 公开Release包含两个DEB、两个RPM、一个Arch包和`SHA256SUMS`，六个资产名称与manifest一致。
- 公开下载后的五项package checksum全部通过。

## 进度日志

### 2026-08-02

- Iteration 040完成Nix profile manager；最终CI run `30749503577`在Linux、Windows和macOS全部通过。
- `Build-v0.3.0-beta.2`指向`63a2a57`，当前HEAD在其后21个提交；公开beta.2仍保持原样，不重写历史tag。
- ROADMAP stable门槛仍要求Windows/macOS发布物；当前Package workflow只产出Linux五包，因此本轮继续发布prerelease。
- 由于当前GitHub PAT没有workflow dispatch权限，`gh workflow run package.yml --ref main`返回403；正式tag触发的Package workflow仍完成了等价的metadata、五包、bundle与release验证。

## Git提交

- `ad48ffa release: prepare 0.3.0 beta.3`：版本、发布说明、Arch依赖与Package metadata preflight。
- `Build-v0.3.0-beta.3` annotated tag指向`ad48ffa`。

## 验证记录

- 本地：`cargo fmt --all -- --check`、workspace check/test/clippy/build、release build、Windows GNU check均通过；串行测试49+5+6+74个单元测试及契约测试通过，预期ignored保持未执行。
- 本地包：DEB/RPM内部版本均为`0.3.0~beta.3-1`；Arch `.SRCINFO`由Arch容器`makepkg --printsrcinfo`重现一致。
- 主CI：run `30750706138` 的Linux、Windows、macOS三平台全部通过；主CI run `30750706138`对应候选提交`ad48ffa`。
- Package：run `30750862306` 的`validate-metadata`、两个DEB、两个RPM、Arch、bundle、release全部通过；Arch job耗时21分07秒。
- 公网Release：`Build-v0.3.0-beta.3`为非draft prerelease，公开下载六个资产后`sha256sum -c SHA256SUMS`通过；DEB/RPM/Arch内部版本分别为`0.3.0~beta.3-1`、`0.3.0beta.3-1`。
- stable复核：`Build-v0.2.4`继续是Latest stable；本轮没有移动或覆盖既有tag。

## 遗留项 / 下一轮

- Windows/macOS安装包、签名与stable发布继续按ROADMAP阶段6推进；下一轮应优先评估native安装包产线与签名门槛。
