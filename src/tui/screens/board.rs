//! Responsive Weekly Board rendering.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::{
    domain::{Occurrence, OccurrenceState},
    tui::{
        model::{BoardLayout, BoardState},
        widgets::truncate_with_ellipsis,
    },
};

const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Render the board and update scroll offsets to keep its selection visible.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &mut BoardState) {
    let layout = BoardLayout::for_size(area.width, area.height);
    if layout == BoardLayout::TooSmall {
        render_too_small(frame, area);
        return;
    }

    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(area);
    match layout {
        BoardLayout::SevenColumns => render_seven_columns(frame, body, state),
        BoardLayout::TwoRows => render_two_rows(frame, body, state),
        BoardLayout::SelectedDay => render_selected_day(frame, body, state),
        BoardLayout::TooSmall => unreachable!("handled before board layout"),
    }
    render_footer(frame, footer, state);
}

fn render_seven_columns(frame: &mut Frame<'_>, area: Rect, state: &mut BoardState) {
    let columns = Layout::horizontal([Constraint::Ratio(1, 7); 7]).split(area);
    for (day, column) in columns.iter().copied().enumerate() {
        render_day(frame, column, state, day);
    }
}

fn render_two_rows(frame: &mut Frame<'_>, area: Rect, state: &mut BoardState) {
    let rows = Layout::vertical([Constraint::Ratio(1, 2); 2]).split(area);
    let first = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(rows[0]);
    let second = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(rows[1]);
    for (day, column) in first.iter().copied().enumerate() {
        render_day(frame, column, state, day);
    }
    for (offset, column) in second.iter().copied().enumerate() {
        render_day(frame, column, state, offset + 4);
    }
}

fn render_selected_day(frame: &mut Frame<'_>, area: Rect, state: &mut BoardState) {
    let [strip, pane] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(area);
    let cells = Layout::horizontal([Constraint::Ratio(1, 7); 7]).split(strip);
    for (day, cell) in cells.iter().copied().enumerate() {
        let date = state.day_date(day).as_date();
        let selected = day == state.selected_day();
        let label = format!("{}{:02}", &DAY_NAMES[day][..1], date.day());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(if selected {
                BorderType::Double
            } else {
                BorderType::Plain
            })
            .border_style(if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            });
        frame.render_widget(Paragraph::new(label).centered().block(block), cell);
    }
    render_day(frame, pane, state, state.selected_day());
}

fn render_day(frame: &mut Frame<'_>, area: Rect, state: &mut BoardState, day: usize) {
    let selected_day = day == state.selected_day();
    let visible_rows = usize::from(area.height.saturating_sub(2));
    state.ensure_selection_visible(day, visible_rows);
    let offset = state.scroll_offset(day);
    let occurrences = state.occurrences_for_day(day).collect::<Vec<_>>();
    let date = state.day_date(day).as_date();
    let is_today = state.day_date(day) == state.today();

    let mut title = format!("{} {:02}", DAY_NAMES[day], date.day());
    if is_today {
        title.push_str(" *");
    }
    if selected_day {
        title = format!("> {title}");
    }
    let hidden_above = offset > 0;
    let hidden_below = offset.saturating_add(visible_rows) < occurrences.len();
    if hidden_above {
        title.push_str(" ↑");
    }
    if hidden_below {
        title.push_str(" ↓");
    }

    let border_style = if selected_day {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if is_today {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if selected_day {
            BorderType::Double
        } else {
            BorderType::Plain
        })
        .border_style(border_style)
        .title(title);

    let lines = if occurrences.is_empty() {
        vec![Line::styled(
            "No chores",
            Style::default().add_modifier(Modifier::DIM),
        )]
    } else {
        occurrences
            .into_iter()
            .enumerate()
            .skip(offset)
            .take(visible_rows)
            .map(|(position, occurrence)| {
                occurrence_line(
                    occurrence,
                    selected_day && state.selected_occurrence_position() == Some(position),
                    state.today(),
                    usize::from(area.width.saturating_sub(2)),
                )
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn occurrence_line<'a>(
    occurrence: &Occurrence,
    selected: bool,
    today: crate::domain::CalendarDate,
    width: usize,
) -> Line<'a> {
    let completed = matches!(occurrence.state(), OccurrenceState::Completed { .. });
    let missed = occurrence.is_missed(today);
    let cursor = if selected { '>' } else { ' ' };
    let checkbox = if completed { "[x]" } else { "[ ]" };
    let alert = if missed { '!' } else { ' ' };
    let prefix = format!("{cursor}{checkbox}{alert} ");
    let name_width = width.saturating_sub(prefix.chars().count());
    let name = truncate_with_ellipsis(occurrence.name().as_str(), name_width);
    let mut style = Style::default();
    if completed {
        style = style.add_modifier(Modifier::DIM | Modifier::CROSSED_OUT);
    }
    if missed {
        style = style.fg(Color::Red).add_modifier(Modifier::BOLD);
    }
    if selected {
        style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
    }
    Line::from(Span::styled(format!("{prefix}{name}"), style))
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, state: &BoardState) {
    let statistics = state.statistics();
    let completion = statistics
        .completion_percentage()
        .map_or_else(|| "—".to_owned(), |value| format!("{value}%"));
    let summary = if area.width < 70 {
        format!(
            "{} Done:{}/{} Comp:{} Missed:{}",
            state.week(),
            statistics.completed(),
            statistics.total(),
            completion,
            statistics.missed()
        )
    } else {
        format!(
            "{}  Done: {} / {}  Completion: {}  Missed: {}",
            state.week(),
            statistics.completed(),
            statistics.total(),
            completion,
            statistics.missed()
        )
    };
    let default_hints = if area.width < 70 {
        "? Help  q Quit  ←/→ Day  [/] Week"
    } else {
        "? Help  q Quit  ←/h →/l Day  ↑/k ↓/j Chore  [/] Week  Space Toggle  a Add  e Edit  c List"
    };
    let hints = state.status().unwrap_or(default_hints);
    frame.render_widget(
        Paragraph::new(vec![Line::from(summary), Line::from(hints)]),
        area,
    );
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect) {
    let message = vec![
        Line::styled(
            "Terminal too small",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::from("Required: 40x14"),
        Line::from(format!("Current: {}x{}", area.width, area.height)),
        Line::from("? Help  q Quit"),
    ];
    frame.render_widget(
        Paragraph::new(message)
            .centered()
            .block(Block::default().borders(Borders::ALL).title(" ChoreTUI ")),
        area,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::domain::{
        CalendarDate, ChoreId, ChoreName, IsoWeek, OccurrenceId, OccurrenceSeed, ScheduleId,
        Timestamp,
    };

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

    fn render_text(width: u16, height: u16, state: &mut BoardState) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .expect("board should render");
        let buffer = terminal.backend().buffer();
        let mut output = String::new();
        for y in 0..height {
            for x in 0..width {
                output.push_str(buffer.cell((x, y)).expect("cell should exist").symbol());
            }
            output.push('\n');
        }
        output
    }

    #[test]
    fn all_specification_boundary_dimensions_render_without_panicking() {
        let monday = date(2026, 9, 7);
        for (width, height) in [(39, 13), (40, 14), (69, 20), (70, 20), (109, 20), (110, 20)] {
            let mut state = BoardState::new(IsoWeek::containing(monday), monday, Vec::new());
            let output = render_text(width, height, &mut state);
            if width == 39 {
                assert!(output.contains("Terminal too small"));
                assert!(output.contains("Current: 39x13"));
            } else {
                assert!(output.contains("2026-W37"));
                assert!(output.contains("Comp"));
                assert!(output.contains("Missed:0") || output.contains("Missed: 0"));
                assert!(output.contains("? Help"));
                assert!(output.contains("No chores"));
            }
        }
    }

    #[test]
    fn markers_truncation_empty_days_and_statistics_are_textually_accessible() {
        let monday = date(2026, 9, 7);
        let today = date(2026, 9, 14);
        let items = vec![
            occurrence(
                monday,
                "An exceptionally long chore name",
                OccurrenceState::Pending,
            ),
            occurrence(
                monday,
                "Done",
                OccurrenceState::Completed { at: timestamp(20) },
            ),
        ];
        let mut state = BoardState::new(IsoWeek::containing(monday), today, items);
        let output = render_text(110, 20, &mut state);
        assert!(output.contains(">[ ]!"));
        assert!(output.contains("[x]"));
        assert!(output.contains('…'));
        assert!(output.contains("Done: 1 / 2"));
        assert!(output.contains("Completion: 50%"));
        assert!(output.contains("Missed: 1"));
        assert!(output.contains("No chores"));
    }

    #[test]
    fn scrolling_exposes_non_color_indicators_for_hidden_rows() {
        let monday = date(2026, 9, 7);
        let items = (0..12)
            .map(|number| occurrence(monday, &format!("Chore {number}"), OccurrenceState::Pending))
            .collect();
        let mut state = BoardState::new(IsoWeek::containing(monday), monday, items);
        for _ in 0..10 {
            state
                .handle_input(
                    crate::tui::model::BoardInput::NextOccurrence,
                    BoardLayout::SevenColumns,
                )
                .unwrap();
        }
        let output = render_text(110, 20, &mut state);
        assert!(output.contains('↑'));
        assert!(output.contains('>'));
    }
}
