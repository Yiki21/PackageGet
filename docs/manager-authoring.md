# Package Manager Authoring

Updater使用编译时显式注册的package manager扩展。manager实现依赖`updater-manager-api`中的对象安全trait，不依赖Iced、UI模型、`PackageManagerType`或legacy adapter。

## Crate边界

- `updater-manager-api`：稳定ID、descriptor、配置、package模型、progress与typed error契约。
- `updater-managers`：Updater自带的manager实现，以及`builtin_managers()` catalog。
- `updater_core`：`ManagerRegistry`、duplicate/capability检查和应用组合入口。
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

## 实现要求

- ID使用稳定的小写namespace格式，例如`org.example:packages`；发布后不要复用ID表示另一种manager。
- descriptor只广告已经实现并有测试保护的capability。
- `ManagerConfig.id`和每个`PackageTarget.manager_id`必须在边界验证。
- Config要求`ManagerConfig.settings`为JSON object；manager拥有其内部schema并负责typed解析与校验，core只负责不透明保存。
- manager settings升级必须由manager自身保持兼容；不要把manager私有字段提升为Config顶层字段。
- `PackageInfo.name`与`PackageTarget.name`使用manager真实write identity；展示别名放在metadata/origin中。
- 所有write target先整组验证，再开始命令与progress，防止部分写入。
- manager内部可以批处理或逐项串行，但不能改变core的跨manager串行语义。
- 命令、HTTP和文件系统边界使用固定timeout与结构化`ManagerErrorKind`。
- 默认测试离线；真实宿主或网络smoke必须显式`#[ignore]`且保持只读。

仓库中的可执行外部manager契约测试见`core/tests/manager_registry.rs`，built-in catalog契约见`managers/tests/builtin_catalog.rs`。
Config磁盘schema和失败语义见[`configuration.md`](configuration.md)。
