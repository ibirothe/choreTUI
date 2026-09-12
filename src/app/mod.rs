//! Application orchestration and semantic command dispatch.

pub mod editor;
pub mod use_cases;

use std::io;

use time::OffsetDateTime;

use crate::{
    app::editor::{ChoreSubmission, EditorRecord},
    domain::{
        CalendarDate, ChoreId, Occurrence, OccurrenceId, Timestamp,
        ports::{ChoreRepository, Clock, OccurrenceRepository, ScheduleRepository},
    },
    storage::SqliteStore,
    tui::{self, BoardApplication},
};

use self::use_cases::{AtomicEditorStore, BoardDataError, EditorDataError, TransactionalStore, UseCases};

/// Sanitizable application failure; detailed sources are logged by the TUI.
#[derive(Debug, thiserror::Error)]
pub enum ApplicationError<E: std::error::Error + 'static> {
    #[error(transparent)]
    Board(#[from] BoardDataError<E>),
    #[error(transparent)]
    Editor(#[from] EditorDataError<E>),
}

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
    P: TransactionalStore
        + AtomicEditorStore<Error = <P as TransactionalStore>::Error>
        + OccurrenceRepository<Error = <P as TransactionalStore>::Error>
        + ChoreRepository<Error = <P as TransactionalStore>::Error>
        + ScheduleRepository<Error = <P as TransactionalStore>::Error>,
    C: Clock,
{
    type Error = ApplicationError<<P as TransactionalStore>::Error>;

    fn today(&self) -> CalendarDate {
        Self::today(self)
    }

    fn load_week(&mut self, week: crate::domain::IsoWeek) -> Result<Vec<Occurrence>, Self::Error> {
        Self::load_week(self, week).map_err(ApplicationError::Board)
    }

    fn toggle_completion(&mut self, id: OccurrenceId) -> Result<Occurrence, Self::Error> {
        Self::toggle_completion(self, id)
            .map_err(BoardDataError::Persistence)
            .map_err(ApplicationError::Board)
    }

    fn load_editor(&mut self, id: ChoreId) -> Result<EditorRecord, Self::Error> {
        Self::load_editor(self, id).map_err(ApplicationError::Editor)
    }

    fn save_editor(&mut self, submission: ChoreSubmission) -> Result<ChoreId, Self::Error> {
        Self::save_editor(self, submission).map_err(ApplicationError::Editor)
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
