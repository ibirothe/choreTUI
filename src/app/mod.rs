//! Application orchestration and semantic command dispatch.

pub mod editor;
pub mod kanban;
pub mod use_cases;

use std::{collections::HashSet, io};

use time::OffsetDateTime;

use crate::{
    app::editor::{CatalogPlanningChore, ChoreSubmission, EditorRecord},
    domain::{
        CalendarDate, Chore, ChoreId, Occurrence, OccurrenceId, Timestamp,
        ports::{ChoreRepository, Clock, OccurrenceRepository, ScheduleRepository},
    },
    storage::SqliteStore,
    tui::{self, BoardApplication},
};

use crate::{
    app::kanban::{KanbanDayExportReport, KanbanDestinationScope, export_day_to_kanban},
    kanban::KanbanClient,
};

use self::use_cases::{
    AtomicEditorStore, BoardDataError, EditorDataError, TransactionalStore, UseCases,
};

/// Sanitizable application failure; detailed sources are logged by the TUI.
#[derive(Debug, thiserror::Error)]
pub enum ApplicationError<E: std::error::Error + 'static> {
    #[error(transparent)]
    Board(#[from] BoardDataError<E>),
    #[error(transparent)]
    Editor(#[from] EditorDataError<E>),
}

/// Production wall clock with local-date and UTC-timestamp semantics.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn today(&self) -> CalendarDate {
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        CalendarDate::from_date(now.date())
    }

    fn now(&self) -> Timestamp {
        Timestamp::from_datetime(OffsetDateTime::now_utc())
    }
}

struct ConfiguredApplication<A> {
    inner: A,
    kanban: Option<ConfiguredKanban>,
}

struct ConfiguredKanban {
    client: KanbanClient,
    scope: KanbanDestinationScope,
    label: String,
}

impl<A> ConfiguredApplication<A> {
    fn new(inner: A, config: &crate::config::Config) -> Self {
        let kanban = config.kanban.as_ref().and_then(|kanban| {
            let label = kanban.endpoint.to_string();
            KanbanDestinationScope::new(kanban.endpoint.socket_addr().to_string())
                .ok()
                .map(|scope| ConfiguredKanban {
                    client: KanbanClient::new(kanban),
                    scope,
                    label,
                })
        });
        Self { inner, kanban }
    }
}

impl<A: BoardApplication> BoardApplication for ConfiguredApplication<A> {
    type Error = A::Error;

    fn today(&self) -> CalendarDate {
        self.inner.today()
    }

    fn load_week(&mut self, week: crate::domain::IsoWeek) -> Result<Vec<Occurrence>, Self::Error> {
        self.inner.load_week(week)
    }

    fn toggle_completion(&mut self, id: OccurrenceId) -> Result<Occurrence, Self::Error> {
        self.inner.toggle_completion(id)
    }

    fn load_editor(&mut self, id: ChoreId) -> Result<EditorRecord, Self::Error> {
        self.inner.load_editor(id)
    }

    fn save_editor(&mut self, submission: ChoreSubmission) -> Result<ChoreId, Self::Error> {
        self.inner.save_editor(submission)
    }

    fn list_chores(&mut self) -> Result<Vec<Chore>, Self::Error> {
        self.inner.list_chores()
    }

    fn catalog_planning(&mut self) -> Result<Vec<CatalogPlanningChore>, Self::Error> {
        self.inner.catalog_planning()
    }

    fn catalog_dismissals(&mut self) -> Result<HashSet<String>, Self::Error> {
        self.inner.catalog_dismissals()
    }

    fn dismiss_catalog_template(&mut self, template_id: &str) -> Result<(), Self::Error> {
        self.inner.dismiss_catalog_template(template_id)
    }

    fn reset_catalog_dismissals(&mut self) -> Result<usize, Self::Error> {
        self.inner.reset_catalog_dismissals()
    }

    fn disable_chore(&mut self, id: ChoreId) -> Result<(), Self::Error> {
        self.inner.disable_chore(id)
    }

    fn enable_chore(&mut self, id: ChoreId) -> Result<(), Self::Error> {
        self.inner.enable_chore(id)
    }

    fn delete_chore(&mut self, id: ChoreId) -> Result<(), Self::Error> {
        self.inner.delete_chore(id)
    }

    fn kanban_destination_label(&self) -> Option<String> {
        self.kanban.as_ref().map(|kanban| kanban.label.clone())
    }

    fn export_selected_day(
        &mut self,
        date: CalendarDate,
        occurrences: &[Occurrence],
    ) -> Option<KanbanDayExportReport> {
        self.kanban
            .as_ref()
            .map(|kanban| export_day_to_kanban(&kanban.client, &kanban.scope, date, occurrences))
    }
}

impl<P, C> BoardApplication for UseCases<P, C>
where
    P: TransactionalStore
        + AtomicEditorStore<Error = <P as TransactionalStore>::Error>
        + OccurrenceRepository<Error = <P as TransactionalStore>::Error>
        + ChoreRepository<Error = <P as TransactionalStore>::Error>
        + ScheduleRepository<Error = <P as TransactionalStore>::Error>,
    C: Clock,
{
    type Error = ApplicationError<<P as TransactionalStore>::Error>;

    fn today(&self) -> CalendarDate {
        Self::today(self)
    }

    fn load_week(&mut self, week: crate::domain::IsoWeek) -> Result<Vec<Occurrence>, Self::Error> {
        Self::load_week(self, week).map_err(ApplicationError::Board)
    }

    fn toggle_completion(&mut self, id: OccurrenceId) -> Result<Occurrence, Self::Error> {
        Self::toggle_completion(self, id)
            .map_err(BoardDataError::Persistence)
            .map_err(ApplicationError::Board)
    }

    fn load_editor(&mut self, id: ChoreId) -> Result<EditorRecord, Self::Error> {
        Self::load_editor(self, id).map_err(ApplicationError::Editor)
    }

    fn save_editor(&mut self, submission: ChoreSubmission) -> Result<ChoreId, Self::Error> {
        Self::save_editor(self, submission).map_err(ApplicationError::Editor)
    }

    fn list_chores(&mut self) -> Result<Vec<Chore>, Self::Error> {
        Self::list_chores(self, true)
            .map_err(EditorDataError::Persistence)
            .map_err(ApplicationError::Editor)
    }
    fn catalog_planning(&mut self) -> Result<Vec<CatalogPlanningChore>, Self::Error> {
        Self::catalog_planning(self)
            .map_err(EditorDataError::Persistence)
            .map_err(ApplicationError::Editor)
    }

    fn catalog_dismissals(&mut self) -> Result<HashSet<String>, Self::Error> {
        Self::catalog_dismissals(self)
            .map_err(EditorDataError::Persistence)
            .map_err(ApplicationError::Editor)
    }

    fn dismiss_catalog_template(&mut self, template_id: &str) -> Result<(), Self::Error> {
        Self::dismiss_catalog_template(self, template_id)
            .map_err(EditorDataError::Persistence)
            .map_err(ApplicationError::Editor)
    }

    fn reset_catalog_dismissals(&mut self) -> Result<usize, Self::Error> {
        Self::reset_catalog_dismissals(self)
            .map_err(EditorDataError::Persistence)
            .map_err(ApplicationError::Editor)
    }

    fn disable_chore(&mut self, id: ChoreId) -> Result<(), Self::Error> {
        Self::disable_chore(self, id)
            .map_err(BoardDataError::Persistence)
            .map_err(ApplicationError::Board)
    }

    fn enable_chore(&mut self, id: ChoreId) -> Result<(), Self::Error> {
        Self::reenable_chore(self, id)
            .map(|_| ())
            .map_err(BoardDataError::Persistence)
            .map_err(ApplicationError::Board)
    }

    fn delete_chore(&mut self, id: ChoreId) -> Result<(), Self::Error> {
        Self::soft_delete_chore(self, id)
            .map_err(BoardDataError::Persistence)
            .map_err(ApplicationError::Board)
    }
}

/// Run the interactive application.
///
/// # Errors
///
/// Returns an I/O error when the terminal cannot be entered, rendered, or
/// restored.
pub fn run(store: SqliteStore, config: &crate::config::Config) -> io::Result<()> {
    tracing::info!("starting ChoreTUI");
    tui::run(
        ConfiguredApplication::new(UseCases::new(store, SystemClock), config),
        config,
    )
}
