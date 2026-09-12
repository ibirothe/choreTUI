//! Application-level orchestration for transactional persistence operations.

use std::error::Error;

use crate::{
    domain::{
        ChoreId, ChoreName, Description, IsoWeek, Occurrence, OccurrenceId, Schedule, Timestamp,
        WeekError,
        ports::{Clock, OccurrenceRepository},
    },
    recurrence::{DateRange, RecurrenceError},
};

/// Failure while preparing or loading Weekly Board data.
#[derive(Debug, thiserror::Error)]
pub enum BoardDataError<E: Error + 'static> {
    /// Persistence operation failed.
    #[error("persistence operation failed: {0}")]
    Persistence(#[source] E),
    /// ISO-week boundaries could not be represented.
    #[error(transparent)]
    Week(#[from] WeekError),
    /// The derived materialization range was invalid.
    #[error(transparent)]
    Recurrence(#[from] RecurrenceError),
}

/// Transaction boundary required by mutating application use cases.
///
/// Adapters implement each method as one atomic persistence transaction.
pub trait TransactionalStore {
    /// Adapter-specific failure type.
    type Error: Error + Send + Sync + 'static;

    /// Materialize one inclusive date range idempotently.
    ///
    /// # Errors
    ///
    /// Returns the adapter error when the transaction fails.
    fn materialize_range(
        &mut self,
        range: DateRange,
        created_at: Timestamp,
    ) -> Result<usize, Self::Error>;

    /// Replace a schedule effective on the supplied date.
    ///
    /// # Errors
    ///
    /// Returns the adapter error when the transaction fails.
    fn revise_schedule(
        &mut self,
        chore_id: ChoreId,
        replacement: &Schedule,
        effective: crate::domain::CalendarDate,
    ) -> Result<(), Self::Error>;

    /// Rename a chore and eligible occurrence snapshots.
    ///
    /// # Errors
    ///
    /// Returns the adapter error when the transaction fails.
    fn rename_chore(
        &mut self,
        chore_id: ChoreId,
        name: ChoreName,
        description: Option<Description>,
        today: crate::domain::CalendarDate,
        updated_at: Timestamp,
    ) -> Result<(), Self::Error>;

    /// Disable a chore effective today.
    ///
    /// # Errors
    ///
    /// Returns the adapter error when the transaction fails.
    fn disable_chore(
        &mut self,
        chore_id: ChoreId,
        today: crate::domain::CalendarDate,
        updated_at: Timestamp,
    ) -> Result<(), Self::Error>;

    /// Soft-delete a chore effective today.
    ///
    /// # Errors
    ///
    /// Returns the adapter error when the transaction fails.
    fn soft_delete_chore(
        &mut self,
        chore_id: ChoreId,
        today: crate::domain::CalendarDate,
        deleted_at: Timestamp,
    ) -> Result<(), Self::Error>;

    /// Re-enable a chore with an equivalent schedule anchored today.
    ///
    /// # Errors
    ///
    /// Returns the adapter error when the transaction fails.
    fn reenable_chore(
        &mut self,
        chore_id: ChoreId,
        today: crate::domain::CalendarDate,
        updated_at: Timestamp,
    ) -> Result<Schedule, Self::Error>;

    /// Toggle an occurrence's completion state.
    ///
    /// # Errors
    ///
    /// Returns the adapter error when the transaction fails.
    fn toggle_completion(
        &mut self,
        occurrence_id: OccurrenceId,
        at: Timestamp,
    ) -> Result<Occurrence, Self::Error>;
}

/// Clock-aware application facade used by CLI and TUI adapters.
pub struct UseCases<P, C> {
    persistence: P,
    clock: C,
}

impl<P, C> UseCases<P, C>
where
    P: TransactionalStore,
    C: Clock,
{
    /// Create an application facade.
    pub const fn new(persistence: P, clock: C) -> Self {
        Self { persistence, clock }
    }

    /// Return today's local date from the injected clock.
    #[must_use]
    pub fn today(&self) -> crate::domain::CalendarDate {
        self.clock.today()
    }

    /// Materialize a range using the current timestamp.
    ///
    /// # Errors
    ///
    /// Returns the persistence adapter error when materialization fails.
    pub fn materialize_range(&mut self, range: DateRange) -> Result<usize, P::Error> {
        self.persistence.materialize_range(range, self.clock.now())
    }

    /// Revise a schedule effective today.
    ///
    /// # Errors
    ///
    /// Returns the persistence adapter error when the revision fails.
    pub fn revise_schedule(
        &mut self,
        chore_id: ChoreId,
        replacement: &Schedule,
    ) -> Result<(), P::Error> {
        self.persistence
            .revise_schedule(chore_id, replacement, self.clock.today())
    }

    /// Rename a chore effective today.
    ///
    /// # Errors
    ///
    /// Returns the persistence adapter error when the rename fails.
    pub fn rename_chore(
        &mut self,
        chore_id: ChoreId,
        name: ChoreName,
        description: Option<Description>,
    ) -> Result<(), P::Error> {
        self.persistence.rename_chore(
            chore_id,
            name,
            description,
            self.clock.today(),
            self.clock.now(),
        )
    }

    /// Disable a chore effective today.
    ///
    /// # Errors
    ///
    /// Returns the persistence adapter error when disabling fails.
    pub fn disable_chore(&mut self, chore_id: ChoreId) -> Result<(), P::Error> {
        self.persistence
            .disable_chore(chore_id, self.clock.today(), self.clock.now())
    }

    /// Soft-delete a chore effective today.
    ///
    /// # Errors
    ///
    /// Returns the persistence adapter error when deletion fails.
    pub fn soft_delete_chore(&mut self, chore_id: ChoreId) -> Result<(), P::Error> {
        self.persistence
            .soft_delete_chore(chore_id, self.clock.today(), self.clock.now())
    }

    /// Re-enable a chore effective today.
    ///
    /// # Errors
    ///
    /// Returns the persistence adapter error when re-enabling fails.
    pub fn reenable_chore(&mut self, chore_id: ChoreId) -> Result<Schedule, P::Error> {
        self.persistence
            .reenable_chore(chore_id, self.clock.today(), self.clock.now())
    }

    /// Toggle completion at the current timestamp.
    ///
    /// # Errors
    ///
    /// Returns the persistence adapter error when toggling fails.
    pub fn toggle_completion(
        &mut self,
        occurrence_id: OccurrenceId,
    ) -> Result<Occurrence, P::Error> {
        self.persistence
            .toggle_completion(occurrence_id, self.clock.now())
    }

    /// Consume the facade and return its adapter and clock.
    pub fn into_parts(self) -> (P, C) {
        (self.persistence, self.clock)
    }
}

impl<P, C> UseCases<P, C>
where
    P: TransactionalStore + OccurrenceRepository<Error = <P as TransactionalStore>::Error>,
    C: Clock,
{
    /// Materialize and reload every occurrence due in one ISO week.
    ///
    /// # Errors
    ///
    /// Returns a board-data error when week arithmetic, materialization, or
    /// persistence loading fails.
    pub fn load_week(
        &mut self,
        week: IsoWeek,
    ) -> Result<Vec<Occurrence>, BoardDataError<<P as TransactionalStore>::Error>> {
        let start = week.monday()?;
        let end = week
            .next()?
            .monday()?
            .as_date()
            .previous_day()
            .map(crate::domain::CalendarDate::from_date)
            .ok_or(WeekError::DateOutOfRange)?;
        let range = DateRange::new(start, end)?;
        self.persistence
            .materialize_range(range, self.clock.now())
            .map_err(BoardDataError::Persistence)?;
        self.persistence
            .for_week(week)
            .map_err(BoardDataError::Persistence)
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use tempfile::tempdir_in;

    use super::*;
    use crate::{
        domain::{
            CalendarDate, Chore, ChoreId, ChoreName, IsoWeekday, OccurrenceState,
            RecurrenceInterval, ScheduleId, ScheduleWindow, WeeklyStatistics,
            ports::{ChoreRepository, ScheduleRepository},
        },
        storage::SqliteStore,
        tui::{
            BoardRuntime,
            model::{BoardInput, BoardLayout},
        },
    };

    #[derive(Clone, Copy)]
    struct FixedClock {
        today: CalendarDate,
        now: Timestamp,
    }

    impl Clock for FixedClock {
        fn today(&self) -> CalendarDate {
            self.today
        }

        fn now(&self) -> Timestamp {
            self.now
        }
    }

    fn date(year: i32, month: u8, day: u8) -> CalendarDate {
        CalendarDate::new(year, month, day).expect("test date should be valid")
    }

    fn timestamp(seconds: i64) -> Timestamp {
        Timestamp::from_unix_timestamp(seconds).expect("test timestamp should be valid")
    }

    fn seeded_store(path: &std::path::Path, monday: CalendarDate) -> SqliteStore {
        let mut store = SqliteStore::open(path).expect("test database should open");
        let chore_id = ChoreId::new();
        let created_at = timestamp(1);
        let chore = Chore::new(
            chore_id,
            ChoreName::new("Bins").expect("test name should be valid"),
            None,
            created_at,
        );
        ChoreRepository::save(&mut store, &chore).expect("test chore should save");
        let schedule = Schedule::weekly(
            ScheduleId::new(),
            chore_id,
            RecurrenceInterval::new(1).expect("test interval should be valid"),
            [IsoWeekday::Monday],
            ScheduleWindow::new(monday, monday, None, created_at)
                .expect("test window should be valid"),
        )
        .expect("test schedule should be valid");
        ScheduleRepository::insert(&mut store, &schedule).expect("test schedule should save");
        store
    }

    #[test]
    fn sqlite_toggle_refreshes_statistics_and_survives_restart_and_reopen() {
        let temporary = tempdir_in(env::current_dir().expect("working directory should exist"))
            .expect("temporary directory should be created");
        let database = temporary.path().join("completion.db");
        let monday = date(2026, 9, 7);
        let thursday = date(2026, 9, 10);
        let clock = FixedClock {
            today: thursday,
            now: timestamp(100),
        };
        let store = seeded_store(&database, monday);
        let mut runtime = BoardRuntime::new(UseCases::new(store, clock));

        let before = runtime.state().statistics();
        assert_eq!(
            before,
            WeeklyStatistics::calculate(
                runtime.state().week(),
                runtime.state().occurrences_for_day(0),
                thursday
            )
        );
        assert_eq!(before.completed(), 0);
        assert_eq!(before.missed(), 1);
        for _ in 0..3 {
            runtime.handle_input(BoardInput::PreviousDay, BoardLayout::SevenColumns);
        }
        let due_date = runtime
            .state()
            .selected_occurrence()
            .expect("Monday occurrence should be selected")
            .due_date();
        runtime.handle_input(BoardInput::ToggleCompletion, BoardLayout::SevenColumns);
        assert_eq!(runtime.state().statistics().completed(), 1);
        assert_eq!(runtime.state().statistics().missed(), 0);
        assert_eq!(
            runtime
                .state()
                .selected_occurrence()
                .map(Occurrence::due_date),
            Some(due_date)
        );

        let (use_cases, _) = runtime.into_parts();
        let (store, _) = use_cases.into_parts();
        drop(store);
        let reopened = SqliteStore::open(&database).expect("database should reopen");
        let mut restarted = BoardRuntime::new(UseCases::new(
            reopened,
            FixedClock {
                today: monday,
                now: timestamp(200),
            },
        ));
        assert!(matches!(
            restarted
                .state()
                .selected_occurrence()
                .map(Occurrence::state),
            Some(OccurrenceState::Completed { .. })
        ));

        restarted.handle_input(BoardInput::ToggleCompletion, BoardLayout::SevenColumns);
        let pending = restarted
            .state()
            .selected_occurrence()
            .expect("occurrence should remain selected");
        assert_eq!(pending.state(), OccurrenceState::Pending);
        assert_eq!(pending.completed_at(), None);
    }
}
