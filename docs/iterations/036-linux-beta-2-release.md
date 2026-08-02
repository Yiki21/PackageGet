# Iteration 036：Linux 0.3.0-beta.2 发布

- 日期：2026-08-02
- 状态：已完成
- ROADMAP阶段：阶段6 Linux发布物与阶段7 manager增量交付
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

将Iteration 025至035的跨平台manager合同与`uv tool`、`.NET global tools`实现发布为`0.3.0-beta.2` unsigned Linux prerelease；继续产出两个DEB、两个RPM、一个Arch包与`SHA256SUMS`，不提前宣称Windows/macOS安装包或stable完成。

## 范围决策

- workspace、DEB/RPM、Arch、README与release notes统一更新到`0.3.0-beta.2`。
- 发布内容强调manager实现和原生合同；Windows/macOS只有源码级GUI与manager CI准入，本轮仍不提供对应安装包。
- tag只在版本提交的最终Linux/Windows/macOS CI通过后创建。
- 发布后必须从公开GitHub Release重新下载全部资产并执行`sha256sum -c SHA256SUMS`，不能只信任workflow内部artifact。

## 实施计划

- [x] 更新版本、Arch metadata、README与release notes。
- [x] 通过本地串行质量门禁和版本一致性检查。
- [x] 推送版本提交并等待最终三平台CI通过。
- [x] 创建并推送`Build-v0.3.0-beta.2` tag。
- [x] 验证五个package job、bundle、release与公开资产checksum。
- [x] 完成迭代记录并推送最终文档提交。

## 验收标准

- Cargo workspace四个package版本均为`0.3.0-beta.2`。
- DEB/RPM内部版本为`0.3.0~beta.2-1`，Arch版本为`0.3.0beta.2-1`。
- GitHub Release标记为prerelease，`Build-v0.2.4`继续保持Latest stable。
- 公开Release包含两个DEB、两个RPM、一个Arch包和`SHA256SUMS`，六个资产名称与manifest一致。
- 公开下载后的五项package checksum全部通过。

## 进度日志

### 2026-08-02

- Iteration 035完成`.NET global tools` manager；最终CI run `30738497490`在Linux、Windows和macOS全部通过。
- ROADMAP stable门槛仍要求Windows/macOS发布物；当前package workflow只产出Linux五包，因此本轮选择`0.3.0-beta.2` prerelease而非stable。
- workspace四个crate、DEB/RPM/Arch metadata、README与release notes统一更新到`0.3.0-beta.2`；Arch `.SRCINFO`由只读输入目录重新生成并确认与提交内容完全一致。
- 版本提交`660d055`对应CI run `30739756336`在Linux、Windows和macOS全部通过。
- 首轮Package run `30739868922`发现`ui/Cargo.toml`的RPM专用显式版本仍为`0.3.0~beta.1`；两个RPM均已构建，但被beta.2 metadata断言拒绝，因此没有进入bundle或创建Release。
- 将RPM版本同步为`0.3.0~beta.2`，本地重新生成RPM并用workflow同一Bash断言验证；修复提交`63a2a57`对应CI run `30740079174`在三平台全部通过。
- 在确认首轮没有GitHub Release后，将annotated tag `Build-v0.3.0-beta.2`重建到`63a2a57`。第二轮Package run `30740205983`的五个package job、bundle与release全部通过。
- GitHub Release为prerelease，`Build-v0.2.4`继续保持Latest stable。公开Release包含两个DEB、两个RPM、一个Arch包和`SHA256SUMS`。
- 从公开Release重新下载全部六个资产；五个package checksum全部通过，`SHA256SUMS`自身SHA-256为`9c36f96ac260ad2eb240d87c05bf57739afd38840e82a8da4997870a08dfd896`。

## Git提交

- `75fe05d docs: start 0.3.0 beta.2 release iteration`
- `660d055 chore: prepare 0.3.0 beta.2 release`
- `63a2a57 fix: sync rpm prerelease version`

## 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo check --workspace --all-targets --locked --jobs 1`：通过。
- `cargo test --workspace --all-targets --locked --jobs 1 -- --test-threads=1`：227项通过，16项忽略。
- `cargo clippy --workspace --all-targets --locked --jobs 1 -- -D warnings`：通过。
- `cargo build --workspace --locked --jobs 1`：通过。
- `cargo build --release --locked --jobs 1 -p updater`：通过。
- `cargo check --workspace --target x86_64-pc-windows-gnu --locked --jobs 1`：通过。
- Arch `makepkg --printsrcinfo`：与`packaging/arch/.SRCINFO`完全一致。
- 本地RPM实际metadata：`updater 0.3.0~beta.2-1 x86_64`，通过workflow同款版本断言。
- GitHub Actions CI run `30740079174`：Linux、Windows、macOS全部通过。
- GitHub Actions Package run `30740205983`：DEB amd64/arm64、RPM x86_64/aarch64、Arch x86_64、bundle与release全部通过。
- 公开资产回下载后执行`sha256sum -c SHA256SUMS`：五项全部通过。
- DEB metadata：`0.3.0~beta.2-1`，架构分别为`amd64`与`arm64`；RPM metadata：`0.3.0~beta.2-1`，架构分别为`x86_64`与`aarch64`；Arch `.PKGINFO`：`0.3.0beta.2-1 x86_64`。

## 遗留项 / 下一轮

- Windows/macOS安装包、签名与stable发布继续按ROADMAP阶段6推进。
- 本轮release使用`softprops/action-gh-release@v2`时出现Node.js 20兼容性提示，但不影响发布；紧随其后合入的Dependabot PR #1已将其升级到v3，并同步将`actions/checkout`升级到v7。
