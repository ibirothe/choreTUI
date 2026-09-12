//! Searchable lifecycle-management view for active and deleted chores.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::domain::{Chore, ChoreId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputMode {
    Navigate,
    Filter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeletedView {
    ActiveOnly,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeletePolicy {
    Confirm,
    Immediate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeleteChoice {
    Cancel,
    Delete,
}

/// Persistence action requested by the chore list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoreListAction {
    Close,
    Edit(ChoreId),
    Disable(ChoreId),
    Enable(ChoreId),
    Delete(ChoreId),
    Help,
}

/// Deterministic list, filter, selection, scrolling, and confirmation state.
#[derive(Clone, Debug)]
pub struct ChoreListState {
    items: Vec<Chore>,
    selected: usize,
    scroll: usize,
    filter: String,
    input_mode: InputMode,
    deleted_view: DeletedView,
    delete_policy: DeletePolicy,
    delete_candidate: Option<ChoreId>,
    delete_choice: DeleteChoice,
    status: Option<String>,
}

impl ChoreListState {
    #[must_use]
    pub const fn new(items: Vec<Chore>, confirm_delete: bool) -> Self {
        Self {
            items,
            selected: 0,
            scroll: 0,
            filter: String::new(),
            input_mode: InputMode::Navigate,
            deleted_view: DeletedView::ActiveOnly,
            delete_policy: if confirm_delete {
                DeletePolicy::Confirm
            } else {
                DeletePolicy::Immediate
            },
            delete_candidate: None,
            delete_choice: DeleteChoice::Cancel,
            status: None,
        }
    }

    #[must_use]
    pub fn selected_chore(&self) -> Option<&Chore> {
        self.visible_indices()
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
    }

    #[must_use]
    pub fn filter(&self) -> &str {
        &self.filter
    }

    #[must_use]
    pub const fn is_filtering(&self) -> bool {
        matches!(self.input_mode, InputMode::Filter)
    }

    #[must_use]
    pub const fn shows_deleted(&self) -> bool {
        matches!(self.deleted_view, DeletedView::All)
    }

    #[must_use]
    pub const fn is_confirming_delete(&self) -> bool {
        self.delete_candidate.is_some()
    }

    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
    }

    /// Replace list data while retaining the selected identity when possible.
    pub fn replace_items(&mut self, items: Vec<Chore>) {
        let selected = self.selected_chore().map(Chore::id);
        self.items = items;
        let visible = self.visible_indices();
        self.selected = selected
            .and_then(|id| {
                visible
                    .iter()
                    .position(|index| self.items[*index].id() == id)
            })
            .unwrap_or_else(|| self.selected.min(visible.len().saturating_sub(1)));
    }

    /// Open delete confirmation for a specific non-deleted chore.
    pub fn request_delete(&mut self, id: ChoreId) -> Option<ChoreListAction> {
        if self.delete_policy == DeletePolicy::Confirm {
            self.delete_candidate = Some(id);
            self.delete_choice = DeleteChoice::Cancel;
            None
        } else {
            Some(ChoreListAction::Delete(id))
        }
    }

    /// Apply one key according to modal precedence.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<ChoreListAction> {
        if matches!(key.kind, KeyEventKind::Release) {
            return None;
        }
        if self.delete_candidate.is_some() {
            return self.handle_confirmation(key.code);
        }
        if self.input_mode == InputMode::Filter {
            return self.handle_filter_key(key.code);
        }
        self.status = None;
        match key.code {
            KeyCode::Esc => Some(ChoreListAction::Close),
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Filter;
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = self
                    .selected
                    .saturating_add(1)
                    .min(self.visible_indices().len().saturating_sub(1));
                None
            }
            KeyCode::Enter | KeyCode::Char('e') => self.edit_action(),
            KeyCode::Char(' ') => self.lifecycle_action(),
            KeyCode::Char('D') => self.delete_action(),
            KeyCode::Char('x') => {
                self.deleted_view = match self.deleted_view {
                    DeletedView::ActiveOnly => DeletedView::All,
                    DeletedView::All => DeletedView::ActiveOnly,
                };
                self.selected = 0;
                self.scroll = 0;
                None
            }
            KeyCode::Char('?') => Some(ChoreListAction::Help),
            _ => None,
        }
    }

    pub fn ensure_selection_visible(&mut self, rows: usize) {
        if rows == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll.saturating_add(rows) {
            self.scroll = self.selected.saturating_add(1).saturating_sub(rows);
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, chore)| self.deleted_view == DeletedView::All || !chore.is_deleted())
            .filter(|(_, chore)| chore.name().as_str().to_lowercase().contains(&needle))
            .map(|(index, _)| index)
            .collect()
    }

    fn handle_filter_key(&mut self, code: KeyCode) -> Option<ChoreListAction> {
        match code {
            KeyCode::Esc | KeyCode::Enter => self.input_mode = InputMode::Navigate,
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
                self.scroll = 0;
            }
            KeyCode::Char(character) => {
                self.filter.push(character);
                self.selected = 0;
                self.scroll = 0;
            }
            _ => {}
        }
        None
    }

    fn handle_confirmation(&mut self, code: KeyCode) -> Option<ChoreListAction> {
        match code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                self.delete_choice = match self.delete_choice {
                    DeleteChoice::Cancel => DeleteChoice::Delete,
                    DeleteChoice::Delete => DeleteChoice::Cancel,
                };
                None
            }
            KeyCode::Char('y' | 'Y') => self.delete_candidate.take().map(ChoreListAction::Delete),
            KeyCode::Enter if self.delete_choice == DeleteChoice::Delete => {
                self.delete_candidate.take().map(ChoreListAction::Delete)
            }
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                self.delete_candidate = None;
                self.delete_choice = DeleteChoice::Cancel;
                None
            }
            _ => None,
        }
    }

    fn edit_action(&mut self) -> Option<ChoreListAction> {
        let chore = self.selected_chore()?;
        if chore.is_deleted() {
            self.status = Some("Deleted chores are read-only.".to_owned());
            None
        } else {
            Some(ChoreListAction::Edit(chore.id()))
        }
    }

    fn lifecycle_action(&mut self) -> Option<ChoreListAction> {
        let chore = self.selected_chore()?;
        if chore.is_deleted() {
            self.status = Some("Deleted chores cannot be re-enabled.".to_owned());
            None
        } else if chore.is_enabled() {
            Some(ChoreListAction::Disable(chore.id()))
        } else {
            Some(ChoreListAction::Enable(chore.id()))
        }
    }

    fn delete_action(&mut self) -> Option<ChoreListAction> {
        let chore = self.selected_chore()?;
        if chore.is_deleted() {
            self.status = Some("Chore is already deleted.".to_owned());
            None
        } else {
            self.request_delete(chore.id())
        }
    }
}

/// Render the chore list and optional delete confirmation.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &mut ChoreListState) {
    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(area);
    let visible_rows = usize::from(body.height.saturating_sub(2));
    state.ensure_selection_visible(visible_rows);
    let visible = state.visible_indices();
    let lines = if visible.is_empty() {
        vec![Line::from("No matching chores")]
    } else {
        visible
            .into_iter()
            .enumerate()
            .skip(state.scroll)
            .take(visible_rows)
            .map(|(position, index)| chore_line(&state.items[index], position == state.selected))
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Chore List ")),
        body,
    );
    let filter = if state.input_mode == InputMode::Filter {
        "Filter>"
    } else {
        "Filter:"
    };
    let mode = if state.deleted_view == DeletedView::All {
        "showing deleted"
    } else {
        "active only"
    };
    let help = state.status().unwrap_or(
        "↑/k ↓/j move  / filter  Enter/e edit  Space enable/disable  D delete  x deleted  Esc back",
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("{filter} {}  ({mode})", state.filter)),
            Line::from(help),
        ]),
        footer,
    );
    if state.delete_candidate.is_some() {
        render_confirmation(frame, area, state.delete_choice == DeleteChoice::Delete);
    }
}

fn chore_line(chore: &Chore, selected: bool) -> Line<'static> {
    let state = if chore.is_deleted() {
        "deleted"
    } else if chore.is_enabled() {
        "enabled"
    } else {
        "disabled"
    };
    let cursor = if selected { '>' } else { ' ' };
    let read_only = if chore.is_deleted() {
        " [read-only]"
    } else {
        ""
    };
    let mut style = Style::default();
    if selected {
        style = style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
    }
    if chore.is_deleted() {
        style = style.fg(Color::DarkGray);
    }
    Line::styled(
        format!("{cursor} [{}] {}{read_only}", state, chore.name()),
        style,
    )
}

fn render_confirmation(frame: &mut Frame<'_>, area: Rect, delete_selected: bool) {
    let popup = centered(area, 52, 5);
    let cancel = if delete_selected {
        "Cancel"
    } else {
        "> Cancel"
    };
    let delete = if delete_selected {
        "> Delete"
    } else {
        "Delete"
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(format!(
            "Soft-delete this chore? History is retained.\n{cancel}    {delete}"
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Confirm delete "),
        ),
        popup,
    );
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
    use crate::domain::{ChoreName, Timestamp};

    fn chore(name: &str) -> Chore {
        Chore::new(
            ChoreId::new(),
            ChoreName::new(name).expect("name should be valid"),
            None,
            Timestamp::from_unix_timestamp(1).expect("timestamp should be valid"),
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    #[test]
    fn filtering_and_selection_are_deterministic() {
        let mut state = ChoreListState::new(vec![chore("Bins"), chore("Laundry")], true);
        state.handle_key(key(KeyCode::Char('/')));
        state.handle_key(key(KeyCode::Char('l')));
        state.handle_key(key(KeyCode::Enter));
        assert_eq!(state.filter(), "l");
        assert_eq!(
            state.selected_chore().map(|item| item.name().as_str()),
            Some("Laundry")
        );
    }

    #[test]
    fn delete_confirmation_defaults_to_cancel() {
        let item = chore("Bins");
        let id = item.id();
        let mut state = ChoreListState::new(vec![item], true);
        assert_eq!(state.handle_key(key(KeyCode::Char('D'))), None);
        assert!(state.is_confirming_delete());
        assert_eq!(state.handle_key(key(KeyCode::Enter)), None);
        assert!(!state.is_confirming_delete());
        assert_eq!(state.request_delete(id), None);
        state.handle_key(key(KeyCode::Right));
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            Some(ChoreListAction::Delete(id))
        );
    }
}
