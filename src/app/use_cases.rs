//! Application-level orchestration for transactional persistence operations.

use std::error::Error;

use crate::{
    app::editor::{ChoreSubmission, EditorRecord, SchedulePattern},
    domain::{
        CalendarDate, Chore, ChoreId, ChoreName, Description, IsoWeek, Occurrence, OccurrenceId,
        Schedule, ScheduleId, ScheduleWindow, Timestamp, ValidationError, WeekError,
        ports::{ChoreRepository, Clock, OccurrenceRepository, ScheduleRepository},
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

/// Failure while loading or saving editor data.
#[derive(Debug, thiserror::Error)]
pub enum EditorDataError<E: Error + 'static> {
    #[error("persistence operation failed: {0}")]
    Persistence(#[source] E),
    #[error("chore was not found")]
    MissingChore,
    #[error("schedule was not found or is invalid")]
    MissingSchedule,
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

/// All fields needed by an adapter to update one chore atomically.
pub struct EditorUpdate<'a> {
    pub chore_id: ChoreId,
    pub name: ChoreName,
    pub description: Option<Description>,
    pub enabled: bool,
    pub replacement: Option<&'a Schedule>,
    pub today: CalendarDate,
    pub updated_at: Timestamp,
}

/// Atomic persistence boundary for editor saves.
pub trait AtomicEditorStore {
    type Error: Error + Send + Sync + 'static;

    /// Persist a new chore and its initial schedule in one transaction.
    ///
    /// # Errors
    ///
    /// Returns the adapter error and rolls back both records when persistence fails.
    fn create_editor_chore(
        &mut self,
        chore: &Chore,
        schedule: &Schedule,
    ) -> Result<(), Self::Error>;

    /// Persist metadata, lifecycle and optional schedule revision atomically.
    ///
    /// # Errors
    ///
    /// Returns the adapter error and rolls back the complete update on failure.
    fn update_editor_chore(&mut self, update: EditorUpdate<'_>) -> Result<(), Self::Error>;
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
    P: AtomicEditorStore
        + ChoreRepository<Error = <P as AtomicEditorStore>::Error>
        + ScheduleRepository<Error = <P as AtomicEditorStore>::Error>,
    C: Clock,
{
    /// List chores in repository-defined display order.
    ///
    /// # Errors
    ///
    /// Returns the persistence adapter error when loading fails.
    pub fn list_chores(
        &self,
        include_deleted: bool,
    ) -> Result<Vec<Chore>, <P as AtomicEditorStore>::Error> {
        self.persistence.list(include_deleted)
    }

    /// Load values for editing.
    ///
    /// # Errors
    ///
    /// Returns an editor-data error when the chore or its schedule cannot be loaded.
    pub fn load_editor(
        &self,
        chore_id: ChoreId,
    ) -> Result<EditorRecord, EditorDataError<<P as AtomicEditorStore>::Error>> {
        let chore = self
            .persistence
            .find(chore_id)
            .map_err(EditorDataError::Persistence)?
            .ok_or(EditorDataError::MissingChore)?;
        let schedule = self
            .persistence
            .for_chore(chore_id)
            .map_err(EditorDataError::Persistence)?
            .into_iter()
            .last()
            .ok_or(EditorDataError::MissingSchedule)?;
        let pattern =
            SchedulePattern::from_schedule(&schedule).ok_or(EditorDataError::MissingSchedule)?;
        Ok(EditorRecord {
            id: chore.id(),
            name: chore.name().clone(),
            description: chore.description().cloned(),
            enabled: chore.is_enabled(),
            pattern,
        })
    }

    /// Atomically create or update a chore, with recurrence changes effective today.
    ///
    /// # Errors
    ///
    /// Returns validation or persistence errors without partially saving the form.
    pub fn save_editor(
        &mut self,
        submission: ChoreSubmission,
    ) -> Result<ChoreId, EditorDataError<<P as AtomicEditorStore>::Error>> {
        let today = self.clock.today();
        let now = self.clock.now();
        if let Some(chore_id) = submission.id {
            let current = self.load_editor(chore_id)?;
            let needs_schedule =
                current.pattern != submission.pattern || (!current.enabled && submission.enabled);
            let replacement = needs_schedule
                .then(|| {
                    build_schedule(
                        chore_id,
                        &submission.pattern,
                        today,
                        now,
                        submission.enabled,
                    )
                })
                .transpose()?;
            self.persistence
                .update_editor_chore(EditorUpdate {
                    chore_id,
                    name: submission.name,
                    description: submission.description,
                    enabled: submission.enabled,
                    replacement: replacement.as_ref(),
                    today,
                    updated_at: now,
                })
                .map_err(EditorDataError::Persistence)?;
            Ok(chore_id)
        } else {
            let chore_id = ChoreId::new();
            let mut chore = Chore::new(chore_id, submission.name, submission.description, now);
            if !submission.enabled {
                let _ = chore.set_enabled(false, now);
            }
            let schedule = build_schedule(
                chore_id,
                &submission.pattern,
                today,
                now,
                submission.enabled,
            )?;
            self.persistence
                .create_editor_chore(&chore, &schedule)
                .map_err(EditorDataError::Persistence)?;
            Ok(chore_id)
        }
    }
}

fn build_schedule(
    chore_id: ChoreId,
    pattern: &SchedulePattern,
    today: CalendarDate,
    now: Timestamp,
    enabled: bool,
) -> Result<Schedule, ValidationError> {
    let until = (!enabled).then_some(today);
    let window = ScheduleWindow::new(today, today, until, now)?;
    Ok(match pattern {
        SchedulePattern::Weekly { interval, weekdays } => Schedule::weekly(
            ScheduleId::new(),
            chore_id,
            *interval,
            weekdays.iter().copied(),
            window,
        )?,
        SchedulePattern::DailyInterval { interval } => {
            Schedule::daily_interval(ScheduleId::new(), chore_id, *interval, window)
        }
        SchedulePattern::Monthly { day } => {
            Schedule::monthly(ScheduleId::new(), chore_id, *day, window)
        }
    })
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

    use crossterm::event::{KeyCode, KeyEvent};
    use tempfile::tempdir_in;

    use super::*;
    use crate::{
        domain::{
            CalendarDate, Chore, ChoreId, ChoreName, IsoWeekday, MonthlyDay, OccurrenceState,
            RecurrenceInterval, RecurrenceKind, ScheduleId, ScheduleWindow, WeeklyStatistics,
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

    #[test]
    fn editor_creates_all_recurrences_and_revises_effective_today() {
        let today = date(2026, 9, 10);
        let mut application = UseCases::new(
            SqliteStore::open_in_memory().expect("database should open"),
            FixedClock {
                today,
                now: timestamp(100),
            },
        );
        let weekly = application
            .save_editor(ChoreSubmission {
                id: None,
                name: ChoreName::new("Bins").expect("name should be valid"),
                description: None,
                enabled: true,
                pattern: SchedulePattern::Weekly {
                    interval: RecurrenceInterval::new(2).expect("interval should be valid"),
                    weekdays: vec![IsoWeekday::Thursday],
                },
            })
            .expect("weekly chore should save");
        let daily = application
            .save_editor(ChoreSubmission {
                id: None,
                name: ChoreName::new("Water").expect("name should be valid"),
                description: None,
                enabled: true,
                pattern: SchedulePattern::DailyInterval {
                    interval: RecurrenceInterval::new(3).expect("interval should be valid"),
                },
            })
            .expect("daily chore should save");
        let monthly = application
            .save_editor(ChoreSubmission {
                id: None,
                name: ChoreName::new("Filter").expect("name should be valid"),
                description: None,
                enabled: true,
                pattern: SchedulePattern::Monthly {
                    day: MonthlyDay::new(31).expect("monthly day should be valid"),
                },
            })
            .expect("monthly chore should save");

        application
            .save_editor(ChoreSubmission {
                id: Some(weekly),
                name: ChoreName::new("Bins revised").expect("name should be valid"),
                description: Description::optional("Effective today")
                    .expect("description should be valid"),
                enabled: true,
                pattern: SchedulePattern::DailyInterval {
                    interval: RecurrenceInterval::new(1).expect("interval should be valid"),
                },
            })
            .expect("revision should save");
        let occurrences = application
            .load_week(IsoWeek::containing(today))
            .expect("edited week should refresh");
        assert!(
            occurrences
                .iter()
                .any(|item| item.chore_id() == weekly && item.name().as_str() == "Bins revised")
        );

        let (store, _) = application.into_parts();
        let weekly_revisions =
            ScheduleRepository::for_chore(&store, weekly).expect("weekly revisions should load");
        assert_eq!(weekly_revisions.len(), 2);
        assert_eq!(
            weekly_revisions.last().map(Schedule::kind),
            Some(RecurrenceKind::DailyInterval)
        );
        assert_eq!(
            ScheduleRepository::for_chore(&store, daily).expect("daily schedule should load")[0]
                .kind(),
            RecurrenceKind::DailyInterval
        );
        assert_eq!(
            ScheduleRepository::for_chore(&store, monthly).expect("monthly schedule should load")
                [0]
            .kind(),
            RecurrenceKind::Monthly
        );
    }

    #[test]
    fn chore_list_lifecycle_transitions_refresh_the_board() {
        let temporary = tempdir_in(env::current_dir().expect("working directory should exist"))
            .expect("temporary directory should be created");
        let monday = date(2026, 9, 7);
        let store = seeded_store(&temporary.path().join("lifecycle.db"), monday);
        let mut runtime = BoardRuntime::new(UseCases::new(
            store,
            FixedClock {
                today: monday,
                now: timestamp(100),
            },
        ));
        assert!(runtime.state().selected_occurrence().is_some());

        runtime.handle_input(BoardInput::DisableChore, BoardLayout::SevenColumns);
        assert!(runtime.state().selected_occurrence().is_none());
        runtime.handle_input(BoardInput::OpenChoreList, BoardLayout::SevenColumns);
        runtime.handle_key(
            KeyEvent::from(KeyCode::Char(' ')),
            BoardLayout::SevenColumns,
        );
        assert!(runtime.state().selected_occurrence().is_some());

        runtime.handle_key(
            KeyEvent::from(KeyCode::Char('D')),
            BoardLayout::SevenColumns,
        );
        runtime.handle_key(KeyEvent::from(KeyCode::Enter), BoardLayout::SevenColumns);
        assert!(
            runtime
                .chore_list()
                .is_some_and(|list| !list.is_confirming_delete())
        );
        runtime.handle_key(
            KeyEvent::from(KeyCode::Char('D')),
            BoardLayout::SevenColumns,
        );
        runtime.handle_key(KeyEvent::from(KeyCode::Right), BoardLayout::SevenColumns);
        runtime.handle_key(KeyEvent::from(KeyCode::Enter), BoardLayout::SevenColumns);
        assert!(runtime.state().selected_occurrence().is_none());
        runtime.handle_key(
            KeyEvent::from(KeyCode::Char('x')),
            BoardLayout::SevenColumns,
        );
        assert!(
            runtime
                .chore_list()
                .and_then(|list| list.selected_chore())
                .is_some_and(Chore::is_deleted)
        );
    }
}
