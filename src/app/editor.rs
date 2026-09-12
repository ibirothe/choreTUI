//! Validated data exchanged between the chore editor and application layer.

use crate::domain::{
    ChoreId, ChoreName, Description, IsoWeekday, MonthlyDay, RecurrenceInterval, RecurrenceKind,
    Schedule,
};

/// A recurrence pattern without persistence-specific revision metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchedulePattern {
    Weekly {
        interval: RecurrenceInterval,
        weekdays: Vec<IsoWeekday>,
    },
    DailyInterval {
        interval: RecurrenceInterval,
    },
    Monthly {
        day: MonthlyDay,
    },
}

impl SchedulePattern {
    /// Copy the editable recurrence values from a persisted schedule.
    #[must_use]
    pub fn from_schedule(schedule: &Schedule) -> Option<Self> {
        match schedule.kind() {
            RecurrenceKind::Weekly => Some(Self::Weekly {
                interval: schedule.interval(),
                weekdays: schedule.weekdays().iter().copied().collect(),
            }),
            RecurrenceKind::DailyInterval => Some(Self::DailyInterval {
                interval: schedule.interval(),
            }),
            RecurrenceKind::Monthly => schedule.monthly_day().map(|day| Self::Monthly { day }),
        }
    }
}

/// Existing values used to populate the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorRecord {
    pub id: ChoreId,
    pub name: ChoreName,
    pub description: Option<Description>,
    pub enabled: bool,
    pub pattern: SchedulePattern,
}

/// Fully validated add/edit request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoreSubmission {
    pub id: Option<ChoreId>,
    pub name: ChoreName,
    pub description: Option<Description>,
    pub enabled: bool,
    pub pattern: SchedulePattern,
}
