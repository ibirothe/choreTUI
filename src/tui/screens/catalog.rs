//! Keyboard-driven activity-catalog browsing, search, facets, and preview.

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::{
    app::editor::CatalogPlanningChore,
    catalog::{
        ActivityCatalog, ActivityContext, ActivityTemplate, ActivityType, Area, CadenceKind,
        DurationBand, Effort,
    },
    tui::widgets::truncate_with_ellipsis,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputMode {
    Navigate,
    Search,
    Filter,
}

/// Controlled catalog facet currently receiving keyboard input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterFocus {
    Area,
    ActivityType,
    Effort,
    Duration,
    Context,
    Cadence,
}

impl FilterFocus {
    const ALL: [Self; 6] = [
        Self::Area,
        Self::ActivityType,
        Self::Effort,
        Self::Duration,
        Self::Context,
        Self::Cadence,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Area => "Area",
            Self::ActivityType => "Type",
            Self::Effort => "Effort",
            Self::Duration => "Duration",
            Self::Context => "Context",
            Self::Cadence => "Cadence",
        }
    }
}

#[derive(Clone, Debug, Default)]
struct CatalogFilters {
    areas: HashSet<Area>,
    activity_types: HashSet<ActivityType>,
    efforts: HashSet<Effort>,
    durations: HashSet<DurationBand>,
    contexts: HashSet<ActivityContext>,
    cadences: HashSet<CadenceKind>,
}

impl CatalogFilters {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn selected_count(&self) -> usize {
        self.areas.len()
            + self.activity_types.len()
            + self.efforts.len()
            + self.durations.len()
            + self.contexts.len()
            + self.cadences.len()
    }

    fn matches(&self, activity: &ActivityTemplate) -> bool {
        (self.areas.is_empty() || activity.areas.iter().any(|area| self.areas.contains(area)))
            && (self.activity_types.is_empty()
                || activity
                    .activity_types
                    .iter()
                    .any(|kind| self.activity_types.contains(kind)))
            && (self.efforts.is_empty() || self.efforts.contains(&activity.effort))
            && (self.durations.is_empty() || self.durations.contains(&activity.duration_band()))
            && (self.contexts.is_empty()
                || activity
                    .contexts
                    .iter()
                    .any(|context| self.contexts.contains(context)))
            && (self.cadences.is_empty()
                || activity
                    .suggested_cadence
                    .as_ref()
                    .is_some_and(|cadence| self.cadences.contains(&cadence.kind)))
    }
}

/// Action leaving the catalog browser or opening contextual help.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogAction {
    Close,
    Help,
    Select(String),
    Dismiss(String),
    ResetDismissals,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanningStatus {
    Planned,
    PossibleDuplicate,
    Unplanned,
}

/// Transparent, deterministic lens used by guided planning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuidedSweep {
    CoverageGaps,
    Bathroom,
    QuickTasks,
    MonthlyMaintenance,
    SafetyChecks,
}

impl GuidedSweep {
    const fn label(self) -> &'static str {
        match self {
            Self::CoverageGaps => "Coverage gaps",
            Self::Bathroom => "Bathroom",
            Self::QuickTasks => "Quick tasks",
            Self::MonthlyMaintenance => "Monthly maintenance",
            Self::SafetyChecks => "Safety checks",
        }
    }
}

/// Deterministic search, multi-facet, selection, and scroll state.
#[derive(Clone, Debug)]
pub struct CatalogBrowserState {
    catalog: ActivityCatalog,
    search: String,
    filters: CatalogFilters,
    input_mode: InputMode,
    filter_focus: FilterFocus,
    facet_cursor: usize,
    selected: usize,
    scroll: usize,
    preview_expanded: bool,
    planned_template_ids: HashSet<String>,
    possible_duplicate_ids: HashSet<String>,
    status: Option<String>,
    guided_sweep: Option<GuidedSweep>,
    dismissed_template_ids: HashSet<String>,
    reveal_hidden: bool,
    uncovered_areas: HashSet<Area>,
    uncovered_activity_types: HashSet<ActivityType>,
}

impl CatalogBrowserState {
    #[must_use]
    pub fn new(catalog: ActivityCatalog) -> Self {
        Self {
            catalog,
            search: String::new(),
            filters: CatalogFilters::default(),
            input_mode: InputMode::Navigate,
            filter_focus: FilterFocus::Area,
            facet_cursor: 0,
            selected: 0,
            scroll: 0,
            preview_expanded: false,
            planned_template_ids: HashSet::new(),
            possible_duplicate_ids: HashSet::new(),
            status: None,
            guided_sweep: None,
            dismissed_template_ids: HashSet::new(),
            reveal_hidden: false,
            uncovered_areas: ActivityCatalog::facet_metadata()
                .areas
                .iter()
                .copied()
                .collect(),
            uncovered_activity_types: ActivityCatalog::facet_metadata()
                .activity_types
                .iter()
                .copied()
                .collect(),
        }
    }

    #[must_use]
    pub fn guided(catalog: ActivityCatalog) -> Self {
        let mut state = Self::new(catalog);
        state.guided_sweep = Some(GuidedSweep::CoverageGaps);
        state
    }

    #[must_use]
    pub const fn is_guided(&self) -> bool {
        self.guided_sweep.is_some()
    }

    pub fn set_dismissals(&mut self, dismissals: HashSet<String>) {
        self.dismissed_template_ids = dismissals;
        self.normalize_selection(None);
    }

    pub fn mark_dismissed(&mut self, template_id: String) {
        self.dismissed_template_ids.insert(template_id);
        self.normalize_selection(None);
    }

    pub fn clear_dismissals(&mut self) {
        self.dismissed_template_ids.clear();
        self.normalize_selection(None);
    }

    /// Refresh exact provenance matches and conservative normalized-name matches.
    pub fn set_planning(&mut self, chores: &[CatalogPlanningChore]) {
        self.planned_template_ids = chores
            .iter()
            .filter_map(|chore| chore.provenance.as_ref())
            .map(|provenance| provenance.template_id.clone())
            .collect();
        let names = chores
            .iter()
            .map(|chore| normalize_name(&chore.name))
            .collect::<HashSet<_>>();
        self.possible_duplicate_ids = self
            .catalog
            .activities()
            .filter(|activity| {
                !self.planned_template_ids.contains(&activity.id)
                    && names.contains(&normalize_name(&activity.name))
            })
            .map(|activity| activity.id.clone())
            .collect();
        let mut covered_areas = HashSet::new();
        let mut covered_types = HashSet::new();
        for chore in chores {
            let matched = chore
                .provenance
                .as_ref()
                .and_then(|value| self.catalog.find(&value.template_id))
                .or_else(|| {
                    let name = normalize_name(&chore.name);
                    self.catalog
                        .activities()
                        .find(|activity| normalize_name(&activity.name) == name)
                });
            if let Some(activity) = matched {
                covered_areas.extend(activity.areas.iter().copied());
                covered_types.extend(activity.activity_types.iter().copied());
            }
        }
        self.uncovered_areas = ActivityCatalog::facet_metadata()
            .areas
            .iter()
            .copied()
            .filter(|area| !covered_areas.contains(area))
            .collect();
        self.uncovered_activity_types = ActivityCatalog::facet_metadata()
            .activity_types
            .iter()
            .copied()
            .filter(|kind| !covered_types.contains(kind))
            .collect();
        self.normalize_selection(None);
    }

    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
    }

    #[must_use]
    pub fn planning_status(&self, template_id: &str) -> PlanningStatus {
        if self.planned_template_ids.contains(template_id) {
            PlanningStatus::Planned
        } else if self.possible_duplicate_ids.contains(template_id) {
            PlanningStatus::PossibleDuplicate
        } else {
            PlanningStatus::Unplanned
        }
    }

    #[must_use]
    pub fn template(&self, template_id: &str) -> Option<ActivityTemplate> {
        self.catalog.find(template_id).cloned()
    }

    #[must_use]
    pub fn provenance(&self) -> crate::catalog::CatalogProvenance<'_> {
        self.catalog.provenance()
    }

    #[must_use]
    pub fn search(&self) -> &str {
        &self.search
    }

    #[must_use]
    pub const fn is_searching(&self) -> bool {
        matches!(self.input_mode, InputMode::Search)
    }

    #[must_use]
    pub const fn is_filtering(&self) -> bool {
        matches!(self.input_mode, InputMode::Filter)
    }

    #[must_use]
    pub const fn filter_focus(&self) -> FilterFocus {
        self.filter_focus
    }

    #[must_use]
    pub fn active_filter_count(&self) -> usize {
        self.filters.selected_count() + usize::from(!self.search.is_empty())
    }

    #[must_use]
    pub fn matching_activities(&self) -> Vec<&ActivityTemplate> {
        let needle = self.search.to_lowercase();
        self.catalog
            .activities()
            .filter(|activity| {
                needle.is_empty()
                    || activity.name.to_lowercase().contains(&needle)
                    || activity
                        .description
                        .as_ref()
                        .is_some_and(|description| description.to_lowercase().contains(&needle))
            })
            .filter(|activity| self.filters.matches(activity))
            .filter(|activity| self.matches_guided_sweep(activity))
            .filter(|activity| {
                self.guided_sweep.is_none()
                    || self.reveal_hidden
                    || (self.planning_status(&activity.id) == PlanningStatus::Unplanned
                        && !self.dismissed_template_ids.contains(&activity.id))
            })
            .collect()
    }

    #[must_use]
    pub fn selected_activity(&self) -> Option<&ActivityTemplate> {
        self.matching_activities().get(self.selected).copied()
    }

    #[must_use]
    pub const fn selected_position(&self) -> usize {
        self.selected
    }

    /// Apply one keyboard event according to search/filter modal precedence.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<CatalogAction> {
        if matches!(key.kind, KeyEventKind::Release) {
            return None;
        }
        match self.input_mode {
            InputMode::Search => self.handle_search_key(key),
            InputMode::Filter => self.handle_filter_key(key),
            InputMode::Navigate => self.handle_navigation_key(key),
        }
    }

    pub fn ensure_selection_visible(&mut self, rows: usize) {
        self.normalize_selection(None);
        if rows == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll.saturating_add(rows) {
            self.scroll = self.selected.saturating_add(1).saturating_sub(rows);
        }
    }

    fn handle_navigation_key(&mut self, key: KeyEvent) -> Option<CatalogAction> {
        match key.code {
            KeyCode::Esc => return Some(CatalogAction::Close),
            KeyCode::Char('?') => return Some(CatalogAction::Help),
            KeyCode::Char('/') => self.input_mode = InputMode::Search,
            KeyCode::Char('f') | KeyCode::Tab | KeyCode::BackTab => {
                self.input_mode = InputMode::Filter;
            }
            KeyCode::Char('c') => self.clear_all(),
            KeyCode::Char('1'..='5') if self.is_guided() => {
                self.guided_sweep = match key.code {
                    KeyCode::Char('1') => Some(GuidedSweep::CoverageGaps),
                    KeyCode::Char('2') => Some(GuidedSweep::Bathroom),
                    KeyCode::Char('3') => Some(GuidedSweep::QuickTasks),
                    KeyCode::Char('4') => Some(GuidedSweep::MonthlyMaintenance),
                    _ => Some(GuidedSweep::SafetyChecks),
                };
                self.selected = 0;
                self.scroll = 0;
            }
            KeyCode::Char('v') if self.is_guided() => {
                self.reveal_hidden = !self.reveal_hidden;
                self.normalize_selection(None);
            }
            KeyCode::Char('x') if self.is_guided() => {
                return self
                    .selected_activity()
                    .map(|activity| CatalogAction::Dismiss(activity.id.clone()));
            }
            KeyCode::Char('R') if self.is_guided() => return Some(CatalogAction::ResetDismissals),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.matching_activities().len().saturating_sub(1),
            KeyCode::Char('p') => {
                self.preview_expanded = !self.preview_expanded;
            }
            KeyCode::Enter => {
                return self
                    .selected_activity()
                    .map(|activity| CatalogAction::Select(activity.id.clone()));
            }
            _ => {}
        }
        None
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> Option<CatalogAction> {
        let selected_id = self.selected_activity().map(|activity| activity.id.clone());
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.input_mode = InputMode::Navigate,
            KeyCode::Backspace => {
                self.search.pop();
                self.normalize_selection(selected_id.as_deref());
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.search.push(character);
                self.normalize_selection(selected_id.as_deref());
            }
            _ => {}
        }
        None
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> Option<CatalogAction> {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.input_mode = InputMode::Navigate,
            KeyCode::Tab => self.move_filter_focus(1),
            KeyCode::BackTab => self.move_filter_focus(-1),
            KeyCode::Left | KeyCode::Char('h' | 'k') | KeyCode::Up => {
                self.move_facet_cursor(-1);
            }
            KeyCode::Right | KeyCode::Char('l' | 'j') | KeyCode::Down => {
                self.move_facet_cursor(1);
            }
            KeyCode::Char(' ') => self.toggle_current_facet(),
            KeyCode::Char('c') => self.clear_all(),
            KeyCode::Char('?') => return Some(CatalogAction::Help),
            _ => {}
        }
        None
    }

    fn move_selection(&mut self, delta: isize) {
        let maximum = self.matching_activities().len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(maximum);
    }

    fn clear_all(&mut self) {
        let selected_id = self.selected_activity().map(|activity| activity.id.clone());
        self.search.clear();
        self.filters.clear();
        self.normalize_selection(selected_id.as_deref());
    }

    fn normalize_selection(&mut self, preferred_id: Option<&str>) {
        let (preferred_position, result_count) = {
            let activities = self.matching_activities();
            (
                preferred_id.and_then(|id| {
                    activities
                        .iter()
                        .position(|activity| activity.id.as_str() == id)
                }),
                activities.len(),
            )
        };
        if let Some(position) = preferred_position {
            self.selected = position;
        } else {
            self.selected = self.selected.min(result_count.saturating_sub(1));
        }
        self.scroll = self.scroll.min(self.selected);
    }

    fn move_filter_focus(&mut self, delta: isize) {
        let current = FilterFocus::ALL
            .iter()
            .position(|focus| *focus == self.filter_focus)
            .unwrap_or(0);
        let next = (isize::try_from(current).unwrap_or(0) + delta)
            .rem_euclid(isize::try_from(FilterFocus::ALL.len()).unwrap_or(1));
        self.filter_focus = FilterFocus::ALL[usize::try_from(next).unwrap_or(0)];
        self.facet_cursor = 0;
    }

    fn move_facet_cursor(&mut self, delta: isize) {
        let count = self.focused_option_count();
        if count == 0 {
            return;
        }
        let next = (isize::try_from(self.facet_cursor).unwrap_or(0) + delta)
            .rem_euclid(isize::try_from(count).unwrap_or(1));
        self.facet_cursor = usize::try_from(next).unwrap_or(0);
    }

    fn focused_option_count(&self) -> usize {
        let metadata = ActivityCatalog::facet_metadata();
        match self.filter_focus {
            FilterFocus::Area => metadata.areas.len(),
            FilterFocus::ActivityType => metadata.activity_types.len(),
            FilterFocus::Effort => metadata.efforts.len(),
            FilterFocus::Duration => metadata.duration_bands.len(),
            FilterFocus::Context => metadata.contexts.len(),
            FilterFocus::Cadence => metadata.cadence_kinds.len(),
        }
    }

    fn toggle_current_facet(&mut self) {
        let selected_id = self.selected_activity().map(|activity| activity.id.clone());
        let metadata = ActivityCatalog::facet_metadata();
        match self.filter_focus {
            FilterFocus::Area => {
                toggle_set(&mut self.filters.areas, metadata.areas[self.facet_cursor]);
            }
            FilterFocus::ActivityType => toggle_set(
                &mut self.filters.activity_types,
                metadata.activity_types[self.facet_cursor],
            ),
            FilterFocus::Effort => toggle_set(
                &mut self.filters.efforts,
                metadata.efforts[self.facet_cursor],
            ),
            FilterFocus::Duration => toggle_set(
                &mut self.filters.durations,
                metadata.duration_bands[self.facet_cursor],
            ),
            FilterFocus::Context => toggle_set(
                &mut self.filters.contexts,
                metadata.contexts[self.facet_cursor],
            ),
            FilterFocus::Cadence => toggle_set(
                &mut self.filters.cadences,
                metadata.cadence_kinds[self.facet_cursor],
            ),
        }
        self.normalize_selection(selected_id.as_deref());
    }

    fn focused_options(&self) -> Vec<(&'static str, bool)> {
        let metadata = ActivityCatalog::facet_metadata();
        match self.filter_focus {
            FilterFocus::Area => metadata
                .areas
                .iter()
                .map(|value| (area_label(*value), self.filters.areas.contains(value)))
                .collect(),
            FilterFocus::ActivityType => metadata
                .activity_types
                .iter()
                .map(|value| {
                    (
                        activity_type_label(*value),
                        self.filters.activity_types.contains(value),
                    )
                })
                .collect(),
            FilterFocus::Effort => metadata
                .efforts
                .iter()
                .map(|value| (effort_label(*value), self.filters.efforts.contains(value)))
                .collect(),
            FilterFocus::Duration => metadata
                .duration_bands
                .iter()
                .map(|value| {
                    (
                        duration_label(*value),
                        self.filters.durations.contains(value),
                    )
                })
                .collect(),
            FilterFocus::Context => metadata
                .contexts
                .iter()
                .map(|value| (context_label(*value), self.filters.contexts.contains(value)))
                .collect(),
            FilterFocus::Cadence => metadata
                .cadence_kinds
                .iter()
                .map(|value| {
                    (
                        cadence_kind_label(*value),
                        self.filters.cadences.contains(value),
                    )
                })
                .collect(),
        }
    }

    fn filter_summary(&self) -> String {
        let mut dimensions = Vec::new();
        push_selected_labels(
            &mut dimensions,
            "Area",
            ActivityCatalog::facet_metadata().areas,
            &self.filters.areas,
            area_label,
        );
        push_selected_labels(
            &mut dimensions,
            "Type",
            ActivityCatalog::facet_metadata().activity_types,
            &self.filters.activity_types,
            activity_type_label,
        );
        push_selected_labels(
            &mut dimensions,
            "Effort",
            ActivityCatalog::facet_metadata().efforts,
            &self.filters.efforts,
            effort_label,
        );
        push_selected_labels(
            &mut dimensions,
            "Duration",
            ActivityCatalog::facet_metadata().duration_bands,
            &self.filters.durations,
            duration_label,
        );
        push_selected_labels(
            &mut dimensions,
            "Context",
            ActivityCatalog::facet_metadata().contexts,
            &self.filters.contexts,
            context_label,
        );
        push_selected_labels(
            &mut dimensions,
            "Cadence",
            ActivityCatalog::facet_metadata().cadence_kinds,
            &self.filters.cadences,
            cadence_kind_label,
        );
        if dimensions.is_empty() {
            "None".to_owned()
        } else {
            dimensions.join("; ")
        }
    }

    fn match_reason(&self, activity: &ActivityTemplate) -> String {
        if let Some(reason) = self.guided_reason(activity) {
            return reason;
        }
        let mut reasons = Vec::new();
        if !self.search.is_empty() {
            reasons.push(format!("text ‘{}’", self.search));
        }
        push_match_reason(
            &mut reasons,
            "area",
            &activity.areas,
            &self.filters.areas,
            area_label,
        );
        push_match_reason(
            &mut reasons,
            "type",
            &activity.activity_types,
            &self.filters.activity_types,
            activity_type_label,
        );
        if self.filters.efforts.contains(&activity.effort) {
            reasons.push(format!("effort {}", effort_label(activity.effort)));
        }
        if self.filters.durations.contains(&activity.duration_band()) {
            reasons.push(format!(
                "duration {}",
                duration_label(activity.duration_band())
            ));
        }
        push_match_reason(
            &mut reasons,
            "context",
            &activity.contexts,
            &self.filters.contexts,
            context_label,
        );
        if let Some(cadence) = activity
            .suggested_cadence
            .as_ref()
            .filter(|cadence| self.filters.cadences.contains(&cadence.kind))
        {
            reasons.push(format!("cadence {}", cadence_kind_label(cadence.kind)));
        }
        if reasons.is_empty() {
            "Browse result — no filters required".to_owned()
        } else {
            format!("Matches: {}", reasons.join(", "))
        }
    }

    fn matches_guided_sweep(&self, activity: &ActivityTemplate) -> bool {
        match self.guided_sweep {
            None => true,
            Some(GuidedSweep::CoverageGaps) => {
                activity
                    .areas
                    .iter()
                    .any(|area| self.uncovered_areas.contains(area))
                    || activity
                        .activity_types
                        .iter()
                        .any(|kind| self.uncovered_activity_types.contains(kind))
            }
            Some(GuidedSweep::Bathroom) => activity.areas.contains(&Area::Bathroom),
            Some(GuidedSweep::QuickTasks) => activity.effort == Effort::Quick,
            Some(GuidedSweep::MonthlyMaintenance) => {
                activity.activity_types.contains(&ActivityType::Maintenance)
                    && activity
                        .suggested_cadence
                        .as_ref()
                        .is_some_and(|cadence| cadence.kind == CadenceKind::Months)
            }
            Some(GuidedSweep::SafetyChecks) => safety_template(&activity.id),
        }
    }

    fn guided_reason(&self, activity: &ActivityTemplate) -> Option<String> {
        Some(match self.guided_sweep? {
            GuidedSweep::CoverageGaps => {
                if let Some(area) = activity
                    .areas
                    .iter()
                    .find(|area| self.uncovered_areas.contains(area))
                {
                    format!(
                        "Suggested because no active planned chore covers {}.",
                        area_label(*area)
                    )
                } else if let Some(kind) = activity
                    .activity_types
                    .iter()
                    .find(|kind| self.uncovered_activity_types.contains(kind))
                {
                    format!(
                        "Suggested because no active planned chore covers {} work.",
                        activity_type_label(*kind)
                    )
                } else {
                    "No uncovered facet applies.".to_owned()
                }
            }
            GuidedSweep::Bathroom => {
                "Included in the user-selected Bathroom planning sweep.".to_owned()
            }
            GuidedSweep::QuickTasks => {
                "Included because the catalog marks this as a quick-effort task.".to_owned()
            }
            GuidedSweep::MonthlyMaintenance => {
                "Included because it is maintenance with a monthly cadence suggestion.".to_owned()
            }
            GuidedSweep::SafetyChecks => {
                "Included in the curated household safety-check sweep.".to_owned()
            }
        })
    }
}

fn safety_template(template_id: &str) -> bool {
    matches!(
        template_id,
        "whole_home.test_smoke_alarms"
            | "whole_home.test_carbon_monoxide_alarms"
            | "whole_home.inspect_fire_extinguisher"
            | "whole_home.inspect_visible_leaks"
            | "whole_home.inspect_first_aid_kit"
            | "whole_home.practice_fire_escape_plan"
            | "whole_home.inspect_alarm_age"
            | "outdoor.inspect_paths_railings"
    )
}

fn toggle_set<T: Copy + Eq + std::hash::Hash>(set: &mut HashSet<T>, value: T) {
    if !set.remove(&value) {
        set.insert(value);
    }
}

fn normalize_name(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn push_selected_labels<T: Copy + Eq + std::hash::Hash>(
    output: &mut Vec<String>,
    name: &str,
    values: &[T],
    selected: &HashSet<T>,
    label: fn(T) -> &'static str,
) {
    let labels = values
        .iter()
        .copied()
        .filter(|value| selected.contains(value))
        .map(label)
        .collect::<Vec<_>>();
    if !labels.is_empty() {
        output.push(format!("{name}={}", labels.join("|")));
    }
}

fn push_match_reason<T: Copy + Eq + std::hash::Hash>(
    output: &mut Vec<String>,
    name: &str,
    values: &[T],
    selected: &HashSet<T>,
    label: fn(T) -> &'static str,
) {
    let labels = values
        .iter()
        .copied()
        .filter(|value| selected.contains(value))
        .map(label)
        .collect::<Vec<_>>();
    if !labels.is_empty() {
        output.push(format!("{name} {}", labels.join(" or ")));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatalogLayout {
    Wide,
    Stacked,
    Compact,
    TooSmall,
}

impl CatalogLayout {
    const fn for_size(width: u16, height: u16) -> Self {
        if width >= 100 && height >= 20 {
            Self::Wide
        } else if width >= 70 && height >= 20 {
            Self::Stacked
        } else if width >= 40 && height >= 14 {
            Self::Compact
        } else {
            Self::TooSmall
        }
    }
}

/// Render the catalog browser and keep the selected result visible.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &mut CatalogBrowserState) {
    let layout = CatalogLayout::for_size(area.width, area.height);
    if layout == CatalogLayout::TooSmall {
        render_too_small(frame, area);
        return;
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(area);
    render_header(frame, header, state);
    match layout {
        CatalogLayout::Wide => {
            let [filters, results, preview] = Layout::horizontal([
                Constraint::Length(28),
                Constraint::Percentage(42),
                Constraint::Min(28),
            ])
            .areas(body);
            render_filter_editor(frame, filters, state);
            render_results(frame, results, state);
            render_preview(frame, preview, state);
        }
        CatalogLayout::Stacked => {
            let [results, preview] =
                Layout::vertical([Constraint::Percentage(55), Constraint::Min(5)]).areas(body);
            render_results(frame, results, state);
            render_preview(frame, preview, state);
        }
        CatalogLayout::Compact if state.preview_expanded => {
            render_preview(frame, body, state);
        }
        CatalogLayout::Compact => render_results(frame, body, state),
        CatalogLayout::TooSmall => unreachable!("handled before catalog layout"),
    }
    render_footer(frame, footer, state);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, state: &CatalogBrowserState) {
    let mode = match state.input_mode {
        InputMode::Navigate => "BROWSE",
        InputMode::Search => "SEARCH",
        InputMode::Filter => "FILTER",
    };
    let search = if state.search.is_empty() {
        "(none)"
    } else {
        state.search.as_str()
    };
    let options = state.focused_options();
    let current_option =
        options
            .get(state.facet_cursor)
            .map_or("—".to_owned(), |(label, selected)| {
                format!(
                    "{}[{}] {label}",
                    if state.is_filtering() { ">" } else { "" },
                    if *selected { 'x' } else { ' ' }
                )
            });
    let lines = if let Some(sweep) = state.guided_sweep {
        vec![
            Line::from(format!("[GUIDED {mode}] What might I be overlooking?")),
            Line::from(format!(
                "Sweep: {} · 1 Gaps  2 Bathroom  3 Quick  4 Monthly maintenance  5 Safety",
                sweep.label()
            )),
            Line::from(format!(
                "Query: {search} · {} dismissed · hidden items {}",
                state.dismissed_template_ids.len(),
                if state.reveal_hidden {
                    "shown"
                } else {
                    "excluded"
                }
            )),
        ]
    } else {
        vec![
            Line::from(format!("[{mode}] Query: {search}")),
            Line::from(format!(
                "Filters ({}): {}",
                state.filters.selected_count(),
                state.filter_summary()
            )),
            Line::from(format!(
                "{}: {}  ({}/{})",
                state.filter_focus.label(),
                current_option,
                state.facet_cursor.saturating_add(1),
                options.len()
            )),
        ]
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Activity Catalog "),
        ),
        area,
    );
}

fn render_filter_editor(frame: &mut Frame<'_>, area: Rect, state: &CatalogBrowserState) {
    let rows = usize::from(area.height.saturating_sub(3));
    let options = state.focused_options();
    let start = state
        .facet_cursor
        .saturating_add(1)
        .saturating_sub(rows.max(1));
    let mut lines = vec![
        Line::from("Tab facet · Space toggle"),
        Line::from(state.filter_summary()),
    ];
    lines.extend(options.iter().enumerate().skip(start).take(rows).map(
        |(index, (label, selected))| {
            let cursor = if state.is_filtering() && index == state.facet_cursor {
                '>'
            } else {
                ' '
            };
            let mark = if *selected { 'x' } else { ' ' };
            Line::styled(
                format!("{cursor}[{mark}] {label}"),
                if cursor == '>' {
                    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
                } else {
                    Style::default()
                },
            )
        },
    ));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", state.filter_focus.label())),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_results(frame: &mut Frame<'_>, area: Rect, state: &mut CatalogBrowserState) {
    let rows = usize::from(area.height.saturating_sub(2));
    state.ensure_selection_visible(rows);
    let activities = state.matching_activities();
    let hidden_above = state.scroll > 0;
    let hidden_below = state.scroll.saturating_add(rows) < activities.len();
    let indicators = match (hidden_above, hidden_below) {
        (true, true) => " ↑↓",
        (true, false) => " ↑",
        (false, true) => " ↓",
        (false, false) => "",
    };
    let title = format!(
        " Results {} / {}{} ",
        activities.len(),
        state.catalog.activities().count(),
        indicators
    );
    let lines = if activities.is_empty() && state.is_guided() {
        vec![
            Line::styled(
                "Nothing is being flagged in this sweep.",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::from("This is a planning aid, not a household score."),
            Line::from("Try another numbered sweep or press v to reveal hidden items."),
        ]
    } else if activities.is_empty() {
        vec![
            Line::styled(
                "No activities match.",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::from("Press c to clear search and filters."),
            Line::from("Press / to revise search or f to revise filters."),
        ]
    } else {
        activities
            .iter()
            .enumerate()
            .skip(state.scroll)
            .take(rows)
            .map(|(position, activity)| {
                result_line(
                    activity,
                    position == state.selected,
                    usize::from(area.width.saturating_sub(2)),
                    state.planning_status(&activity.id),
                    state.dismissed_template_ids.contains(&activity.id),
                )
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn result_line(
    activity: &ActivityTemplate,
    selected: bool,
    width: usize,
    planning: PlanningStatus,
    dismissed: bool,
) -> Line<'static> {
    let cursor = if selected { ">" } else { " " };
    let area = activity.areas.first().copied().map_or("—", area_label);
    let kind = activity
        .activity_types
        .first()
        .copied()
        .map_or("—", activity_type_label);
    let marker = if dismissed {
        " [dismissed]"
    } else {
        match planning {
            PlanningStatus::Planned => " [planned]",
            PlanningStatus::PossibleDuplicate => " [possible duplicate]",
            PlanningStatus::Unplanned => "",
        }
    };
    let text = format!(
        "{cursor} {}{marker} · {area} · {kind} · {} · {}",
        activity.name,
        effort_label(activity.effort),
        duration_label(activity.duration_band())
    );
    Line::from(Span::styled(
        truncate_with_ellipsis(&text, width),
        if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
        },
    ))
}

fn render_preview(frame: &mut Frame<'_>, area: Rect, state: &CatalogBrowserState) {
    let lines = state.selected_activity().map_or_else(
        || vec![Line::from("No activity selected.")],
        |activity| {
            vec![
                Line::styled(
                    activity.name.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Line::from(
                    activity
                        .description
                        .clone()
                        .unwrap_or_else(|| "No additional description.".to_owned()),
                ),
                Line::from(""),
                Line::from(format!(
                    "Area: {}",
                    join_labels(&activity.areas, area_label)
                )),
                Line::from(format!(
                    "Type: {}",
                    join_labels(&activity.activity_types, activity_type_label)
                )),
                Line::from(format!(
                    "Effort: {} · Duration: about {} min ({})",
                    effort_label(activity.effort),
                    activity.estimated_minutes,
                    duration_label(activity.duration_band())
                )),
                Line::from(format!(
                    "Context: {}",
                    if activity.contexts.is_empty() {
                        "None".to_owned()
                    } else {
                        join_labels(&activity.contexts, context_label)
                    }
                )),
                Line::from(format!("Suggested cadence: {}", cadence_label(activity))),
                Line::from(format!(
                    "Planning: {}",
                    if state.dismissed_template_ids.contains(&activity.id) {
                        "Dismissed as not relevant"
                    } else {
                        match state.planning_status(&activity.id) {
                            PlanningStatus::Planned => "Planned from this catalog template",
                            PlanningStatus::PossibleDuplicate => "Possible duplicate by chore name",
                            PlanningStatus::Unplanned => "Not planned",
                        }
                    }
                )),
                Line::from(""),
                Line::from(state.match_reason(activity)),
            ]
        },
    );
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .title(" Preview — suggestion only "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, state: &CatalogBrowserState) {
    let hint = if state.is_guided() && matches!(state.input_mode, InputMode::Navigate) {
        "1–5 Sweep  j/k Move  Enter Use  x Dismiss  v Show hidden  R Reset  ? Help  Esc Back"
    } else {
        match state.input_mode {
            InputMode::Navigate => {
                "j/k Move  Enter Use  p Preview  / Search  f/Tab Filters  c Clear  ? Help  Esc Back"
            }
            InputMode::Search => "Type to search  Backspace edit  Enter/Esc browse",
            InputMode::Filter => {
                "Tab facet  h/l or arrows option  Space toggle  c Clear  Enter/Esc browse  ? Help"
            }
        }
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(hint),
            Line::from(state.status.as_deref().unwrap_or(
                "Enter copies the suggestion into an editable form; only Save writes it.",
            )),
        ]),
        area,
    );
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Terminal too small for catalog",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::from("Required: 40x14"),
            Line::from(format!("Current: {}x{}", area.width, area.height)),
            Line::from("? Help  Esc Back"),
        ])
        .centered()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Activity Catalog "),
        ),
        area,
    );
}

fn join_labels<T: Copy>(values: &[T], label: fn(T) -> &'static str) -> String {
    values
        .iter()
        .copied()
        .map(label)
        .collect::<Vec<_>>()
        .join(", ")
}

const fn area_label(value: Area) -> &'static str {
    match value {
        Area::Bathroom => "Bathroom",
        Area::Kitchen => "Kitchen",
        Area::Bedroom => "Bedroom",
        Area::LivingSpace => "Living space",
        Area::Workspace => "Workspace",
        Area::Entrance => "Entrance",
        Area::Storage => "Storage",
        Area::Laundry => "Laundry",
        Area::Outdoor => "Outdoor",
        Area::WholeHome => "Whole home",
    }
}

const fn activity_type_label(value: ActivityType) -> &'static str {
    match value {
        ActivityType::Cleaning => "Cleaning",
        ActivityType::Laundry => "Laundry",
        ActivityType::Maintenance => "Maintenance",
        ActivityType::Inspection => "Inspection",
        ActivityType::Organization => "Organization",
        ActivityType::Replenishment => "Replenishment",
        ActivityType::Disposal => "Disposal",
        ActivityType::Care => "Care",
    }
}

const fn effort_label(value: Effort) -> &'static str {
    match value {
        Effort::Quick => "Quick",
        Effort::Medium => "Medium",
        Effort::Substantial => "Substantial",
    }
}

const fn duration_label(value: DurationBand) -> &'static str {
    match value {
        DurationBand::UnderTenMinutes => "<10 min",
        DurationBand::TenToThirtyMinutes => "10–30 min",
        DurationBand::OverThirtyMinutes => ">30 min",
    }
}

const fn context_label(value: ActivityContext) -> &'static str {
    match value {
        ActivityContext::Indoors => "Indoors",
        ActivityContext::Outdoors => "Outdoors",
        ActivityContext::Physical => "Physical",
        ActivityContext::Quiet => "Quiet",
        ActivityContext::NoPreparation => "No preparation",
        ActivityContext::Errand => "Errand",
    }
}

const fn cadence_kind_label(value: CadenceKind) -> &'static str {
    match value {
        CadenceKind::Days => "Daily",
        CadenceKind::Weeks => "Weekly",
        CadenceKind::Months => "Monthly",
        CadenceKind::Seasonal => "Seasonal",
    }
}

fn cadence_label(activity: &ActivityTemplate) -> String {
    let Some(cadence) = &activity.suggested_cadence else {
        return "None".to_owned();
    };
    match (cadence.kind, cadence.interval) {
        (CadenceKind::Seasonal, _) => "Seasonal".to_owned(),
        (CadenceKind::Days, Some(1)) => "Daily".to_owned(),
        (CadenceKind::Weeks, Some(1)) => "Weekly".to_owned(),
        (CadenceKind::Months, Some(1)) => "Monthly".to_owned(),
        (CadenceKind::Days, Some(interval)) => format!("Every {interval} days"),
        (CadenceKind::Weeks, Some(interval)) => format!("Every {interval} weeks"),
        (CadenceKind::Months, Some(interval)) => format!("Every {interval} months"),
        (_, None) => "None".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn state() -> CatalogBrowserState {
        CatalogBrowserState::new(ActivityCatalog::bundled().expect("catalog should load"))
    }

    fn render_text(width: u16, height: u16, state: &mut CatalogBrowserState) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .expect("catalog should render");
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
    fn browse_first_default_contains_the_complete_catalog() {
        let state = state();
        assert_eq!(state.matching_activities().len(), 148);
        assert_eq!(state.active_filter_count(), 0);
        assert!(state.search().is_empty());
    }

    #[test]
    fn planning_prefers_provenance_and_falls_back_to_normalized_name() {
        let mut state = state();
        let exact_id = "bathroom.scrub_shower";
        let duplicate = state
            .catalog
            .activities()
            .find(|activity| activity.id != exact_id)
            .expect("another template should exist")
            .clone();
        state.set_planning(&[
            CatalogPlanningChore {
                name: "A renamed chore".to_owned(),
                provenance: Some(crate::app::editor::TemplateProvenance {
                    template_id: exact_id.to_owned(),
                    schema_version: 1,
                    catalog_version: 1,
                    locale: "en".to_owned(),
                }),
            },
            CatalogPlanningChore {
                name: format!("  {}  ", duplicate.name.to_uppercase()),
                provenance: None,
            },
        ]);

        assert_eq!(state.planning_status(exact_id), PlanningStatus::Planned);
        assert_eq!(
            state.planning_status(&duplicate.id),
            PlanningStatus::PossibleDuplicate
        );
    }

    #[test]
    fn enter_selects_without_mutating_browser_context() {
        let mut state = state();
        state.handle_key(key(KeyCode::Char('/')));
        for character in "scrub".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }
        state.handle_key(key(KeyCode::Enter));
        let expected = state
            .selected_activity()
            .expect("search should select an activity")
            .id
            .clone();

        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            Some(CatalogAction::Select(expected))
        );
        assert_eq!(state.search(), "scrub");
    }

    #[test]
    fn guided_gaps_are_broad_then_shrink_with_transparent_coverage() {
        let catalog = ActivityCatalog::bundled().expect("catalog should load");
        let mut guided = CatalogBrowserState::guided(catalog);
        guided.set_planning(&[]);
        assert_eq!(guided.matching_activities().len(), 148);
        assert!(
            guided
                .selected_activity()
                .map(|activity| guided
                    .match_reason(activity)
                    .contains("no active planned chore"))
                .unwrap_or(false)
        );

        guided.set_planning(&[CatalogPlanningChore {
            name: "Renamed bathroom task".to_owned(),
            provenance: Some(crate::app::editor::TemplateProvenance {
                template_id: "bathroom.clean_basin".to_owned(),
                schema_version: 1,
                catalog_version: 2,
                locale: "en".to_owned(),
            }),
        }]);
        assert!(
            guided
                .matching_activities()
                .iter()
                .all(|activity| activity.id != "bathroom.clean_basin")
        );
        assert!(
            guided
                .matching_activities()
                .iter()
                .any(|activity| activity.id == "kitchen.clean_sink")
        );
    }

    #[test]
    fn guided_mode_excludes_planned_legacy_and_dismissed_items_until_revealed() {
        let catalog = ActivityCatalog::bundled().expect("catalog should load");
        let chores = catalog
            .activities()
            .map(|activity| CatalogPlanningChore {
                name: activity.name.clone(),
                provenance: Some(crate::app::editor::TemplateProvenance {
                    template_id: activity.id.clone(),
                    schema_version: 1,
                    catalog_version: 2,
                    locale: "en".to_owned(),
                }),
            })
            .collect::<Vec<_>>();
        let mut comprehensive = CatalogBrowserState::guided(catalog.clone());
        comprehensive.set_planning(&chores);
        assert!(comprehensive.matching_activities().is_empty());

        let mut guided = CatalogBrowserState::guided(catalog);
        guided.handle_key(key(KeyCode::Char('2')));
        guided.set_planning(&[CatalogPlanningChore {
            name: "CLEAN THE BATHROOM BASIN".to_owned(),
            provenance: None,
        }]);
        guided.set_dismissals(HashSet::from(["bathroom.clean_toilet".to_owned()]));
        let visible = guided
            .matching_activities()
            .iter()
            .map(|activity| activity.id.as_str())
            .collect::<Vec<_>>();
        assert!(!visible.contains(&"bathroom.clean_basin"));
        assert!(!visible.contains(&"bathroom.clean_toilet"));

        guided.handle_key(key(KeyCode::Char('v')));
        let revealed = guided
            .matching_activities()
            .iter()
            .map(|activity| activity.id.as_str())
            .collect::<Vec<_>>();
        assert!(revealed.contains(&"bathroom.clean_basin"));
        assert!(revealed.contains(&"bathroom.clean_toilet"));
    }

    #[test]
    fn text_search_is_incremental_case_insensitive_and_checks_descriptions() {
        let mut state = state();
        state.handle_key(key(KeyCode::Char('/')));
        for character in "QUALIFIED SERVICE".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }
        assert!(state.is_searching());
        assert!(
            state
                .matching_activities()
                .iter()
                .any(|activity| { activity.id == "laundry.clean_dryer_duct" })
        );
        state.handle_key(key(KeyCode::Enter));
        assert!(!state.is_searching());
    }

    #[test]
    fn filters_use_and_across_dimensions_and_or_within_one_dimension() {
        let mut state = state();
        state.handle_key(key(KeyCode::Char('f')));
        state.handle_key(key(KeyCode::Char(' '))); // Bathroom
        state.handle_key(key(KeyCode::Tab));
        state.handle_key(key(KeyCode::Char(' '))); // Cleaning
        state.handle_key(key(KeyCode::Tab));
        state.handle_key(key(KeyCode::Char(' '))); // Quick

        let results = state.matching_activities();
        assert!(
            results
                .iter()
                .any(|activity| { activity.id == "bathroom.wipe_fixtures" })
        );
        assert!(results.iter().all(|activity| {
            activity.areas.contains(&Area::Bathroom)
                && activity.activity_types.contains(&ActivityType::Cleaning)
                && activity.effort == Effort::Quick
        }));

        state.handle_key(key(KeyCode::BackTab));
        state.handle_key(key(KeyCode::Right));
        state.handle_key(key(KeyCode::Char(' '))); // Laundry OR Cleaning
        assert!(
            state
                .matching_activities()
                .iter()
                .any(|activity| { activity.id == "bathroom.replace_towels" })
        );
    }

    #[test]
    fn zero_results_recover_and_selection_remains_valid() {
        let mut state = state();
        state.handle_key(key(KeyCode::End));
        state.handle_key(key(KeyCode::Char('/')));
        for character in "no activity can contain this exact text".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }
        assert!(state.matching_activities().is_empty());
        assert!(state.selected_activity().is_none());
        state.handle_key(key(KeyCode::Enter));
        state.handle_key(key(KeyCode::Char('c')));
        assert_eq!(state.matching_activities().len(), 148);
        assert!(state.selected_activity().is_some());
        assert!(state.selected_position() < 148);
    }

    #[test]
    fn cancel_and_help_are_explicit_non_mutating_actions() {
        let mut state = state();
        assert_eq!(
            state.handle_key(key(KeyCode::Char('?'))),
            Some(CatalogAction::Help)
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Esc)),
            Some(CatalogAction::Close)
        );
        assert_eq!(state.matching_activities().len(), 148);
    }

    #[test]
    fn renderer_handles_boundaries_empty_results_and_scroll_markers() {
        for (width, height) in [(39, 13), (40, 14), (69, 20), (70, 20), (99, 20), (100, 20)] {
            let mut state = state();
            state.handle_key(key(KeyCode::End));
            let output = render_text(width, height, &mut state);
            if width == 39 {
                assert!(output.contains("Terminal too small for catalog"));
            } else {
                assert!(output.contains("Activity Catalog"));
                assert!(output.contains("Results 148 / 148"));
                assert!(output.contains("↑"));
            }
        }

        let mut empty = state();
        empty.handle_key(key(KeyCode::Char('/')));
        for character in "zzzz-no-result".chars() {
            empty.handle_key(key(KeyCode::Char(character)));
        }
        let output = render_text(70, 20, &mut empty);
        assert!(output.contains("No activities match"));
        assert!(output.contains("Press c to clear"));
    }
}
