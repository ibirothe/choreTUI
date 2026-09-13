//! Deterministic Weekly Board selection and command model.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use time::{Duration, Weekday};

use crate::domain::{
    CalendarDate, ChoreId, IsoWeek, IsoWeekday, Occurrence, OccurrenceId, OccurrenceState,
    WeekError, WeeklyStatistics,
};

/// Responsive board presentation chosen from the current terminal dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardLayout {
    /// Seven equal day columns.
    SevenColumns,
    /// Monday–Thursday above Friday–Sunday.
    TwoRows,
    /// Compact week strip and one selected-day pane.
    SelectedDay,
    /// Safety view used when the terminal cannot fit the board.
    TooSmall,
}

impl BoardLayout {
    /// Select the most detailed layout supported by the supplied dimensions.
    #[must_use]
    pub const fn for_size(width: u16, height: u16) -> Self {
        if width >= 110 && height >= 18 {
            Self::SevenColumns
        } else if width >= 70 && height >= 20 {
            Self::TwoRows
        } else if width >= 40 && height >= 14 {
            Self::SelectedDay
        } else {
            Self::TooSmall
        }
    }
}

/// Semantic board input shared by Vim and conventional bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardInput {
    PreviousDay,
    NextDay,
    PreviousOccurrence,
    NextOccurrence,
    ToggleCompletion,
    AddChore,
    EditChore,
    DisableChore,
    DeleteChore,
    OpenChoreList,
    OpenGuidedPlanning,
    PreviousWeek,
    NextWeek,
    CurrentWeek,
    Help,
    Quit,
}

/// Command for the application layer after local selection handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardCommand {
    ToggleCompletion(OccurrenceId),
    AddChore(IsoWeekday),
    EditChore(ChoreId),
    DisableChore(ChoreId),
    DeleteChore(ChoreId),
    OpenChoreList,
    OpenGuidedPlanning,
    LoadWeek(IsoWeek),
    Help,
    Quit,
}

/// Convert a terminal key event into a semantic board input.
#[must_use]
pub const fn input_for_key(key: KeyEvent) -> Option<BoardInput> {
    if matches!(key.kind, KeyEventKind::Release) {
        return None;
    }
    match key.code {
        KeyCode::Left | KeyCode::Char('h') => Some(BoardInput::PreviousDay),
        KeyCode::Right | KeyCode::Char('l') => Some(BoardInput::NextDay),
        KeyCode::Up | KeyCode::Char('k') => Some(BoardInput::PreviousOccurrence),
        KeyCode::Down | KeyCode::Char('j') => Some(BoardInput::NextOccurrence),
        KeyCode::Char(' ') => Some(BoardInput::ToggleCompletion),
        KeyCode::Char('a') => Some(BoardInput::AddChore),
        KeyCode::Char('e') => Some(BoardInput::EditChore),
        KeyCode::Char('d') => Some(BoardInput::DisableChore),
        KeyCode::Char('D') => Some(BoardInput::DeleteChore),
        KeyCode::Char('c') => Some(BoardInput::OpenChoreList),
        KeyCode::Char('g') => Some(BoardInput::OpenGuidedPlanning),
        KeyCode::Char('[') | KeyCode::PageUp => Some(BoardInput::PreviousWeek),
        KeyCode::Char(']') | KeyCode::PageDown => Some(BoardInput::NextWeek),
        KeyCode::Char('t') => Some(BoardInput::CurrentWeek),
        KeyCode::Char('?') => Some(BoardInput::Help),
        KeyCode::Char('q') => Some(BoardInput::Quit),
        _ => None,
    }
}

/// Weekly Board view state, independent of persistence and terminal I/O.
#[derive(Clone, Debug)]
pub struct BoardState {
    week: IsoWeek,
    today: CalendarDate,
    occurrences: Vec<Occurrence>,
    show_completed: bool,
    selected_day: usize,
    selected_occurrence: Option<usize>,
    scroll_offsets: [usize; 7],
    status: Option<String>,
}

impl BoardState {
    /// Build a board and normalize selection for the initially selected weekday.
    #[must_use]
    pub fn new(week: IsoWeek, today: CalendarDate, occurrences: Vec<Occurrence>) -> Self {
        let mut state = Self {
            week,
            today,
            occurrences,
            show_completed: true,
            selected_day: if week.contains(today) {
                weekday_index(today.as_date().weekday())
            } else {
                0
            },
            selected_occurrence: None,
            scroll_offsets: [0; 7],
            status: None,
        };
        state.select_preferred_occurrence();
        state
    }

    #[must_use]
    pub const fn week(&self) -> IsoWeek {
        self.week
    }

    #[must_use]
    pub const fn today(&self) -> CalendarDate {
        self.today
    }

    #[must_use]
    pub const fn selected_day(&self) -> usize {
        self.selected_day
    }

    #[must_use]
    pub const fn selected_occurrence_position(&self) -> Option<usize> {
        self.selected_occurrence
    }

    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Set or clear the one-line application feedback message.
    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
    }

    /// Control completed-row visibility without changing weekly statistics.
    pub fn set_show_completed(&mut self, show_completed: bool) {
        self.show_completed = show_completed;
        self.select_preferred_occurrence();
    }

    /// Return the date at an ISO weekday position (`0` = Monday).
    #[must_use]
    pub fn day_date(&self, day: usize) -> CalendarDate {
        let monday = self.week.monday().unwrap_or(self.today);
        let offset = i64::try_from(day.min(6)).unwrap_or(6);
        CalendarDate::from_date(
            monday
                .as_date()
                .checked_add(Duration::days(offset))
                .unwrap_or(monday.as_date()),
        )
    }

    /// Visible, non-skipped occurrences for one day in repository order.
    pub fn occurrences_for_day(&self, day: usize) -> impl Iterator<Item = &Occurrence> {
        let date = self.day_date(day);
        self.occurrences.iter().filter(move |item| {
            item.due_date() == date
                && item.state() != OccurrenceState::Skipped
                && (self.show_completed
                    || !matches!(item.state(), OccurrenceState::Completed { .. }))
        })
    }

    #[must_use]
    pub fn selected_occurrence(&self) -> Option<&Occurrence> {
        self.selected_occurrence
            .and_then(|position| self.occurrences_for_day(self.selected_day).nth(position))
    }

    #[must_use]
    pub fn statistics(&self) -> WeeklyStatistics {
        WeeklyStatistics::calculate(self.week, &self.occurrences, self.today)
    }

    #[must_use]
    pub fn scroll_offset(&self, day: usize) -> usize {
        self.scroll_offsets[day.min(6)]
    }

    /// Keep the selected occurrence visible within a pane of `visible_rows` rows.
    pub fn ensure_selection_visible(&mut self, day: usize, visible_rows: usize) {
        if day != self.selected_day || visible_rows == 0 {
            return;
        }
        let Some(selected) = self.selected_occurrence else {
            self.scroll_offsets[day] = 0;
            return;
        };
        let offset = &mut self.scroll_offsets[day];
        if selected < *offset {
            *offset = selected;
        } else if selected >= offset.saturating_add(visible_rows) {
            *offset = selected.saturating_add(1).saturating_sub(visible_rows);
        }
    }

    /// Apply a semantic input. Selection inputs are handled locally; application
    /// operations are returned as commands.
    ///
    /// # Errors
    ///
    /// Returns a week-range error when previous or next week navigation exceeds
    /// the date library's supported range.
    pub fn handle_input(
        &mut self,
        input: BoardInput,
        layout: BoardLayout,
    ) -> Result<Option<BoardCommand>, WeekError> {
        if layout == BoardLayout::TooSmall {
            return Ok(match input {
                BoardInput::Help => Some(BoardCommand::Help),
                BoardInput::Quit => Some(BoardCommand::Quit),
                _ => None,
            });
        }

        match input {
            BoardInput::PreviousDay => self.move_day(-1),
            BoardInput::NextDay => self.move_day(1),
            BoardInput::PreviousOccurrence => self.move_occurrence(-1),
            BoardInput::NextOccurrence => self.move_occurrence(1),
            BoardInput::ToggleCompletion => {
                return Ok(self
                    .selected_occurrence()
                    .map(|item| BoardCommand::ToggleCompletion(item.id())));
            }
            BoardInput::AddChore => {
                let weekday = match self.selected_day {
                    0 => IsoWeekday::Monday,
                    1 => IsoWeekday::Tuesday,
                    2 => IsoWeekday::Wednesday,
                    3 => IsoWeekday::Thursday,
                    4 => IsoWeekday::Friday,
                    5 => IsoWeekday::Saturday,
                    _ => IsoWeekday::Sunday,
                };
                return Ok(Some(BoardCommand::AddChore(weekday)));
            }
            BoardInput::EditChore => {
                return Ok(self
                    .selected_occurrence()
                    .map(|item| BoardCommand::EditChore(item.chore_id())));
            }
            BoardInput::DisableChore => {
                return Ok(self
                    .selected_occurrence()
                    .map(|item| BoardCommand::DisableChore(item.chore_id())));
            }
            BoardInput::DeleteChore => {
                return Ok(self
                    .selected_occurrence()
                    .map(|item| BoardCommand::DeleteChore(item.chore_id())));
            }
            BoardInput::OpenChoreList => return Ok(Some(BoardCommand::OpenChoreList)),
            BoardInput::OpenGuidedPlanning => {
                return Ok(Some(BoardCommand::OpenGuidedPlanning));
            }
            BoardInput::PreviousWeek => {
                return self.week.previous().map(BoardCommand::LoadWeek).map(Some);
            }
            BoardInput::NextWeek => {
                return self.week.next().map(BoardCommand::LoadWeek).map(Some);
            }
            BoardInput::CurrentWeek => {
                return Ok(Some(BoardCommand::LoadWeek(IsoWeek::containing(
                    self.today,
                ))));
            }
            BoardInput::Help => return Ok(Some(BoardCommand::Help)),
            BoardInput::Quit => return Ok(Some(BoardCommand::Quit)),
        }
        Ok(None)
    }

    /// Replace board data after a week load, preserving the weekday and choosing
    /// the nearest non-empty day only when that weekday is empty.
    pub fn replace_week(&mut self, week: IsoWeek, occurrences: Vec<Occurrence>) {
        self.week = week;
        self.occurrences = occurrences;
        self.scroll_offsets = [0; 7];
        if self.day_len(self.selected_day) == 0 {
            let preserved = self.selected_day;
            if let Some(day) = (1_usize..=6).find_map(|distance| {
                (0_usize..=6)
                    .filter(|candidate| candidate.abs_diff(preserved) == distance)
                    .find(|candidate| self.day_len(*candidate) > 0)
            }) {
                self.selected_day = day;
            }
        }
        self.select_preferred_occurrence();
    }

    /// Replace occurrences after a mutation while retaining the same item when
    /// it remains present on the selected day.
    pub fn refresh_occurrences(
        &mut self,
        occurrences: Vec<Occurrence>,
        preferred: Option<OccurrenceId>,
    ) {
        self.occurrences = occurrences;
        self.scroll_offsets = [0; 7];
        self.selected_occurrence = preferred.and_then(|id| {
            self.occurrences_for_day(self.selected_day)
                .position(|item| item.id() == id)
        });
        if self.selected_occurrence.is_none() {
            self.select_preferred_occurrence();
        }
    }

    fn move_day(&mut self, delta: i8) {
        self.selected_day = if delta < 0 {
            (self.selected_day + 6) % 7
        } else {
            (self.selected_day + 1) % 7
        };
        self.select_preferred_occurrence();
    }

    fn move_occurrence(&mut self, delta: i8) {
        let count = self.day_len(self.selected_day);
        let Some(current) = self.selected_occurrence else {
            return;
        };
        self.selected_occurrence = Some(if delta < 0 {
            current.saturating_sub(1)
        } else {
            current.saturating_add(1).min(count.saturating_sub(1))
        });
    }

    fn select_preferred_occurrence(&mut self) {
        let preferred = self
            .occurrences_for_day(self.selected_day)
            .position(|item| item.state() == OccurrenceState::Pending);
        self.selected_occurrence =
            preferred.or_else(|| (self.day_len(self.selected_day) > 0).then_some(0));
        self.scroll_offsets[self.selected_day] = 0;
    }

    fn day_len(&self, day: usize) -> usize {
        self.occurrences_for_day(day).count()
    }
}

const fn weekday_index(weekday: Weekday) -> usize {
    match weekday {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyEventState, KeyModifiers};

    use super::*;
    use crate::domain::{ChoreName, OccurrenceSeed, ScheduleId, Timestamp};

    fn date(year: i32, month: u8, day: u8) -> CalendarDate {
        CalendarDate::new(year, month, day).expect("test date should be valid")
    }

    fn timestamp(seconds: i64) -> Timestamp {
        Timestamp::from_unix_timestamp(seconds).expect("test timestamp should be valid")
    }

    fn occurrence(due: CalendarDate, name: &str, state: OccurrenceState) -> Occurrence {
        Occurrence::restore(
            OccurrenceSeed {
                id: OccurrenceId::new(),
                chore_id: ChoreId::new(),
                schedule_id: ScheduleId::new(),
                nominal_date: due,
                due_date: due,
                name: ChoreName::new(name).expect("test name should be valid"),
                description: None,
                created_at: timestamp(1),
            },
            state,
            timestamp(1),
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn responsive_layouts_activate_at_exact_boundaries() {
        assert_eq!(BoardLayout::for_size(39, 13), BoardLayout::TooSmall);
        assert_eq!(BoardLayout::for_size(40, 14), BoardLayout::SelectedDay);
        assert_eq!(BoardLayout::for_size(69, 20), BoardLayout::SelectedDay);
        assert_eq!(BoardLayout::for_size(70, 20), BoardLayout::TwoRows);
        assert_eq!(BoardLayout::for_size(109, 20), BoardLayout::TwoRows);
        assert_eq!(BoardLayout::for_size(110, 20), BoardLayout::SevenColumns);
        assert_eq!(BoardLayout::for_size(110, 17), BoardLayout::SelectedDay);
    }

    #[test]
    fn vim_and_arrow_bindings_map_to_identical_inputs() {
        for (vim, conventional, expected) in [
            ('h', KeyCode::Left, BoardInput::PreviousDay),
            ('l', KeyCode::Right, BoardInput::NextDay),
            ('k', KeyCode::Up, BoardInput::PreviousOccurrence),
            ('j', KeyCode::Down, BoardInput::NextOccurrence),
            ('[', KeyCode::PageUp, BoardInput::PreviousWeek),
            (']', KeyCode::PageDown, BoardInput::NextWeek),
        ] {
            assert_eq!(input_for_key(key(KeyCode::Char(vim))), Some(expected));
            assert_eq!(input_for_key(key(conventional)), Some(expected));
        }

        assert_eq!(
            input_for_key(key(KeyCode::Char('g'))),
            Some(BoardInput::OpenGuidedPlanning)
        );
    }

    #[test]
    fn day_navigation_wraps_and_selects_first_pending_then_first_item() {
        let monday = date(2026, 9, 7);
        let tuesday = date(2026, 9, 8);
        let complete_at = timestamp(5);
        let items = vec![
            occurrence(
                tuesday,
                "completed first",
                OccurrenceState::Completed { at: complete_at },
            ),
            occurrence(tuesday, "pending second", OccurrenceState::Pending),
        ];
        let mut state = BoardState::new(IsoWeek::containing(monday), monday, items);
        assert_eq!(state.selected_day(), 0);
        state
            .handle_input(BoardInput::PreviousDay, BoardLayout::SevenColumns)
            .expect("navigation should succeed");
        assert_eq!(state.selected_day(), 6);
        state
            .handle_input(BoardInput::NextDay, BoardLayout::SevenColumns)
            .expect("navigation should succeed");
        state
            .handle_input(BoardInput::NextDay, BoardLayout::SevenColumns)
            .expect("navigation should succeed");
        assert_eq!(state.selected_day(), 1);
        assert_eq!(state.selected_occurrence_position(), Some(1));
        assert_eq!(
            state.selected_occurrence().map(|item| item.name().as_str()),
            Some("pending second")
        );
    }

    #[test]
    fn occurrence_navigation_clamps_and_empty_days_are_stable() {
        let monday = date(2026, 9, 7);
        let items = vec![
            occurrence(monday, "one", OccurrenceState::Pending),
            occurrence(monday, "two", OccurrenceState::Pending),
        ];
        let mut state = BoardState::new(IsoWeek::containing(monday), monday, items);
        state
            .handle_input(BoardInput::NextOccurrence, BoardLayout::SevenColumns)
            .unwrap();
        state
            .handle_input(BoardInput::NextOccurrence, BoardLayout::SevenColumns)
            .unwrap();
        assert_eq!(state.selected_occurrence_position(), Some(1));
        state
            .handle_input(BoardInput::NextDay, BoardLayout::SevenColumns)
            .unwrap();
        state
            .handle_input(BoardInput::NextOccurrence, BoardLayout::SevenColumns)
            .unwrap();
        assert_eq!(state.selected_occurrence_position(), None);
    }

    #[test]
    fn week_replacement_preserves_weekday_or_chooses_nearest_non_empty_day() {
        let monday = date(2026, 9, 7);
        let mut state = BoardState::new(IsoWeek::containing(monday), monday, Vec::new());
        state
            .handle_input(BoardInput::NextDay, BoardLayout::SevenColumns)
            .unwrap();
        state
            .handle_input(BoardInput::NextDay, BoardLayout::SevenColumns)
            .unwrap();
        let next = state.week().next().unwrap();
        state.replace_week(
            next,
            vec![occurrence(
                date(2026, 9, 17),
                "Thu",
                OccurrenceState::Pending,
            )],
        );
        assert_eq!(state.selected_day(), 3);

        let following = next.next().unwrap();
        state.replace_week(
            following,
            vec![occurrence(
                date(2026, 9, 24),
                "Thu",
                OccurrenceState::Pending,
            )],
        );
        assert_eq!(state.selected_day(), 3);
    }

    #[test]
    fn too_small_mode_allows_only_help_and_quit() {
        let today = date(2026, 9, 7);
        let mut state = BoardState::new(IsoWeek::containing(today), today, Vec::new());
        assert_eq!(
            state
                .handle_input(BoardInput::NextDay, BoardLayout::TooSmall)
                .unwrap(),
            None
        );
        assert_eq!(
            state
                .handle_input(BoardInput::Help, BoardLayout::TooSmall)
                .unwrap(),
            Some(BoardCommand::Help)
        );
        assert_eq!(
            state
                .handle_input(BoardInput::Quit, BoardLayout::TooSmall)
                .unwrap(),
            Some(BoardCommand::Quit)
        );
    }

    #[test]
    fn action_dispatch_uses_current_selection_and_iso_week_arithmetic() {
        let monday = date(2026, 12, 28);
        let selected = occurrence(monday, "selected", OccurrenceState::Pending);
        let selected_id = selected.id();
        let chore_id = selected.chore_id();
        let mut state = BoardState::new(IsoWeek::containing(monday), monday, vec![selected]);
        assert_eq!(
            state
                .handle_input(BoardInput::ToggleCompletion, BoardLayout::SevenColumns)
                .unwrap(),
            Some(BoardCommand::ToggleCompletion(selected_id))
        );
        assert_eq!(
            state
                .handle_input(BoardInput::EditChore, BoardLayout::SevenColumns)
                .unwrap(),
            Some(BoardCommand::EditChore(chore_id))
        );
        assert_eq!(
            state
                .handle_input(BoardInput::NextWeek, BoardLayout::SevenColumns)
                .unwrap(),
            Some(BoardCommand::LoadWeek(IsoWeek::new(2027, 1).unwrap()))
        );
    }

    #[test]
    fn hiding_completed_rows_keeps_them_in_statistics() {
        let monday = date(2026, 9, 7);
        let completed = occurrence(
            monday,
            "Done",
            OccurrenceState::Completed { at: timestamp(2) },
        );
        let mut state = BoardState::new(IsoWeek::containing(monday), monday, vec![completed]);
        assert_eq!(state.statistics().completed(), 1);
        assert_eq!(state.occurrences_for_day(0).count(), 1);
        state.set_show_completed(false);
        assert_eq!(state.occurrences_for_day(0).count(), 0);
        assert_eq!(state.statistics().completed(), 1);
    }
}
