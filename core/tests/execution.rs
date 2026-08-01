use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use updater_core::{
    CancellationToken, Config, ManagerConfig, ManagerRegistry, OperationProgress,
    execute_package_groups,
};
use updater_manager_api::{
    ManagerAvailability, ManagerCapabilities, ManagerCapability, ManagerCategory,
    ManagerDescriptor, ManagerError, ManagerErrorKind, ManagerId, ManagerResult, PackageAction,
    PackageManager, PackageOrigin, PackageScope, PackageTarget, Platform, ProgressEvent,
    ProgressSink,
};

struct FakeManager {
    descriptor: ManagerDescriptor,
    execution_order: Arc<Mutex<Vec<ManagerId>>>,
    received_targets: Option<Arc<Mutex<Vec<PackageTarget>>>>,
    fail_after: Option<usize>,
}

impl FakeManager {
    fn new(
        id: &str,
        capabilities: impl Into<ManagerCapabilities>,
        execution_order: Arc<Mutex<Vec<ManagerId>>>,
        fail_after: Option<usize>,
    ) -> Self {
        Self {
            descriptor: ManagerDescriptor::new(
                ManagerId::parse(id).expect("valid fake manager ID"),
                id,
                ManagerCategory::Development,
                [Platform::Linux],
                capabilities,
            )
            .expect("valid fake manager descriptor"),
            execution_order,
            received_targets: None,
            fail_after,
        }
    }

    fn with_target_log(mut self, received_targets: Arc<Mutex<Vec<PackageTarget>>>) -> Self {
        self.received_targets = Some(received_targets);
        self
    }
}

#[async_trait]
impl PackageManager for FakeManager {
    fn descriptor(&self) -> &ManagerDescriptor {
        &self.descriptor
    }

    async fn availability(&self, _config: &ManagerConfig) -> ManagerResult<ManagerAvailability> {
        Ok(ManagerAvailability::Available { version: None })
    }

    async fn execute(
        &self,
        _config: &ManagerConfig,
        action: PackageAction,
        packages: &[PackageTarget],
        progress: &dyn ProgressSink,
    ) -> ManagerResult<()> {
        self.execution_order
            .lock()
            .expect("execution order lock")
            .push(self.descriptor.id().clone());
        if let Some(received_targets) = &self.received_targets {
            received_targets
                .lock()
                .expect("received targets lock")
                .extend_from_slice(packages);
        }
        progress.emit(ProgressEvent::Started {
            action,
            total: packages.len(),
        });

        if let Some(completed) = self.fail_after {
            progress.emit(ProgressEvent::Advanced {
                completed,
                total: packages.len(),
                current_package: packages.get(completed).map(|package| package.name.clone()),
            });
            return Err(ManagerError::new(
                ManagerErrorKind::Other,
                "fake manager failed",
            ));
        }

        progress.emit(ProgressEvent::Finished {
            completed: packages.len(),
            total: packages.len(),
        });
        Ok(())
    }
}

fn manager_id(value: &str) -> ManagerId {
    ManagerId::parse(value).expect("valid test manager ID")
}

fn target(manager: &ManagerId, name: &str) -> PackageTarget {
    PackageTarget::new(manager.clone(), name)
}

fn config(ids: &[ManagerId]) -> Config {
    Config {
        managers: ids.iter().cloned().map(ManagerConfig::new).collect(),
        ..Config::default()
    }
}

#[tokio::test]
async fn executes_groups_in_supplied_order() {
    let first = manager_id("org.example:first");
    let second = manager_id("org.example:second");
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(FakeManager::new(
            first.as_str(),
            [ManagerCapability::Install],
            Arc::clone(&order),
            None,
        )))
        .unwrap();
    registry
        .register(Arc::new(FakeManager::new(
            second.as_str(),
            [ManagerCapability::Install],
            Arc::clone(&order),
            None,
        )))
        .unwrap();

    let outcome = execute_package_groups(
        &registry,
        &config(&[first.clone(), second.clone()]),
        PackageAction::Install,
        &[
            (second.clone(), vec![target(&second, "beta")]),
            (first.clone(), vec![target(&first, "alpha")]),
        ],
        &CancellationToken::default(),
        &|_| {},
    )
    .await;

    assert!(outcome.is_success());
    assert_eq!(outcome.completed_packages, 2);
    assert_eq!(*order.lock().unwrap(), vec![second, first]);
}

#[tokio::test]
async fn preserves_manager_owned_target_identity() {
    let id = manager_id("org.example:scoped");
    let order = Arc::new(Mutex::new(Vec::new()));
    let received_targets = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(
            FakeManager::new(
                id.as_str(),
                [ManagerCapability::Install],
                Arc::clone(&order),
                None,
            )
            .with_target_log(Arc::clone(&received_targets)),
        ))
        .unwrap();
    let mut package = target(&id, "alpha");
    package.scope = PackageScope::User;
    package.origin = Some(PackageOrigin::new("stable").with_reference("stable-id"));

    let outcome = execute_package_groups(
        &registry,
        &config(std::slice::from_ref(&id)),
        PackageAction::Install,
        &[(id, vec![package.clone()])],
        &CancellationToken::default(),
        &|_| {},
    )
    .await;

    assert!(outcome.is_success());
    assert_eq!(*received_targets.lock().unwrap(), vec![package]);
}

#[tokio::test]
async fn unsupported_capability_stops_before_manager_execution() {
    let id = manager_id("org.example:read-only");
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(FakeManager::new(
            id.as_str(),
            [ManagerCapability::Installed],
            Arc::clone(&order),
            None,
        )))
        .unwrap();

    let outcome = execute_package_groups(
        &registry,
        &config(std::slice::from_ref(&id)),
        PackageAction::Update,
        &[(id.clone(), vec![target(&id, "alpha")])],
        &CancellationToken::default(),
        &|_| {},
    )
    .await;

    assert_eq!(outcome.failed_manager, Some(id));
    assert!(
        outcome
            .error
            .unwrap()
            .contains("does not support package updates")
    );
    assert!(order.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failure_reports_partial_progress_and_skips_later_groups() {
    let failing = manager_id("org.example:failing");
    let skipped = manager_id("org.example:skipped");
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ManagerRegistry::new();
    registry
        .register(Arc::new(FakeManager::new(
            failing.as_str(),
            [ManagerCapability::Update],
            Arc::clone(&order),
            Some(1),
        )))
        .unwrap();
    registry
        .register(Arc::new(FakeManager::new(
            skipped.as_str(),
            [ManagerCapability::Update],
            Arc::clone(&order),
            None,
        )))
        .unwrap();

    let outcome = execute_package_groups(
        &registry,
        &config(&[failing.clone(), skipped.clone()]),
        PackageAction::Update,
        &[
            (
                failing.clone(),
                vec![target(&failing, "alpha"), target(&failing, "beta")],
            ),
            (skipped.clone(), vec![target(&skipped, "gamma")]),
        ],
        &CancellationToken::default(),
        &|_| {},
    )
    .await;

    assert_eq!(outcome.completed_packages, 1);
    assert_eq!(outcome.completed_managers, 0);
    assert_eq!(outcome.failed_manager, Some(failing.clone()));
    assert_eq!(*order.lock().unwrap(), vec![failing]);
}

#[tokio::test]
async fn cancellation_is_observed_between_manager_groups() {
    let first = manager_id("org.example:first");
    let second = manager_id("org.example:second");
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ManagerRegistry::new();
    for id in [&first, &second] {
        registry
            .register(Arc::new(FakeManager::new(
                id.as_str(),
                [ManagerCapability::Uninstall],
                Arc::clone(&order),
                None,
            )))
            .unwrap();
    }
    let cancellation = CancellationToken::default();
    let cancellation_for_progress = cancellation.clone();
    let progress = move |event: OperationProgress| {
        if event.completed == 1 {
            cancellation_for_progress.cancel();
        }
    };

    let outcome = execute_package_groups(
        &registry,
        &config(&[first.clone(), second.clone()]),
        PackageAction::Uninstall,
        &[
            (first.clone(), vec![target(&first, "alpha")]),
            (second.clone(), vec![target(&second, "beta")]),
        ],
        &cancellation,
        &progress,
    )
    .await;

    assert_eq!(outcome.completed_packages, 1);
    assert_eq!(outcome.completed_managers, 1);
    assert_eq!(
        outcome.error.as_deref(),
        Some("Stopped before starting another manager")
    );
    assert_eq!(*order.lock().unwrap(), vec![first]);
}
