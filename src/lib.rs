//! `ChoreTUI` application library.
//!
//! Modules follow the dependency boundaries in `docs/specification.md`: the
//! domain and recurrence layers stay independent from terminal and storage
//! adapters.

pub mod app;
pub mod catalog;
pub mod cli;
pub mod config;
pub mod diagnostics;
pub mod doctor;
pub mod kanban;
pub mod domain;
pub mod recurrence;
pub mod storage;
pub mod tui;
