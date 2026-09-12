//! ISO-week value objects and weekly statistics.

use std::{fmt, str::FromStr};

use thiserror::Error;
use time::{Date, Duration, Weekday};

use super::{CalendarDate, Occurrence, OccurrenceState};
use crate::domain::ports::Clock;

/// An invalid ISO-week operation.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum WeekError {
    /// Text does not use the canonical `YYYY-Www` representation.
    #[error("week must use YYYY-Www format")]
    InvalidFormat,
    /// The requested ISO week does not exist in its week-year.
    #[error("invalid ISO week {year}-W{week:02}")]
    InvalidWeek {
        /// ISO week-year.
        year: i32,
        /// ISO week number.
        week: u8,
    },
    /// Navigation exceeded the date library's supported range.
    #[error("ISO week is outside the supported date range")]
    DateOutOfRange,
}

/// An ISO 8601 week identified by its week-year and week number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IsoWeek {
    year: i32,
    week: u8,
}

impl IsoWeek {
    /// Validate an ISO week-year and week number.
    ///
    /// # Errors
    ///
    /// Returns [`WeekError::InvalidWeek`] when that week does not exist.
    pub fn new(year: i32, week: u8) -> Result<Self, WeekError> {
        Date::from_iso_week_date(year, week, Weekday::Monday)
            .map(|_| Self { year, week })
            .map_err(|_| WeekError::InvalidWeek { year, week })
    }

    /// Return the ISO week containing a calendar date.
    #[must_use]
    pub fn containing(date: CalendarDate) -> Self {
        let (year, week, _) = date.as_date().to_iso_week_date();
        Self { year, week }
    }

    /// Return the ISO week supplied by an injected clock.
    #[must_use]
    pub fn current(clock: &impl Clock) -> Self {
        Self::containing(clock.today())
    }

    /// Return the ISO week-year.
    #[must_use]
    pub const fn year(self) -> i32 {
        self.year
    }

    /// Return the week number.
    #[must_use]
    pub const fn number(self) -> u8 {
        self.week
    }

    /// Return the Monday starting this week.
    ///
    /// # Errors
    ///
    /// Returns [`WeekError::InvalidWeek`] if an invalid value is ever
    /// rehydrated outside the public constructors.
    pub fn monday(self) -> Result<CalendarDate, WeekError> {
        Date::from_iso_week_date(self.year, self.week, Weekday::Monday)
            .map(CalendarDate::from_date)
            .map_err(|_| WeekError::InvalidWeek {
                year: self.year,
                week: self.week,
            })
    }

    /// Return whether a date belongs to this ISO week.
    #[must_use]
    pub fn contains(self, date: CalendarDate) -> bool {
        Self::containing(date) == self
    }

    /// Navigate to the previous ISO week.
    ///
    /// # Errors
    ///
    /// Returns [`WeekError::DateOutOfRange`] at the minimum supported date.
    pub fn previous(self) -> Result<Self, WeekError> {
        self.monday()?
            .as_date()
            .checked_sub(Duration::weeks(1))
            .map(CalendarDate::from_date)
            .map(Self::containing)
            .ok_or(WeekError::DateOutOfRange)
    }

    /// Navigate to the next ISO week.
    ///
    /// # Errors
    ///
    /// Returns [`WeekError::DateOutOfRange`] at the maximum supported date.
    pub fn next(self) -> Result<Self, WeekError> {
        self.monday()?
            .as_date()
            .checked_add(Duration::weeks(1))
            .map(CalendarDate::from_date)
            .map(Self::containing)
            .ok_or(WeekError::DateOutOfRange)
    }
}

impl fmt::Display for IsoWeek {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:04}-W{:02}", self.year, self.week)
    }
}

impl FromStr for IsoWeek {
    type Err = WeekError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (year, week) = value.split_once("-W").ok_or(WeekError::InvalidFormat)?;
        if year.len() != 4
            || week.len() != 2
            || !year.bytes().all(|byte| byte.is_ascii_digit())
            || !week.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(WeekError::InvalidFormat);
        }
        let year = year.parse::<i32>().map_err(|_| WeekError::InvalidFormat)?;
        let week = week.parse::<u8>().map_err(|_| WeekError::InvalidFormat)?;
        Self::new(year, week)
    }
}

/// Completion counters for one displayed ISO week.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeeklyStatistics {
    total: u32,
    completed: u32,
    missed: u32,
}

impl WeeklyStatistics {
    /// Compute statistics from occurrences whose due dates belong to a week.
    #[must_use]
    pub fn calculate<'a>(
        week: IsoWeek,
        occurrences: impl IntoIterator<Item = &'a Occurrence>,
        today: CalendarDate,
    ) -> Self {
        occurrences
            .into_iter()
            .filter(|occurrence| week.contains(occurrence.due_date()))
            .filter(|occurrence| occurrence.state() != OccurrenceState::Skipped)
            .fold(Self::default(), |mut statistics, occurrence| {
                statistics.total = statistics.total.saturating_add(1);
                match occurrence.state() {
                    OccurrenceState::Completed { .. } => {
                        statistics.completed = statistics.completed.saturating_add(1);
                    }
                    OccurrenceState::Pending if occurrence.is_missed(today) => {
                        statistics.missed = statistics.missed.saturating_add(1);
                    }
                    OccurrenceState::Pending | OccurrenceState::Skipped => {}
                }
                statistics
            })
    }

    /// Return the pending plus completed count.
    #[must_use]
    pub const fn total(self) -> u32 {
        self.total
    }

    /// Return the completed count.
    #[must_use]
    pub const fn completed(self) -> u32 {
        self.completed
    }

    /// Return the derived missed count.
    #[must_use]
    pub const fn missed(self) -> u32 {
        self.missed
    }

    /// Return completion percentage rounded half up, or `None` for zero total.
    #[must_use]
    pub fn completion_percentage(self) -> Option<u8> {
        if self.total == 0 {
            return None;
        }
        let rounded =
            (u64::from(self.completed) * 100 + u64::from(self.total) / 2) / u64::from(self.total);
        u8::try_from(rounded).ok()
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::domain::{ChoreId, ChoreName, OccurrenceId, OccurrenceSeed, ScheduleId, Timestamp};

    struct FixedClock(CalendarDate);

    impl Clock for FixedClock {
        fn today(&self) -> CalendarDate {
            self.0
        }

        fn now(&self) -> Timestamp {
            timestamp(100)
        }
    }

    fn date(year: i32, month: u8, day: u8) -> CalendarDate {
        CalendarDate::new(year, month, day).expect("test date should be valid")
    }

    fn timestamp(seconds: i64) -> Timestamp {
        Timestamp::from_unix_timestamp(seconds).expect("test timestamp should be valid")
    }

    fn occurrence(due: CalendarDate, state: OccurrenceState) -> Occurrence {
        Occurrence::restore(
            OccurrenceSeed {
                id: OccurrenceId::new(),
                chore_id: ChoreId::new(),
                schedule_id: ScheduleId::new(),
                nominal_date: due,
                due_date: due,
                name: ChoreName::new("Test").expect("name should be valid"),
                description: None,
                created_at: timestamp(1),
            },
            state,
            timestamp(1),
        )
    }

    #[test]
    fn week_format_parse_and_validation_are_canonical() {
        let week = IsoWeek::new(2026, 37).expect("week should be valid");
        assert_eq!(week.to_string(), "2026-W37");
        assert_eq!(IsoWeek::from_str("2026-W37"), Ok(week));
        assert_eq!(IsoWeek::from_str("2026-W7"), Err(WeekError::InvalidFormat));
        assert_eq!(
            IsoWeek::new(2026, 54),
            Err(WeekError::InvalidWeek {
                year: 2026,
                week: 54,
            })
        );
    }

    #[test]
    fn navigation_handles_iso_week_year_boundaries() {
        let week_53 = IsoWeek::new(2020, 53).expect("week should be valid");
        assert_eq!(week_53.next(), IsoWeek::new(2021, 1));
        assert_eq!(
            IsoWeek::new(2025, 1)
                .expect("week should be valid")
                .previous(),
            IsoWeek::new(2024, 52)
        );
        assert_eq!(
            IsoWeek::containing(date(2021, 1, 1)),
            IsoWeek::new(2020, 53).expect("week should be valid")
        );
    }

    #[test]
    fn current_week_uses_injected_clock() {
        let clock = FixedClock(date(2026, 9, 12));
        assert_eq!(
            IsoWeek::current(&clock),
            IsoWeek::new(2026, 37).expect("week should be valid")
        );
    }

    #[test]
    fn empty_week_has_no_completion_percentage() {
        let statistics = WeeklyStatistics::calculate(
            IsoWeek::new(2026, 37).expect("week should be valid"),
            [],
            date(2026, 9, 12),
        );
        assert_eq!(statistics, WeeklyStatistics::default());
        assert_eq!(statistics.completion_percentage(), None);
    }

    #[test]
    fn statistics_filter_week_and_skipped_then_round_half_up() {
        let completed = occurrence(
            date(2026, 9, 7),
            OccurrenceState::Completed { at: timestamp(10) },
        );
        let missed = occurrence(date(2026, 9, 8), OccurrenceState::Pending);
        let future = occurrence(date(2026, 9, 13), OccurrenceState::Pending);
        let skipped = occurrence(date(2026, 9, 9), OccurrenceState::Skipped);
        let other_week = occurrence(date(2026, 9, 14), OccurrenceState::Pending);
        let occurrences = [completed, missed, future, skipped, other_week];
        let statistics = WeeklyStatistics::calculate(
            IsoWeek::new(2026, 37).expect("week should be valid"),
            &occurrences,
            date(2026, 9, 12),
        );
        assert_eq!(statistics.total(), 3);
        assert_eq!(statistics.completed(), 1);
        assert_eq!(statistics.missed(), 1);
        assert_eq!(statistics.completion_percentage(), Some(33));
    }

    #[test]
    fn percentage_rounds_half_up() {
        let completed = occurrence(
            date(2026, 9, 7),
            OccurrenceState::Completed { at: timestamp(10) },
        );
        let pending = (0..7)
            .map(|_| occurrence(date(2026, 9, 8), OccurrenceState::Pending))
            .collect::<Vec<_>>();
        let statistics = WeeklyStatistics::calculate(
            IsoWeek::new(2026, 37).expect("week should be valid"),
            std::iter::once(&completed).chain(pending.iter()),
            date(2026, 9, 7),
        );
        assert_eq!(statistics.completion_percentage(), Some(13));
    }
}
