# Iteration 057: Shared Manager Availability Platform Contract

- 日期：2026-08-04
- 状态：已完成
- ROADMAP阶段：阶段7——扩展 Package Manager生态

## 目标

让新增 package manager 遵守平台 availability 契约时只有一个真实入口，避免 descriptor 已声明平台范围、实现却忘记 runtime guard 的漂移。

## 实施

- `manager_availability` 与 `manager_availability_with_version` 现在接收 `ManagerDescriptor`。
- 共享入口在解析可执行文件、读取自定义路径 metadata 或启动版本命令前检查当前平台，并返回 `UnsupportedPlatform`。
- 普通 manager 的重复 `cfg!`/`unsupported_platform` availability 分支已移除；Nix 在必填 profile 校验前复用同一个平台 policy，确保不支持平台不会被误报为配置错误。
- authoring 文档把共享入口列为新增 manager 的必需路径，并补充“先平台检查、后 I/O”的回归测试。

## 验证

- `cargo fmt --all -- --check`
- `cargo check -p updater-managers --all-targets --locked --jobs 1`
- `cargo test -p updater-managers --all-targets --locked --jobs 1 -- --test-threads=1`
- `cargo clippy -p updater-managers --all-targets --locked --jobs 1 -- -D warnings`

测试覆盖共享入口在不支持平台时不启动命令探测；既有各 manager 的平台 contract 继续保持离线和真实 smoke 分离。
