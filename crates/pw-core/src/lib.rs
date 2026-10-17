//! PageWeave core: the engine-independent foundation shared by the desktop app, CLI, local API and MCP.

pub mod error;
pub mod job;
pub mod journal;
pub mod log;
pub mod ops;
pub mod output;
pub mod range;
pub mod settings;

pub use error::{ErrorCategory, PwError, PwResult};
