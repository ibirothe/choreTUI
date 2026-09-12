//! Inward-facing repository and clock interfaces.

use std::error::Error;

use super::{CalendarDate, Chore, ChoreId, IsoWeek, Occurrence, OccurrenceId, Schedule, Timestamp};

/// A deterministic source of calendar and timestamp values.
pub trait Clock {
    /// Return today's local calendar date.
    fn today(&self) -> CalendarDate;

    /// Return the current normalized UTC timestamp.
    fn now(&self) -> Timestamp;
}

/// Persistence operations for stable chore identities.
pub trait ChoreRepository {
    /// Adapter-specific failure type.
    type Error: Error + Send + Sync + 'static;

    /// Load a chore by identifier.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when the query fails.
    fn find(&self, id: ChoreId) -> Result<Option<Chore>, Self::Error>;

    /// List chores, optionally including soft-deleted records.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when the query fails.
    fn list(&self, include_deleted: bool) -> Result<Vec<Chore>, Self::Error>;

    /// Insert or update one chore.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when persistence fails.
    fn save(&mut self, chore: &Chore) -> Result<(), Self::Error>;
}

/// Persistence operations for immutable schedule revisions.
pub trait ScheduleRepository {
    /// Adapter-specific failure type.
    type Error: Error + Send + Sync + 'static;

    /// Load all revisions for a chore in creation order.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when the query fails.
    fn for_chore(&self, chore_id: ChoreId) -> Result<Vec<Schedule>, Self::Error>;

    /// Insert one new schedule revision.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when persistence fails.
    fn insert(&mut self, schedule: &Schedule) -> Result<(), Self::Error>;
}

/// Persistence operations for independently stateful occurrences.
pub trait OccurrenceRepository {
    /// Adapter-specific failure type.
    type Error: Error + Send + Sync + 'static;

    /// Load one occurrence by identifier.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when the query fails.
    fn find(&self, id: OccurrenceId) -> Result<Option<Occurrence>, Self::Error>;

    /// Load occurrences whose due dates belong to an ISO week.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when the query fails.
    fn for_week(&self, week: IsoWeek) -> Result<Vec<Occurrence>, Self::Error>;

    /// Insert or update one occurrence.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when persistence fails.
    fn save(&mut self, occurrence: &Occurrence) -> Result<(), Self::Error>;
}
