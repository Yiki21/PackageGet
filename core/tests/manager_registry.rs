use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use updater_core::{ManagerRegistry, RegistryError};
use updater_manager_api::{
    ManagerAvailability, ManagerCapabilities, ManagerCapability, ManagerCategory, ManagerConfig,
    ManagerDescriptor, ManagerId, ManagerResult, PackageAction, PackageInfo, PackageManager,
    PackageTarget, Platform, ProgressEvent, ProgressSink,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutionRecord {
    action: PackageAction,
    packages: Vec<PackageTarget>,
}

struct FakeManager {
    descriptor: ManagerDescriptor,
    executions: Arc<Mutex<Vec<ExecutionRecord>>>,
}

impl FakeManager {
    fn new(
        id: &str,
        display_name: &str,
        capabilities: impl Into<ManagerCapabilities>,
    ) -> (Self, Arc<Mutex<Vec<ExecutionRecord>>>) {
        let executions = Arc::new(Mutex::new(Vec::new()));
        let descriptor = ManagerDescriptor::new(
            ManagerId::parse(id).expect("valid fake manager ID"),
            display_name,
            ManagerCategory::Development,
            [Platform::Linux],
            capabilities,
        )
        .expect("valid fake manager descriptor")
        .with_description("External fake manager used by the registry contract test");

        (
            Self {
                descriptor,
                executions: Arc::clone(&executions),
            },
            executions,
        )
    }
}

#[async_trait]
impl PackageManager for FakeManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, _config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        Ok(ManagerAvailability::Available {
            version: Some("1.0.0".to_owned()),
        })
    }

    async fn search(
        &self,
        _config: &ManagerConfig,
        query: &str,
    ) -> ManagerResult<Vec<PackageInfo>> {
        Ok(vec![PackageInfo::new(
            self.descriptor.id().clone(),
            query,
            "1.0.0",
        )])
    }

    async fn execute(
        &self,
        _config: &ManagerConfig,
        action: PackageAction,
        packages: &[PackageTarget],
        progress: &dyn ProgressSink,
    ) -> ManagerResult<()> {
        progress.emit(ProgressEvent::Started {
            action,
            total: packages.len(),
        });
        progress.emit(ProgressEvent::Finished {
            completed: packages.len(),
            total: packages.len(),
        });

        self.executions
            .lock()
            .expect("execution record lock")
            .push(ExecutionRecord {
                action,
                packages: packages.to_vec(),
            });

        Ok(())
    }
}

#[tokio::test]
async fn external_manager_can_register_and_run_through_trait_object() {
    let id = ManagerId::parse("org.example:fake").expect("valid manager ID");
    let (fake, executions) = FakeManager::new(
        id.as_str(),
        "Example Manager",
        [ManagerCapability::Search, ManagerCapability::Install],
    );

    let manager: Arc<dyn PackageManager> = Arc::new(fake);
    let mut registry = ManagerRegistry::new();
    registry.register(manager).expect("register fake manager");

    let config = ManagerConfig::new(id.clone());
    let searchable = registry
        .manager_for(&id, ManagerCapability::Search)
        .expect("search capability");
    assert!(
        searchable
            .availability(&config)
            .await
            .expect("availability check")
            .is_available()
    );

    let results = searchable
        .search(&config, "demo-package")
        .await
        .expect("search packages");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].manager_id, id);
    assert_eq!(results[0].name, "demo-package");

    let installable = registry
        .manager_for(&id, ManagerCapability::Install)
        .expect("install capability");
    let target = PackageTarget::new(id.clone(), "demo-package");
    let events = Arc::new(Mutex::new(Vec::new()));
    let progress = {
        let events = Arc::clone(&events);
        move |event| events.lock().expect("progress event lock").push(event)
    };

    installable
        .execute(
            &config,
            PackageAction::Install,
            std::slice::from_ref(&target),
            &progress,
        )
        .await
        .expect("execute fake install");

    assert_eq!(
        *executions.lock().expect("execution record lock"),
        vec![ExecutionRecord {
            action: PackageAction::Install,
            packages: vec![target],
        }]
    );

    let events = events.lock().expect("progress event lock");
    assert!(matches!(
        events.first(),
        Some(ProgressEvent::Started {
            action: PackageAction::Install,
            total: 1,
        })
    ));
    assert!(matches!(
        events.last(),
        Some(ProgressEvent::Finished {
            completed: 1,
            total: 1,
        })
    ));
}

#[test]
fn registry_rejects_duplicates_and_unsupported_capabilities() {
    let id = ManagerId::parse("org.example:duplicate").expect("valid manager ID");
    let (first, _) = FakeManager::new(id.as_str(), "First", [ManagerCapability::Installed]);
    let (second, _) = FakeManager::new(id.as_str(), "Second", [ManagerCapability::Installed]);

    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(first))
        .expect("register first manager");

    assert!(matches!(
        registry.register(Arc::new(second)),
        Err(RegistryError::DuplicateManager { id: duplicate }) if duplicate == id
    ));
    assert!(matches!(
        registry.manager_for(&id, ManagerCapability::Update),
        Err(RegistryError::UnsupportedCapability {
            id: unsupported,
            capability: ManagerCapability::Update,
        }) if unsupported == id
    ));
}

#[test]
fn registry_orders_managers_by_descriptor_metadata() {
    let (zulu, _) = FakeManager::new(
        "org.example:zulu",
        "Zulu Manager",
        [ManagerCapability::Installed],
    );
    let (alpha, _) = FakeManager::new(
        "org.example:alpha",
        "Alpha Manager",
        [ManagerCapability::Installed],
    );

    let mut registry = ManagerRegistry::new();
    registry.register(Arc::new(zulu)).expect("register zulu");
    registry.register(Arc::new(alpha)).expect("register alpha");

    let names: Vec<_> = registry
        .managers()
        .into_iter()
        .map(|manager| manager.descriptor().display_name().to_owned())
        .collect();
    assert_eq!(names, ["Alpha Manager", "Zulu Manager"]);
}
