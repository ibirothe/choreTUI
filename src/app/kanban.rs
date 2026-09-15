//! Application-facing boundary for the optional kanbanTUI integration.

use std::{collections::BTreeMap, error::Error, fmt};

use serde::Serialize;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;

use crate::domain::{Occurrence, OccurrenceState, Timestamp};

const TRANSFER_FORMAT: &str = "kanbanTUI-board";
const TRANSFER_VERSION: u8 = 1;
const DESTINATION_SCOPE_MAX_BYTES: usize = 64;
const IDEMPOTENCY_PREFIX: &str = "choretui-v1";

/// Stable, non-secret identity for one kanbanTUI destination.
///
/// Callers should use the canonical numeric socket address, without a token or
/// board path. Scoping retry identities to the destination prevents one local
/// server from replaying a receipt created for another server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KanbanDestinationScope(String);

impl KanbanDestinationScope {
    /// Validate a canonical destination identity.
    ///
    /// # Errors
    ///
    /// Returns [`KanbanMappingError::InvalidDestinationScope`] for an empty,
    /// oversized, whitespace-containing, or non-ASCII identity.
    pub fn new(value: impl Into<String>) -> Result<Self, KanbanMappingError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > DESTINATION_SCOPE_MAX_BYTES
            || value.bytes().any(|byte| !(33..=126).contains(&byte))
        {
            return Err(KanbanMappingError::InvalidDestinationScope);
        }
        Ok(Self(value))
    }

    /// Return the canonical non-secret destination identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A chore occurrence cannot be converted into a kanban task.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum KanbanMappingError {
    /// The destination identity cannot produce a valid bounded retry key.
    #[error("kanbanTUI destination identity is invalid")]
    InvalidDestinationScope,
    /// Completed occurrences do not belong in a new execution queue.
    #[error("completed chore occurrences cannot be exported")]
    CompletedOccurrence,
    /// Skipped occurrences do not belong in a new execution queue.
    #[error("skipped chore occurrences cannot be exported")]
    SkippedOccurrence,
    /// A persisted timestamp cannot be represented by the transfer format.
    #[error("chore occurrence timestamp cannot be represented for kanbanTUI")]
    InvalidTimestamp,
    /// Serialization of the fixed transfer model failed unexpectedly.
    #[error("kanbanTUI transfer payload could not be serialized")]
    Serialization,
}

/// Convert one eligible occurrence into a deterministic kanbanTUI v1 request.
///
/// The request contains exactly one TODO task. It deliberately omits the chore
/// description and internal chore/schedule identifiers. The full name is sent
/// unchanged so destination policy remains authoritative.
///
/// # Errors
///
/// Returns an error for completed/skipped occurrences or when their persisted
/// timestamps cannot be represented as RFC 3339.
pub fn map_occurrence_to_kanban(
    occurrence: &Occurrence,
    destination: &KanbanDestinationScope,
) -> Result<KanbanImportRequest, KanbanMappingError> {
    match occurrence.state() {
        OccurrenceState::Pending => {}
        OccurrenceState::Completed { .. } => {
            return Err(KanbanMappingError::CompletedOccurrence);
        }
        OccurrenceState::Skipped => return Err(KanbanMappingError::SkippedOccurrence),
    }

    let source_id = source_task_id(occurrence);
    let created_at = format_timestamp(occurrence.created_at())?;
    let modified_at = format_timestamp(occurrence.updated_at())?;
    let due_tag = format!("due-{}", occurrence.due_date());
    let envelope = TransferEnvelope {
        format: TRANSFER_FORMAT,
        version: TRANSFER_VERSION,
        active: [TransferTask {
            id: source_id,
            state: "todo",
            text: occurrence.name().as_str(),
            created_at: &created_at,
            modified_at: &modified_at,
            completed_at: None,
            position: 1,
            priority: None,
            tags: ["choretui", &due_tag],
        }],
        archived: [],
    };
    let payload = serde_json::to_vec(&envelope).map_err(|_| KanbanMappingError::Serialization)?;
    let idempotency_key = format!(
        "{IDEMPOTENCY_PREFIX}:{}:{}",
        destination.as_str(),
        occurrence.id()
    );
    Ok(KanbanImportRequest::new(payload, idempotency_key))
}

fn source_task_id(occurrence: &Occurrence) -> u64 {
    let (high, low) = occurrence.id().as_uuid().as_u64_pair();
    let folded = high ^ low;
    folded.max(1)
}

fn format_timestamp(timestamp: Timestamp) -> Result<String, KanbanMappingError> {
    timestamp
        .as_datetime()
        .format(&Rfc3339)
        .map_err(|_| KanbanMappingError::InvalidTimestamp)
}

#[derive(Serialize)]
struct TransferEnvelope<'a> {
    format: &'static str,
    version: u8,
    active: [TransferTask<'a>; 1],
    archived: [TransferTask<'a>; 0],
}

#[derive(Serialize)]
struct TransferTask<'a> {
    id: u64,
    state: &'static str,
    text: &'a str,
    created_at: &'a str,
    modified_at: &'a str,
    completed_at: Option<&'a str>,
    position: u8,
    priority: Option<&'a str>,
    tags: [&'a str; 2],
}

/// Opaque kanbanTUI import request.
///
/// The payload is intentionally omitted from `Debug` output because it contains
/// user-authored chore text.
#[derive(Clone, PartialEq, Eq)]
pub struct KanbanImportRequest {
    payload: Vec<u8>,
    idempotency_key: String,
}

impl KanbanImportRequest {
    /// Create a request from a serialized transfer envelope and retry identity.
    #[must_use]
    pub fn new(payload: Vec<u8>, idempotency_key: String) -> Self {
        Self {
            payload,
            idempotency_key,
        }
    }

    /// Return the serialized `kanbanTUI-board` transfer envelope.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Return the stable identity used for safe retries.
    #[must_use]
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
}

impl fmt::Debug for KanbanImportRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KanbanImportRequest")
            .field("payload_bytes", &self.payload.len())
            .field("has_idempotency_key", &!self.idempotency_key.is_empty())
            .finish()
    }
}

/// Semantic result returned by kanbanTUI for one import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KanbanImportOutcome {
    /// The destination board was changed.
    Changed,
    /// The request was accepted without another mutation.
    Unchanged,
}

/// Normalized result of one kanbanTUI merge request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KanbanImportResult {
    /// Whether the destination changed.
    pub outcome: KanbanImportOutcome,
    /// Source-to-destination ID remapping performed by kanbanTUI.
    pub id_mapping: BTreeMap<u64, u64>,
}

/// Application-facing port implemented by the local HTTP adapter.
pub trait KanbanGateway {
    /// Adapter-specific failure type.
    type Error: Error + Send + Sync + 'static;

    /// Check the configured API without reading or mutating its board.
    ///
    /// # Errors
    ///
    /// Returns a classified adapter error when the service is unavailable or
    /// its response violates the expected contract.
    fn health(&self) -> Result<(), Self::Error>;

    /// Merge one already serialized task into the selected kanbanTUI board.
    ///
    /// # Errors
    ///
    /// Returns a classified adapter error. Callers may safely retry with the
    /// same request and idempotency key after an uncertain transport result.
    fn import_task(&self, request: &KanbanImportRequest)
    -> Result<KanbanImportResult, Self::Error>;
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::*;
    use crate::domain::{
        CalendarDate, ChoreId, ChoreName, Description, OccurrenceId, OccurrenceSeed, ScheduleId,
    };

    fn timestamp(seconds: i64) -> Timestamp {
        Timestamp::from_unix_timestamp(seconds).expect("test timestamp should be valid")
    }

    fn occurrence(state: OccurrenceState, name: &str, description: Option<&str>) -> Occurrence {
        let id = OccurrenceId::from_uuid(
            Uuid::parse_str("01890f3e-e5c7-7cc0-98c4-dc0c0c07398f")
                .expect("fixed UUID should parse"),
        )
        .expect("fixed UUID should be version 7");
        Occurrence::restore(
            OccurrenceSeed {
                id,
                chore_id: ChoreId::new(),
                schedule_id: ScheduleId::new(),
                nominal_date: CalendarDate::new(2026, 9, 15).expect("date should be valid"),
                due_date: CalendarDate::new(2026, 9, 16).expect("date should be valid"),
                name: ChoreName::new(name).expect("name should be valid"),
                description: description.and_then(|value| {
                    Description::optional(value).expect("description should be valid")
                }),
                created_at: timestamp(1),
            },
            state,
            timestamp(2),
        )
    }

    fn destination(value: &str) -> KanbanDestinationScope {
        KanbanDestinationScope::new(value).expect("destination should be valid")
    }

    #[test]
    fn mapping_matches_the_one_task_transfer_contract() {
        let occurrence = occurrence(
            OccurrenceState::Pending,
            "Clean every reachable bathroom fixture without truncation",
            Some("private description must not cross the boundary"),
        );
        let request = map_occurrence_to_kanban(&occurrence, &destination("127.0.0.1:8765"))
            .expect("pending occurrence should map");
        let payload: Value =
            serde_json::from_slice(request.payload()).expect("payload should be JSON");
        let task = &payload["active"][0];

        assert_eq!(payload["format"], "kanbanTUI-board");
        assert_eq!(payload["version"], 1);
        assert_eq!(payload["archived"], json!([]));
        assert_eq!(payload["active"].as_array().map(Vec::len), Some(1));
        assert_eq!(task["id"], source_task_id(&occurrence));
        assert_eq!(task["state"], "todo");
        assert_eq!(
            task["text"],
            "Clean every reachable bathroom fixture without truncation"
        );
        assert_eq!(task["created_at"], "1970-01-01T00:00:01Z");
        assert_eq!(task["modified_at"], "1970-01-01T00:00:02Z");
        assert_eq!(task["completed_at"], Value::Null);
        assert_eq!(task["position"], 1);
        assert_eq!(task["priority"], Value::Null);
        assert_eq!(task["tags"], json!(["choretui", "due-2026-09-16"]));
        assert!(!String::from_utf8_lossy(request.payload()).contains("private description"));
        assert!(
            !task["text"]
                .as_str()
                .expect("task text should be a string")
                .contains(&occurrence.id().to_string())
        );
        assert!(
            !task["tags"]
                .as_array()
                .expect("tags should be an array")
                .iter()
                .any(|tag| tag.as_str() == Some(&occurrence.id().to_string()))
        );
    }

    #[test]
    fn retries_and_restarts_produce_byte_identical_requests() {
        let first = occurrence(OccurrenceState::Pending, "Wash windows", None);
        let restored = Occurrence::restore(
            OccurrenceSeed {
                id: first.id(),
                chore_id: first.chore_id(),
                schedule_id: first.schedule_id(),
                nominal_date: first.nominal_date(),
                due_date: first.due_date(),
                name: first.name().clone(),
                description: first.description().cloned(),
                created_at: first.created_at(),
            },
            first.state(),
            first.updated_at(),
        );
        let destination = destination("127.0.0.1:8765");

        let initial = map_occurrence_to_kanban(&first, &destination).expect("mapping should work");
        let retry = map_occurrence_to_kanban(&restored, &destination).expect("mapping should work");

        assert_eq!(initial.payload(), retry.payload());
        assert_eq!(initial.idempotency_key(), retry.idempotency_key());
        assert_eq!(
            initial.idempotency_key(),
            "choretui-v1:127.0.0.1:8765:01890f3e-e5c7-7cc0-98c4-dc0c0c07398f"
        );
    }

    #[test]
    fn destination_changes_retry_identity_without_changing_payload() {
        let occurrence = occurrence(OccurrenceState::Pending, "Water plants", None);
        let first = map_occurrence_to_kanban(&occurrence, &destination("127.0.0.1:8765"))
            .expect("mapping should work");
        let second = map_occurrence_to_kanban(&occurrence, &destination("127.0.0.1:9876"))
            .expect("mapping should work");

        assert_eq!(first.payload(), second.payload());
        assert_ne!(first.idempotency_key(), second.idempotency_key());
    }

    #[test]
    fn completed_and_skipped_occurrences_are_ineligible() {
        let cases = [
            (
                OccurrenceState::Completed { at: timestamp(3) },
                KanbanMappingError::CompletedOccurrence,
            ),
            (
                OccurrenceState::Skipped,
                KanbanMappingError::SkippedOccurrence,
            ),
        ];

        for (state, expected) in cases {
            assert_eq!(
                map_occurrence_to_kanban(
                    &occurrence(state, "Already handled", None),
                    &destination("127.0.0.1:8765"),
                ),
                Err(expected)
            );
        }
    }

    #[test]
    fn destination_scope_is_bounded_and_header_safe() {
        assert!(KanbanDestinationScope::new("127.0.0.1:8765").is_ok());
        for invalid in [
            "",
            "destination with spaces",
            "destination\nwith-header",
            "ü",
            &"x".repeat(DESTINATION_SCOPE_MAX_BYTES + 1),
        ] {
            assert_eq!(
                KanbanDestinationScope::new(invalid),
                Err(KanbanMappingError::InvalidDestinationScope)
            );
        }
    }
}
