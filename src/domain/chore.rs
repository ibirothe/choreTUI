//! Chore, schedule, and occurrence domain types.

use std::{collections::BTreeSet, fmt, num::NonZeroU16};

use thiserror::Error;
use time::{Date, Month, OffsetDateTime, UtcOffset};
use uuid::{Uuid, Version};

/// A domain validation failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    /// A UUID does not use UUID version 7.
    #[error("identifier must be a UUID v7")]
    IdentifierIsNotV7,
    /// A chore name is blank after trimming.
    #[error("chore name must not be empty")]
    EmptyName,
    /// A chore name exceeds the supported length.
    #[error("chore name must contain at most {max} characters")]
    NameTooLong {
        /// Maximum Unicode scalar count.
        max: usize,
    },
    /// A description exceeds the supported length.
    #[error("description must contain at most {max} characters")]
    DescriptionTooLong {
        /// Maximum Unicode scalar count.
        max: usize,
    },
    /// An interval is outside the supported range.
    #[error("recurrence interval must be between 1 and 999")]
    InvalidInterval,
    /// A monthly day is outside the supported range.
    #[error("monthly day must be between 1 and 31")]
    InvalidMonthlyDay,
    /// A weekday number is outside the ISO range.
    #[error("ISO weekday must be between 1 and 7")]
    InvalidWeekday,
    /// A date does not exist in the Gregorian calendar.
    #[error("invalid calendar date")]
    InvalidDate,
    /// A Unix timestamp is outside the supported range.
    #[error("invalid timestamp")]
    InvalidTimestamp,
    /// A schedule ends before it starts.
    #[error("schedule validity end must not precede its start")]
    InvalidValidityWindow,
    /// A weekly schedule has no weekdays.
    #[error("weekly schedule requires at least one weekday")]
    MissingWeeklyWeekday,
    /// A deleted chore is incorrectly marked as enabled.
    #[error("a deleted chore cannot be enabled")]
    DeletedChoreIsEnabled,
}

macro_rules! uuid_v7_id {
    ($name:ident) => {
        #[doc = concat!("A validated UUID v7 ", stringify!($name), ".")]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Generate a new UUID v7 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Validate and wrap an existing UUID.
            ///
            /// # Errors
            ///
            /// Returns [`ValidationError::IdentifierIsNotV7`] for any UUID
            /// version other than 7.
            pub fn from_uuid(value: Uuid) -> Result<Self, ValidationError> {
                if value.get_version() == Some(Version::SortRand) {
                    Ok(Self(value))
                } else {
                    Err(ValidationError::IdentifierIsNotV7)
                }
            }

            /// Return the wrapped UUID.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_v7_id!(ChoreId);
uuid_v7_id!(ScheduleId);
uuid_v7_id!(OccurrenceId);

/// A valid proleptic-Gregorian calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalendarDate(Date);

impl CalendarDate {
    /// Construct a date from numeric calendar components.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidDate`] when the date does not exist.
    pub fn new(year: i32, month: u8, day: u8) -> Result<Self, ValidationError> {
        let month = Month::try_from(month).map_err(|_| ValidationError::InvalidDate)?;
        Date::from_calendar_date(year, month, day)
            .map(Self)
            .map_err(|_| ValidationError::InvalidDate)
    }

    /// Wrap a date already validated by the `time` crate.
    #[must_use]
    pub const fn from_date(value: Date) -> Self {
        Self(value)
    }

    /// Return the wrapped date.
    #[must_use]
    pub const fn as_date(self) -> Date {
        self.0
    }
}

impl fmt::Display for CalendarDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A UTC timestamp normalized to whole-second precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(OffsetDateTime);

impl Timestamp {
    /// Normalize a timestamp to UTC and discard sub-second precision.
    #[must_use]
    pub fn from_datetime(value: OffsetDateTime) -> Self {
        let utc = value.to_offset(UtcOffset::UTC);
        Self(OffsetDateTime::from_unix_timestamp(utc.unix_timestamp()).unwrap_or(utc))
    }

    /// Construct a timestamp from whole Unix seconds.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidTimestamp`] when out of range.
    pub fn from_unix_timestamp(seconds: i64) -> Result<Self, ValidationError> {
        OffsetDateTime::from_unix_timestamp(seconds)
            .map(Self::from_datetime)
            .map_err(|_| ValidationError::InvalidTimestamp)
    }

    /// Return the normalized UTC timestamp.
    #[must_use]
    pub const fn as_datetime(self) -> OffsetDateTime {
        self.0
    }
}

/// A trimmed, non-empty chore name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChoreName(String);

impl ChoreName {
    /// Maximum number of Unicode scalar values in a name.
    pub const MAX_CHARS: usize = 80;

    /// Validate and normalize a chore name.
    ///
    /// # Errors
    ///
    /// Returns an error for blank input or more than 80 characters.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into().trim().to_owned();
        if value.is_empty() {
            return Err(ValidationError::EmptyName);
        }
        if value.chars().count() > Self::MAX_CHARS {
            return Err(ValidationError::NameTooLong {
                max: Self::MAX_CHARS,
            });
        }
        Ok(Self(value))
    }

    /// Borrow the normalized name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ChoreName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A trimmed, non-empty optional chore description.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Description(String);

impl Description {
    /// Maximum number of Unicode scalar values in a description.
    pub const MAX_CHARS: usize = 1_000;

    /// Normalize optional text, mapping blank input to `None`.
    ///
    /// # Errors
    ///
    /// Returns an error when the trimmed text exceeds 1,000 characters.
    pub fn optional(value: impl Into<String>) -> Result<Option<Self>, ValidationError> {
        let value = value.into().trim().to_owned();
        if value.is_empty() {
            return Ok(None);
        }
        if value.chars().count() > Self::MAX_CHARS {
            return Err(ValidationError::DescriptionTooLong {
                max: Self::MAX_CHARS,
            });
        }
        Ok(Some(Self(value)))
    }

    /// Borrow the normalized description.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An ISO weekday numbered Monday (`1`) through Sunday (`7`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum IsoWeekday {
    /// Monday.
    Monday = 1,
    /// Tuesday.
    Tuesday = 2,
    /// Wednesday.
    Wednesday = 3,
    /// Thursday.
    Thursday = 4,
    /// Friday.
    Friday = 5,
    /// Saturday.
    Saturday = 6,
    /// Sunday.
    Sunday = 7,
}

impl IsoWeekday {
    /// Return the ISO weekday number.
    #[must_use]
    pub const fn number(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for IsoWeekday {
    type Error = ValidationError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Monday),
            2 => Ok(Self::Tuesday),
            3 => Ok(Self::Wednesday),
            4 => Ok(Self::Thursday),
            5 => Ok(Self::Friday),
            6 => Ok(Self::Saturday),
            7 => Ok(Self::Sunday),
            _ => Err(ValidationError::InvalidWeekday),
        }
    }
}

/// A recurrence interval in the inclusive range `1..=999`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecurrenceInterval(NonZeroU16);

impl RecurrenceInterval {
    /// Validate an interval.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidInterval`] outside `1..=999`.
    pub fn new(value: u16) -> Result<Self, ValidationError> {
        if value > 999 {
            return Err(ValidationError::InvalidInterval);
        }
        NonZeroU16::new(value)
            .map(Self)
            .ok_or(ValidationError::InvalidInterval)
    }

    /// Return the interval value.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

/// A requested monthly day in the inclusive range `1..=31`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonthlyDay(u8);

impl MonthlyDay {
    /// Validate a monthly day.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidMonthlyDay`] outside `1..=31`.
    pub fn new(value: u8) -> Result<Self, ValidationError> {
        if (1..=31).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ValidationError::InvalidMonthlyDay)
        }
    }

    /// Return the requested day.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// The supported recurrence families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecurrenceKind {
    /// One or more weekdays every N ISO weeks.
    Weekly,
    /// One occurrence every N calendar days.
    DailyInterval,
    /// One occurrence each month on a requested day.
    Monthly,
}

/// Shared immutable dates for one schedule revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduleWindow {
    anchor_date: CalendarDate,
    valid_from: CalendarDate,
    valid_until: Option<CalendarDate>,
    created_at: Timestamp,
}

impl ScheduleWindow {
    /// Validate schedule phase and validity dates.
    ///
    /// # Errors
    ///
    /// Returns an error when `valid_until` precedes `valid_from`.
    pub fn new(
        anchor_date: CalendarDate,
        valid_from: CalendarDate,
        valid_until: Option<CalendarDate>,
        created_at: Timestamp,
    ) -> Result<Self, ValidationError> {
        if valid_until.is_some_and(|end| end < valid_from) {
            return Err(ValidationError::InvalidValidityWindow);
        }
        Ok(Self {
            anchor_date,
            valid_from,
            valid_until,
            created_at,
        })
    }

    /// Return the recurrence phase anchor.
    #[must_use]
    pub const fn anchor_date(self) -> CalendarDate {
        self.anchor_date
    }

    /// Return the inclusive validity start.
    #[must_use]
    pub const fn valid_from(self) -> CalendarDate {
        self.valid_from
    }

    /// Return the optional inclusive validity end.
    #[must_use]
    pub const fn valid_until(self) -> Option<CalendarDate> {
        self.valid_until
    }

    /// Return the creation timestamp.
    #[must_use]
    pub const fn created_at(self) -> Timestamp {
        self.created_at
    }
}

/// An immutable recurrence schedule revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    id: ScheduleId,
    chore_id: ChoreId,
    kind: RecurrenceKind,
    interval: RecurrenceInterval,
    weekdays: BTreeSet<IsoWeekday>,
    monthly_day: Option<MonthlyDay>,
    window: ScheduleWindow,
}

impl Schedule {
    /// Create a weekly or every-N-weeks schedule.
    ///
    /// Duplicate weekdays are normalized away.
    ///
    /// # Errors
    ///
    /// Returns an error when no weekday is supplied.
    pub fn weekly(
        id: ScheduleId,
        chore_id: ChoreId,
        interval: RecurrenceInterval,
        weekdays: impl IntoIterator<Item = IsoWeekday>,
        window: ScheduleWindow,
    ) -> Result<Self, ValidationError> {
        let weekdays = weekdays.into_iter().collect::<BTreeSet<_>>();
        if weekdays.is_empty() {
            return Err(ValidationError::MissingWeeklyWeekday);
        }
        Ok(Self {
            id,
            chore_id,
            kind: RecurrenceKind::Weekly,
            interval,
            weekdays,
            monthly_day: None,
            window,
        })
    }

    /// Create an every-N-days schedule.
    #[must_use]
    pub fn daily_interval(
        id: ScheduleId,
        chore_id: ChoreId,
        interval: RecurrenceInterval,
        window: ScheduleWindow,
    ) -> Self {
        Self {
            id,
            chore_id,
            kind: RecurrenceKind::DailyInterval,
            interval,
            weekdays: BTreeSet::new(),
            monthly_day: None,
            window,
        }
    }

    /// Create a monthly schedule.
    #[must_use]
    pub fn monthly(
        id: ScheduleId,
        chore_id: ChoreId,
        day: MonthlyDay,
        window: ScheduleWindow,
    ) -> Self {
        Self {
            id,
            chore_id,
            kind: RecurrenceKind::Monthly,
            interval: RecurrenceInterval(NonZeroU16::MIN),
            weekdays: BTreeSet::new(),
            monthly_day: Some(day),
            window,
        }
    }

    /// Return the schedule identifier.
    #[must_use]
    pub const fn id(&self) -> ScheduleId {
        self.id
    }

    /// Return the owning chore identifier.
    #[must_use]
    pub const fn chore_id(&self) -> ChoreId {
        self.chore_id
    }

    /// Return the recurrence family.
    #[must_use]
    pub const fn kind(&self) -> RecurrenceKind {
        self.kind
    }

    /// Return the recurrence interval.
    #[must_use]
    pub const fn interval(&self) -> RecurrenceInterval {
        self.interval
    }

    /// Return the sorted weekly weekdays.
    #[must_use]
    pub const fn weekdays(&self) -> &BTreeSet<IsoWeekday> {
        &self.weekdays
    }

    /// Return the monthly day, if applicable.
    #[must_use]
    pub const fn monthly_day(&self) -> Option<MonthlyDay> {
        self.monthly_day
    }

    /// Return the immutable schedule window.
    #[must_use]
    pub const fn window(&self) -> ScheduleWindow {
        self.window
    }
}

/// Timestamps used to rehydrate a persisted chore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChoreTimestamps {
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Last update timestamp.
    pub updated_at: Timestamp,
    /// Optional soft-deletion timestamp.
    pub deleted_at: Option<Timestamp>,
}

/// A chore's stable identity and current lifecycle state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chore {
    id: ChoreId,
    name: ChoreName,
    description: Option<Description>,
    enabled: bool,
    timestamps: ChoreTimestamps,
}

impl Chore {
    /// Create an enabled chore.
    #[must_use]
    pub const fn new(
        id: ChoreId,
        name: ChoreName,
        description: Option<Description>,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id,
            name,
            description,
            enabled: true,
            timestamps: ChoreTimestamps {
                created_at,
                updated_at: created_at,
                deleted_at: None,
            },
        }
    }

    /// Rehydrate a chore while checking lifecycle invariants.
    ///
    /// # Errors
    ///
    /// Returns an error when a deleted chore is marked as enabled.
    pub fn restore(
        id: ChoreId,
        name: ChoreName,
        description: Option<Description>,
        enabled: bool,
        timestamps: ChoreTimestamps,
    ) -> Result<Self, ValidationError> {
        if enabled && timestamps.deleted_at.is_some() {
            return Err(ValidationError::DeletedChoreIsEnabled);
        }
        Ok(Self {
            id,
            name,
            description,
            enabled,
            timestamps,
        })
    }

    /// Change user-visible metadata.
    pub fn rename(
        &mut self,
        name: ChoreName,
        description: Option<Description>,
        updated_at: Timestamp,
    ) {
        self.name = name;
        self.description = description;
        self.timestamps.updated_at = updated_at;
    }

    /// Set the enabled state of a non-deleted chore.
    ///
    /// Returns `false` when the chore is deleted and therefore unchanged.
    pub fn set_enabled(&mut self, enabled: bool, updated_at: Timestamp) -> bool {
        if self.timestamps.deleted_at.is_some() {
            return false;
        }
        self.enabled = enabled;
        self.timestamps.updated_at = updated_at;
        true
    }

    /// Soft-delete and disable the chore.
    pub fn mark_deleted(&mut self, deleted_at: Timestamp) {
        self.enabled = false;
        self.timestamps.updated_at = deleted_at;
        self.timestamps.deleted_at = Some(deleted_at);
    }

    /// Return the stable identifier.
    #[must_use]
    pub const fn id(&self) -> ChoreId {
        self.id
    }

    /// Return the current name.
    #[must_use]
    pub const fn name(&self) -> &ChoreName {
        &self.name
    }

    /// Return the optional description.
    #[must_use]
    pub const fn description(&self) -> Option<&Description> {
        self.description.as_ref()
    }

    /// Return whether the chore currently generates occurrences.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Return whether the chore has been soft-deleted.
    #[must_use]
    pub const fn is_deleted(&self) -> bool {
        self.timestamps.deleted_at.is_some()
    }

    /// Return the lifecycle timestamps.
    #[must_use]
    pub const fn timestamps(&self) -> ChoreTimestamps {
        self.timestamps
    }
}

/// Valid data needed to materialize one pending occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccurrenceSeed {
    /// Occurrence identifier.
    pub id: OccurrenceId,
    /// Owning chore identifier.
    pub chore_id: ChoreId,
    /// Source schedule revision identifier.
    pub schedule_id: ScheduleId,
    /// Immutable recurrence-generated date.
    pub nominal_date: CalendarDate,
    /// Current displayed due date.
    pub due_date: CalendarDate,
    /// Chore name snapshot.
    pub name: ChoreName,
    /// Optional description snapshot.
    pub description: Option<Description>,
    /// Materialization timestamp.
    pub created_at: Timestamp,
}

/// Persisted state of an occurrence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccurrenceState {
    /// Not yet completed or skipped.
    Pending,
    /// Completed at the contained actual timestamp.
    Completed {
        /// Actual completion timestamp.
        at: Timestamp,
    },
    /// Explicitly skipped; reserved for post-MVP workflows.
    Skipped,
}

/// An invalid occurrence state transition.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum StateTransitionError {
    /// Completion toggling is not defined for a skipped occurrence.
    #[error("a skipped occurrence cannot toggle completion")]
    SkippedOccurrence,
}

/// A materialized, independently stateful chore occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    seed: OccurrenceSeed,
    state: OccurrenceState,
    updated_at: Timestamp,
}

impl Occurrence {
    /// Create a pending occurrence from valid materialization data.
    #[must_use]
    pub const fn pending(seed: OccurrenceSeed) -> Self {
        let updated_at = seed.created_at;
        Self {
            seed,
            state: OccurrenceState::Pending,
            updated_at,
        }
    }

    /// Rehydrate an occurrence from persistence.
    #[must_use]
    pub const fn restore(
        seed: OccurrenceSeed,
        state: OccurrenceState,
        updated_at: Timestamp,
    ) -> Self {
        Self {
            seed,
            state,
            updated_at,
        }
    }

    /// Toggle pending/completed state using the supplied actual timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error for a skipped occurrence.
    pub fn toggle_completion(&mut self, at: Timestamp) -> Result<(), StateTransitionError> {
        self.state = match self.state {
            OccurrenceState::Pending => OccurrenceState::Completed { at },
            OccurrenceState::Completed { .. } => OccurrenceState::Pending,
            OccurrenceState::Skipped => return Err(StateTransitionError::SkippedOccurrence),
        };
        self.updated_at = at;
        Ok(())
    }

    /// Return whether this occurrence is pending and due before today.
    #[must_use]
    pub fn is_missed(&self, today: CalendarDate) -> bool {
        self.state == OccurrenceState::Pending && self.seed.due_date < today
    }

    /// Return the occurrence identifier.
    #[must_use]
    pub const fn id(&self) -> OccurrenceId {
        self.seed.id
    }

    /// Return the owning chore identifier.
    #[must_use]
    pub const fn chore_id(&self) -> ChoreId {
        self.seed.chore_id
    }

    /// Return the source schedule identifier.
    #[must_use]
    pub const fn schedule_id(&self) -> ScheduleId {
        self.seed.schedule_id
    }

    /// Return the immutable nominal date.
    #[must_use]
    pub const fn nominal_date(&self) -> CalendarDate {
        self.seed.nominal_date
    }

    /// Return the displayed due date.
    #[must_use]
    pub const fn due_date(&self) -> CalendarDate {
        self.seed.due_date
    }

    /// Return the name snapshot.
    #[must_use]
    pub const fn name(&self) -> &ChoreName {
        &self.seed.name
    }

    /// Return the description snapshot.
    #[must_use]
    pub const fn description(&self) -> Option<&Description> {
        self.seed.description.as_ref()
    }

    /// Return the current persisted state.
    #[must_use]
    pub const fn state(&self) -> OccurrenceState {
        self.state
    }

    /// Return the actual completion timestamp, if completed.
    #[must_use]
    pub const fn completed_at(&self) -> Option<Timestamp> {
        match self.state {
            OccurrenceState::Completed { at } => Some(at),
            OccurrenceState::Pending | OccurrenceState::Skipped => None,
        }
    }

    /// Return the materialization timestamp.
    #[must_use]
    pub const fn created_at(&self) -> Timestamp {
        self.seed.created_at
    }

    /// Return the last state-change timestamp.
    #[must_use]
    pub const fn updated_at(&self) -> Timestamp {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;
    use uuid::Uuid;

    use super::*;

    fn date(year: i32, month: u8, day: u8) -> CalendarDate {
        CalendarDate::new(year, month, day).expect("test date should be valid")
    }

    fn timestamp(seconds: i64) -> Timestamp {
        Timestamp::from_unix_timestamp(seconds).expect("test timestamp should be valid")
    }

    fn occurrence(due_date: CalendarDate, state: OccurrenceState) -> Occurrence {
        Occurrence::restore(
            OccurrenceSeed {
                id: OccurrenceId::new(),
                chore_id: ChoreId::new(),
                schedule_id: ScheduleId::new(),
                nominal_date: due_date,
                due_date,
                name: ChoreName::new("Vacuum").expect("name should be valid"),
                description: None,
                created_at: timestamp(1),
            },
            state,
            timestamp(1),
        )
    }

    #[test]
    fn identifiers_accept_only_uuid_v7() {
        assert!(ChoreId::from_uuid(Uuid::now_v7()).is_ok());
        assert_eq!(
            ChoreId::from_uuid(Uuid::nil()),
            Err(ValidationError::IdentifierIsNotV7)
        );
    }

    #[test]
    fn names_are_trimmed_and_bounded_by_unicode_scalars() {
        let name = ChoreName::new("  Plants  ").expect("name should be valid");
        assert_eq!(name.as_str(), "Plants");
        assert_eq!(ChoreName::new("  "), Err(ValidationError::EmptyName));
        assert_eq!(
            ChoreName::new("🪴".repeat(81)),
            Err(ValidationError::NameTooLong { max: 80 })
        );
    }

    #[test]
    fn description_and_date_boundaries_are_enforced() {
        assert_eq!(Description::optional(" \n ").expect("blank is valid"), None);
        assert_eq!(
            Description::optional("x".repeat(1_001)),
            Err(ValidationError::DescriptionTooLong { max: 1_000 })
        );
        assert!(CalendarDate::new(2028, 2, 29).is_ok());
        assert_eq!(
            CalendarDate::new(2027, 2, 29),
            Err(ValidationError::InvalidDate)
        );
    }

    #[test]
    fn timestamps_are_normalized_to_utc_seconds() {
        let value = datetime!(2026-09-12 10:11:12.987_654 +02:00);
        assert_eq!(
            Timestamp::from_datetime(value).as_datetime(),
            datetime!(2026-09-12 8:11:12 UTC)
        );
    }

    #[test]
    fn recurrence_value_boundaries_are_enforced() {
        assert!(RecurrenceInterval::new(1).is_ok());
        assert!(RecurrenceInterval::new(999).is_ok());
        assert_eq!(
            RecurrenceInterval::new(0),
            Err(ValidationError::InvalidInterval)
        );
        assert_eq!(
            RecurrenceInterval::new(1_000),
            Err(ValidationError::InvalidInterval)
        );
        assert!(MonthlyDay::new(31).is_ok());
        assert_eq!(MonthlyDay::new(0), Err(ValidationError::InvalidMonthlyDay));
        assert_eq!(IsoWeekday::try_from(7), Ok(IsoWeekday::Sunday));
        assert_eq!(
            IsoWeekday::try_from(8),
            Err(ValidationError::InvalidWeekday)
        );
    }

    #[test]
    fn weekly_schedule_requires_and_normalizes_weekdays() {
        let window = ScheduleWindow::new(date(2026, 9, 12), date(2026, 9, 12), None, timestamp(1))
            .expect("window should be valid");
        let interval = RecurrenceInterval::new(2).expect("interval should be valid");
        assert_eq!(
            Schedule::weekly(ScheduleId::new(), ChoreId::new(), interval, [], window),
            Err(ValidationError::MissingWeeklyWeekday)
        );
        let schedule = Schedule::weekly(
            ScheduleId::new(),
            ChoreId::new(),
            interval,
            [IsoWeekday::Friday, IsoWeekday::Monday, IsoWeekday::Friday],
            window,
        )
        .expect("schedule should be valid");
        assert_eq!(
            schedule.weekdays().iter().copied().collect::<Vec<_>>(),
            vec![IsoWeekday::Monday, IsoWeekday::Friday]
        );
    }

    #[test]
    fn schedule_window_and_non_weekly_shapes_are_validated_by_construction() {
        assert_eq!(
            ScheduleWindow::new(
                date(2026, 9, 12),
                date(2026, 9, 12),
                Some(date(2026, 9, 11)),
                timestamp(1),
            ),
            Err(ValidationError::InvalidValidityWindow)
        );
        let window = ScheduleWindow::new(date(2026, 9, 12), date(2026, 9, 12), None, timestamp(1))
            .expect("window should be valid");
        let daily = Schedule::daily_interval(
            ScheduleId::new(),
            ChoreId::new(),
            RecurrenceInterval::new(3).expect("interval should be valid"),
            window,
        );
        assert_eq!(daily.kind(), RecurrenceKind::DailyInterval);
        assert!(daily.weekdays().is_empty());
        assert_eq!(daily.monthly_day(), None);

        let monthly = Schedule::monthly(
            ScheduleId::new(),
            ChoreId::new(),
            MonthlyDay::new(31).expect("day should be valid"),
            window,
        );
        assert_eq!(monthly.kind(), RecurrenceKind::Monthly);
        assert_eq!(monthly.interval().get(), 1);
        assert_eq!(monthly.monthly_day().map(MonthlyDay::get), Some(31));
        assert!(monthly.weekdays().is_empty());
    }

    #[test]
    fn deleted_chore_cannot_be_enabled() {
        let timestamps = ChoreTimestamps {
            created_at: timestamp(1),
            updated_at: timestamp(2),
            deleted_at: Some(timestamp(2)),
        };
        assert_eq!(
            Chore::restore(
                ChoreId::new(),
                ChoreName::new("Kitchen").expect("name should be valid"),
                None,
                true,
                timestamps,
            ),
            Err(ValidationError::DeletedChoreIsEnabled)
        );

        let mut chore = Chore::new(
            ChoreId::new(),
            ChoreName::new("Kitchen").expect("name should be valid"),
            None,
            timestamp(1),
        );
        chore.mark_deleted(timestamp(2));
        assert!(!chore.set_enabled(true, timestamp(3)));
        assert!(chore.is_deleted());
        assert!(!chore.is_enabled());
    }

    #[test]
    fn completion_toggle_sets_and_clears_actual_timestamp() {
        let mut value = occurrence(date(2026, 9, 10), OccurrenceState::Pending);
        value
            .toggle_completion(timestamp(20))
            .expect("pending occurrence should complete");
        assert_eq!(value.completed_at(), Some(timestamp(20)));
        value
            .toggle_completion(timestamp(30))
            .expect("completed occurrence should reopen");
        assert_eq!(value.state(), OccurrenceState::Pending);
        assert_eq!(value.completed_at(), None);
        assert_eq!(value.updated_at(), timestamp(30));
    }

    #[test]
    fn missed_state_is_derived_and_skipped_toggle_is_rejected() {
        let today = date(2026, 9, 12);
        assert!(occurrence(date(2026, 9, 11), OccurrenceState::Pending).is_missed(today));
        assert!(!occurrence(today, OccurrenceState::Pending).is_missed(today));
        assert!(
            !occurrence(
                date(2026, 9, 11),
                OccurrenceState::Completed { at: timestamp(20) }
            )
            .is_missed(today)
        );

        let mut skipped = occurrence(today, OccurrenceState::Skipped);
        assert_eq!(
            skipped.toggle_completion(timestamp(2)),
            Err(StateTransitionError::SkippedOccurrence)
        );
    }
}
