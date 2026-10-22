//! mixum Agent Runtime 领域层：运行时后端契约与共享类型。
//!
//! 本 crate 只放三件事：领域类型、后端契约、跨后端公共工具。
//! 各 runtime 的 CLI/API 细节必须留在 `mixum-env-runtime-*` 中，
//! 不得泄漏到这里，更不得泄漏到 Tauri 层。
//!
//! 依赖方向见 `docs/module-map.md`。

pub mod container;
pub mod error;
pub mod migration;
pub mod process;
pub mod runtime;

pub use container::{Container, ContainerState, CreateSpec, InspectedContainer, Mount, PortMapping};
pub use error::EnvError;
pub use migration::{MartRef, MigrationManifest, MigrationSource, MIGRATION_VERSION};
pub use runtime::{BackendRegistry, BackendStatus, RuntimeBackend, RuntimeKind, RuntimeStatus};
