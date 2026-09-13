//! Keyboard-driven add/edit form for chores and recurrence revisions.

use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use time::Weekday;

use crate::{
    app::editor::{ChoreSubmission, EditorRecord, SchedulePattern, TemplateProvenance},
    catalog::{ActivityTemplate, CadenceKind, CatalogProvenance},
    domain::{
        CalendarDate, ChoreId, ChoreName, Description, IsoWeekday, MonthlyDay, RecurrenceInterval,
    },
};

const WEEKDAYS: [IsoWeekday; 7] = [
    IsoWeekday::Monday,
    IsoWeekday::Tuesday,
    IsoWeekday::Wednesday,
    IsoWeekday::Thursday,
    IsoWeekday::Friday,
    IsoWeekday::Saturday,
    IsoWeekday::Sunday,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorMode {
    Add,
    Edit(ChoreId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EditorField {
    Name,
    Description,
    Recurrence,
    Interval,
    Weekdays,
    MonthlyDay,
    Enabled,
    Save,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecurrenceChoice {
    Weekly,
    Daily,
    Monthly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    name: String,
    description: String,
    recurrence: RecurrenceChoice,
    interval: String,
    weekdays: BTreeSet<IsoWeekday>,
    monthly_day: String,
    enabled: bool,
}

#[derive(Clone, Debug)]
pub struct EditorState {
    mode: EditorMode,
    values: Snapshot,
    initial: Snapshot,
    focus: EditorField,
    weekday_cursor: usize,
    errors: BTreeMap<EditorField, String>,
    save_error: Option<String>,
    confirming_cancel: bool,
    provenance: Option<crate::app::editor::TemplateProvenance>,
    notice: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorAction {
    Save(ChoreSubmission),
    Close,
}

impl EditorState {
    #[must_use]
    pub fn add(preselected: IsoWeekday) -> Self {
        let values = Snapshot {
            name: String::new(),
            description: String::new(),
            recurrence: RecurrenceChoice::Weekly,
            interval: "1".to_owned(),
            weekdays: BTreeSet::from([preselected]),
            monthly_day: "1".to_owned(),
            enabled: true,
        };
        Self::with_values(EditorMode::Add, values)
    }

    #[must_use]
    pub fn edit(record: EditorRecord) -> Self {
        let (recurrence, interval, weekdays, monthly_day) = match record.pattern {
            SchedulePattern::Weekly { interval, weekdays } => (
                RecurrenceChoice::Weekly,
                interval.get().to_string(),
                weekdays.into_iter().collect(),
                "1".to_owned(),
            ),
            SchedulePattern::DailyInterval { interval } => (
                RecurrenceChoice::Daily,
                interval.get().to_string(),
                BTreeSet::new(),
                "1".to_owned(),
            ),
            SchedulePattern::Monthly { day } => (
                RecurrenceChoice::Monthly,
                "1".to_owned(),
                BTreeSet::new(),
                day.get().to_string(),
            ),
        };
        Self::with_values(
            EditorMode::Edit(record.id),
            Snapshot {
                name: record.name.as_str().to_owned(),
                description: record
                    .description
                    .as_ref()
                    .map_or_else(String::new, |value| value.as_str().to_owned()),
                recurrence,
                interval,
                weekdays,
                monthly_day,
                enabled: record.enabled,
            },
        )
    }

    /// Copy a catalog suggestion into an ordinary, fully editable add form.
    #[must_use]
    pub fn from_template(
        template: &ActivityTemplate,
        catalog: CatalogProvenance<'_>,
        selected_date: CalendarDate,
        possible_duplicate: bool,
    ) -> Self {
        let weekday = match selected_date.as_date().weekday() {
            Weekday::Monday => IsoWeekday::Monday,
            Weekday::Tuesday => IsoWeekday::Tuesday,
            Weekday::Wednesday => IsoWeekday::Wednesday,
            Weekday::Thursday => IsoWeekday::Thursday,
            Weekday::Friday => IsoWeekday::Friday,
            Weekday::Saturday => IsoWeekday::Saturday,
            Weekday::Sunday => IsoWeekday::Sunday,
        };
        let mut notice = possible_duplicate.then(|| {
            "Possible duplicate: an active chore has the same name. Saving another is allowed."
                .to_owned()
        });
        let (recurrence, interval, weekdays, monthly_day) = match template
            .suggested_cadence
            .as_ref()
            .map(|cadence| (cadence.kind, cadence.interval))
        {
            Some((CadenceKind::Days, Some(interval))) => (
                RecurrenceChoice::Daily,
                interval.to_string(),
                BTreeSet::new(),
                "1".to_owned(),
            ),
            Some((CadenceKind::Weeks, Some(interval))) => (
                RecurrenceChoice::Weekly,
                interval.to_string(),
                BTreeSet::from([weekday]),
                "1".to_owned(),
            ),
            Some((CadenceKind::Months, Some(1))) => (
                RecurrenceChoice::Monthly,
                "1".to_owned(),
                BTreeSet::new(),
                selected_date.as_date().day().to_string(),
            ),
            unsupported => {
                if unsupported.is_some() {
                    let cadence_notice = "The suggested cadence is not representable exactly; review recurrence before saving.";
                    notice = Some(match notice {
                        Some(existing) => format!("{existing} {cadence_notice}"),
                        None => cadence_notice.to_owned(),
                    });
                }
                (
                    RecurrenceChoice::Weekly,
                    "1".to_owned(),
                    BTreeSet::from([weekday]),
                    "1".to_owned(),
                )
            }
        };
        let mut state = Self::with_values(
            EditorMode::Add,
            Snapshot {
                name: template.name.clone(),
                description: template.description.clone().unwrap_or_default(),
                recurrence,
                interval,
                weekdays,
                monthly_day,
                enabled: true,
            },
        );
        state.provenance = Some(TemplateProvenance {
            template_id: template.id.clone(),
            schema_version: catalog.schema_version,
            catalog_version: catalog.catalog_version,
            locale: catalog.locale.to_owned(),
        });
        state.notice = notice;
        state
    }

    fn with_values(mode: EditorMode, values: Snapshot) -> Self {
        Self {
            mode,
            initial: values.clone(),
            values,
            focus: EditorField::Name,
            weekday_cursor: 0,
            errors: BTreeMap::new(),
            save_error: None,
            confirming_cancel: false,
            provenance: None,
            notice: None,
        }
    }

    #[must_use]
    pub const fn mode(&self) -> EditorMode {
        self.mode
    }

    #[must_use]
    pub const fn can_browse_catalog(&self) -> bool {
        matches!(self.mode, EditorMode::Add) && self.provenance.is_none() && !self.confirming_cancel
    }

    #[must_use]
    pub const fn focus(&self) -> EditorField {
        self.focus
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.values != self.initial
    }

    #[must_use]
    pub const fn is_confirming_cancel(&self) -> bool {
        self.confirming_cancel
    }

    #[must_use]
    pub fn error(&self, field: EditorField) -> Option<&str> {
        self.errors.get(&field).map(String::as_str)
    }

    #[must_use]
    pub fn save_error(&self) -> Option<&str> {
        self.save_error.as_deref()
    }

    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn set_save_error(&mut self, message: String) {
        self.save_error = Some(message);
    }

    /// Apply one key and return a persistence or close action when requested.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<EditorAction> {
        if matches!(key.kind, KeyEventKind::Release) {
            return None;
        }
        if self.confirming_cancel {
            return match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => Some(EditorAction::Close),
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.confirming_cancel = false;
                    None
                }
                _ => None,
            };
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            return self.validate().map(EditorAction::Save);
        }
        match key.code {
            KeyCode::Tab => self.move_focus(1),
            KeyCode::BackTab => self.move_focus(-1),
            KeyCode::Esc => return self.cancel(),
            KeyCode::Enter if self.focus == EditorField::Save => {
                return self.validate().map(EditorAction::Save);
            }
            KeyCode::Enter if self.focus == EditorField::Cancel => return self.cancel(),
            KeyCode::Left | KeyCode::Up => self.change_choice(-1),
            KeyCode::Right | KeyCode::Down => self.change_choice(1),
            KeyCode::Char(' ') => self.toggle_choice(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.type_character(character);
            }
            _ => {}
        }
        None
    }

    fn cancel(&mut self) -> Option<EditorAction> {
        if self.is_dirty() {
            self.confirming_cancel = true;
            None
        } else {
            Some(EditorAction::Close)
        }
    }

    fn fields(&self) -> Vec<EditorField> {
        let mut fields = vec![
            EditorField::Name,
            EditorField::Description,
            EditorField::Recurrence,
        ];
        match self.values.recurrence {
            RecurrenceChoice::Weekly => {
                fields.extend([EditorField::Interval, EditorField::Weekdays]);
            }
            RecurrenceChoice::Daily => fields.push(EditorField::Interval),
            RecurrenceChoice::Monthly => fields.push(EditorField::MonthlyDay),
        }
        fields.extend([EditorField::Enabled, EditorField::Save, EditorField::Cancel]);
        fields
    }

    fn move_focus(&mut self, delta: isize) {
        let fields = self.fields();
        let current = fields
            .iter()
            .position(|field| *field == self.focus)
            .unwrap_or(0);
        let len = isize::try_from(fields.len()).unwrap_or(1);
        let next = (isize::try_from(current).unwrap_or(0) + delta).rem_euclid(len);
        self.focus = fields[usize::try_from(next).unwrap_or(0)];
    }

    fn change_choice(&mut self, delta: i16) {
        match self.focus {
            EditorField::Recurrence => {
                let current = match self.values.recurrence {
                    RecurrenceChoice::Weekly => 0_i16,
                    RecurrenceChoice::Daily => 1,
                    RecurrenceChoice::Monthly => 2,
                };
                self.values.recurrence = match (current + delta).rem_euclid(3) {
                    0 => RecurrenceChoice::Weekly,
                    1 => RecurrenceChoice::Daily,
                    _ => RecurrenceChoice::Monthly,
                };
            }
            EditorField::Interval => change_number(&mut self.values.interval, delta, 1, 999),
            EditorField::MonthlyDay => change_number(&mut self.values.monthly_day, delta, 1, 31),
            EditorField::Weekdays => {
                self.weekday_cursor = usize::try_from(
                    (i16::try_from(self.weekday_cursor).unwrap_or(0) + delta).rem_euclid(7),
                )
                .unwrap_or(0);
            }
            EditorField::Enabled => self.values.enabled = !self.values.enabled,
            _ => {}
        }
        self.save_error = None;
    }

    fn toggle_choice(&mut self) {
        match self.focus {
            EditorField::Weekdays => {
                let weekday = WEEKDAYS[self.weekday_cursor];
                if !self.values.weekdays.remove(&weekday) {
                    self.values.weekdays.insert(weekday);
                }
            }
            EditorField::Enabled => self.values.enabled = !self.values.enabled,
            _ => {}
        }
        self.save_error = None;
    }

    fn backspace(&mut self) {
        match self.focus {
            EditorField::Name => {
                self.values.name.pop();
            }
            EditorField::Description => {
                self.values.description.pop();
            }
            EditorField::Interval => {
                self.values.interval.pop();
            }
            EditorField::MonthlyDay => {
                self.values.monthly_day.pop();
            }
            _ => {}
        }
        self.save_error = None;
    }

    fn type_character(&mut self, character: char) {
        match self.focus {
            EditorField::Name => self.values.name.push(character),
            EditorField::Description => self.values.description.push(character),
            EditorField::Interval if character.is_ascii_digit() => {
                self.values.interval.push(character);
            }
            EditorField::MonthlyDay if character.is_ascii_digit() => {
                self.values.monthly_day.push(character);
            }
            _ => {}
        }
        self.save_error = None;
    }

    fn validate(&mut self) -> Option<ChoreSubmission> {
        self.errors.clear();
        self.save_error = None;
        let name = ChoreName::new(self.values.name.clone())
            .map_err(|error| {
                self.errors.insert(EditorField::Name, error.to_string());
            })
            .ok();
        let description = Description::optional(self.values.description.clone())
            .map_err(|error| {
                self.errors
                    .insert(EditorField::Description, error.to_string());
            })
            .ok();
        let pattern = self.validate_pattern();
        if let Some(first) = self
            .fields()
            .into_iter()
            .find(|field| self.errors.contains_key(field))
        {
            self.focus = first;
            return None;
        }
        let (Some(name), Some(description), Some(pattern)) = (name, description, pattern) else {
            return None;
        };
        Some(ChoreSubmission {
            id: match self.mode {
                EditorMode::Add => None,
                EditorMode::Edit(id) => Some(id),
            },
            name,
            description,
            enabled: self.values.enabled,
            pattern,
            provenance: self.provenance.clone(),
        })
    }

    fn validate_pattern(&mut self) -> Option<SchedulePattern> {
        match self.values.recurrence {
            RecurrenceChoice::Weekly => {
                let interval = parse_interval(&self.values.interval)
                    .map_err(|message| {
                        self.errors.insert(EditorField::Interval, message);
                    })
                    .ok()?;
                if self.values.weekdays.is_empty() {
                    self.errors.insert(
                        EditorField::Weekdays,
                        "select at least one weekday".to_owned(),
                    );
                    return None;
                }
                Some(SchedulePattern::Weekly {
                    interval,
                    weekdays: self.values.weekdays.iter().copied().collect(),
                })
            }
            RecurrenceChoice::Daily => parse_interval(&self.values.interval)
                .map(|interval| SchedulePattern::DailyInterval { interval })
                .map_err(|message| {
                    self.errors.insert(EditorField::Interval, message);
                })
                .ok(),
            RecurrenceChoice::Monthly => self
                .values
                .monthly_day
                .parse::<u8>()
                .ok()
                .and_then(|value| MonthlyDay::new(value).ok())
                .map(|day| SchedulePattern::Monthly { day })
                .or_else(|| {
                    self.errors.insert(
                        EditorField::MonthlyDay,
                        "monthly day must be between 1 and 31".to_owned(),
                    );
                    None
                }),
        }
    }
}

fn parse_interval(value: &str) -> Result<RecurrenceInterval, String> {
    value
        .parse::<u16>()
        .ok()
        .and_then(|value| RecurrenceInterval::new(value).ok())
        .ok_or_else(|| "interval must be between 1 and 999".to_owned())
}

fn change_number(value: &mut String, delta: i16, minimum: i16, maximum: i16) {
    let current = value.parse::<i16>().unwrap_or(minimum);
    *value = (current + delta).clamp(minimum, maximum).to_string();
}

/// Render the editor and its dirty-cancel confirmation.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &EditorState) {
    let title = editor_title(state.mode);
    let recurrence = match state.values.recurrence {
        RecurrenceChoice::Weekly => "Weekly",
        RecurrenceChoice::Daily => "Every N days",
        RecurrenceChoice::Monthly => "Monthly",
    };
    let weekday_text = weekday_text(state);
    let mut lines = vec![
        field_line(state, EditorField::Name, "Name", &state.values.name),
        field_line(
            state,
            EditorField::Description,
            "Description",
            &state.values.description,
        ),
        field_line(state, EditorField::Recurrence, "Recurrence", recurrence),
    ];
    add_recurrence_lines(&mut lines, state, &weekday_text);
    lines.push(field_line(
        state,
        EditorField::Enabled,
        "Enabled",
        if state.values.enabled { "[x]" } else { "[ ]" },
    ));
    lines.push(Line::from(""));
    lines.push(field_line(state, EditorField::Save, "", "[ Save ]"));
    lines.push(field_line(state, EditorField::Cancel, "", "[ Cancel ]"));
    if let Some(error) = state.save_error() {
        lines.push(Line::styled(
            error.to_owned(),
            Style::default().fg(Color::Red),
        ));
    }
    if let Some(notice) = state.notice() {
        lines.push(Line::styled(
            notice.to_owned(),
            Style::default().fg(Color::Yellow),
        ));
    }
    lines.push(Line::from(if state.can_browse_catalog() {
        "F2 catalog  Tab focus  Arrows choose  Space toggle  Ctrl+S save  Esc cancel"
    } else {
        "Tab focus  Arrows choose  Space toggle  Ctrl+S save  Esc cancel"
    }));
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
    if state.confirming_cancel {
        let popup = centered(area, 48, 5);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new("Discard unsaved changes?\nEnter/y: discard   Esc/n: keep editing")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Confirm cancel "),
                ),
            popup,
        );
    }
}

fn editor_title(mode: EditorMode) -> &'static str {
    match mode {
        EditorMode::Add => " Add Chore ",
        EditorMode::Edit(_) => " Edit Chore — recurrence effective today ",
    }
}

fn weekday_text(state: &EditorState) -> String {
    WEEKDAYS
        .iter()
        .enumerate()
        .map(|(index, day)| {
            let mark = if state.values.weekdays.contains(day) {
                'x'
            } else {
                ' '
            };
            let cursor = if state.weekday_cursor == index {
                '>'
            } else {
                ' '
            };
            format!(
                "{cursor}[{mark}]{}",
                ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"][index]
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn add_recurrence_lines(lines: &mut Vec<Line<'static>>, state: &EditorState, weekday_text: &str) {
    match state.values.recurrence {
        RecurrenceChoice::Weekly => {
            lines.push(field_line(
                state,
                EditorField::Interval,
                "Every N weeks",
                &state.values.interval,
            ));
            lines.push(field_line(
                state,
                EditorField::Weekdays,
                "Weekdays",
                weekday_text,
            ));
        }
        RecurrenceChoice::Daily => lines.push(field_line(
            state,
            EditorField::Interval,
            "Every N days",
            &state.values.interval,
        )),
        RecurrenceChoice::Monthly => lines.push(field_line(
            state,
            EditorField::MonthlyDay,
            "Day of month",
            &state.values.monthly_day,
        )),
    }
}

fn field_line(state: &EditorState, field: EditorField, label: &str, value: &str) -> Line<'static> {
    let cursor = if state.focus == field { ">" } else { " " };
    let error = state
        .error(field)
        .map_or_else(String::new, |message| format!("  Error: {message}"));
    let style = if state.focus == field {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::styled(format!("{cursor} {label}: {value}{error}"), style)
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height.min(area.height)),
        Constraint::Fill(1),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width.min(area.width)),
        Constraint::Fill(1),
    ])
    .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn add_preselects_board_weekday_and_validates_name_inline() {
        let mut state = EditorState::add(IsoWeekday::Thursday);
        state.focus = EditorField::Save;
        assert_eq!(state.handle_key(key(KeyCode::Enter)), None);
        assert_eq!(state.focus(), EditorField::Name);
        assert!(state.error(EditorField::Name).is_some());
        state.values.name = "Bins".to_owned();
        state.focus = EditorField::Save;
        let Some(EditorAction::Save(submission)) = state.handle_key(key(KeyCode::Enter)) else {
            panic!("valid form should save")
        };
        assert!(
            matches!(submission.pattern, SchedulePattern::Weekly { weekdays, .. } if weekdays == vec![IsoWeekday::Thursday])
        );
    }

    #[test]
    fn dirty_cancel_requires_explicit_confirmation() {
        let mut state = EditorState::add(IsoWeekday::Monday);
        state.handle_key(key(KeyCode::Char('x')));
        assert_eq!(state.handle_key(key(KeyCode::Esc)), None);
        assert!(state.is_confirming_cancel());
        assert_eq!(state.handle_key(key(KeyCode::Char('n'))), None);
        assert!(!state.is_confirming_cancel());
        state.handle_key(key(KeyCode::Esc));
        assert_eq!(
            state.handle_key(key(KeyCode::Char('y'))),
            Some(EditorAction::Close)
        );
    }

    #[test]
    fn catalog_template_prefills_editable_fields_and_provenance() {
        let catalog = crate::catalog::ActivityCatalog::bundled().expect("catalog should load");
        let template = catalog
            .find("bathroom.scrub_shower")
            .expect("template should exist");
        let mut state = EditorState::from_template(
            template,
            catalog.provenance(),
            CalendarDate::new(2026, 9, 10).expect("date should be valid"),
            false,
        );
        state.values.name.push_str(" upstairs");
        state.focus = EditorField::Save;

        let Some(EditorAction::Save(submission)) = state.handle_key(key(KeyCode::Enter)) else {
            panic!("catalog form should validate")
        };
        assert_eq!(submission.name.as_str(), "Scrub the shower upstairs");
        assert_eq!(
            submission.provenance.map(|value| value.template_id),
            Some("bathroom.scrub_shower".to_owned())
        );
    }
}
