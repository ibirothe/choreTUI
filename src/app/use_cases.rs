//! Application-level orchestration for transactional persistence operations.

use std::error::Error;

use crate::{
    domain::{
        ChoreId, ChoreName, Description, Occurrence, OccurrenceId, Schedule, Timestamp,
        ports::Clock,
    },
    recurrence::DateRange,
};

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
