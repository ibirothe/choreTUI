//! Persistence adapters.

pub mod migrations;
pub mod sqlite;

pub use migrations::LATEST_VERSION;
pub use sqlite::{DatabaseDiagnostics, SqliteError, SqliteStore};
