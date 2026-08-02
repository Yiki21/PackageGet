# Iteration 036：Linux 0.3.0-beta.2 发布

- 日期：2026-08-02
- 状态：进行中
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

- [ ] 更新版本、Arch metadata、README与release notes。
- [ ] 通过本地串行质量门禁和版本一致性检查。
- [ ] 推送版本提交并等待最终三平台CI通过。
- [ ] 创建并推送`Build-v0.3.0-beta.2` tag。
- [ ] 验证五个package job、bundle、release与公开资产checksum。
- [ ] 完成迭代记录并推送最终文档提交。

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

## Git提交

- 待记录。

## 验证记录

- 待记录。

## 遗留项 / 下一轮

- Windows/macOS安装包、签名与stable发布继续按ROADMAP阶段6推进。
