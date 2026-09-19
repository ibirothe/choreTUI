//! Confirmation and result view for an explicit selected-day export.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::kanban::{
        KanbanDayExportReport, KanbanGatewayFailure, KanbanTaskExportOutcome, KanbanTaskFailure,
    },
    domain::CalendarDate,
};

/// Action produced by the export overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KanbanExportAction {
    Confirm,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfirmationChoice {
    Cancel,
    Export,
}

/// Modal state that never contains credentials or authorization data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KanbanExportState {
    date: CalendarDate,
    destination: String,
    eligible: usize,
    excluded: usize,
    choice: ConfirmationChoice,
    report: Option<KanbanDayExportReport>,
    selected_result: usize,
}

impl KanbanExportState {
    #[must_use]
    pub fn confirmation(
        date: CalendarDate,
        destination: String,
        eligible: usize,
        excluded: usize,
    ) -> Self {
        Self {
            date,
            destination,
            eligible,
            excluded,
            choice: ConfirmationChoice::Cancel,
            report: None,
            selected_result: 0,
        }
    }

    #[must_use]
    pub const fn date(&self) -> CalendarDate {
        self.date
    }

    #[must_use]
    pub const fn report(&self) -> Option<&KanbanDayExportReport> {
        self.report.as_ref()
    }

    pub fn set_report(&mut self, report: KanbanDayExportReport) {
        self.report = Some(report);
        self.selected_result = 0;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<KanbanExportAction> {
        if matches!(key.kind, KeyEventKind::Release) {
            return None;
        }
        if let Some(report) = &self.report {
            let maximum = report.tasks.len().saturating_sub(1);
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    return Some(KanbanExportAction::Close);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.selected_result = self.selected_result.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.selected_result = self.selected_result.saturating_add(1).min(maximum);
                }
                KeyCode::PageUp => {
                    self.selected_result = self.selected_result.saturating_sub(10);
                }
                KeyCode::PageDown => {
                    self.selected_result = self.selected_result.saturating_add(10).min(maximum);
                }
                KeyCode::Home => self.selected_result = 0,
                KeyCode::End => self.selected_result = maximum,
                _ => {}
            }
            return None;
        }

        match key.code {
            KeyCode::Char('y' | 'Y') => Some(KanbanExportAction::Confirm),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(KanbanExportAction::Close),
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                self.choice = match self.choice {
                    ConfirmationChoice::Cancel => ConfirmationChoice::Export,
                    ConfirmationChoice::Export => ConfirmationChoice::Cancel,
                };
                None
            }
            KeyCode::Enter => Some(match self.choice {
                ConfirmationChoice::Cancel => KanbanExportAction::Close,
                ConfirmationChoice::Export => KanbanExportAction::Confirm,
            }),
            _ => None,
        }
    }
}

/// Render either the once-only confirmation or the bounded, navigable result.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &KanbanExportState) {
    frame.render_widget(Clear, area);
    if let Some(report) = &state.report {
        render_result(frame, area, state, report);
    } else {
        render_confirmation(frame, area, state);
    }
}

fn render_confirmation(frame: &mut Frame<'_>, area: Rect, state: &KanbanExportState) {
    let text = vec![
        Line::from(format!("Date: {}", state.date)),
        Line::from(format!("Destination: {}", state.destination)),
        Line::from(format!("Eligible: {}", state.eligible)),
        Line::from(format!("Excluded (completed/skipped): {}", state.excluded)),
        Line::from(""),
        choice_line("Cancel", state.choice == ConfirmationChoice::Cancel),
        choice_line("Export", state.choice == ConfirmationChoice::Export),
        Line::from("Tab/Arrows select  Enter activates  y Export  n/Esc Cancel"),
    ];
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Export selected day? "),
        ),
        area,
    );
}

fn choice_line(label: &'static str, selected: bool) -> Line<'static> {
    let marker = if selected { ">" } else { " " };
    let style = if selected {
        Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default()
    };
    Line::from(Span::styled(format!("{marker} {label}"), style))
}

fn render_result(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &KanbanExportState,
    report: &KanbanDayExportReport,
) {
    let summary = report.summary();
    if area.height < 7 {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(format!("{} — {}", report.date, result_status(report))),
                Line::from(format!(
                    "I:{} R:{} S:{} F:{}",
                    summary.changed, summary.unchanged, summary.skipped, summary.failed
                )),
                Line::from("Enter/Esc close"),
            ])
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Export result "),
            ),
            area,
        );
        return;
    }
    let [header, tasks, footer] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("{} — {}", report.date, result_status(report))),
            Line::from(format!(
                "Imported: {}  Replayed: {}  Skipped: {}  Failed: {}",
                summary.changed, summary.unchanged, summary.skipped, summary.failed
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Export result "),
        ),
        header,
    );

    let visible = usize::from(tasks.height.saturating_sub(2)).max(1);
    let start = state
        .selected_result
        .saturating_add(1)
        .saturating_sub(visible);
    let lines = report
        .tasks
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(index, task)| {
            let marker = if index == state.selected_result {
                ">"
            } else {
                " "
            };
            let line = format!("{marker} {} — {}", task.name, outcome_label(task.outcome));
            if index == state.selected_result {
                Line::styled(line, Style::default().add_modifier(Modifier::REVERSED))
            } else {
                Line::from(line)
            }
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if lines.is_empty() {
            vec![Line::from("No occurrences on the selected day.")]
        } else {
            lines
        })
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(" Tasks ")),
        tasks,
    );
    frame.render_widget(
        Paragraph::new("↑/↓ navigate  PageUp/PageDown  Enter/Esc close"),
        footer,
    );
}

fn result_status(report: &KanbanDayExportReport) -> &'static str {
    let summary = report.summary();
    let accepted = summary.changed + summary.unchanged;
    match (accepted, summary.changed, summary.failed) {
        (0, _, failed) if failed > 0 => "Total failure",
        (_, _, failed) if failed > 0 => "Partial success",
        (_, 0, 0) if summary.unchanged > 0 => "Safe replay",
        (_, _, 0) if accepted > 0 => "Complete success",
        _ => "Nothing eligible",
    }
}

fn outcome_label(outcome: KanbanTaskExportOutcome) -> &'static str {
    match outcome {
        KanbanTaskExportOutcome::Changed => "imported",
        KanbanTaskExportOutcome::Unchanged => "safe replay",
        KanbanTaskExportOutcome::Skipped(_) => "excluded",
        KanbanTaskExportOutcome::Failed(KanbanTaskFailure::Mapping(_)) => "mapping failed",
        KanbanTaskExportOutcome::Failed(KanbanTaskFailure::Gateway(failure)) => match failure {
            KanbanGatewayFailure::Authentication => "authentication failed",
            KanbanGatewayFailure::DestinationUnavailable => "destination unavailable",
            KanbanGatewayFailure::UncertainOutcome => "outcome uncertain; safe to retry",
            KanbanGatewayFailure::IdempotencyConflict => "retry identity conflict",
            KanbanGatewayFailure::PolicyViolation => "rejected by destination policy",
            KanbanGatewayFailure::Rejected => "request rejected",
        },
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::domain::OccurrenceId;

    fn date() -> CalendarDate {
        CalendarDate::new(2026, 9, 19).expect("date should be valid")
    }

    #[test]
    fn cancel_is_default_and_confirmation_happens_once() {
        let mut state = KanbanExportState::confirmation(date(), "127.0.0.1:8765".into(), 2, 1);
        assert_eq!(
            state.handle_key(KeyEvent::from(KeyCode::Enter)),
            Some(KanbanExportAction::Close)
        );

        let mut state = KanbanExportState::confirmation(date(), "127.0.0.1:8765".into(), 2, 1);
        assert_eq!(
            state.handle_key(KeyEvent::from(KeyCode::Char('y'))),
            Some(KanbanExportAction::Confirm)
        );
    }

    #[test]
    fn result_view_is_navigable_and_closeable() {
        let mut state = KanbanExportState::confirmation(date(), "destination".into(), 0, 0);
        state.set_report(KanbanDayExportReport {
            date: date(),
            tasks: Vec::new(),
        });
        assert_eq!(state.handle_key(KeyEvent::from(KeyCode::Down)), None);
        assert_eq!(
            state.handle_key(KeyEvent::from(KeyCode::Esc)),
            Some(KanbanExportAction::Close)
        );
    }

    #[test]
    fn compact_terminal_keeps_final_status_visible() {
        let mut state = KanbanExportState::confirmation(date(), "destination".into(), 1, 0);
        state.set_report(KanbanDayExportReport {
            date: date(),
            tasks: vec![crate::app::kanban::KanbanTaskExportResult {
                occurrence_id: OccurrenceId::new(),
                name: "Laundry".to_owned(),
                outcome: KanbanTaskExportOutcome::Changed,
            }],
        });
        let mut terminal = Terminal::new(TestBackend::new(40, 5)).expect("terminal should build");

        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .expect("result should render");

        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..5 {
            for x in 0..40 {
                text.push_str(buffer.cell((x, y)).expect("cell should exist").symbol());
            }
        }
        assert!(text.contains("Complete success"));
        assert!(text.contains("Enter/Esc close"));
    }
}
