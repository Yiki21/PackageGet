//! Package-manager registry, execution coordination, and application storage.

mod builtin_managers;
pub mod error;
mod execution;
mod registry;
mod storage;

pub use builtin_managers::register_builtin_managers;
pub use execution::{
    CancellationToken, OperationOutcome, OperationProgress, execute_package_groups,
};
pub use registry::{ManagerRegistry, RegistryError};
pub use storage::Config;
pub use updater_manager_api::ManagerConfig;

type CoreResult<T> = Result<T, error::CoreError>;
