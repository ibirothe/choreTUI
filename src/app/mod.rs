//! Application orchestration and semantic command dispatch.

pub mod use_cases;

use std::io;

use time::OffsetDateTime;

use crate::{
    domain::{
        CalendarDate, Occurrence, OccurrenceId, Timestamp,
        ports::{Clock, OccurrenceRepository},
    },
    storage::SqliteStore,
    tui::{self, BoardApplication},
};

use self::use_cases::{BoardDataError, TransactionalStore, UseCases};

/// Production wall clock with local-date and UTC-timestamp semantics.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn today(&self) -> CalendarDate {
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        CalendarDate::from_date(now.date())
    }

    fn now(&self) -> Timestamp {
        Timestamp::from_datetime(OffsetDateTime::now_utc())
    }
}

impl<P, C> BoardApplication for UseCases<P, C>
where
    P: TransactionalStore + OccurrenceRepository<Error = <P as TransactionalStore>::Error>,
    C: Clock,
{
    type Error = BoardDataError<<P as TransactionalStore>::Error>;

    fn today(&self) -> CalendarDate {
        Self::today(self)
    }

    fn load_week(&mut self, week: crate::domain::IsoWeek) -> Result<Vec<Occurrence>, Self::Error> {
        Self::load_week(self, week)
    }

    fn toggle_completion(&mut self, id: OccurrenceId) -> Result<Occurrence, Self::Error> {
        Self::toggle_completion(self, id).map_err(BoardDataError::Persistence)
    }
}

/// Run the interactive application.
///
/// # Errors
///
/// Returns an I/O error when the terminal cannot be entered, rendered, or
/// restored.
pub fn run(store: SqliteStore) -> io::Result<()> {
    tracing::info!("starting ChoreTUI");
    tui::run(UseCases::new(store, SystemClock))
}
