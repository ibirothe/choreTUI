//! Application orchestration and semantic command dispatch.

pub mod use_cases;

use std::io;

use time::OffsetDateTime;

use crate::{
    domain::{CalendarDate, IsoWeek},
    tui::{self, model::BoardState},
};

/// Run the interactive application.
///
/// # Errors
///
/// Returns an I/O error when the terminal cannot be entered, rendered, or
/// restored.
pub fn run() -> io::Result<()> {
    tracing::info!("starting ChoreTUI");
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let today = CalendarDate::from_date(now.date());
    tui::run(BoardState::new(
        IsoWeek::containing(today),
        today,
        Vec::new(),
    ))
}
