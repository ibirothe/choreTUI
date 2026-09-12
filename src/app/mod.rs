//! Application orchestration and semantic command dispatch.

pub mod use_cases;

use std::io;

use crate::tui;

/// Run the interactive application.
///
/// The bootstrap renders one frame and exits; the persistent event loop is
/// introduced with the Weekly Board ticket.
///
/// # Errors
///
/// Returns an I/O error when the terminal cannot be entered, rendered, or
/// restored.
pub fn run() -> io::Result<()> {
    tracing::info!("starting ChoreTUI");
    tui::run_placeholder()
}
