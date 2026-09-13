//! SQLite repository adapter and transactional persistence use cases.

use std::{collections::HashSet, path::Path, time::Duration as StdDuration};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, params};
use thiserror::Error;
use time::{Date, Duration, format_description::well_known::Rfc3339, macros::format_description};
use uuid::Uuid;

use crate::{
    app::{
        editor::TemplateProvenance,
        use_cases::{AtomicEditorStore, EditorUpdate, TransactionalStore},
    },
    domain::{
        CalendarDate, Chore, ChoreId, ChoreName, ChoreTimestamps, Description, IsoWeek, IsoWeekday,
        MonthlyDay, Occurrence, OccurrenceId, OccurrenceSeed, OccurrenceState, RecurrenceInterval,
        RecurrenceKind, Schedule, ScheduleId, ScheduleWindow, StateTransitionError, Timestamp,
        ValidationError, WeekError,
        ports::{ChoreRepository, OccurrenceRepository, ScheduleRepository},
    },
    recurrence::{DateRange, expand},
};

use super::migrations;

const DATE_FORMAT: &[time::format_description::BorrowedFormatItem<'static>] =
    format_description!("[year]-[month]-[day]");

/// Failure while opening, migrating, reading, or mutating SQLite state.
#[derive(Debug, Error)]
pub enum SqliteError {
    /// SQLite rejected an operation.
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    /// Persisted or supplied data violates a domain invariant.
    #[error(transparent)]
    Domain(#[from] ValidationError),
    /// ISO-week conversion failed.
    #[error(transparent)]
    Week(#[from] WeekError),
    /// An occurrence state transition was invalid.
    #[error(transparent)]
    StateTransition(#[from] StateTransitionError),
    /// A stored value could not be decoded.
    #[error("invalid stored {field}: {value}")]
    InvalidStoredValue {
        /// Column or value category.
        field: &'static str,
        /// Rejected persisted text.
        value: String,
    },
    /// Formatting a validated domain value failed.
    #[error("could not format {field}: {message}")]
    Format {
        /// Value category.
        field: &'static str,
        /// Underlying formatter message.
        message: String,
    },
    /// The database was created by a newer application version.
    #[error(
        "database schema version {found} is newer than supported version {supported}; run a newer ChoreTUI or use `chore doctor`"
    )]
    NewerSchema {
        /// Version found in the database.
        found: i64,
        /// Latest version understood here.
        supported: i64,
    },
    /// A required record does not exist.
    #[error("{entity} was not found")]
    NotFound {
        /// Domain entity name.
        entity: &'static str,
    },
    /// A lifecycle operation requires an active schedule.
    #[error("chore has no active schedule")]
    MissingActiveSchedule,
    /// A supplied replacement schedule does not belong to the target chore.
    #[error("replacement schedule belongs to a different chore")]
    ScheduleChoreMismatch,
    /// A supplied replacement schedule has the wrong effective date.
    #[error("replacement schedule anchor and validity start must equal the effective date")]
    InvalidReplacementWindow,
    /// Date arithmetic exceeded the supported calendar range.
    #[error("calendar date is outside the supported range")]
    DateOutOfRange,
    /// A deleted chore cannot be re-enabled.
    #[error("a deleted chore cannot be re-enabled")]
    DeletedChore,
}

/// SQLite-backed implementation of the persistence ports.
pub struct SqliteStore {
    connection: Connection,
}

/// Read-only database health details used by `chore doctor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseDiagnostics {
    /// Applied schema version.
    pub schema_version: i64,
    /// Whether foreign-key enforcement is active on the diagnostic connection.
    pub foreign_keys: bool,
    /// Rows returned by SQLite's integrity check.
    pub integrity: Vec<String>,
}

impl SqliteStore {
    /// Open, configure, and migrate a database file.
    ///
    /// # Errors
    ///
    /// Returns an actionable storage error when opening, configuration, or a
    /// migration fails.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SqliteError> {
        let connection = Connection::open(path)?;
        Self::initialize(connection)
    }

    /// Open a configured in-memory database, primarily for tests and tools.
    ///
    /// # Errors
    ///
    /// Returns a storage error when configuration or migration fails.
    pub fn open_in_memory() -> Result<Self, SqliteError> {
        let connection = Connection::open_in_memory()?;
        Self::initialize(connection)
    }

    fn initialize(mut connection: Connection) -> Result<Self, SqliteError> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.busy_timeout(StdDuration::from_secs(5))?;
        migrations::migrate(&mut connection)?;
        Ok(Self { connection })
    }

    /// Inspect an existing database through a query-only connection without
    /// running migrations or changing domain data.
    ///
    /// # Errors
    ///
    /// Returns a storage error for unavailable, locked, corrupt, or unsupported
    /// databases.
    pub fn inspect_read_only(path: impl AsRef<Path>) -> Result<DatabaseDiagnostics, SqliteError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(StdDuration::from_secs(5))?;
        connection.pragma_update(None, "query_only", "ON")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let schema_version = migrations::schema_version(&connection)?;
        if schema_version > migrations::LATEST_VERSION {
            return Err(SqliteError::NewerSchema {
                found: schema_version,
                supported: migrations::LATEST_VERSION,
            });
        }
        let foreign_keys = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        let mut statement = connection.prepare("PRAGMA integrity_check")?;
        let integrity = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(DatabaseDiagnostics {
            schema_version,
            foreign_keys,
            integrity,
        })
    }

    /// Run SQLite's integrity check and return its diagnostic rows.
    ///
    /// # Errors
    ///
    /// Returns a storage error when SQLite cannot perform the check.
    pub fn integrity_check(&self) -> Result<Vec<String>, SqliteError> {
        let mut statement = self.connection.prepare("PRAGMA integrity_check")?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(SqliteError::from)
    }

    /// Generate and insert all missing occurrences in a date range.
    ///
    /// The operation is idempotent through the database uniqueness constraint
    /// on `(schedule_id, nominal_date)`.
    ///
    /// # Errors
    ///
    /// Returns a storage error and rolls the complete range back atomically.
    pub fn materialize_range(
        &mut self,
        range: DateRange,
        created_at: Timestamp,
    ) -> Result<usize, SqliteError> {
        let transaction = self.connection.transaction()?;
        let schedule_ids = generation_schedule_ids(&transaction, range)?;
        let mut inserted = 0;

        for schedule_id in schedule_ids {
            let schedule = load_schedule(&transaction, schedule_id)?
                .ok_or(SqliteError::NotFound { entity: "schedule" })?;
            let chore = load_chore(&transaction, schedule.chore_id())?
                .ok_or(SqliteError::NotFound { entity: "chore" })?;
            for nominal_date in expand(&schedule, range) {
                let occurrence = Occurrence::pending(OccurrenceSeed {
                    id: OccurrenceId::new(),
                    chore_id: chore.id(),
                    schedule_id: schedule.id(),
                    nominal_date,
                    due_date: nominal_date,
                    name: chore.name().clone(),
                    description: chore.description().cloned(),
                    created_at,
                });
                inserted += insert_occurrence_if_missing(&transaction, &occurrence)?;
            }
        }

        transaction.commit()?;
        Ok(inserted)
    }

    /// Replace an active schedule effective on a given date.
    ///
    /// Future pending occurrences from the replaced revision are removed;
    /// historical and completed rows remain unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error and rolls back all changes if any step fails.
    pub fn revise_schedule(
        &mut self,
        chore_id: ChoreId,
        replacement: &Schedule,
        effective: CalendarDate,
    ) -> Result<(), SqliteError> {
        if replacement.chore_id() != chore_id {
            return Err(SqliteError::ScheduleChoreMismatch);
        }
        let replacement_window = replacement.window();
        if replacement_window.anchor_date() != effective
            || replacement_window.valid_from() != effective
            || replacement_window.valid_until().is_some()
        {
            return Err(SqliteError::InvalidReplacementWindow);
        }

        let transaction = self.connection.transaction()?;
        let active =
            active_schedule(&transaction, chore_id)?.ok_or(SqliteError::MissingActiveSchedule)?;
        close_schedule(&transaction, &active, effective)?;
        insert_schedule(&transaction, replacement)?;
        transaction.execute(
            "DELETE FROM occurrences \
             WHERE schedule_id = ?1 AND state = 'pending' AND due_date >= ?2",
            params![active.id().to_string(), format_date(effective)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Rename a chore and refresh only eligible future pending snapshots.
    ///
    /// # Errors
    ///
    /// Returns an error and rolls back the entire rename on failure.
    pub fn rename_chore(
        &mut self,
        chore_id: ChoreId,
        name: ChoreName,
        description: Option<Description>,
        today: CalendarDate,
        updated_at: Timestamp,
    ) -> Result<(), SqliteError> {
        let transaction = self.connection.transaction()?;
        let mut chore =
            load_chore(&transaction, chore_id)?.ok_or(SqliteError::NotFound { entity: "chore" })?;
        chore.rename(name, description, updated_at);
        save_chore(&transaction, &chore)?;
        transaction.execute(
            "UPDATE occurrences SET name_snapshot = ?1, description_snapshot = ?2, updated_at = ?3 \
             WHERE chore_id = ?4 AND state = 'pending' AND due_date >= ?5",
            params![
                chore.name().as_str(),
                chore.description().map(Description::as_str),
                format_timestamp(updated_at)?,
                chore_id.to_string(),
                format_date(today)?,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Disable a chore, close its active schedule, and remove future pending
    /// occurrences.
    ///
    /// # Errors
    ///
    /// Returns an error and rolls back all changes if any step fails.
    pub fn disable_chore(
        &mut self,
        chore_id: ChoreId,
        today: CalendarDate,
        updated_at: Timestamp,
    ) -> Result<(), SqliteError> {
        self.deactivate_chore(chore_id, today, updated_at, false)
    }

    /// Soft-delete a chore while retaining every historical occurrence.
    ///
    /// # Errors
    ///
    /// Returns an error and rolls back all changes if any step fails.
    pub fn soft_delete_chore(
        &mut self,
        chore_id: ChoreId,
        today: CalendarDate,
        deleted_at: Timestamp,
    ) -> Result<(), SqliteError> {
        self.deactivate_chore(chore_id, today, deleted_at, true)
    }

    fn deactivate_chore(
        &mut self,
        chore_id: ChoreId,
        today: CalendarDate,
        updated_at: Timestamp,
        delete: bool,
    ) -> Result<(), SqliteError> {
        let transaction = self.connection.transaction()?;
        let mut chore =
            load_chore(&transaction, chore_id)?.ok_or(SqliteError::NotFound { entity: "chore" })?;
        if delete {
            chore.mark_deleted(updated_at);
        } else {
            let _changed = chore.set_enabled(false, updated_at);
        }
        save_chore(&transaction, &chore)?;
        if let Some(active) = active_schedule(&transaction, chore_id)? {
            close_schedule(&transaction, &active, today)?;
        }
        transaction.execute(
            "DELETE FROM occurrences \
             WHERE chore_id = ?1 AND state = 'pending' AND due_date >= ?2",
            params![chore_id.to_string(), format_date(today)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Re-enable a chore with a new equivalent schedule anchored today.
    ///
    /// # Errors
    ///
    /// Returns an error and rolls back all changes if the chore or its latest
    /// schedule cannot be loaded.
    pub fn reenable_chore(
        &mut self,
        chore_id: ChoreId,
        today: CalendarDate,
        updated_at: Timestamp,
    ) -> Result<Schedule, SqliteError> {
        let transaction = self.connection.transaction()?;
        let mut chore =
            load_chore(&transaction, chore_id)?.ok_or(SqliteError::NotFound { entity: "chore" })?;
        if chore.is_deleted() {
            return Err(SqliteError::DeletedChore);
        }
        let latest = latest_schedule(&transaction, chore_id)?
            .ok_or(SqliteError::NotFound { entity: "schedule" })?;
        let replacement = equivalent_schedule(&latest, today, updated_at)?;
        let _changed = chore.set_enabled(true, updated_at);
        save_chore(&transaction, &chore)?;
        insert_schedule(&transaction, &replacement)?;
        transaction.commit()?;
        Ok(replacement)
    }

    /// Toggle one occurrence between pending and completed atomically.
    ///
    /// # Errors
    ///
    /// Returns an error when the occurrence does not exist or is skipped.
    pub fn toggle_completion(
        &mut self,
        occurrence_id: OccurrenceId,
        at: Timestamp,
    ) -> Result<Occurrence, SqliteError> {
        let transaction = self.connection.transaction()?;
        let mut occurrence =
            load_occurrence(&transaction, occurrence_id)?.ok_or(SqliteError::NotFound {
                entity: "occurrence",
            })?;
        occurrence.toggle_completion(at)?;
        save_occurrence(&transaction, &occurrence)?;
        transaction.commit()?;
        Ok(occurrence)
    }
}

impl ChoreRepository for SqliteStore {
    type Error = SqliteError;

    fn find(&self, id: ChoreId) -> Result<Option<Chore>, Self::Error> {
        load_chore(&self.connection, id)
    }

    fn list(&self, include_deleted: bool) -> Result<Vec<Chore>, Self::Error> {
        let sql = if include_deleted {
            "SELECT id FROM chores ORDER BY lower(name), created_at"
        } else {
            "SELECT id FROM chores WHERE deleted_at IS NULL ORDER BY lower(name), created_at"
        };
        let ids = query_id_strings(&self.connection, sql)?;
        ids.into_iter()
            .map(|id| parse_chore_id(&id).and_then(|id| load_chore(&self.connection, id)))
            .map(|result| {
                result.and_then(|value| value.ok_or(SqliteError::NotFound { entity: "chore" }))
            })
            .collect()
    }

    fn save(&mut self, chore: &Chore) -> Result<(), Self::Error> {
        save_chore(&self.connection, chore)
    }
}

impl AtomicEditorStore for SqliteStore {
    type Error = SqliteError;

    fn create_editor_chore(
        &mut self,
        chore: &Chore,
        schedule: &Schedule,
        provenance: Option<&TemplateProvenance>,
    ) -> Result<(), Self::Error> {
        if schedule.chore_id() != chore.id() {
            return Err(SqliteError::ScheduleChoreMismatch);
        }
        if chore.is_enabled() != schedule.window().valid_until().is_none() {
            return Err(SqliteError::InvalidReplacementWindow);
        }
        let transaction = self.connection.transaction()?;
        save_chore(&transaction, chore)?;
        insert_schedule(&transaction, schedule)?;
        if let Some(provenance) = provenance {
            transaction.execute(
                "INSERT INTO chore_catalog_provenance(\
                    chore_id, template_id, schema_version, catalog_version, locale, created_at\
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    chore.id().to_string(),
                    provenance.template_id.as_str(),
                    i64::from(provenance.schema_version),
                    i64::from(provenance.catalog_version),
                    provenance.locale.as_str(),
                    format_timestamp(chore.timestamps().created_at)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn catalog_provenance(
        &self,
        chore_id: ChoreId,
    ) -> Result<Option<TemplateProvenance>, Self::Error> {
        self.connection
            .query_row(
                "SELECT template_id, schema_version, catalog_version, locale \
                 FROM chore_catalog_provenance WHERE chore_id = ?1",
                [chore_id.to_string()],
                |row| {
                    Ok(TemplateProvenance {
                        template_id: row.get(0)?,
                        schema_version: row.get(1)?,
                        catalog_version: row.get(2)?,
                        locale: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(SqliteError::from)
    }

    fn catalog_dismissals(&self) -> Result<HashSet<String>, Self::Error> {
        let mut statement = self
            .connection
            .prepare("SELECT template_id FROM catalog_dismissals ORDER BY template_id")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()
            .map_err(SqliteError::from)
    }

    fn dismiss_catalog_template(
        &mut self,
        template_id: &str,
        dismissed_at: Timestamp,
    ) -> Result<(), Self::Error> {
        self.connection.execute(
            "INSERT INTO catalog_dismissals(template_id, dismissed_at) VALUES (?1, ?2) \
             ON CONFLICT(template_id) DO UPDATE SET dismissed_at = excluded.dismissed_at",
            params![template_id, format_timestamp(dismissed_at)?],
        )?;
        Ok(())
    }

    fn reset_catalog_dismissals(&mut self) -> Result<usize, Self::Error> {
        self.connection
            .execute("DELETE FROM catalog_dismissals", [])
            .map_err(SqliteError::from)
    }

    fn update_editor_chore(&mut self, update: EditorUpdate<'_>) -> Result<(), Self::Error> {
        if let Some(replacement) = update.replacement {
            let window = replacement.window();
            if replacement.chore_id() != update.chore_id {
                return Err(SqliteError::ScheduleChoreMismatch);
            }
            if window.anchor_date() != update.today
                || window.valid_from() != update.today
                || (update.enabled != window.valid_until().is_none())
            {
                return Err(SqliteError::InvalidReplacementWindow);
            }
        }

        let transaction = self.connection.transaction()?;
        let mut chore = load_chore(&transaction, update.chore_id)?
            .ok_or(SqliteError::NotFound { entity: "chore" })?;
        if chore.is_deleted() {
            return Err(SqliteError::DeletedChore);
        }
        let was_enabled = chore.is_enabled();
        chore.rename(update.name, update.description, update.updated_at);
        let _ = chore.set_enabled(update.enabled, update.updated_at);
        save_chore(&transaction, &chore)?;
        transaction.execute(
            "UPDATE occurrences SET name_snapshot = ?1, description_snapshot = ?2, updated_at = ?3 \
             WHERE chore_id = ?4 AND state = 'pending' AND due_date >= ?5",
            params![
                chore.name().as_str(),
                chore.description().map(Description::as_str),
                format_timestamp(update.updated_at)?,
                update.chore_id.to_string(),
                format_date(update.today)?,
            ],
        )?;

        let active = active_schedule(&transaction, update.chore_id)?;
        if (update.replacement.is_some() || (was_enabled && !update.enabled))
            && let Some(schedule) = active.as_ref()
        {
            close_schedule(&transaction, schedule, update.today)?;
        }
        if !was_enabled && update.enabled && update.replacement.is_none() {
            return Err(SqliteError::MissingActiveSchedule);
        }
        if let Some(replacement) = update.replacement {
            insert_schedule(&transaction, replacement)?;
        }
        if update.replacement.is_some() || !update.enabled {
            transaction.execute(
                "DELETE FROM occurrences WHERE chore_id = ?1 AND state = 'pending' AND due_date >= ?2",
                params![update.chore_id.to_string(), format_date(update.today)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}

impl ScheduleRepository for SqliteStore {
    type Error = SqliteError;

    fn for_chore(&self, chore_id: ChoreId) -> Result<Vec<Schedule>, Self::Error> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM schedules WHERE chore_id = ?1 ORDER BY created_at, id")?;
        let ids = statement
            .query_map([chore_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        ids.into_iter()
            .map(|id| parse_schedule_id(&id).and_then(|id| load_schedule(&self.connection, id)))
            .map(|result| {
                result.and_then(|value| value.ok_or(SqliteError::NotFound { entity: "schedule" }))
            })
            .collect()
    }

    fn insert(&mut self, schedule: &Schedule) -> Result<(), Self::Error> {
        let transaction = self.connection.transaction()?;
        insert_schedule(&transaction, schedule)?;
        transaction.commit()?;
        Ok(())
    }
}

impl OccurrenceRepository for SqliteStore {
    type Error = SqliteError;

    fn find(&self, id: OccurrenceId) -> Result<Option<Occurrence>, Self::Error> {
        load_occurrence(&self.connection, id)
    }

    fn for_week(&self, week: IsoWeek) -> Result<Vec<Occurrence>, Self::Error> {
        let start = week.monday()?;
        let end = start
            .as_date()
            .checked_add(Duration::days(6))
            .map(CalendarDate::from_date)
            .ok_or(SqliteError::DateOutOfRange)?;
        let mut statement = self.connection.prepare(
            "SELECT id FROM occurrences WHERE due_date BETWEEN ?1 AND ?2 \
             ORDER BY due_date, name_snapshot, id",
        )?;
        let ids = statement
            .query_map(params![format_date(start)?, format_date(end)?], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        ids.into_iter()
            .map(|id| parse_occurrence_id(&id).and_then(|id| load_occurrence(&self.connection, id)))
            .map(|result| {
                result.and_then(|value| {
                    value.ok_or(SqliteError::NotFound {
                        entity: "occurrence",
                    })
                })
            })
            .collect()
    }

    fn save(&mut self, occurrence: &Occurrence) -> Result<(), Self::Error> {
        save_occurrence(&self.connection, occurrence)
    }
}

impl TransactionalStore for SqliteStore {
    type Error = SqliteError;

    fn materialize_range(
        &mut self,
        range: DateRange,
        created_at: Timestamp,
    ) -> Result<usize, Self::Error> {
        Self::materialize_range(self, range, created_at)
    }

    fn revise_schedule(
        &mut self,
        chore_id: ChoreId,
        replacement: &Schedule,
        effective: CalendarDate,
    ) -> Result<(), Self::Error> {
        Self::revise_schedule(self, chore_id, replacement, effective)
    }

    fn rename_chore(
        &mut self,
        chore_id: ChoreId,
        name: ChoreName,
        description: Option<Description>,
        today: CalendarDate,
        updated_at: Timestamp,
    ) -> Result<(), Self::Error> {
        Self::rename_chore(self, chore_id, name, description, today, updated_at)
    }

    fn disable_chore(
        &mut self,
        chore_id: ChoreId,
        today: CalendarDate,
        updated_at: Timestamp,
    ) -> Result<(), Self::Error> {
        Self::disable_chore(self, chore_id, today, updated_at)
    }

    fn soft_delete_chore(
        &mut self,
        chore_id: ChoreId,
        today: CalendarDate,
        deleted_at: Timestamp,
    ) -> Result<(), Self::Error> {
        Self::soft_delete_chore(self, chore_id, today, deleted_at)
    }

    fn reenable_chore(
        &mut self,
        chore_id: ChoreId,
        today: CalendarDate,
        updated_at: Timestamp,
    ) -> Result<Schedule, Self::Error> {
        Self::reenable_chore(self, chore_id, today, updated_at)
    }

    fn toggle_completion(
        &mut self,
        occurrence_id: OccurrenceId,
        at: Timestamp,
    ) -> Result<Occurrence, Self::Error> {
        Self::toggle_completion(self, occurrence_id, at)
    }
}

fn save_chore(connection: &Connection, chore: &Chore) -> Result<(), SqliteError> {
    let timestamps = chore.timestamps();
    connection.execute(
        "INSERT INTO chores(id, name, description, enabled, created_at, updated_at, deleted_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
         description = excluded.description, enabled = excluded.enabled, \
         updated_at = excluded.updated_at, deleted_at = excluded.deleted_at",
        params![
            chore.id().to_string(),
            chore.name().as_str(),
            chore.description().map(Description::as_str),
            chore.is_enabled(),
            format_timestamp(timestamps.created_at)?,
            format_timestamp(timestamps.updated_at)?,
            timestamps.deleted_at.map(format_timestamp).transpose()?,
        ],
    )?;
    Ok(())
}

fn insert_schedule(connection: &Connection, schedule: &Schedule) -> Result<(), SqliteError> {
    let window = schedule.window();
    connection.execute(
        "INSERT INTO schedules( \
           id, chore_id, kind, interval, anchor_date, monthly_day, \
           valid_from, valid_until, created_at \
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            schedule.id().to_string(),
            schedule.chore_id().to_string(),
            recurrence_kind_text(schedule.kind()),
            i64::from(schedule.interval().get()),
            format_date(window.anchor_date())?,
            schedule.monthly_day().map(MonthlyDay::get),
            format_date(window.valid_from())?,
            window.valid_until().map(format_date).transpose()?,
            format_timestamp(window.created_at())?,
        ],
    )?;
    for weekday in schedule.weekdays() {
        connection.execute(
            "INSERT INTO schedule_weekdays(schedule_id, weekday) VALUES (?1, ?2)",
            params![schedule.id().to_string(), i64::from(weekday.number())],
        )?;
    }
    Ok(())
}

fn insert_occurrence_if_missing(
    connection: &Connection,
    occurrence: &Occurrence,
) -> Result<usize, SqliteError> {
    let (state, completed_at) = occurrence_state_values(occurrence)?;
    connection
        .execute(
            "INSERT OR IGNORE INTO occurrences( \
               id, chore_id, schedule_id, nominal_date, due_date, name_snapshot, \
               description_snapshot, state, completed_at, created_at, updated_at \
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                occurrence.id().to_string(),
                occurrence.chore_id().to_string(),
                occurrence.schedule_id().to_string(),
                format_date(occurrence.nominal_date())?,
                format_date(occurrence.due_date())?,
                occurrence.name().as_str(),
                occurrence.description().map(Description::as_str),
                state,
                completed_at,
                format_timestamp(occurrence.created_at())?,
                format_timestamp(occurrence.updated_at())?,
            ],
        )
        .map_err(SqliteError::from)
}

fn save_occurrence(connection: &Connection, occurrence: &Occurrence) -> Result<(), SqliteError> {
    let (state, completed_at) = occurrence_state_values(occurrence)?;
    connection.execute(
        "INSERT INTO occurrences( \
           id, chore_id, schedule_id, nominal_date, due_date, name_snapshot, \
           description_snapshot, state, completed_at, created_at, updated_at \
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT(id) DO UPDATE SET due_date = excluded.due_date, \
         state = excluded.state, completed_at = excluded.completed_at, \
         updated_at = excluded.updated_at",
        params![
            occurrence.id().to_string(),
            occurrence.chore_id().to_string(),
            occurrence.schedule_id().to_string(),
            format_date(occurrence.nominal_date())?,
            format_date(occurrence.due_date())?,
            occurrence.name().as_str(),
            occurrence.description().map(Description::as_str),
            state,
            completed_at,
            format_timestamp(occurrence.created_at())?,
            format_timestamp(occurrence.updated_at())?,
        ],
    )?;
    Ok(())
}

fn occurrence_state_values(
    occurrence: &Occurrence,
) -> Result<(&'static str, Option<String>), SqliteError> {
    match occurrence.state() {
        OccurrenceState::Pending => Ok(("pending", None)),
        OccurrenceState::Completed { at } => Ok(("completed", Some(format_timestamp(at)?))),
        OccurrenceState::Skipped => Ok(("skipped", None)),
    }
}

fn load_chore(connection: &Connection, id: ChoreId) -> Result<Option<Chore>, SqliteError> {
    connection
        .query_row(
            "SELECT id, name, description, enabled, created_at, updated_at, deleted_at \
             FROM chores WHERE id = ?1",
            [id.to_string()],
            row_to_chore,
        )
        .optional()
        .map_err(SqliteError::from)
}

fn row_to_chore(row: &Row<'_>) -> rusqlite::Result<Chore> {
    let id = parse_chore_id_row(&row.get::<_, String>(0)?, 0)?;
    let name = ChoreName::new(row.get::<_, String>(1)?).map_err(|error| row_error(1, error))?;
    let description = row
        .get::<_, Option<String>>(2)?
        .map(Description::optional)
        .transpose()
        .map_err(|error| row_error(2, error))?
        .flatten();
    let enabled = row.get(3)?;
    let created_at = parse_timestamp_row(&row.get::<_, String>(4)?, 4)?;
    let updated_at = parse_timestamp_row(&row.get::<_, String>(5)?, 5)?;
    let deleted_at = row
        .get::<_, Option<String>>(6)?
        .map(|value| parse_timestamp_row(&value, 6))
        .transpose()?;
    Chore::restore(
        id,
        name,
        description,
        enabled,
        ChoreTimestamps {
            created_at,
            updated_at,
            deleted_at,
        },
    )
    .map_err(|error| row_error(0, error))
}

#[derive(Debug)]
struct RawSchedule {
    id: ScheduleId,
    chore_id: ChoreId,
    kind: String,
    interval: u16,
    anchor_date: CalendarDate,
    monthly_day: Option<u8>,
    valid_from: CalendarDate,
    valid_until: Option<CalendarDate>,
    created_at: Timestamp,
}

fn load_schedule(connection: &Connection, id: ScheduleId) -> Result<Option<Schedule>, SqliteError> {
    let raw = connection
        .query_row(
            "SELECT id, chore_id, kind, interval, anchor_date, monthly_day, \
             valid_from, valid_until, created_at FROM schedules WHERE id = ?1",
            [id.to_string()],
            row_to_raw_schedule,
        )
        .optional()?;
    raw.map(|raw| hydrate_schedule(connection, raw)).transpose()
}

fn row_to_raw_schedule(row: &Row<'_>) -> rusqlite::Result<RawSchedule> {
    Ok(RawSchedule {
        id: parse_schedule_id_row(&row.get::<_, String>(0)?, 0)?,
        chore_id: parse_chore_id_row(&row.get::<_, String>(1)?, 1)?,
        kind: row.get(2)?,
        interval: row.get(3)?,
        anchor_date: parse_date_row(&row.get::<_, String>(4)?, 4)?,
        monthly_day: row.get(5)?,
        valid_from: parse_date_row(&row.get::<_, String>(6)?, 6)?,
        valid_until: row
            .get::<_, Option<String>>(7)?
            .map(|value| parse_date_row(&value, 7))
            .transpose()?,
        created_at: parse_timestamp_row(&row.get::<_, String>(8)?, 8)?,
    })
}

fn hydrate_schedule(connection: &Connection, raw: RawSchedule) -> Result<Schedule, SqliteError> {
    let window = ScheduleWindow::new(
        raw.anchor_date,
        raw.valid_from,
        raw.valid_until,
        raw.created_at,
    )?;
    let interval = RecurrenceInterval::new(raw.interval)?;
    match raw.kind.as_str() {
        "weekly" => {
            let mut statement = connection.prepare(
                "SELECT weekday FROM schedule_weekdays WHERE schedule_id = ?1 ORDER BY weekday",
            )?;
            let weekdays = statement
                .query_map([raw.id.to_string()], |row| row.get::<_, u8>(0))?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(IsoWeekday::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            Schedule::weekly(raw.id, raw.chore_id, interval, weekdays, window)
                .map_err(SqliteError::from)
        }
        "daily_interval" => Ok(Schedule::daily_interval(
            raw.id,
            raw.chore_id,
            interval,
            window,
        )),
        "monthly" => Ok(Schedule::monthly(
            raw.id,
            raw.chore_id,
            MonthlyDay::new(
                raw.monthly_day
                    .ok_or_else(|| SqliteError::InvalidStoredValue {
                        field: "monthly_day",
                        value: "NULL".to_owned(),
                    })?,
            )?,
            window,
        )),
        _ => Err(SqliteError::InvalidStoredValue {
            field: "schedule kind",
            value: raw.kind,
        }),
    }
}

fn load_occurrence(
    connection: &Connection,
    id: OccurrenceId,
) -> Result<Option<Occurrence>, SqliteError> {
    connection
        .query_row(
            "SELECT id, chore_id, schedule_id, nominal_date, due_date, \
             name_snapshot, description_snapshot, state, completed_at, created_at, updated_at \
             FROM occurrences WHERE id = ?1",
            [id.to_string()],
            row_to_occurrence,
        )
        .optional()
        .map_err(SqliteError::from)
}

fn row_to_occurrence(row: &Row<'_>) -> rusqlite::Result<Occurrence> {
    let completed_at = row
        .get::<_, Option<String>>(8)?
        .map(|value| parse_timestamp_row(&value, 8))
        .transpose()?;
    let state_text = row.get::<_, String>(7)?;
    let state = match (state_text.as_str(), completed_at) {
        ("pending", None) => OccurrenceState::Pending,
        ("completed", Some(at)) => OccurrenceState::Completed { at },
        ("skipped", None) => OccurrenceState::Skipped,
        _ => return Err(row_error(7, PersistedStateError(state_text))),
    };
    let description = row
        .get::<_, Option<String>>(6)?
        .map(Description::optional)
        .transpose()
        .map_err(|error| row_error(6, error))?
        .flatten();
    Ok(Occurrence::restore(
        OccurrenceSeed {
            id: parse_occurrence_id_row(&row.get::<_, String>(0)?, 0)?,
            chore_id: parse_chore_id_row(&row.get::<_, String>(1)?, 1)?,
            schedule_id: parse_schedule_id_row(&row.get::<_, String>(2)?, 2)?,
            nominal_date: parse_date_row(&row.get::<_, String>(3)?, 3)?,
            due_date: parse_date_row(&row.get::<_, String>(4)?, 4)?,
            name: ChoreName::new(row.get::<_, String>(5)?).map_err(|error| row_error(5, error))?,
            description,
            created_at: parse_timestamp_row(&row.get::<_, String>(9)?, 9)?,
        },
        state,
        parse_timestamp_row(&row.get::<_, String>(10)?, 10)?,
    ))
}

#[derive(Debug, Error)]
#[error("invalid occurrence state: {0}")]
struct PersistedStateError(String);

fn generation_schedule_ids(
    connection: &Connection,
    range: DateRange,
) -> Result<Vec<ScheduleId>, SqliteError> {
    let mut statement = connection.prepare(
        "SELECT s.id FROM schedules s JOIN chores c ON c.id = s.chore_id \
         WHERE c.enabled = 1 AND c.deleted_at IS NULL \
           AND s.valid_from <= ?2 AND (s.valid_until IS NULL OR s.valid_until >= ?1) \
         ORDER BY s.created_at, s.id",
    )?;
    statement
        .query_map(
            params![format_date(range.start())?, format_date(range.end())?],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|value| parse_schedule_id(&value))
        .collect()
}

fn active_schedule(
    connection: &Connection,
    chore_id: ChoreId,
) -> Result<Option<Schedule>, SqliteError> {
    let id = connection
        .query_row(
            "SELECT id FROM schedules WHERE chore_id = ?1 AND valid_until IS NULL",
            [chore_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    id.map(|value| parse_schedule_id(&value))
        .transpose()?
        .map(|id| load_schedule(connection, id))
        .transpose()
        .map(Option::flatten)
}

fn latest_schedule(
    connection: &Connection,
    chore_id: ChoreId,
) -> Result<Option<Schedule>, SqliteError> {
    let id = connection
        .query_row(
            "SELECT id FROM schedules WHERE chore_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
            [chore_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    id.map(|value| parse_schedule_id(&value))
        .transpose()?
        .map(|id| load_schedule(connection, id))
        .transpose()
        .map(Option::flatten)
}

fn close_schedule(
    connection: &Connection,
    schedule: &Schedule,
    effective: CalendarDate,
) -> Result<(), SqliteError> {
    let previous_day = effective
        .as_date()
        .checked_sub(Duration::days(1))
        .map(CalendarDate::from_date)
        .ok_or(SqliteError::DateOutOfRange)?;
    let valid_until = previous_day.max(schedule.window().valid_from());
    connection.execute(
        "UPDATE schedules SET valid_until = ?1 WHERE id = ?2 AND valid_until IS NULL",
        params![format_date(valid_until)?, schedule.id().to_string()],
    )?;
    Ok(())
}

fn equivalent_schedule(
    source: &Schedule,
    today: CalendarDate,
    created_at: Timestamp,
) -> Result<Schedule, SqliteError> {
    let window = ScheduleWindow::new(today, today, None, created_at)?;
    let id = ScheduleId::new();
    match source.kind() {
        RecurrenceKind::Weekly => Schedule::weekly(
            id,
            source.chore_id(),
            source.interval(),
            source.weekdays().iter().copied(),
            window,
        )
        .map_err(SqliteError::from),
        RecurrenceKind::DailyInterval => Ok(Schedule::daily_interval(
            id,
            source.chore_id(),
            source.interval(),
            window,
        )),
        RecurrenceKind::Monthly => Ok(Schedule::monthly(
            id,
            source.chore_id(),
            source
                .monthly_day()
                .ok_or_else(|| SqliteError::InvalidStoredValue {
                    field: "monthly_day",
                    value: "NULL".to_owned(),
                })?,
            window,
        )),
    }
}

fn recurrence_kind_text(kind: RecurrenceKind) -> &'static str {
    match kind {
        RecurrenceKind::Weekly => "weekly",
        RecurrenceKind::DailyInterval => "daily_interval",
        RecurrenceKind::Monthly => "monthly",
    }
}

fn query_id_strings(connection: &Connection, sql: &str) -> Result<Vec<String>, SqliteError> {
    let mut statement = connection.prepare(sql)?;
    statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(SqliteError::from)
}

fn format_date(value: CalendarDate) -> Result<String, SqliteError> {
    value
        .as_date()
        .format(DATE_FORMAT)
        .map_err(|error| SqliteError::Format {
            field: "calendar date",
            message: error.to_string(),
        })
}

fn format_timestamp(value: Timestamp) -> Result<String, SqliteError> {
    value
        .as_datetime()
        .format(&Rfc3339)
        .map_err(|error| SqliteError::Format {
            field: "timestamp",
            message: error.to_string(),
        })
}

fn parse_date(value: &str) -> Result<CalendarDate, SqliteError> {
    Date::parse(value, DATE_FORMAT)
        .map(CalendarDate::from_date)
        .map_err(|_| SqliteError::InvalidStoredValue {
            field: "calendar date",
            value: value.to_owned(),
        })
}

fn parse_timestamp(value: &str) -> Result<Timestamp, SqliteError> {
    time::OffsetDateTime::parse(value, &Rfc3339)
        .map(Timestamp::from_datetime)
        .map_err(|_| SqliteError::InvalidStoredValue {
            field: "timestamp",
            value: value.to_owned(),
        })
}

fn parse_chore_id(value: &str) -> Result<ChoreId, SqliteError> {
    parse_uuid(value, "chore identifier")
        .and_then(|value| ChoreId::from_uuid(value).map_err(Into::into))
}

fn parse_schedule_id(value: &str) -> Result<ScheduleId, SqliteError> {
    parse_uuid(value, "schedule identifier")
        .and_then(|value| ScheduleId::from_uuid(value).map_err(Into::into))
}

fn parse_occurrence_id(value: &str) -> Result<OccurrenceId, SqliteError> {
    parse_uuid(value, "occurrence identifier")
        .and_then(|value| OccurrenceId::from_uuid(value).map_err(Into::into))
}

fn parse_uuid(value: &str, field: &'static str) -> Result<Uuid, SqliteError> {
    Uuid::parse_str(value).map_err(|_| SqliteError::InvalidStoredValue {
        field,
        value: value.to_owned(),
    })
}

fn parse_chore_id_row(value: &str, index: usize) -> rusqlite::Result<ChoreId> {
    parse_chore_id(value).map_err(|error| row_error(index, error))
}

fn parse_schedule_id_row(value: &str, index: usize) -> rusqlite::Result<ScheduleId> {
    parse_schedule_id(value).map_err(|error| row_error(index, error))
}

fn parse_occurrence_id_row(value: &str, index: usize) -> rusqlite::Result<OccurrenceId> {
    parse_occurrence_id(value).map_err(|error| row_error(index, error))
}

fn parse_date_row(value: &str, index: usize) -> rusqlite::Result<CalendarDate> {
    parse_date(value).map_err(|error| row_error(index, error))
}

fn parse_timestamp_row(value: &str, index: usize) -> rusqlite::Result<Timestamp> {
    parse_timestamp(value).map_err(|error| row_error(index, error))
}

fn row_error(
    index: usize,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
mod tests {
    use tempfile::{TempDir, tempdir_in};

    use super::*;

    fn date(year: i32, month: u8, day: u8) -> CalendarDate {
        CalendarDate::new(year, month, day).expect("test date should be valid")
    }

    fn timestamp(seconds: i64) -> Timestamp {
        Timestamp::from_unix_timestamp(seconds).expect("test timestamp should be valid")
    }

    fn name(value: &str) -> ChoreName {
        ChoreName::new(value).expect("test name should be valid")
    }

    fn range(start_day: u8, end_day: u8) -> DateRange {
        DateRange::new(date(2026, 9, start_day), date(2026, 9, end_day))
            .expect("test range should be valid")
    }

    fn temp_directory() -> TempDir {
        tempdir_in(std::env::current_dir().expect("working directory should exist"))
            .expect("temporary directory should exist")
    }

    fn seed_daily(store: &mut SqliteStore, start_day: u8, interval: u16) -> (Chore, Schedule) {
        let created_at = timestamp(1);
        let chore = Chore::new(ChoreId::new(), name("Vacuum"), None, created_at);
        ChoreRepository::save(store, &chore).expect("chore should save");
        let schedule = Schedule::daily_interval(
            ScheduleId::new(),
            chore.id(),
            RecurrenceInterval::new(interval).expect("interval should be valid"),
            ScheduleWindow::new(
                date(2026, 9, start_day),
                date(2026, 9, start_day),
                None,
                created_at,
            )
            .expect("window should be valid"),
        );
        ScheduleRepository::insert(store, &schedule).expect("schedule should save");
        (chore, schedule)
    }

    fn occurrence_on(store: &SqliteStore, day: u8) -> Occurrence {
        let id = store
            .connection
            .query_row(
                "SELECT id FROM occurrences WHERE due_date = ?1",
                [format_date(date(2026, 9, day)).expect("date should format")],
                |row| row.get::<_, String>(0),
            )
            .expect("occurrence should exist");
        load_occurrence(
            &store.connection,
            parse_occurrence_id(&id).expect("id is valid"),
        )
        .expect("occurrence should load")
        .expect("occurrence should exist")
    }

    #[test]
    fn fresh_and_already_migrated_file_open_with_required_pragmas() {
        let directory = temp_directory();
        let path = directory.path().join("chores.db");

        let first = SqliteStore::open(&path).expect("fresh database should open");
        assert_eq!(
            first
                .connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .expect("pragma should load"),
            1
        );
        assert_eq!(
            first
                .connection
                .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
                .expect("pragma should load"),
            "wal"
        );
        assert_eq!(
            first
                .connection
                .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                .expect("pragma should load"),
            5_000
        );
        drop(first);

        let reopened = SqliteStore::open(&path).expect("migrated database should reopen");
        assert_eq!(
            reopened
                .connection
                .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("migration count should load"),
            migrations::LATEST_VERSION
        );
        assert_eq!(
            reopened.integrity_check().expect("check should run"),
            ["ok"]
        );
    }

    #[test]
    fn newer_schema_is_refused_without_mutation() {
        let directory = temp_directory();
        let path = directory.path().join("future.db");
        let connection = Connection::open(&path).expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL); \
                 INSERT INTO schema_migrations VALUES (99, 'future');",
            )
            .expect("future fixture should be created");
        drop(connection);

        assert!(matches!(
            SqliteStore::open(&path),
            Err(SqliteError::NewerSchema {
                found: 99,
                supported: migrations::LATEST_VERSION,
            })
        ));
    }

    #[test]
    fn sqlite_constraints_and_foreign_keys_are_enforced() {
        let store = SqliteStore::open_in_memory().expect("database should open");
        assert!(
            store
                .connection
                .execute(
                    "INSERT INTO chores VALUES ('x', '', NULL, 1, 'a', 'a', NULL)",
                    [],
                )
                .is_err()
        );
        assert!(
            store
                .connection
                .execute(
                    "INSERT INTO schedules VALUES ( \
                       'x', 'missing', 'daily_interval', 1, '2026-09-01', NULL, \
                       '2026-09-01', NULL, '2026-09-01T00:00:00Z' \
                     )",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn repository_round_trips_weekly_schedule_weekdays() {
        let mut store = SqliteStore::open_in_memory().expect("database should open");
        let chore = Chore::new(ChoreId::new(), name("Bins"), None, timestamp(1));
        ChoreRepository::save(&mut store, &chore).expect("chore should save");
        let schedule = Schedule::weekly(
            ScheduleId::new(),
            chore.id(),
            RecurrenceInterval::new(2).expect("interval should be valid"),
            [IsoWeekday::Monday, IsoWeekday::Friday],
            ScheduleWindow::new(date(2026, 9, 1), date(2026, 9, 1), None, timestamp(1))
                .expect("window should be valid"),
        )
        .expect("schedule should be valid");
        ScheduleRepository::insert(&mut store, &schedule).expect("schedule should save");

        assert_eq!(
            ScheduleRepository::for_chore(&store, chore.id()).expect("schedules should load"),
            [schedule]
        );
    }

    #[test]
    fn repeated_materialization_is_idempotent_and_round_trips() {
        let mut store = SqliteStore::open_in_memory().expect("database should open");
        let (_chore, schedule) = seed_daily(&mut store, 1, 2);

        assert_eq!(
            store
                .materialize_range(range(1, 6), timestamp(10))
                .expect("materialization should work"),
            3
        );
        assert_eq!(
            store
                .materialize_range(range(1, 6), timestamp(20))
                .expect("repeat should work"),
            0
        );
        let occurrences =
            OccurrenceRepository::for_week(&store, IsoWeek::containing(date(2026, 9, 1)))
                .expect("week should load");
        assert_eq!(occurrences.len(), 3);
        assert!(
            occurrences
                .iter()
                .all(|occurrence| occurrence.schedule_id() == schedule.id())
        );
    }

    #[test]
    fn failed_schedule_revision_rolls_back_closed_schedule() {
        let mut store = SqliteStore::open_in_memory().expect("database should open");
        let (chore, schedule) = seed_daily(&mut store, 1, 1);
        let replacement = Schedule::daily_interval(
            schedule.id(),
            chore.id(),
            RecurrenceInterval::new(2).expect("interval should be valid"),
            ScheduleWindow::new(date(2026, 9, 5), date(2026, 9, 5), None, timestamp(20))
                .expect("window should be valid"),
        );

        assert!(
            store
                .revise_schedule(chore.id(), &replacement, date(2026, 9, 5))
                .is_err()
        );
        assert_eq!(
            active_schedule(&store.connection, chore.id())
                .expect("active schedule should load")
                .expect("active schedule should remain")
                .id(),
            schedule.id()
        );
    }

    #[test]
    fn revision_replaces_only_eligible_future_pending_rows() {
        let mut store = SqliteStore::open_in_memory().expect("database should open");
        let (chore, original) = seed_daily(&mut store, 1, 1);
        store
            .materialize_range(range(1, 10), timestamp(10))
            .expect("materialization should work");
        let completed = occurrence_on(&store, 6);
        store
            .toggle_completion(completed.id(), timestamp(20))
            .expect("completion should toggle");
        let skipped = occurrence_on(&store, 8);
        store
            .connection
            .execute(
                "UPDATE occurrences SET state = 'skipped' WHERE id = ?1",
                [skipped.id().to_string()],
            )
            .expect("skip fixture should update");
        let replacement = Schedule::daily_interval(
            ScheduleId::new(),
            chore.id(),
            RecurrenceInterval::new(2).expect("interval should be valid"),
            ScheduleWindow::new(date(2026, 9, 5), date(2026, 9, 5), None, timestamp(30))
                .expect("window should be valid"),
        );

        store
            .revise_schedule(chore.id(), &replacement, date(2026, 9, 5))
            .expect("revision should succeed");
        assert_eq!(
            store
                .materialize_range(range(1, 10), timestamp(40))
                .expect("replacement should materialize"),
            3
        );
        assert_eq!(
            occurrence_on(&store, 6).state(),
            OccurrenceState::Completed { at: timestamp(20) }
        );
        assert_eq!(occurrence_on(&store, 8).state(), OccurrenceState::Skipped);
        assert_eq!(occurrence_on(&store, 5).schedule_id(), replacement.id());
        assert_eq!(occurrence_on(&store, 7).schedule_id(), replacement.id());
        assert_eq!(occurrence_on(&store, 9).schedule_id(), replacement.id());
        let original_future_pending = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM occurrences \
                 WHERE schedule_id = ?1 AND state = 'pending' AND due_date >= '2026-09-05'",
                [original.id().to_string()],
                |row| row.get::<_, i64>(0),
            )
            .expect("count should load");
        assert_eq!(original_future_pending, 0);
    }

    #[test]
    fn rename_updates_only_future_pending_snapshots() {
        let mut store = SqliteStore::open_in_memory().expect("database should open");
        let (chore, _schedule) = seed_daily(&mut store, 1, 1);
        store
            .materialize_range(range(1, 5), timestamp(10))
            .expect("materialization should work");
        let completed = occurrence_on(&store, 2);
        store
            .toggle_completion(completed.id(), timestamp(20))
            .expect("completion should toggle");

        store
            .rename_chore(
                chore.id(),
                name("Clean floors"),
                Description::optional("Downstairs").expect("description should be valid"),
                date(2026, 9, 3),
                timestamp(30),
            )
            .expect("rename should succeed");

        assert_eq!(occurrence_on(&store, 1).name().as_str(), "Vacuum");
        assert_eq!(occurrence_on(&store, 2).name().as_str(), "Vacuum");
        assert_eq!(occurrence_on(&store, 3).name().as_str(), "Clean floors");
        assert_eq!(
            occurrence_on(&store, 3)
                .description()
                .map(Description::as_str),
            Some("Downstairs")
        );
    }

    #[test]
    fn disable_and_reenable_leave_a_non_backfilled_gap() {
        let mut store = SqliteStore::open_in_memory().expect("database should open");
        let (chore, _schedule) = seed_daily(&mut store, 1, 1);
        store
            .materialize_range(range(1, 10), timestamp(10))
            .expect("materialization should work");
        store
            .disable_chore(chore.id(), date(2026, 9, 4), timestamp(20))
            .expect("disable should work");
        store
            .reenable_chore(chore.id(), date(2026, 9, 7), timestamp(30))
            .expect("re-enable should work");
        assert_eq!(
            store
                .materialize_range(range(1, 10), timestamp(40))
                .expect("materialization should work"),
            4
        );

        let dates = store
            .connection
            .prepare("SELECT due_date FROM occurrences ORDER BY due_date")
            .expect("query should prepare")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query should run")
            .collect::<Result<Vec<_>, _>>()
            .expect("dates should load");
        assert_eq!(
            dates,
            [
                "2026-09-01",
                "2026-09-02",
                "2026-09-03",
                "2026-09-07",
                "2026-09-08",
                "2026-09-09",
                "2026-09-10",
            ]
        );
    }

    #[test]
    fn soft_delete_retains_history_and_suppresses_generation() {
        let mut store = SqliteStore::open_in_memory().expect("database should open");
        let (chore, _schedule) = seed_daily(&mut store, 1, 1);
        store
            .materialize_range(range(1, 7), timestamp(10))
            .expect("materialization should work");
        let completed = occurrence_on(&store, 2);
        store
            .toggle_completion(completed.id(), timestamp(20))
            .expect("completion should toggle");

        store
            .soft_delete_chore(chore.id(), date(2026, 9, 4), timestamp(30))
            .expect("delete should work");
        assert_eq!(
            store
                .materialize_range(range(1, 7), timestamp(40))
                .expect("materialization should work"),
            0
        );
        let stored = ChoreRepository::find(&store, chore.id())
            .expect("chore should load")
            .expect("chore should remain");
        assert!(stored.is_deleted());
        assert!(!stored.is_enabled());
        let count = store
            .connection
            .query_row("SELECT COUNT(*) FROM occurrences", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("count should load");
        assert_eq!(count, 3);
        assert_eq!(
            occurrence_on(&store, 2).state(),
            OccurrenceState::Completed { at: timestamp(20) }
        );
    }

    #[test]
    fn editor_create_rolls_back_chore_when_schedule_insert_fails() {
        let mut store = SqliteStore::open_in_memory().expect("database should open");
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_editor_schedule BEFORE INSERT ON schedules \
                 BEGIN SELECT RAISE(ABORT, 'injected schedule failure'); END;",
            )
            .expect("failure trigger should install");
        let chore = Chore::new(ChoreId::new(), name("Rollback"), None, timestamp(1));
        let schedule = Schedule::daily_interval(
            ScheduleId::new(),
            chore.id(),
            RecurrenceInterval::new(1).expect("interval should be valid"),
            ScheduleWindow::new(date(2026, 9, 1), date(2026, 9, 1), None, timestamp(1))
                .expect("window should be valid"),
        );

        assert!(
            AtomicEditorStore::create_editor_chore(&mut store, &chore, &schedule, None).is_err()
        );
        assert!(
            ChoreRepository::find(&store, chore.id())
                .expect("lookup should succeed")
                .is_none()
        );
    }

    #[test]
    fn editor_create_rolls_back_chore_and_schedule_when_provenance_insert_fails() {
        let mut store = SqliteStore::open_in_memory().expect("database should open");
        let chore = Chore::new(ChoreId::new(), name("Rollback"), None, timestamp(1));
        let schedule = Schedule::daily_interval(
            ScheduleId::new(),
            chore.id(),
            RecurrenceInterval::new(1).expect("interval should be valid"),
            ScheduleWindow::new(date(2026, 9, 1), date(2026, 9, 1), None, timestamp(1))
                .expect("window should be valid"),
        );
        let invalid = TemplateProvenance {
            template_id: "bathroom.scrub_shower".to_owned(),
            schema_version: 1,
            catalog_version: 2,
            locale: String::new(),
        };

        assert!(
            AtomicEditorStore::create_editor_chore(&mut store, &chore, &schedule, Some(&invalid),)
                .is_err()
        );
        assert!(
            ChoreRepository::find(&store, chore.id())
                .expect("lookup should succeed")
                .is_none()
        );
        assert!(
            ScheduleRepository::for_chore(&store, chore.id())
                .expect("schedule lookup should succeed")
                .is_empty()
        );
    }
}
