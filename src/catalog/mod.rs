//! Versioned schema for curated activity-catalog content.

use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::Hash;

use serde::Deserialize;
use thiserror::Error;

/// Catalog schema understood by this release.
pub const SUPPORTED_SCHEMA_VERSION: u16 = 1;
const BUNDLED_CATALOG: &str = include_str!("../../catalog/en-v1.toml");

/// One localized catalog document.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogDocument {
    /// Version of the structural schema, independent of content releases.
    pub schema_version: u16,
    /// Monotonically increasing release of content within this schema.
    pub catalog_version: u32,
    /// BCP 47-style language tag for the user-facing text in this document.
    pub locale: String,
    /// Curated activity templates.
    pub activities: Vec<ActivityTemplate>,
}

impl CatalogDocument {
    /// Parse and validate one TOML catalog document.
    ///
    /// # Errors
    ///
    /// Returns a parse error for malformed TOML or a semantic validation
    /// error for content that violates the versioned catalog contract.
    pub fn parse(input: &str) -> Result<Self, CatalogError> {
        let document: Self = toml::from_str(input)?;
        document.validate()?;
        Ok(document)
    }

    /// Validate the document and every contained template.
    ///
    /// # Errors
    ///
    /// Returns the first schema or curation invariant that is violated.
    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(CatalogError::UnsupportedSchemaVersion(self.schema_version));
        }
        if self.catalog_version == 0 {
            return Err(CatalogError::CatalogVersion(self.catalog_version));
        }
        if !valid_locale(&self.locale) {
            return Err(CatalogError::Locale(self.locale.clone()));
        }
        if self.activities.is_empty() {
            return Err(CatalogError::Empty);
        }

        let mut identifiers = HashMap::new();
        for activity in &self.activities {
            validate_activity(activity)?;
            if identifiers
                .insert(activity.id.as_str(), activity.status)
                .is_some()
            {
                return Err(CatalogError::DuplicateIdentifier(activity.id.clone()));
            }
        }
        for activity in &self.activities {
            validate_replacement(activity, &identifiers)?;
        }
        Ok(())
    }
}

/// Immutable, deterministically ordered activity catalog used by application
/// and presentation layers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityCatalog {
    document: CatalogDocument,
}

impl ActivityCatalog {
    /// Load and validate the catalog compiled into the application binary.
    ///
    /// # Errors
    ///
    /// Returns a catalog validation error when bundled product content violates
    /// the supported schema. CI validates the same data before release.
    pub fn bundled() -> Result<Self, CatalogError> {
        Self::from_document(CatalogDocument::parse(BUNDLED_CATALOG)?)
    }

    /// Build an immutable catalog from a validated document.
    ///
    /// # Errors
    ///
    /// Returns the first schema or content invariant that is violated.
    pub fn from_document(mut document: CatalogDocument) -> Result<Self, CatalogError> {
        document.validate()?;
        document
            .activities
            .sort_by_cached_key(|activity| (activity.name.to_lowercase(), activity.id.clone()));
        Ok(Self { document })
    }

    /// Return active templates in deterministic, case-insensitive name order.
    pub fn activities(&self) -> impl Iterator<Item = &ActivityTemplate> {
        self.document
            .activities
            .iter()
            .filter(|activity| activity.status == TemplateStatus::Active)
    }

    /// Return active and deprecated templates in deterministic name order.
    pub fn all_templates(&self) -> impl ExactSizeIterator<Item = &ActivityTemplate> {
        self.document.activities.iter()
    }

    /// Find a template by its stable, locale-independent identifier.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&ActivityTemplate> {
        self.document
            .activities
            .iter()
            .find(|activity| activity.id == id)
    }

    /// Return version and locale information suitable for persisted provenance.
    #[must_use]
    pub const fn provenance(&self) -> CatalogProvenance<'_> {
        CatalogProvenance {
            schema_version: self.document.schema_version,
            catalog_version: self.document.catalog_version,
            locale: &self.document.locale,
        }
    }

    /// Return every supported controlled facet value in display order.
    #[must_use]
    pub const fn facet_metadata() -> FacetMetadata {
        FacetMetadata {
            areas: &Area::ALL,
            activity_types: &ActivityType::ALL,
            efforts: &Effort::ALL,
            contexts: &ActivityContext::ALL,
            cadence_kinds: &CadenceKind::ALL,
            duration_bands: &DurationBand::ALL,
        }
    }
}

/// Catalog identity copied alongside a chosen template ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatalogProvenance<'a> {
    /// Structural schema version.
    pub schema_version: u16,
    /// Content release within the schema.
    pub catalog_version: u32,
    /// Locale of the copied user-facing text.
    pub locale: &'a str,
}

/// Complete controlled-facet vocabulary for catalog browsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FacetMetadata {
    /// Areas in stable display order.
    pub areas: &'static [Area],
    /// Activity types in stable display order.
    pub activity_types: &'static [ActivityType],
    /// Effort levels in ascending order.
    pub efforts: &'static [Effort],
    /// Situational contexts in stable display order.
    pub contexts: &'static [ActivityContext],
    /// Suggested cadence families in stable display order.
    pub cadence_kinds: &'static [CadenceKind],
    /// Derived duration bands in ascending order.
    pub duration_bands: &'static [DurationBand],
}

/// A curated suggestion that can later be copied into a user-owned chore.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActivityTemplate {
    /// Stable, locale-independent catalog identity.
    pub id: String,
    /// Short verb-object display name.
    pub name: String,
    /// Optional explanation of what the activity includes.
    pub description: Option<String>,
    /// Physical or conceptual areas where the activity applies.
    pub areas: Vec<Area>,
    /// Kinds of work represented by the activity.
    pub activity_types: Vec<ActivityType>,
    /// Qualitative activation effort, distinct from elapsed duration.
    pub effort: Effort,
    /// Typical active time, from 1 through 480 minutes.
    pub estimated_minutes: u16,
    /// Situational facets that help users find suitable activities.
    #[serde(default)]
    pub contexts: Vec<ActivityContext>,
    /// Optional, non-binding cadence recommendation.
    pub suggested_cadence: Option<CadenceSuggestion>,
    /// Lifecycle state inside the bundled catalog.
    #[serde(default)]
    pub status: TemplateStatus,
    /// Stable replacement identifier for a deprecated template, when any.
    pub replaced_by: Option<String>,
}

/// Physical or conceptual activity area.
#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    /// Bathroom and toilet spaces.
    Bathroom,
    /// Kitchen and food-preparation spaces.
    Kitchen,
    /// Bedrooms.
    Bedroom,
    /// Living and dining spaces.
    LivingSpace,
    /// Desk, study, and home-office spaces.
    Workspace,
    /// Entrance and hallway spaces.
    Entrance,
    /// Cupboards, cellar, attic, and other storage.
    Storage,
    /// Laundry area and clothing-care space.
    Laundry,
    /// Garden, balcony, and other outdoor spaces.
    Outdoor,
    /// Bicycle, car, and other personal transport.
    Vehicle,
    /// Work affecting the entire home.
    WholeHome,
    /// Personal paperwork and household administration.
    PersonalAdmin,
}

impl Area {
    /// All area values in stable display order.
    pub const ALL: [Self; 12] = [
        Self::Bathroom,
        Self::Kitchen,
        Self::Bedroom,
        Self::LivingSpace,
        Self::Workspace,
        Self::Entrance,
        Self::Storage,
        Self::Laundry,
        Self::Outdoor,
        Self::Vehicle,
        Self::WholeHome,
        Self::PersonalAdmin,
    ];
}

/// Functional kind of work.
#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    /// Removing dirt or residue.
    Cleaning,
    /// Washing or caring for textiles.
    Laundry,
    /// Preventive or corrective upkeep.
    Maintenance,
    /// Checking condition, function, or safety.
    Inspection,
    /// Sorting, arranging, or reducing clutter.
    Organization,
    /// Restocking consumable supplies.
    Replenishment,
    /// Recycling, discarding, or taking waste out.
    Disposal,
    /// Caring for plants, animals, or household items.
    Care,
    /// Paperwork and household administration.
    Administration,
}

impl ActivityType {
    /// All activity-type values in stable display order.
    pub const ALL: [Self; 9] = [
        Self::Cleaning,
        Self::Laundry,
        Self::Maintenance,
        Self::Inspection,
        Self::Organization,
        Self::Replenishment,
        Self::Disposal,
        Self::Care,
        Self::Administration,
    ];
}

/// Qualitative effort needed to start and complete an activity.
#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    /// Low activation energy and limited setup.
    Quick,
    /// Moderate setup, attention, or physical effort.
    Medium,
    /// Significant setup, attention, or physical effort.
    Substantial,
}

impl Effort {
    /// All effort values from lowest to highest activation effort.
    pub const ALL: [Self; 3] = [Self::Quick, Self::Medium, Self::Substantial];
}

/// Situational filter for activity discovery.
#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityContext {
    /// Performed inside.
    Indoors,
    /// Performed outside.
    Outdoors,
    /// Requires meaningful physical effort.
    Physical,
    /// Suitable when noise should be limited.
    Quiet,
    /// Can be started without gathering special equipment.
    NoPreparation,
    /// Requires leaving home or combining with an errand.
    Errand,
}

impl ActivityContext {
    /// All situational context values in stable display order.
    pub const ALL: [Self; 6] = [
        Self::Indoors,
        Self::Outdoors,
        Self::Physical,
        Self::Quiet,
        Self::NoPreparation,
        Self::Errand,
    ];
}

/// Non-binding recurrence guidance attached to a template.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CadenceSuggestion {
    /// Human-scale cadence family.
    pub kind: CadenceKind,
    /// Interval for day/week/month cadences; absent for seasonal guidance.
    pub interval: Option<u16>,
}

/// Supported recommendation cadence families.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadenceKind {
    /// Every N days.
    Days,
    /// Every N weeks.
    Weeks,
    /// Every N months.
    Months,
    /// Climate- and household-dependent seasonal timing.
    Seasonal,
}

impl CadenceKind {
    /// All cadence families in stable display order.
    pub const ALL: [Self; 4] = [Self::Days, Self::Weeks, Self::Months, Self::Seasonal];
}

/// Derived duration range used by catalog filters.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum DurationBand {
    /// Less than ten minutes.
    UnderTenMinutes,
    /// From ten through thirty minutes.
    TenToThirtyMinutes,
    /// More than thirty minutes.
    OverThirtyMinutes,
}

impl DurationBand {
    /// All duration bands from shortest to longest.
    pub const ALL: [Self; 3] = [
        Self::UnderTenMinutes,
        Self::TenToThirtyMinutes,
        Self::OverThirtyMinutes,
    ];
}

impl ActivityTemplate {
    /// Classify the validated active-time estimate for filtering.
    #[must_use]
    pub const fn duration_band(&self) -> DurationBand {
        match self.estimated_minutes {
            0..10 => DurationBand::UnderTenMinutes,
            10..=30 => DurationBand::TenToThirtyMinutes,
            _ => DurationBand::OverThirtyMinutes,
        }
    }
}

/// Lifecycle state of a bundled activity template.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateStatus {
    /// Offered to users.
    #[default]
    Active,
    /// Retained only for stable identity and migration hints.
    Deprecated,
}

/// Catalog parsing or semantic validation failure.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// TOML syntax or typed decoding failed.
    #[error("catalog TOML is invalid: {0}")]
    Parse(#[from] toml::de::Error),
    /// The document uses an unsupported schema.
    #[error("unsupported catalog schema version {0}")]
    UnsupportedSchemaVersion(u16),
    /// The content release must be positive.
    #[error("invalid catalog content version {0}")]
    CatalogVersion(u32),
    /// The locale is empty or structurally invalid.
    #[error("invalid catalog locale {0:?}")]
    Locale(String),
    /// A catalog must contain at least one template.
    #[error("catalog contains no activities")]
    Empty,
    /// An activity identifier is malformed.
    #[error("activity identifier {0:?} is invalid")]
    Identifier(String),
    /// Two activities share an identifier.
    #[error("duplicate activity identifier {0:?}")]
    DuplicateIdentifier(String),
    /// A user-facing name violates length or whitespace rules.
    #[error("activity {id:?} has an invalid name")]
    Name {
        /// Affected stable identifier.
        id: String,
    },
    /// A description violates length or whitespace rules.
    #[error("activity {id:?} has an invalid description")]
    Description {
        /// Affected stable identifier.
        id: String,
    },
    /// A required multi-value facet is empty.
    #[error("activity {id:?} has no {facet}")]
    MissingFacet {
        /// Affected stable identifier.
        id: String,
        /// Facet name.
        facet: &'static str,
    },
    /// A template repeats a value within one facet.
    #[error("activity {id:?} repeats {value:?} in {facet}")]
    DuplicateFacet {
        /// Affected stable identifier.
        id: String,
        /// Facet name.
        facet: &'static str,
        /// Repeated controlled value.
        value: String,
    },
    /// Estimated active time is outside the supported range.
    #[error("activity {id:?} has invalid duration {minutes} minutes")]
    Duration {
        /// Affected stable identifier.
        id: String,
        /// Rejected duration.
        minutes: u16,
    },
    /// Cadence interval is missing, unexpected, or outside its range.
    #[error("activity {id:?} has an invalid cadence interval")]
    Cadence {
        /// Affected stable identifier.
        id: String,
    },
    /// An active template points at a replacement.
    #[error("active activity {id:?} cannot declare a replacement")]
    ActiveReplacement {
        /// Affected stable identifier.
        id: String,
    },
    /// A replacement reference is malformed, self-referential, or missing.
    #[error("activity {id:?} has invalid replacement {replacement:?}")]
    Replacement {
        /// Affected stable identifier.
        id: String,
        /// Rejected replacement identifier.
        replacement: String,
    },
}

fn validate_activity(activity: &ActivityTemplate) -> Result<(), CatalogError> {
    if !valid_identifier(&activity.id) {
        return Err(CatalogError::Identifier(activity.id.clone()));
    }
    if !valid_text(&activity.name, 80) {
        return Err(CatalogError::Name {
            id: activity.id.clone(),
        });
    }
    if activity
        .description
        .as_ref()
        .is_some_and(|description| !valid_text(description, 500))
    {
        return Err(CatalogError::Description {
            id: activity.id.clone(),
        });
    }
    if activity.areas.is_empty() {
        return Err(missing_facet(activity, "areas"));
    }
    if activity.activity_types.is_empty() {
        return Err(missing_facet(activity, "activity types"));
    }
    ensure_unique(&activity.id, "areas", &activity.areas)?;
    ensure_unique(&activity.id, "activity types", &activity.activity_types)?;
    ensure_unique(&activity.id, "contexts", &activity.contexts)?;
    if !(1..=480).contains(&activity.estimated_minutes) {
        return Err(CatalogError::Duration {
            id: activity.id.clone(),
            minutes: activity.estimated_minutes,
        });
    }
    if activity
        .suggested_cadence
        .as_ref()
        .is_some_and(|cadence| !valid_cadence(cadence))
    {
        return Err(CatalogError::Cadence {
            id: activity.id.clone(),
        });
    }
    if activity.status == TemplateStatus::Active && activity.replaced_by.is_some() {
        return Err(CatalogError::ActiveReplacement {
            id: activity.id.clone(),
        });
    }
    Ok(())
}

fn validate_replacement(
    activity: &ActivityTemplate,
    identifiers: &HashMap<&str, TemplateStatus>,
) -> Result<(), CatalogError> {
    let Some(replacement) = activity.replaced_by.as_deref() else {
        return Ok(());
    };
    if !valid_identifier(replacement)
        || replacement == activity.id.as_str()
        || identifiers.get(replacement) != Some(&TemplateStatus::Active)
    {
        return Err(CatalogError::Replacement {
            id: activity.id.clone(),
            replacement: replacement.to_owned(),
        });
    }
    Ok(())
}

fn missing_facet(activity: &ActivityTemplate, facet: &'static str) -> CatalogError {
    CatalogError::MissingFacet {
        id: activity.id.clone(),
        facet,
    }
}

fn ensure_unique<T>(id: &str, facet: &'static str, values: &[T]) -> Result<(), CatalogError>
where
    T: Copy + Debug + Eq + Hash,
{
    let mut unique = HashSet::new();
    for value in values {
        if !unique.insert(*value) {
            return Err(CatalogError::DuplicateFacet {
                id: id.to_owned(),
                facet,
                value: format!("{value:?}"),
            });
        }
    }
    Ok(())
}

fn valid_cadence(cadence: &CadenceSuggestion) -> bool {
    match cadence.kind {
        CadenceKind::Seasonal => cadence.interval.is_none(),
        CadenceKind::Days | CadenceKind::Weeks | CadenceKind::Months => cadence
            .interval
            .is_some_and(|interval| (1..=999).contains(&interval)),
    }
}

fn valid_text(value: &str, maximum: usize) -> bool {
    value == value.trim() && (1..=maximum).contains(&value.chars().count())
}

fn valid_identifier(identifier: &str) -> bool {
    if !(3..=80).contains(&identifier.len()) {
        return false;
    }
    let mut bytes = identifier.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase()) {
        return false;
    }
    let mut separator = false;
    for byte in bytes {
        let current_separator = matches!(byte, b'.' | b'_' | b'-');
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || current_separator)
            || (separator && current_separator)
        {
            return false;
        }
        separator = current_separator;
    }
    !separator
}

fn valid_locale(locale: &str) -> bool {
    let mut parts = locale.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    if !(2..=3).contains(&language.len()) || !language.bytes().all(|byte| byte.is_ascii_lowercase())
    {
        return false;
    }
    parts.all(|part| {
        (2..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        ActivityCatalog, ActivityContext, ActivityType, Area, CadenceKind, CatalogDocument,
        CatalogError, DurationBand, Effort, SUPPORTED_SCHEMA_VERSION, TemplateStatus,
    };

    const VALID: &str = include_str!("../../catalog/examples-v1.toml");

    #[test]
    fn representative_catalog_parses_and_exposes_controlled_facets() {
        let catalog = CatalogDocument::parse(VALID).expect("example catalog should be valid");
        let fixtures = &catalog.activities[0];

        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.locale, "en");
        assert_eq!(fixtures.id, "bathroom.wipe_fixtures");
        assert_eq!(fixtures.areas, vec![Area::Bathroom]);
        assert_eq!(fixtures.activity_types, vec![ActivityType::Cleaning]);
        assert_eq!(fixtures.effort, Effort::Quick);
        assert_eq!(
            fixtures
                .suggested_cadence
                .as_ref()
                .map(|cadence| cadence.kind),
            Some(CadenceKind::Weeks)
        );
    }

    #[test]
    fn typed_decoding_rejects_unknown_facets() {
        let invalid = VALID.replace("areas = [\"bathroom\"]", "areas = [\"spaceship\"]");
        assert!(matches!(
            CatalogDocument::parse(&invalid),
            Err(CatalogError::Parse(_))
        ));
    }

    #[test]
    fn validation_rejects_invalid_duration_and_cadence() {
        let invalid_duration = VALID.replacen("estimated_minutes = 5", "estimated_minutes = 0", 1);
        assert!(matches!(
            CatalogDocument::parse(&invalid_duration),
            Err(CatalogError::Duration { .. })
        ));

        let invalid_cadence = VALID.replacen("interval = 1", "interval = 0", 1);
        assert!(matches!(
            CatalogDocument::parse(&invalid_cadence),
            Err(CatalogError::Cadence { .. })
        ));
    }

    #[test]
    fn validation_rejects_duplicate_identifiers_and_facet_values() {
        let duplicate_id = VALID.replace(
            "id = \"bathroom.clean_drain\"",
            "id = \"bathroom.wipe_fixtures\"",
        );
        assert!(matches!(
            CatalogDocument::parse(&duplicate_id),
            Err(CatalogError::DuplicateIdentifier(_))
        ));

        let duplicate_facet = VALID.replacen(
            "areas = [\"bathroom\"]",
            "areas = [\"bathroom\", \"bathroom\"]",
            1,
        );
        assert!(matches!(
            CatalogDocument::parse(&duplicate_facet),
            Err(CatalogError::DuplicateFacet { .. })
        ));
    }

    #[test]
    fn deprecated_templates_may_point_to_a_stable_replacement() {
        let catalog = CatalogDocument::parse(VALID).expect("example catalog should be valid");
        let deprecated = catalog
            .activities
            .iter()
            .find(|activity| activity.status == TemplateStatus::Deprecated)
            .expect("fixture should contain a deprecated activity");

        assert_eq!(
            deprecated.replaced_by.as_deref(),
            Some("bathroom.wipe_fixtures")
        );
    }

    #[test]
    fn bundled_catalog_is_complete_valid_and_deterministic() {
        let catalog = ActivityCatalog::bundled().expect("bundled catalog should be valid");
        let activities = catalog.activities().collect::<Vec<_>>();

        assert_eq!(activities.len(), 120);
        assert_eq!(catalog.all_templates().len(), 120);
        assert!(activities.iter().all(|activity| {
            activity.status == TemplateStatus::Active
                && activity.replaced_by.is_none()
                && !activity.name.is_empty()
        }));
        assert!(activities.windows(2).all(|pair| {
            (pair[0].name.to_lowercase(), pair[0].id.as_str())
                <= (pair[1].name.to_lowercase(), pair[1].id.as_str())
        }));
    }

    #[test]
    fn bundled_catalog_covers_every_controlled_discovery_facet() {
        let catalog = ActivityCatalog::bundled().expect("bundled catalog should be valid");
        let areas = catalog
            .activities()
            .flat_map(|activity| activity.areas.iter().copied())
            .collect::<HashSet<_>>();
        let activity_types = catalog
            .activities()
            .flat_map(|activity| activity.activity_types.iter().copied())
            .collect::<HashSet<_>>();
        let efforts = catalog
            .activities()
            .map(|activity| activity.effort)
            .collect::<HashSet<_>>();
        let contexts = catalog
            .activities()
            .flat_map(|activity| activity.contexts.iter().copied())
            .collect::<HashSet<_>>();
        let cadence_kinds = catalog
            .activities()
            .filter_map(|activity| activity.suggested_cadence.as_ref())
            .map(|cadence| cadence.kind)
            .collect::<HashSet<_>>();
        let duration_bands = catalog
            .activities()
            .map(super::ActivityTemplate::duration_band)
            .collect::<HashSet<_>>();

        assert_eq!(areas, Area::ALL.into_iter().collect());
        assert_eq!(activity_types, ActivityType::ALL.into_iter().collect());
        assert_eq!(efforts, Effort::ALL.into_iter().collect());
        assert_eq!(contexts, ActivityContext::ALL.into_iter().collect());
        assert_eq!(cadence_kinds, CadenceKind::ALL.into_iter().collect());
        assert_eq!(duration_bands, DurationBand::ALL.into_iter().collect());
    }

    #[test]
    fn bundled_catalog_exposes_provenance_and_immutable_lookup() {
        let catalog = ActivityCatalog::bundled().expect("bundled catalog should be valid");
        let provenance = catalog.provenance();
        let original = catalog
            .find("bathroom.wipe_fixtures")
            .expect("known activity should exist");
        let mut detached = original.clone();
        detached.name = "Changed copy".to_owned();

        assert_eq!(provenance.schema_version, SUPPORTED_SCHEMA_VERSION);
        assert_eq!(provenance.catalog_version, 1);
        assert_eq!(provenance.locale, "en");
        assert_eq!(
            catalog
                .find("bathroom.wipe_fixtures")
                .map(|activity| activity.name.as_str()),
            Some("Wipe bathroom fixtures")
        );
    }

    #[test]
    fn facet_metadata_has_stable_complete_order() {
        let facets = ActivityCatalog::facet_metadata();

        assert_eq!(facets.areas, &Area::ALL);
        assert_eq!(facets.activity_types, &ActivityType::ALL);
        assert_eq!(facets.efforts, &Effort::ALL);
        assert_eq!(facets.contexts, &ActivityContext::ALL);
        assert_eq!(facets.cadence_kinds, &CadenceKind::ALL);
        assert_eq!(facets.duration_bands, &DurationBand::ALL);
    }
}
