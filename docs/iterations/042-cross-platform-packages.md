# Iteration 042：Windows/macOS 原生发布物

- 日期：2026-08-03
- 状态：进行中
- ROADMAP阶段：阶段6跨平台发布物
- 开发方式：直接在`main`上形成小步、线性的Git提交

## 本轮目标

在不改变现有Linux五包产线的前提下，为Windows x86_64交付portable ZIP与per-user installer，为macOS arm64/x86_64交付标准`.app`归档与`.dmg`；所有产物进入同一个checksum manifest和tag Release，并准确标注unsigned限制。

## 范围决策

- Windows使用GitHub Windows runner已提供的Inno Setup构建per-user installer，不要求管理员权限，不把未签名产物描述为已通过SmartScreen信任。
- Windows二进制嵌入仓库图标、产品名、文件描述和Cargo package版本；installer使用稳定`com.ayi.updater` identity。
- macOS在arm64与Intel原生runner上分别构建，不用Linux交叉编译替代原生链接；`.app`包含`Info.plist`、`.icns`、bundle ID和最低macOS版本。
- GitHub Release上传`.app.zip`而不是裸目录，同时上传对应DMG；两种形式都从同一个已验证`.app`生成。
- 本轮不配置Windows code signing、Apple Developer ID signing或notarization；这些需要证书、账户与CI secrets，是stable发布前的独立凭据门槛。
- 本轮不修改Activity schema、失败计划重试或当前manager事务取消语义。
- 同轮响应用户报告的紧凑侧边栏回归：Updates badge不再横向挤压SVG，固定图标层保持与其他导航项同尺寸并居中；不扩大到完整阶段5视觉重构。

## 实施计划

- [x] 增加Windows应用资源与Inno Setup定义。
- [x] 增加macOS app bundle、icon与DMG构建脚本。
- [x] 修复紧凑侧边栏Updates图标被badge压缩并偏移的问题。
- [x] 扩展Package workflow的Windows/macOS矩阵、结构校验、bundle checksum和Release上传。
- [x] 更新README、ROADMAP发布检查点和迭代索引。
- [x] 通过本地脚本检查与串行Rust质量门禁。
- [ ] 推送并通过原生Windows/macOS Package jobs后完成本迭代。

## 验收标准

- Windows release输入包含一个x86_64 portable ZIP和一个installer EXE，ZIP内的`updater.exe`具有应用图标与版本资源。
- macOS每个架构包含一个`.app.zip`和一个DMG；bundle ID为`com.ayi.updater`，版本与Cargo一致，Mach-O架构与文件名一致。
- bundle job严格验证5个Linux包、2个Windows产物、4个macOS产物，并为全部11项生成`SHA256SUMS`。
- tag Release上传全部11个产物和checksum manifest，README明确说明unsigned启动限制。

## 进度日志

### 2026-08-03

- `Build-v0.3.0-beta.3`已验证Linux五包，但ROADMAP stable完成标准仍缺Windows/macOS发布物。
- GitHub runner-images公开清单确认当前`windows-latest`提供Inno Setup；Windows使用仓库内由512px PNG生成的多尺寸ICO，macOS使用系统`iconutil`、`plutil`、`ditto`与`hdiutil`。
- 紧凑侧边栏改为固定28×28图标层承载16px SVG，badge叠加到右上角而不参与横向布局；`cargo test -p updater --bin updater --locked --jobs 1 -- --test-threads=1`的49项测试通过。当前会话未暴露Agent Workspace MCP调用接口，因此尚未取得隔离桌面截图证据。
- 本地`cargo fmt`、workspace check/test/clippy/build和Windows GNU workspace check均通过；全workspace确定性测试为49+5+6+74项单元测试及各manager/core契约测试通过，预期opt-in smoke保持ignored。
- `actionlint v1.7.12`、`shellcheck packaging/macos/package.sh`、release metadata验证与`git diff --check`通过；锁文件只加入Windows resource所需依赖，没有顺带升级既有`windows-sys`解析。
- 首轮Package run `30780508822`确认Windows release binary与资源可原生编译，但Inno拒绝把SemVer prerelease字符串写入数字`VersionInfoProductVersion`；installer PE版本改用`0.3.0.0`形式并增加对应断言，应用自身ProductVersion仍保留完整Cargo版本。

## 遗留项 / 下一轮

- 等待本轮原生Package CI结果后记录run、产物结构与公开checksum证据。
- 1.0前继续完成阶段5 Activity/失败恢复与真实process lifecycle取消，并确定签名/notarization的发布政策与凭据。
