//! Pure recurrence range expansion.
//!
//! This module deliberately depends only on immutable domain values. It does
//! not materialize occurrences and has no knowledge of persistence or UI
//! state.

use thiserror::Error;
use time::{Date, Duration};

use crate::domain::{CalendarDate, IsoWeekday, RecurrenceKind, Schedule};

/// An invalid recurrence query.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum RecurrenceError {
    /// The inclusive query end precedes its start.
    #[error("recurrence range end must not precede its start")]
    InvalidRange,
}

/// An inclusive range of calendar dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateRange {
    start: CalendarDate,
    end: CalendarDate,
}

impl DateRange {
    /// Validate an inclusive date range.
    ///
    /// # Errors
    ///
    /// Returns [`RecurrenceError::InvalidRange`] when `end` precedes `start`.
    pub fn new(start: CalendarDate, end: CalendarDate) -> Result<Self, RecurrenceError> {
        if end < start {
            return Err(RecurrenceError::InvalidRange);
        }
        Ok(Self { start, end })
    }

    /// Return the inclusive start date.
    #[must_use]
    pub const fn start(self) -> CalendarDate {
        self.start
    }

    /// Return the inclusive end date.
    #[must_use]
    pub const fn end(self) -> CalendarDate {
        self.end
    }
}

/// Expand a schedule into sorted, unique nominal dates within an inclusive
/// query range.
///
/// The query is intersected with the schedule's validity window and anchor.
/// A validity end is inclusive. Dates before the recurrence anchor can never
/// be returned.
#[must_use]
pub fn expand(schedule: &Schedule, range: DateRange) -> Vec<CalendarDate> {
    let window = schedule.window();
    let start = range
        .start()
        .max(window.valid_from())
        .max(window.anchor_date());
    let end = window
        .valid_until()
        .map_or(range.end(), |valid_until| range.end().min(valid_until));

    if start > end {
        return Vec::new();
    }

    let mut dates = Vec::new();
    let mut candidate = start.as_date();
    let end = end.as_date();

    loop {
        if matches_schedule(schedule, candidate) {
            dates.push(CalendarDate::from_date(candidate));
        }
        if candidate == end {
            break;
        }
        let Some(next) = candidate.checked_add(Duration::days(1)) else {
            break;
        };
        candidate = next;
    }

    dates
}

fn matches_schedule(schedule: &Schedule, candidate: Date) -> bool {
    let anchor = schedule.window().anchor_date().as_date();
    match schedule.kind() {
        RecurrenceKind::Weekly => {
            let weekday = IsoWeekday::try_from(candidate.weekday().number_days_from_monday() + 1)
                .is_ok_and(|weekday| schedule.weekdays().contains(&weekday));
            weekday && matches_weekly_phase(anchor, candidate, i64::from(schedule.interval().get()))
        }
        RecurrenceKind::DailyInterval => {
            calendar_day_distance(anchor, candidate) % i64::from(schedule.interval().get()) == 0
        }
        RecurrenceKind::Monthly => schedule.monthly_day().is_some_and(|day| {
            candidate.day() == day.get().min(candidate.month().length(candidate.year()))
        }),
    }
}

fn matches_weekly_phase(anchor: Date, candidate: Date, interval: i64) -> bool {
    let anchor_monday =
        i64::from(anchor.to_julian_day()) - i64::from(anchor.weekday().number_days_from_monday());
    let candidate_monday = i64::from(candidate.to_julian_day())
        - i64::from(candidate.weekday().number_days_from_monday());
    let elapsed_weeks = (candidate_monday - anchor_monday) / 7;
    elapsed_weeks >= 0 && elapsed_weeks % interval == 0
}

fn calendar_day_distance(start: Date, end: Date) -> i64 {
    i64::from(end.to_julian_day()) - i64::from(start.to_julian_day())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;

    use super::*;
    use crate::domain::{
        ChoreId, MonthlyDay, RecurrenceInterval, ScheduleId, ScheduleWindow, Timestamp,
    };

    fn date(year: i32, month: u8, day: u8) -> CalendarDate {
        CalendarDate::new(year, month, day).expect("test date should be valid")
    }

    fn timestamp() -> Timestamp {
        Timestamp::from_unix_timestamp(0).expect("test timestamp should be valid")
    }

    fn window(
        anchor: CalendarDate,
        valid_from: CalendarDate,
        valid_until: Option<CalendarDate>,
    ) -> ScheduleWindow {
        ScheduleWindow::new(anchor, valid_from, valid_until, timestamp())
            .expect("test window should be valid")
    }

    fn range(start: CalendarDate, end: CalendarDate) -> DateRange {
        DateRange::new(start, end).expect("test range should be valid")
    }

    fn weekly(
        interval: u16,
        weekdays: impl IntoIterator<Item = IsoWeekday>,
        window: ScheduleWindow,
    ) -> Schedule {
        Schedule::weekly(
            ScheduleId::new(),
            ChoreId::new(),
            RecurrenceInterval::new(interval).expect("test interval should be valid"),
            weekdays,
            window,
        )
        .expect("test schedule should be valid")
    }

    fn daily(interval: u16, window: ScheduleWindow) -> Schedule {
        Schedule::daily_interval(
            ScheduleId::new(),
            ChoreId::new(),
            RecurrenceInterval::new(interval).expect("test interval should be valid"),
            window,
        )
    }

    fn monthly(day: u8, window: ScheduleWindow) -> Schedule {
        Schedule::monthly(
            ScheduleId::new(),
            ChoreId::new(),
            MonthlyDay::new(day).expect("test monthly day should be valid"),
            window,
        )
    }

    #[test]
    fn rejects_reversed_query_ranges() {
        assert_eq!(
            DateRange::new(date(2026, 9, 2), date(2026, 9, 1)),
            Err(RecurrenceError::InvalidRange)
        );
    }

    #[test]
    fn weekly_rules_emit_each_matching_weekday_once() {
        let schedule = weekly(
            1,
            [
                IsoWeekday::Monday,
                IsoWeekday::Wednesday,
                IsoWeekday::Wednesday,
                IsoWeekday::Sunday,
            ],
            window(date(2026, 9, 2), date(2026, 9, 1), None),
        );

        assert_eq!(
            expand(&schedule, range(date(2026, 8, 31), date(2026, 9, 13))),
            [
                date(2026, 9, 2),
                date(2026, 9, 6),
                date(2026, 9, 7),
                date(2026, 9, 9),
                date(2026, 9, 13),
            ]
        );
    }

    #[test]
    fn every_n_week_phase_survives_iso_year_boundaries() {
        let schedule = weekly(
            2,
            [IsoWeekday::Monday],
            window(date(2020, 12, 28), date(2020, 12, 1), None),
        );

        assert_eq!(
            expand(&schedule, range(date(2020, 12, 1), date(2021, 2, 1))),
            [date(2020, 12, 28), date(2021, 1, 11), date(2021, 1, 25),]
        );
    }

    #[test]
    fn daily_interval_uses_calendar_days_across_month_and_year_boundaries() {
        let schedule = daily(3, window(date(2027, 12, 30), date(2027, 12, 1), None));

        assert_eq!(
            expand(&schedule, range(date(2027, 12, 20), date(2028, 1, 9))),
            [
                date(2027, 12, 30),
                date(2028, 1, 2),
                date(2028, 1, 5),
                date(2028, 1, 8),
            ]
        );
    }

    #[test]
    fn monthly_days_clamp_in_short_and_leap_months() {
        let schedule = monthly(31, window(date(2027, 1, 31), date(2027, 1, 1), None));

        assert_eq!(
            expand(&schedule, range(date(2027, 1, 1), date(2028, 4, 30))),
            [
                date(2027, 1, 31),
                date(2027, 2, 28),
                date(2027, 3, 31),
                date(2027, 4, 30),
                date(2027, 5, 31),
                date(2027, 6, 30),
                date(2027, 7, 31),
                date(2027, 8, 31),
                date(2027, 9, 30),
                date(2027, 10, 31),
                date(2027, 11, 30),
                date(2027, 12, 31),
                date(2028, 1, 31),
                date(2028, 2, 29),
                date(2028, 3, 31),
                date(2028, 4, 30),
            ]
        );
    }

    #[test]
    fn day_29_clamps_only_in_non_leap_february() {
        let schedule = monthly(29, window(date(2027, 1, 29), date(2027, 1, 1), None));

        assert_eq!(
            expand(&schedule, range(date(2027, 2, 1), date(2028, 2, 29))),
            [
                date(2027, 2, 28),
                date(2027, 3, 29),
                date(2027, 4, 29),
                date(2027, 5, 29),
                date(2027, 6, 29),
                date(2027, 7, 29),
                date(2027, 8, 29),
                date(2027, 9, 29),
                date(2027, 10, 29),
                date(2027, 11, 29),
                date(2027, 12, 29),
                date(2028, 1, 29),
                date(2028, 2, 29),
            ]
        );
    }

    #[test]
    fn range_anchor_and_validity_bounds_are_all_inclusive() {
        let schedule = daily(
            1,
            window(date(2026, 9, 3), date(2026, 9, 5), Some(date(2026, 9, 7))),
        );

        assert_eq!(
            expand(&schedule, range(date(2026, 9, 1), date(2026, 9, 10))),
            [date(2026, 9, 5), date(2026, 9, 6), date(2026, 9, 7)]
        );
    }

    #[test]
    fn validity_gaps_are_not_backfilled_by_a_later_schedule_revision() {
        let before_gap = daily(
            1,
            window(date(2026, 9, 1), date(2026, 9, 1), Some(date(2026, 9, 3))),
        );
        let after_gap = daily(1, window(date(2026, 9, 7), date(2026, 9, 7), None));
        let query = range(date(2026, 9, 1), date(2026, 9, 10));

        let dates = [expand(&before_gap, query), expand(&after_gap, query)]
            .concat()
            .into_iter()
            .collect::<BTreeSet<_>>();

        assert_eq!(
            dates,
            [
                date(2026, 9, 1),
                date(2026, 9, 2),
                date(2026, 9, 3),
                date(2026, 9, 7),
                date(2026, 9, 8),
                date(2026, 9, 9),
                date(2026, 9, 10),
            ]
            .into_iter()
            .collect()
        );
    }

    proptest! {
        #[test]
        fn expansion_is_sorted_unique_and_inside_all_bounds(
            query_start_offset in 0_i64..1_500,
            query_length in 0_i64..400,
            anchor_offset in 0_i64..1_500,
            valid_start_offset in 0_i64..1_500,
            valid_length in 0_i64..400,
            interval in 1_u16..20,
            weekday_numbers in prop::collection::vec(1_u8..=7, 1..=7),
        ) {
            let epoch = Date::from_calendar_date(2020, time::Month::January, 1)
                .expect("test epoch should be valid");
            let query_start = CalendarDate::from_date(epoch + Duration::days(query_start_offset));
            let query_end = CalendarDate::from_date(
                epoch + Duration::days(query_start_offset + query_length),
            );
            let anchor = CalendarDate::from_date(epoch + Duration::days(anchor_offset));
            let valid_from = CalendarDate::from_date(epoch + Duration::days(valid_start_offset));
            let valid_until = CalendarDate::from_date(
                epoch + Duration::days(valid_start_offset + valid_length),
            );
            let weekdays = weekday_numbers
                .into_iter()
                .map(|number| IsoWeekday::try_from(number).expect("generated weekday is valid"));
            let schedule = weekly(
                interval,
                weekdays,
                window(anchor, valid_from, Some(valid_until)),
            );
            let dates = expand(&schedule, range(query_start, query_end));

            prop_assert!(dates.windows(2).all(|pair| pair[0] < pair[1]));
            let all_inside_bounds = dates.iter().all(|candidate| {
                *candidate >= query_start
                    && *candidate <= query_end
                    && *candidate >= anchor
                    && *candidate >= valid_from
                    && *candidate <= valid_until
            });
            prop_assert!(all_inside_bounds);
        }
    }
}
