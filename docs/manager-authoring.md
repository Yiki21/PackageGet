# Package Manager Authoring

Updater使用编译时显式注册的package manager扩展。manager实现依赖`updater-manager-api`中的对象安全trait，不依赖Iced、UI模型、`PackageManagerType`或legacy adapter。

## Crate边界

- `updater-manager-api`：稳定ID、descriptor、配置、package模型、progress与typed error契约。
- `updater-managers`：Updater自带的manager实现，以及`builtin_managers()` catalog。
- `updater_core`：`ManagerRegistry`、duplicate/capability检查、配置检测和跨manager串行执行。
- 第三方manager crate：实现`PackageManager`，由最终应用显式注册。

同一workspace中的第三方实现可以继承workspace依赖，不需要建立额外的`crates/`分组目录：

```toml
[dependencies]
async-trait.workspace = true
updater-manager-api.workspace = true
```

## 最小实现

下面的manager只声明availability和exact search，因此descriptor也只广告`Search`。未广告的操作会保留公共trait提供的typed `Unsupported`默认行为。

```rust
use async_trait::async_trait;
use updater_manager_api::{
    AuthorizationHint, ManagerAvailability, ManagerCapabilities, ManagerCapability,
    ManagerCategory, ManagerConfig, ManagerDescriptor, ManagerId, ManagerResult,
    PackageInfo, PackageManager, PackageScope, Platform, SupportedPlatforms,
};

#[derive(Debug)]
pub struct ExampleManager {
    descriptor: ManagerDescriptor,
}

impl ExampleManager {
    pub fn new() -> Self {
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse("org.example:packages")
                .expect("static manager ID must remain valid"),
            "Example Packages",
            ManagerCategory::Development,
            SupportedPlatforms::from([Platform::Linux]),
            ManagerCapabilities::from([ManagerCapability::Search]),
        )
        .expect("static descriptor must remain valid")
        .with_description("Exact package lookup for the example registry")
        .with_authorization(AuthorizationHint::None);
        Self { descriptor }
    }
}

#[async_trait]
impl PackageManager for ExampleManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(
        &self,
        _config: &ManagerConfig,
    ) -> ManagerResult<ManagerAvailability> {
        Ok(ManagerAvailability::Available { version: None })
    }

    async fn search(
        &self,
        _config: &ManagerConfig,
        query: &str,
    ) -> ManagerResult<Vec<PackageInfo>> {
        let mut package = PackageInfo::new(
            self.descriptor.id().clone(),
            query,
            "Not Installed",
        );
        package.scope = PackageScope::User;
        Ok(vec![package])
    }
}
```

静态ID和descriptor构造失败代表代码中的不变量被破坏，因此示例只在这些常量上使用`expect`。命令、网络、配置、解析和文件系统错误必须返回`ManagerError`，不能panic或静默转换为空结果。

## 显式注册

应用组合层依赖第三方crate与`updater_core`，并把实例注册为对象安全trait object：

```rust
use std::sync::Arc;

use example_manager::ExampleManager;
use updater_core::ManagerRegistry;

let mut registry = ManagerRegistry::new();
registry.register(Arc::new(ExampleManager::new()))?;
```

`ManagerRegistry::register`拒绝重复`ManagerId`。调用前使用`manager_for(id, capability)`做capability检查；不要通过display name、Rust类型名或闭合enum分发。

内置manager由`updater_managers::builtin_managers()`提供，并由`updater_core::register_builtin_managers`注册。第三方manager不应修改built-in catalog，而应在应用组合层追加注册。

## UI identity与缺失manager

UI只使用`ManagerId`作为state、message、selection和operation outcome中的manager identity。display name、description、category、platform、capability与authorization均从已注册manager的`ManagerDescriptor`读取，不能把display name当作key。

Config中的unknown manager ID不会被过滤。当前build未注册对应实现时，Settings仍显示稳定ID与unavailable状态，保存时也保留原`ManagerConfig`；只有用户显式移除时才删除。第三方manager接入最终应用后，应同时加入该应用创建UI catalog所使用的registry。

UI catalog与执行任务共享同一个`ManagerRegistry`。已注册且已配置的第三方trait object可以直接参与availability、installed/count、updates、search与execute；调用前由registry拒绝unknown ID或未广告的capability。跨manager写操作通过`updater_core::execute_package_groups`保持输入顺序、首次失败停止、部分结果和组间协作取消，不需要也不允许增加闭合enum映射。

## 实现要求

- ID使用稳定的小写namespace格式，例如`org.example:packages`；发布后不要复用ID表示另一种manager。
- descriptor只广告已经实现并有测试保护的capability。
- `ManagerConfig.id`和每个`PackageTarget.manager_id`必须在边界验证。
- Config要求`ManagerConfig.settings`为JSON object；manager拥有其内部schema并负责typed解析与运行时校验。带一等Settings UI的built-in可以在core额外冻结必填持久化不变量，例如Nix的单一绝对user profile；其他manager-private字段仍由core不透明保存。
- manager settings升级必须由manager自身保持兼容；不要把manager私有字段提升为Config顶层字段。
- `PackageInfo.name`与`PackageTarget.name`使用manager真实write identity；展示别名放在metadata/origin中。
- `package_info`是可选的按需只读详情扩展点；只有能以低副作用、稳定结构化输出提供 richer metadata 的 manager 才实现它，不能在 installed/updates 列表加载时顺带执行单包详情命令。
- 所有write target先整组验证，再开始命令与progress，防止部分写入。
- manager内部可以批处理或逐项串行，但不能改变core的跨manager串行语义。
- 命令、HTTP和文件系统边界使用固定timeout与结构化`ManagerErrorKind`。
- 默认测试离线；真实宿主或网络smoke必须显式`#[ignore]`且保持只读。

## 新 manager 接入清单

按以下顺序接入一个新 manager，避免把实现细节扩散到 core 或 UI：

1. 先确定不可变的 namespaced `ManagerId`、支持平台、category、authorization 和实际 capability；descriptor 只广告已经实现的操作。
2. 在 `updater-managers/src/<manager>.rs` 中实现 `PackageManager`。先调用 `config.validate_for(self.descriptor())`，再由 manager 自己解析并校验 `settings`；不要新增 `Config` 顶层字段，也不要在 core 增加该 manager 的 ID 分支。
3. 对 manager-private settings 提供同模块的 typed getter/setter（如果 UI 需要编辑），并为 malformed settings、wrong identity、path/scope/origin 约束写单测。
4. `availability` 必须通过 `managers::command::manager_availability`（或带自定义版本解析的同一入口）执行平台检查；公共入口会在任何命令探测前根据 descriptor 返回结构化 `UnsupportedPlatform`，不能依赖 catalog 过滤掩盖错误。
5. 添加 `managers/tests/<manager>_contract.rs`，覆盖 descriptor、空输入、命令 argv、结构化输出解析、写操作 target 冻结和 capability 边界。网络或宿主 CLI smoke 只能是显式 ignored 的只读测试。
6. 将实例加入 `builtin_managers()` 和 catalog contract；不要修改通用 catalog 测试去强制 manager 声明不存在的 CRUD 能力。
7. 只有存在真正的 manager-private 设置时才添加 UI 控件；控件直接调用该 manager 的 typed getter/setter。普通 manager 不需要新增 UI helper、enum 分发或通用动态表单。
8. 如果实现 `package_info`，为单包 query、target identity mismatch、未安装包和 malformed output 添加离线契约测试；UI 会在用户选择包后异步调用，并在失败时提供 retry。
9. 完成 `cargo fmt`、workspace check/test/clippy 后，再补 README、ROADMAP 和 release notes 中的能力与平台说明。

仓库中的可执行外部manager契约测试见`core/tests/manager_registry.rs`，跨manager执行契约见`core/tests/execution.rs`，built-in catalog契约见`managers/tests/builtin_catalog.rs`。
Config磁盘schema和失败语义见[`configuration.md`](configuration.md)。
