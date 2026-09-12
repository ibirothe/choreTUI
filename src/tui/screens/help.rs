//! Contextual, scrollable keyboard reference.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

/// Screen whose commands lead the help content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpContext {
    Board,
    Editor,
    ChoreList,
}

/// Scroll position and source context for the help overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpState {
    context: HelpContext,
    scroll: usize,
}

impl HelpState {
    #[must_use]
    pub const fn new(context: HelpContext) -> Self {
        Self { context, scroll: 0 }
    }

    #[must_use]
    pub const fn context(&self) -> HelpContext {
        self.context
    }

    #[must_use]
    pub const fn scroll(&self) -> usize {
        self.scroll
    }

    /// Apply scrolling or close keys. Returns `true` when help should close.
    pub fn handle_key(&mut self, key: KeyEvent, visible_rows: usize) -> bool {
        if matches!(key.kind, KeyEventKind::Release) {
            return false;
        }
        let maximum = help_lines(self.context)
            .len()
            .saturating_sub(visible_rows);
        match key.code {
            KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_add(1).min(maximum);
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(visible_rows.max(1)),
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(visible_rows.max(1)).min(maximum);
            }
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = maximum,
            _ => {}
        }
        false
    }
}

/// Render contextual commands followed by global commands.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &HelpState) {
    let rows = usize::from(area.height.saturating_sub(2));
    let lines = help_lines(state.context)
        .into_iter()
        .skip(state.scroll)
        .take(rows)
        .map(|line| {
            if line.ends_with(':') {
                Line::styled(line, Style::default().add_modifier(Modifier::BOLD))
            } else {
                Line::from(line)
            }
        })
        .collect::<Vec<_>>();
    let position = format!(" Help  {}/{} ", state.scroll.saturating_add(1), help_lines(state.context).len());
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(position)),
        area,
    );
}

fn help_lines(context: HelpContext) -> Vec<&'static str> {
    let mut lines = match context {
        HelpContext::Board => vec![
            "Weekly Board:",
            "h/Left, l/Right   previous/next day",
            "k/Up, j/Down      previous/next chore",
            "Space             toggle completion",
            "a                 add chore for selected weekday",
            "e                 edit selected chore",
            "d / D             disable / soft-delete chore",
            "c                 open Chore List",
            "[ / ]             previous/next ISO week",
            "PageUp/PageDown   previous/next ISO week",
            "t                 return to current week",
        ],
        HelpContext::Editor => vec![
            "Chore Editor:",
            "Tab/BackTab       move field focus",
            "Arrows            change choices and values",
            "Space             toggle weekday or enabled state",
            "Enter             activate Save or Cancel",
            "Ctrl+S            validate and save atomically",
            "Esc               cancel; dirty forms ask first",
            "Recurrence edits are effective today.",
        ],
        HelpContext::ChoreList => vec![
            "Chore List:",
            "k/Up, j/Down      move selection",
            "/                 filter by chore name",
            "Enter/e           edit selected active chore",
            "Space             enable or disable",
            "D                 soft-delete; history is retained",
            "x                 show/hide deleted chores",
            "Deleted chores are read-only in MVP.",
        ],
    };
    lines.extend([
        "",
        "Global:",
        "?                 open/close contextual help",
        "Esc               close overlay or return",
        "q                 quit from the board; close help",
        "Confirmations     y confirms; n/Esc cancels",
        "                  Tab selects a button; Enter activates",
        "",
        "Diagnostics:",
        "Run `chore doctor` after database or configuration errors.",
        "Close ChoreTUI before copying the database for backup.",
    ]);
    lines
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn help_scrolls_within_bounds_and_closes_with_all_documented_keys() {
        let mut state = HelpState::new(HelpContext::Board);
        for _ in 0..100 {
            assert!(!state.handle_key(KeyEvent::from(KeyCode::Down), 4));
        }
        assert!(state.scroll() > 0);
        assert!(!state.handle_key(KeyEvent::from(KeyCode::Home), 4));
        assert_eq!(state.scroll(), 0);
        for code in [KeyCode::Char('?'), KeyCode::Char('q'), KeyCode::Esc] {
            assert!(state.handle_key(KeyEvent::from(code), 4));
        }
    }

    #[test]
    fn renderer_exposes_context_and_global_commands_without_color() {
        let backend = TestBackend::new(70, 18);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        let state = HelpState::new(HelpContext::ChoreList);
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .expect("help should render");
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..18 {
            for x in 0..70 {
                text.push_str(buffer.cell((x, y)).expect("cell should exist").symbol());
            }
        }
        assert!(text.contains("Chore List:"));
        assert!(text.contains("Space"));
        assert!(text.contains("Global:"));
    }
}
