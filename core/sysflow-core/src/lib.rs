//! SysFlow core: protocol envelope, error codes, schema, config, safe mode,
//! module trait and the registry that the CLI drives.

pub mod batterystats;
pub mod cache;
pub mod command;
pub mod config;
pub mod ctx;
pub mod dumpsys;
pub mod envelope;
pub mod error;
pub mod event;
pub mod history;
pub mod log;
pub mod module;
pub mod registry;
pub mod safe;
pub mod schema;
pub mod screen;
pub mod sysfs;
pub mod util;
pub mod wakeup_sources;

pub use error::{Error, ErrorCode, Result};
pub use module::{ApplyResult, Caps, Module, Sample};
pub use schema::{ItemType, Risk, Schema, SchemaItem, Scope};

/// Binary / workspace version (all crates share it).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
