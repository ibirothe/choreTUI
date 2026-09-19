//! Terminal lifecycle and top-level rendering.

pub mod model;
pub mod screens;
pub mod widgets;

use std::{
    collections::HashSet,
    fmt::Display,
    io::{self, stdout},
};

use crossterm::{
    cursor::{Hide, Show},
    event::{Event, KeyCode, KeyEvent, read},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use self::{
    model::{BoardCommand, BoardLayout, BoardState, input_for_key},
    screens::catalog::{CatalogAction, CatalogBrowserState},
    screens::chore_list::{ChoreListAction, ChoreListState},
    screens::editor::{EditorAction, EditorState},
    screens::help::{HelpContext, HelpState},
    screens::kanban_export::{KanbanExportAction, KanbanExportState},
};
use crate::{
    app::{
        editor::{CatalogPlanningChore, ChoreSubmission, EditorRecord},
        kanban::KanbanDayExportReport,
    },
    domain::{CalendarDate, Chore, ChoreId, IsoWeek, Occurrence, OccurrenceId, OccurrenceState},
};

/// Application operations required by the Weekly Board event loop.
pub trait BoardApplication {
    /// Application-specific failure type; details are written to diagnostics.
    type Error: Display;

    fn today(&self) -> CalendarDate;
    /// Materialize and load one displayed week.
    ///
    /// # Errors
    ///
    /// Returns an application error when materialization or loading fails.
    fn load_week(&mut self, week: IsoWeek) -> Result<Vec<Occurrence>, Self::Error>;
    /// Atomically toggle one occurrence and return its persisted new state.
    ///
    /// # Errors
    ///
    /// Returns an application error when the transaction fails.
    fn toggle_completion(&mut self, id: OccurrenceId) -> Result<Occurrence, Self::Error>;
    /// Load one chore and its latest schedule for editing.
    ///
    /// # Errors
    ///
    /// Returns an application error when the chore or schedule cannot be loaded.
    fn load_editor(&mut self, id: ChoreId) -> Result<EditorRecord, Self::Error>;
    /// Atomically save one validated editor submission.
    ///
    /// # Errors
    ///
    /// Returns an application error when the transaction fails.
    fn save_editor(&mut self, submission: ChoreSubmission) -> Result<ChoreId, Self::Error>;
    /// Load all chores, including soft-deleted records, in display order.
    ///
    /// # Errors
    ///
    /// Returns an application error when persistence cannot be read.
    fn list_chores(&mut self) -> Result<Vec<Chore>, Self::Error>;
    /// Load active chores and catalog identities for planned-activity markers.
    ///
    /// # Errors
    ///
    /// Returns an application error when chores or provenance cannot be loaded.
    fn catalog_planning(&mut self) -> Result<Vec<CatalogPlanningChore>, Self::Error>;
    /// Load locally dismissed catalog template identifiers.
    ///
    /// # Errors
    ///
    /// Returns an application error when dismissals cannot be loaded.
    fn catalog_dismissals(&mut self) -> Result<HashSet<String>, Self::Error>;
    /// Dismiss one catalog suggestion without changing any chore.
    ///
    /// # Errors
    ///
    /// Returns an application error when the dismissal cannot be saved.
    fn dismiss_catalog_template(&mut self, template_id: &str) -> Result<(), Self::Error>;
    /// Reset all local catalog dismissals.
    ///
    /// # Errors
    ///
    /// Returns an application error when dismissals cannot be reset.
    fn reset_catalog_dismissals(&mut self) -> Result<usize, Self::Error>;
    /// Disable a chore effective today.
    ///
    /// # Errors
    ///
    /// Returns an application error when the transaction fails.
    fn disable_chore(&mut self, id: ChoreId) -> Result<(), Self::Error>;
    /// Re-enable a chore effective today.
    ///
    /// # Errors
    ///
    /// Returns an application error when the transaction fails.
    fn enable_chore(&mut self, id: ChoreId) -> Result<(), Self::Error>;
    /// Soft-delete a chore while retaining history.
    ///
    /// # Errors
    ///
    /// Returns an application error when the transaction fails.
    fn delete_chore(&mut self, id: ChoreId) -> Result<(), Self::Error>;

    /// Return a non-secret label when the optional kanbanTUI export is enabled.
    fn kanban_destination_label(&self) -> Option<String> {
        None
    }

    /// Export the supplied materialized snapshot to the configured destination.
    fn export_selected_day(
        &mut self,
        _date: CalendarDate,
        _occurrences: &[Occurrence],
    ) -> Option<KanbanDayExportReport> {
        None
    }
}

/// Deterministic bridge between board commands and application operations.
pub struct BoardRuntime<A> {
    application: A,
    state: BoardState,
    editor: Option<EditorState>,
    catalog_browser: Option<CatalogBrowserState>,
    catalog_origin_editor: Option<EditorState>,
    chore_list: Option<ChoreListState>,
    help: Option<HelpState>,
    kanban_export: Option<KanbanExportState>,
    confirm_delete: bool,
    status_persistent: bool,
}

impl<A: BoardApplication> BoardRuntime<A> {
    /// Load and materialize the current week. A startup failure produces a
    /// usable empty board with persistent, actionable feedback.
    #[must_use]
    pub fn new(application: A) -> Self {
        Self::new_with_config(application, &crate::config::Config::default())
    }

    /// Load the current week with explicit user-interface options.
    #[must_use]
    pub fn new_with_config(mut application: A, config: &crate::config::Config) -> Self {
        let today = application.today();
        let week = IsoWeek::containing(today);
        let (state, status_persistent) = match application.load_week(week) {
            Ok(occurrences) => (BoardState::new(week, today, occurrences), false),
            Err(error) => {
                tracing::error!(%error, %week, "could not load initial board week");
                let mut state = BoardState::new(week, today, Vec::new());
                state.set_status(Some(load_error_message()));
                (state, true)
            }
        };
        let mut state = state;
        state.set_show_completed(config.show_completed);
        Self {
            application,
            state,
            editor: None,
            catalog_browser: None,
            catalog_origin_editor: None,
            chore_list: None,
            help: None,
            kanban_export: None,
            confirm_delete: config.confirm_delete,
            status_persistent,
        }
    }

    #[must_use]
    pub const fn state(&self) -> &BoardState {
        &self.state
    }

    pub const fn state_mut(&mut self) -> &mut BoardState {
        &mut self.state
    }

    #[must_use]
    pub const fn editor(&self) -> Option<&EditorState> {
        self.editor.as_ref()
    }

    #[must_use]
    pub const fn catalog_browser(&self) -> Option<&CatalogBrowserState> {
        self.catalog_browser.as_ref()
    }

    pub fn catalog_browser_mut(&mut self) -> Option<&mut CatalogBrowserState> {
        self.catalog_browser.as_mut()
    }

    #[must_use]
    pub const fn chore_list(&self) -> Option<&ChoreListState> {
        self.chore_list.as_ref()
    }

    pub fn chore_list_mut(&mut self) -> Option<&mut ChoreListState> {
        self.chore_list.as_mut()
    }

    #[must_use]
    pub const fn help(&self) -> Option<&HelpState> {
        self.help.as_ref()
    }

    #[must_use]
    pub const fn kanban_export(&self) -> Option<&KanbanExportState> {
        self.kanban_export.as_ref()
    }

    /// Route a raw key to the active editor or the Weekly Board.
    pub fn handle_key(&mut self, key: KeyEvent, layout: BoardLayout) -> bool {
        if let Some(export) = self.kanban_export.as_mut() {
            match export.handle_key(key) {
                Some(KanbanExportAction::Confirm) => self.confirm_kanban_export(),
                Some(KanbanExportAction::Close) => self.kanban_export = None,
                None => {}
            }
            return false;
        }
        if let Some(help) = self.help.as_mut() {
            if help.handle_key(key, 12) {
                self.help = None;
            }
            return false;
        }
        if self.catalog_browser.is_some() && self.editor.is_none() {
            let action = self
                .catalog_browser
                .as_mut()
                .and_then(|catalog| catalog.handle_key(key));
            match action {
                Some(CatalogAction::Close) => {
                    self.catalog_browser = None;
                    self.editor = self.catalog_origin_editor.take();
                }
                Some(CatalogAction::Help) => {
                    self.help = Some(HelpState::new(HelpContext::Catalog));
                }
                Some(CatalogAction::Select(template_id)) => {
                    self.open_catalog_template(&template_id);
                }
                Some(CatalogAction::Dismiss(template_id)) => {
                    self.dismiss_catalog_template(&template_id);
                }
                Some(CatalogAction::ResetDismissals) => self.reset_catalog_dismissals(),
                None => {}
            }
            return false;
        }
        if key.code == KeyCode::Char('?')
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| !editor.is_confirming_cancel())
        {
            self.help = Some(HelpState::new(HelpContext::Editor));
            return false;
        }
        if let Some(editor) = self.editor.as_mut() {
            if key.code == KeyCode::F(2) && editor.can_browse_catalog() {
                self.open_catalog();
                return false;
            }
            let action = editor.handle_key(key);
            self.handle_editor_action(action);
            return false;
        }
        if let Some(chore_list) = self.chore_list.as_mut() {
            let action = chore_list.handle_key(key);
            self.handle_chore_list_action(action);
            return false;
        }
        input_for_key(key).is_some_and(|input| self.handle_input(input, layout))
    }

    /// Apply an input and return `true` only when the application should quit.
    pub fn handle_input(&mut self, input: model::BoardInput, layout: BoardLayout) -> bool {
        if !self.status_persistent {
            self.state.set_status(None);
        }
        let command = match self.state.handle_input(input, layout) {
            Ok(command) => command,
            Err(error) => {
                tracing::error!(%error, "week navigation failed");
                self.state.set_status(Some(
                    "Error: Week is outside the supported range; choose another week.".to_owned(),
                ));
                self.status_persistent = true;
                return false;
            }
        };
        match command {
            Some(BoardCommand::Quit) => true,
            Some(BoardCommand::LoadWeek(week)) => {
                self.load_week(week);
                false
            }
            Some(BoardCommand::ToggleCompletion(id)) => {
                self.toggle_completion(id);
                false
            }
            Some(BoardCommand::AddChore(weekday)) => {
                self.editor = Some(EditorState::add(weekday));
                false
            }
            Some(BoardCommand::EditChore(id)) => {
                match self.application.load_editor(id) {
                    Ok(record) => self.editor = Some(EditorState::edit(record)),
                    Err(error) => {
                        tracing::error!(%error, chore_id = %id, "could not open chore editor");
                        self.state.set_status(Some(
                            "Error: Could not load chore; retry or run `chore doctor`.".to_owned(),
                        ));
                        self.status_persistent = true;
                    }
                }
                false
            }
            Some(BoardCommand::DisableChore(id)) => {
                self.apply_lifecycle(ChoreListAction::Disable(id));
                false
            }
            Some(BoardCommand::DeleteChore(id)) => {
                if self.confirm_delete {
                    self.open_chore_list(Some(id));
                } else {
                    self.apply_lifecycle(ChoreListAction::Delete(id));
                }
                false
            }
            Some(BoardCommand::OpenChoreList) => {
                self.open_chore_list(None);
                false
            }
            Some(BoardCommand::OpenGuidedPlanning) => {
                self.open_guided_catalog();
                false
            }
            Some(BoardCommand::ExportSelectedDay(date)) => {
                self.open_kanban_export(date);
                false
            }
            Some(BoardCommand::Help) => {
                self.help = Some(HelpState::new(HelpContext::Board));
                self.status_persistent = false;
                false
            }
            None => false,
        }
    }

    /// Consume the runtime, primarily for deterministic integration tests.
    pub fn into_parts(self) -> (A, BoardState) {
        (self.application, self.state)
    }

    fn load_week(&mut self, week: IsoWeek) {
        match self.application.load_week(week) {
            Ok(occurrences) => {
                self.state.replace_week(week, occurrences);
                self.state.set_status(None);
                self.status_persistent = false;
            }
            Err(error) => {
                tracing::error!(%error, %week, "could not load board week");
                self.state.set_status(Some(load_error_message()));
                self.status_persistent = true;
            }
        }
    }

    fn open_kanban_export(&mut self, date: CalendarDate) {
        let Some(destination) = self.application.kanban_destination_label() else {
            self.state.set_status(Some(
                "Export disabled: configure [kanban] endpoint and token_env first.".to_owned(),
            ));
            self.status_persistent = false;
            return;
        };
        let mut eligible = 0;
        let mut excluded = 0;
        for occurrence in self
            .state
            .occurrences()
            .iter()
            .filter(|occurrence| occurrence.due_date() == date)
        {
            if occurrence.state() == OccurrenceState::Pending {
                eligible += 1;
            } else {
                excluded += 1;
            }
        }
        self.kanban_export = Some(KanbanExportState::confirmation(
            date,
            destination,
            eligible,
            excluded,
        ));
    }

    fn confirm_kanban_export(&mut self) {
        let Some(date) = self.kanban_export.as_ref().map(KanbanExportState::date) else {
            return;
        };
        let snapshot = self.state.occurrences().to_vec();
        let Some(report) = self.application.export_selected_day(date, &snapshot) else {
            self.kanban_export = None;
            self.state.set_status(Some(
                "Export disabled: integration is no longer available; check configuration."
                    .to_owned(),
            ));
            self.status_persistent = true;
            return;
        };
        if let Some(export) = self.kanban_export.as_mut() {
            export.set_report(report);
        }
    }

    fn toggle_completion(&mut self, id: OccurrenceId) {
        let updated = match self.application.toggle_completion(id) {
            Ok(updated) => updated,
            Err(error) => {
                tracing::error!(%error, occurrence_id = %id, "could not toggle occurrence");
                self.state.set_status(Some(
                    "Error: Chore was not changed; retry or run `chore doctor`.".to_owned(),
                ));
                self.status_persistent = true;
                return;
            }
        };
        match self.application.load_week(self.state.week()) {
            Ok(occurrences) => {
                self.state.refresh_occurrences(occurrences, Some(id));
                let message = match updated.state() {
                    OccurrenceState::Completed { .. } => "Chore marked complete.",
                    OccurrenceState::Pending => "Chore marked pending.",
                    OccurrenceState::Skipped => "Chore state refreshed.",
                };
                self.state.set_status(Some(message.to_owned()));
                self.status_persistent = false;
            }
            Err(error) => {
                tracing::error!(%error, occurrence_id = %id, "saved toggle but refresh failed");
                self.state.set_status(Some(
                    "Saved, but refresh failed; reopen the week or run `chore doctor`.".to_owned(),
                ));
                self.status_persistent = true;
            }
        }
    }

    fn handle_editor_action(&mut self, action: Option<EditorAction>) {
        match action {
            Some(EditorAction::Close) => self.editor = None,
            Some(EditorAction::Save(submission)) => {
                match self.application.save_editor(submission) {
                    Ok(id) => {
                        self.editor = None;
                        if self.catalog_browser.is_some() {
                            self.catalog_origin_editor = None;
                        }
                        match self.application.load_week(self.state.week()) {
                            Ok(occurrences) => {
                                self.state.refresh_occurrences(occurrences, None);
                                self.state.set_status(Some("Chore saved.".to_owned()));
                                self.status_persistent = false;
                            }
                            Err(error) => {
                                tracing::error!(%error, chore_id = %id, "saved chore but refresh failed");
                                self.state.set_status(Some(
                                    "Saved, but refresh failed; reopen the week or run `chore doctor`."
                                        .to_owned(),
                                ));
                                self.status_persistent = true;
                            }
                        }
                        if self.catalog_browser.is_some() {
                            self.refresh_catalog_planning(Some(
                                "Chore saved; the activity is now marked planned.".to_owned(),
                            ));
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "could not save chore editor");
                        if let Some(editor) = self.editor.as_mut() {
                            editor.set_save_error(
                                "Could not save; nothing changed. Retry or run `chore doctor`."
                                    .to_owned(),
                            );
                        }
                    }
                }
            }
            None => {}
        }
    }

    fn open_catalog(&mut self) {
        let planning = match self.application.catalog_planning() {
            Ok(planning) => planning,
            Err(error) => {
                tracing::error!(%error, "could not load catalog planning state");
                if let Some(editor) = self.editor.as_mut() {
                    editor.set_save_error(
                        "Could not load activity planning state; free-form creation remains available."
                            .to_owned(),
                    );
                }
                return;
            }
        };
        match crate::catalog::ActivityCatalog::bundled() {
            Ok(catalog) => {
                let mut browser = CatalogBrowserState::new(catalog);
                browser.set_planning(&planning);
                self.catalog_origin_editor = self.editor.take();
                self.catalog_browser = Some(browser);
            }
            Err(error) => {
                tracing::error!(%error, "could not load bundled activity catalog");
                if let Some(editor) = self.editor.as_mut() {
                    editor.set_save_error(
                        "Could not load activity catalog; free-form creation remains available."
                            .to_owned(),
                    );
                }
            }
        }
    }

    fn open_guided_catalog(&mut self) {
        let planning = match self.application.catalog_planning() {
            Ok(planning) => planning,
            Err(error) => {
                tracing::error!(%error, "could not load guided planning state");
                self.state.set_status(Some(
                    "Error: Could not load guided planning; retry or run `chore doctor`."
                        .to_owned(),
                ));
                self.status_persistent = true;
                return;
            }
        };
        let dismissals = match self.application.catalog_dismissals() {
            Ok(dismissals) => dismissals,
            Err(error) => {
                tracing::error!(%error, "could not load catalog dismissals");
                self.state.set_status(Some(
                    "Error: Could not load guided planning; retry or run `chore doctor`."
                        .to_owned(),
                ));
                self.status_persistent = true;
                return;
            }
        };
        match crate::catalog::ActivityCatalog::bundled() {
            Ok(catalog) => {
                let mut browser = CatalogBrowserState::guided(catalog);
                browser.set_planning(&planning);
                browser.set_dismissals(dismissals);
                self.catalog_origin_editor = None;
                self.catalog_browser = Some(browser);
            }
            Err(error) => {
                tracing::error!(%error, "could not load bundled activity catalog");
                self.state.set_status(Some(
                    "Error: Could not load activity catalog; free-form creation remains available."
                        .to_owned(),
                ));
                self.status_persistent = true;
            }
        }
    }

    fn dismiss_catalog_template(&mut self, template_id: &str) {
        match self.application.dismiss_catalog_template(template_id) {
            Ok(()) => {
                if let Some(browser) = self.catalog_browser.as_mut() {
                    browser.mark_dismissed(template_id.to_owned());
                    browser.set_status(Some(
                        "Suggestion dismissed; scheduled chores were not changed.".to_owned(),
                    ));
                }
            }
            Err(error) => {
                tracing::error!(%error, %template_id, "could not dismiss catalog template");
                if let Some(browser) = self.catalog_browser.as_mut() {
                    browser.set_status(Some(
                        "Could not dismiss suggestion; nothing changed.".to_owned(),
                    ));
                }
            }
        }
    }

    fn reset_catalog_dismissals(&mut self) {
        match self.application.reset_catalog_dismissals() {
            Ok(count) => {
                if let Some(browser) = self.catalog_browser.as_mut() {
                    browser.clear_dismissals();
                    browser.set_status(Some(format!("Reset {count} dismissed suggestions.")));
                }
            }
            Err(error) => {
                tracing::error!(%error, "could not reset catalog dismissals");
                if let Some(browser) = self.catalog_browser.as_mut() {
                    browser.set_status(Some(
                        "Could not reset dismissed suggestions; nothing changed.".to_owned(),
                    ));
                }
            }
        }
    }

    fn open_catalog_template(&mut self, template_id: &str) {
        let selected_date = self.state.day_date(self.state.selected_day());
        let Some(browser) = self.catalog_browser.as_ref() else {
            return;
        };
        let Some(template) = browser.template(template_id) else {
            return;
        };
        let provenance = browser.provenance();
        let possible_duplicate = matches!(
            browser.planning_status(template_id),
            screens::catalog::PlanningStatus::PossibleDuplicate
        );
        self.editor = Some(EditorState::from_template(
            &template,
            provenance,
            selected_date,
            possible_duplicate,
        ));
    }

    fn refresh_catalog_planning(&mut self, success: Option<String>) {
        match self.application.catalog_planning() {
            Ok(planning) => {
                if let Some(browser) = self.catalog_browser.as_mut() {
                    browser.set_planning(&planning);
                    browser.set_status(success);
                }
            }
            Err(error) => {
                tracing::error!(%error, "could not refresh catalog planning state");
                if let Some(browser) = self.catalog_browser.as_mut() {
                    browser.set_status(Some(
                        "Saved, but planned markers could not be refreshed.".to_owned(),
                    ));
                }
            }
        }
    }

    fn open_chore_list(&mut self, delete_candidate: Option<ChoreId>) {
        match self.application.list_chores() {
            Ok(items) => {
                let mut state = ChoreListState::new(items, self.confirm_delete);
                if let Some(id) = delete_candidate {
                    let _ = state.request_delete(id);
                }
                self.chore_list = Some(state);
            }
            Err(error) => {
                tracing::error!(%error, "could not load chore list");
                self.state.set_status(Some(
                    "Error: Could not load chores; retry or run `chore doctor`.".to_owned(),
                ));
                self.status_persistent = true;
            }
        }
    }

    fn handle_chore_list_action(&mut self, action: Option<ChoreListAction>) {
        match action {
            Some(ChoreListAction::Close) => self.chore_list = None,
            Some(ChoreListAction::Edit(id)) => match self.application.load_editor(id) {
                Ok(record) => {
                    self.chore_list = None;
                    self.editor = Some(EditorState::edit(record));
                }
                Err(error) => self.set_list_error(&error, "Could not load chore for editing."),
            },
            Some(ChoreListAction::Help) => {
                self.help = Some(HelpState::new(HelpContext::ChoreList));
            }
            Some(
                action @ (ChoreListAction::Disable(_)
                | ChoreListAction::Enable(_)
                | ChoreListAction::Delete(_)),
            ) => self.apply_lifecycle(action),
            None => {}
        }
    }

    fn apply_lifecycle(&mut self, action: ChoreListAction) {
        let result = match action {
            ChoreListAction::Disable(id) => self.application.disable_chore(id).map(|()| id),
            ChoreListAction::Enable(id) => self.application.enable_chore(id).map(|()| id),
            ChoreListAction::Delete(id) => self.application.delete_chore(id).map(|()| id),
            _ => return,
        };
        let id = match result {
            Ok(id) => id,
            Err(error) => {
                self.set_list_error(&error, "Lifecycle change failed; nothing changed.");
                return;
            }
        };
        if let Some(list) = self.chore_list.as_mut() {
            match self.application.list_chores() {
                Ok(items) => list.replace_items(items),
                Err(error) => {
                    tracing::error!(%error, chore_id = %id, "saved lifecycle change but list refresh failed");
                    list.set_status(Some(
                        "Saved, but list refresh failed; reopen the list.".to_owned(),
                    ));
                }
            }
        }
        match self.application.load_week(self.state.week()) {
            Ok(occurrences) => {
                self.state.refresh_occurrences(occurrences, None);
                if let Some(list) = self.chore_list.as_mut() {
                    list.set_status(Some("Chore lifecycle updated.".to_owned()));
                } else {
                    self.state
                        .set_status(Some("Chore lifecycle updated.".to_owned()));
                }
                self.status_persistent = false;
            }
            Err(error) => {
                tracing::error!(%error, chore_id = %id, "saved lifecycle change but board refresh failed");
                self.status_persistent = true;
            }
        }
    }

    fn set_list_error(&mut self, error: &A::Error, message: &str) {
        tracing::error!(%error, "chore list action failed");
        if let Some(list) = self.chore_list.as_mut() {
            list.set_status(Some(format!(
                "Error: {message} Retry or run `chore doctor`."
            )));
        } else {
            self.state.set_status(Some(format!(
                "Error: {message} Retry or run `chore doctor`."
            )));
        }
        self.status_persistent = true;
    }
}

fn load_error_message() -> String {
    "Error: Could not load week; current board kept. Retry or run `chore doctor`.".to_owned()
}

trait TerminalControl {
    fn enter(&mut self) -> io::Result<()>;
    fn restore(&mut self) -> io::Result<()>;
}

#[derive(Default)]
struct CrosstermControl;

impl TerminalControl for CrosstermControl {
    fn enter(&mut self) -> io::Result<()> {
        enable_raw_mode()?;

        if let Err(error) = execute!(stdout(), EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(error);
        }

        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        restore_terminal()
    }
}

struct TerminalGuard<C: TerminalControl> {
    control: C,
    active: bool,
}

impl<C: TerminalControl> TerminalGuard<C> {
    fn enter(mut control: C) -> io::Result<Self> {
        control.enter()?;
        Ok(Self {
            control,
            active: true,
        })
    }

    fn restore(&mut self) -> io::Result<()> {
        if self.active {
            self.active = false;
            self.control.restore()?;
        }
        Ok(())
    }
}

impl<C: TerminalControl> Drop for TerminalGuard<C> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Restore the process terminal. The operation is intentionally idempotent so
/// both the panic hook and the lifecycle guard can call it safely.
///
/// # Errors
///
/// Returns an I/O error when raw mode or alternate-screen cleanup fails.
pub fn restore_terminal() -> io::Result<()> {
    let raw_mode_result = disable_raw_mode();
    let screen_result = execute!(stdout(), LeaveAlternateScreen, Show);

    raw_mode_result.and(screen_result)
}

/// Run the Weekly Board event loop and restore the terminal before returning.
///
/// # Errors
///
/// Returns an I/O error when terminal setup or rendering fails. Cleanup still
/// runs through the lifecycle guard.
pub fn run<A: BoardApplication>(application: A, config: &crate::config::Config) -> io::Result<()> {
    let _guard = TerminalGuard::enter(CrosstermControl)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut runtime = BoardRuntime::new_with_config(application, config);

    loop {
        terminal.draw(|frame| {
            if let Some(export) = runtime.kanban_export() {
                screens::kanban_export::render(frame, frame.area(), export);
            } else if let Some(help) = runtime.help() {
                screens::help::render(frame, frame.area(), help);
            } else if let Some(editor) = runtime.editor() {
                screens::editor::render(frame, frame.area(), editor);
            } else if let Some(catalog) = runtime.catalog_browser_mut() {
                screens::catalog::render(frame, frame.area(), catalog);
            } else if let Some(chore_list) = runtime.chore_list_mut() {
                screens::chore_list::render(frame, frame.area(), chore_list);
            } else {
                screens::board::render(frame, frame.area(), runtime.state_mut());
            }
        })?;
        let Event::Key(key) = read()? else {
            continue;
        };
        let area = terminal.size()?;
        let layout = BoardLayout::for_size(area.width, area.height);
        if runtime.handle_key(key, layout) {
            break;
        }
    }

    terminal.show_cursor()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        fmt, io,
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use crossterm::event::{KeyCode, KeyEvent};

    use super::{
        BoardApplication, BoardRuntime, KanbanExportState, TerminalControl, TerminalGuard,
    };
    use crate::{
        app::{
            editor::{CatalogPlanningChore, ChoreSubmission, EditorRecord},
            kanban::{KanbanDayExportReport, KanbanTaskExportOutcome, KanbanTaskExportResult},
        },
        domain::{
            CalendarDate, Chore, ChoreId, ChoreName, IsoWeek, Occurrence, OccurrenceId,
            OccurrenceSeed, OccurrenceState, ScheduleId, Timestamp,
        },
        tui::model::{BoardInput, BoardLayout},
    };

    struct FakeControl {
        enter_count: Arc<AtomicUsize>,
        restore_count: Arc<AtomicUsize>,
    }

    #[derive(Clone, Copy, Debug)]
    struct FakeError;

    impl fmt::Display for FakeError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("injected transaction failure")
        }
    }

    struct FakeApplication {
        today: CalendarDate,
        occurrences: Vec<Occurrence>,
        fail_toggle: bool,
        fail_load: bool,
        dismissals: HashSet<String>,
        kanban_enabled: bool,
        export_dates: Vec<CalendarDate>,
    }

    impl BoardApplication for FakeApplication {
        type Error = FakeError;

        fn today(&self) -> CalendarDate {
            self.today
        }

        fn load_week(&mut self, week: IsoWeek) -> Result<Vec<Occurrence>, Self::Error> {
            if self.fail_load {
                Err(FakeError)
            } else {
                Ok(self
                    .occurrences
                    .iter()
                    .filter(|item| week.contains(item.due_date()))
                    .cloned()
                    .collect())
            }
        }

        fn toggle_completion(&mut self, id: OccurrenceId) -> Result<Occurrence, Self::Error> {
            if self.fail_toggle {
                return Err(FakeError);
            }
            let item = self
                .occurrences
                .iter_mut()
                .find(|item| item.id() == id)
                .ok_or(FakeError)?;
            item.toggle_completion(timestamp(50))
                .map_err(|_| FakeError)?;
            Ok(item.clone())
        }

        fn load_editor(&mut self, _id: ChoreId) -> Result<EditorRecord, Self::Error> {
            Err(FakeError)
        }

        fn save_editor(&mut self, _submission: ChoreSubmission) -> Result<ChoreId, Self::Error> {
            Err(FakeError)
        }

        fn list_chores(&mut self) -> Result<Vec<Chore>, Self::Error> {
            Ok(Vec::new())
        }

        fn catalog_planning(&mut self) -> Result<Vec<CatalogPlanningChore>, Self::Error> {
            Ok(Vec::new())
        }

        fn catalog_dismissals(&mut self) -> Result<HashSet<String>, Self::Error> {
            Ok(self.dismissals.clone())
        }

        fn dismiss_catalog_template(&mut self, template_id: &str) -> Result<(), Self::Error> {
            self.dismissals.insert(template_id.to_owned());
            Ok(())
        }

        fn reset_catalog_dismissals(&mut self) -> Result<usize, Self::Error> {
            let count = self.dismissals.len();
            self.dismissals.clear();
            Ok(count)
        }

        fn disable_chore(&mut self, _id: ChoreId) -> Result<(), Self::Error> {
            Err(FakeError)
        }

        fn enable_chore(&mut self, _id: ChoreId) -> Result<(), Self::Error> {
            Err(FakeError)
        }

        fn delete_chore(&mut self, _id: ChoreId) -> Result<(), Self::Error> {
            Err(FakeError)
        }

        fn kanban_destination_label(&self) -> Option<String> {
            self.kanban_enabled
                .then(|| "http://127.0.0.1:8765".to_owned())
        }

        fn export_selected_day(
            &mut self,
            date: CalendarDate,
            occurrences: &[Occurrence],
        ) -> Option<KanbanDayExportReport> {
            if !self.kanban_enabled {
                return None;
            }
            self.export_dates.push(date);
            Some(KanbanDayExportReport {
                date,
                tasks: occurrences
                    .iter()
                    .filter(|occurrence| occurrence.due_date() == date)
                    .map(|occurrence| KanbanTaskExportResult {
                        occurrence_id: occurrence.id(),
                        name: occurrence.name().as_str().to_owned(),
                        outcome: KanbanTaskExportOutcome::Changed,
                    })
                    .collect(),
            })
        }
    }

    fn date(year: i32, month: u8, day: u8) -> CalendarDate {
        CalendarDate::new(year, month, day).expect("test date should be valid")
    }

    fn timestamp(seconds: i64) -> Timestamp {
        Timestamp::from_unix_timestamp(seconds).expect("test timestamp should be valid")
    }

    fn occurrence(due: CalendarDate) -> Occurrence {
        Occurrence::pending(OccurrenceSeed {
            id: OccurrenceId::new(),
            chore_id: ChoreId::new(),
            schedule_id: ScheduleId::new(),
            nominal_date: due,
            due_date: due,
            name: ChoreName::new("Laundry").expect("test name should be valid"),
            description: None,
            created_at: timestamp(1),
        })
    }

    impl TerminalControl for FakeControl {
        fn enter(&mut self) -> io::Result<()> {
            self.enter_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn restore(&mut self) -> io::Result<()> {
            self.restore_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn guard_restores_once_on_normal_drop() {
        let enter_count = Arc::new(AtomicUsize::new(0));
        let restore_count = Arc::new(AtomicUsize::new(0));

        {
            let _guard = TerminalGuard::enter(FakeControl {
                enter_count: Arc::clone(&enter_count),
                restore_count: Arc::clone(&restore_count),
            })
            .expect("fake terminal should enter");
        }

        assert_eq!(enter_count.load(Ordering::SeqCst), 1);
        assert_eq!(restore_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn guard_restores_during_panic_unwind() {
        let restore_count = Arc::new(AtomicUsize::new(0));
        let observed_count = Arc::clone(&restore_count);

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = TerminalGuard::enter(FakeControl {
                enter_count: Arc::new(AtomicUsize::new(0)),
                restore_count,
            })
            .expect("fake terminal should enter");
            panic!("controlled panic");
        }));

        assert!(result.is_err());
        assert_eq!(observed_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn successful_toggle_reloads_state_statistics_and_keeps_selection() {
        let monday = date(2026, 9, 7);
        let item = occurrence(monday);
        let id = item.id();
        let application = FakeApplication {
            today: monday,
            occurrences: vec![item],
            fail_toggle: false,
            fail_load: false,
            dismissals: HashSet::new(),
            kanban_enabled: false,
            export_dates: Vec::new(),
        };
        let mut runtime = BoardRuntime::new(application);

        assert_eq!(runtime.state().statistics().completed(), 0);
        runtime.handle_input(BoardInput::ToggleCompletion, BoardLayout::SevenColumns);

        assert_eq!(
            runtime.state().selected_occurrence().map(Occurrence::id),
            Some(id)
        );
        assert!(matches!(
            runtime.state().selected_occurrence().map(Occurrence::state),
            Some(OccurrenceState::Completed { .. })
        ));
        assert_eq!(runtime.state().statistics().completed(), 1);
        assert_eq!(runtime.state().status(), Some("Chore marked complete."));
        runtime.handle_input(BoardInput::NextOccurrence, BoardLayout::SevenColumns);
        assert_eq!(runtime.state().status(), None);
    }

    #[test]
    fn add_editor_opens_catalog_and_escape_returns_without_persistence() {
        let monday = date(2026, 9, 7);
        let application = FakeApplication {
            today: monday,
            occurrences: Vec::new(),
            fail_toggle: false,
            fail_load: false,
            dismissals: HashSet::new(),
            kanban_enabled: false,
            export_dates: Vec::new(),
        };
        let mut runtime = BoardRuntime::new(application);

        runtime.handle_input(BoardInput::AddChore, BoardLayout::SevenColumns);
        assert!(runtime.editor().is_some());
        assert!(runtime.catalog_browser().is_none());

        runtime.handle_key(KeyEvent::from(KeyCode::F(2)), BoardLayout::SevenColumns);
        assert!(runtime.editor().is_none());
        assert_eq!(
            runtime
                .catalog_browser()
                .map(|catalog| catalog.matching_activities().len()),
            Some(148)
        );

        runtime.handle_key(KeyEvent::from(KeyCode::Enter), BoardLayout::SevenColumns);
        assert!(runtime.editor().is_some());
        assert!(runtime.catalog_browser().is_some());

        runtime.handle_key(KeyEvent::from(KeyCode::Esc), BoardLayout::SevenColumns);
        assert!(runtime.editor().is_none());
        assert!(runtime.catalog_browser().is_some());

        runtime.handle_key(KeyEvent::from(KeyCode::Esc), BoardLayout::SevenColumns);
        assert!(runtime.catalog_browser().is_none());
        assert!(runtime.editor().is_some());
    }

    #[test]
    fn guided_planning_dismisses_resets_and_routes_selection_to_editor() {
        let monday = date(2026, 9, 7);
        let application = FakeApplication {
            today: monday,
            occurrences: Vec::new(),
            fail_toggle: false,
            fail_load: false,
            dismissals: HashSet::new(),
            kanban_enabled: false,
            export_dates: Vec::new(),
        };
        let mut runtime = BoardRuntime::new(application);

        runtime.handle_input(BoardInput::OpenGuidedPlanning, BoardLayout::SevenColumns);
        let initial = runtime
            .catalog_browser()
            .map(|catalog| catalog.matching_activities().len())
            .expect("guided catalog should open");
        assert_eq!(initial, 148);

        runtime.handle_key(
            KeyEvent::from(KeyCode::Char('x')),
            BoardLayout::SevenColumns,
        );
        assert_eq!(
            runtime
                .catalog_browser()
                .map(|catalog| catalog.matching_activities().len()),
            Some(initial - 1)
        );
        runtime.handle_key(
            KeyEvent::from(KeyCode::Char('R')),
            BoardLayout::SevenColumns,
        );
        assert_eq!(
            runtime
                .catalog_browser()
                .map(|catalog| catalog.matching_activities().len()),
            Some(initial)
        );

        runtime.handle_key(KeyEvent::from(KeyCode::Enter), BoardLayout::SevenColumns);
        assert!(runtime.editor().is_some());
        assert!(runtime.catalog_browser().is_some());
        runtime.handle_key(KeyEvent::from(KeyCode::Esc), BoardLayout::SevenColumns);
        assert!(runtime.editor().is_none());
        assert!(runtime.catalog_browser().is_some());
    }

    #[test]
    fn failed_toggle_keeps_persisted_view_and_error_visible_during_navigation() {
        let monday = date(2026, 9, 7);
        let application = FakeApplication {
            today: monday,
            occurrences: vec![occurrence(monday)],
            fail_toggle: true,
            fail_load: false,
            dismissals: HashSet::new(),
            kanban_enabled: false,
            export_dates: Vec::new(),
        };
        let mut runtime = BoardRuntime::new(application);

        runtime.handle_input(BoardInput::ToggleCompletion, BoardLayout::SevenColumns);
        assert_eq!(
            runtime.state().selected_occurrence().map(Occurrence::state),
            Some(OccurrenceState::Pending)
        );
        let error = runtime.state().status().map(str::to_owned);
        assert!(
            error
                .as_deref()
                .is_some_and(|message| message.contains("not changed"))
        );

        runtime.handle_input(BoardInput::NextOccurrence, BoardLayout::SevenColumns);
        assert_eq!(runtime.state().status(), error.as_deref());
    }

    #[test]
    fn failed_week_load_keeps_previous_week_and_data() {
        let monday = date(2026, 9, 7);
        let application = FakeApplication {
            today: monday,
            occurrences: vec![occurrence(monday)],
            fail_toggle: false,
            fail_load: false,
            dismissals: HashSet::new(),
            kanban_enabled: false,
            export_dates: Vec::new(),
        };
        let mut runtime = BoardRuntime::new(application);
        let old_week = runtime.state().week();
        let old_id = runtime.state().selected_occurrence().map(Occurrence::id);
        runtime.application.fail_load = true;

        runtime.handle_input(BoardInput::NextWeek, BoardLayout::SevenColumns);

        assert_eq!(runtime.state().week(), old_week);
        assert_eq!(
            runtime.state().selected_occurrence().map(Occurrence::id),
            old_id
        );
        assert!(
            runtime
                .state()
                .status()
                .is_some_and(|message| message.contains("current board kept"))
        );
    }

    #[test]
    fn export_uses_selected_date_requires_confirmation_and_preserves_board() {
        let monday = date(2026, 9, 7);
        let tuesday = date(2026, 9, 8);
        let monday_item = occurrence(monday);
        let tuesday_item = occurrence(tuesday);
        let tuesday_id = tuesday_item.id();
        let application = FakeApplication {
            today: monday,
            occurrences: vec![monday_item, tuesday_item],
            fail_toggle: false,
            fail_load: false,
            dismissals: HashSet::new(),
            kanban_enabled: true,
            export_dates: Vec::new(),
        };
        let mut runtime = BoardRuntime::new(application);
        runtime.handle_input(BoardInput::NextDay, BoardLayout::SevenColumns);
        assert_eq!(runtime.state().selected_day(), 1);

        runtime.handle_key(
            KeyEvent::from(KeyCode::Char('x')),
            BoardLayout::SevenColumns,
        );
        assert_eq!(
            runtime.kanban_export().map(KanbanExportState::date),
            Some(tuesday)
        );
        runtime.handle_key(KeyEvent::from(KeyCode::Esc), BoardLayout::SevenColumns);
        assert!(runtime.application.export_dates.is_empty());

        runtime.handle_key(
            KeyEvent::from(KeyCode::Char('x')),
            BoardLayout::SevenColumns,
        );
        runtime.handle_key(
            KeyEvent::from(KeyCode::Char('y')),
            BoardLayout::SevenColumns,
        );
        assert_eq!(runtime.application.export_dates, [tuesday]);
        assert!(
            runtime
                .kanban_export()
                .and_then(KanbanExportState::report)
                .is_some()
        );
        assert_eq!(runtime.state().selected_day(), 1);
        assert_eq!(
            runtime.state().selected_occurrence().map(Occurrence::id),
            Some(tuesday_id)
        );
    }

    #[test]
    fn unconfigured_export_is_contextual_and_sends_nothing() {
        let monday = date(2026, 9, 7);
        let application = FakeApplication {
            today: monday,
            occurrences: vec![occurrence(monday)],
            fail_toggle: false,
            fail_load: false,
            dismissals: HashSet::new(),
            kanban_enabled: false,
            export_dates: Vec::new(),
        };
        let mut runtime = BoardRuntime::new(application);

        runtime.handle_input(BoardInput::ExportSelectedDay, BoardLayout::SevenColumns);

        assert!(runtime.kanban_export().is_none());
        assert!(runtime.application.export_dates.is_empty());
        assert!(
            runtime
                .state()
                .status()
                .is_some_and(|status| status.contains("Export disabled"))
        );
    }
}
