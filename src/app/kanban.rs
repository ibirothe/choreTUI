//! Application-facing boundary for the optional kanbanTUI integration.

use std::{collections::BTreeMap, error::Error, fmt};

use serde::Serialize;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;

use crate::domain::{CalendarDate, Occurrence, OccurrenceId, OccurrenceState, Timestamp};

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

/// Stable application-level category for an adapter import failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KanbanGatewayFailure {
    /// Authentication is missing, invalid, or rejected.
    Authentication,
    /// The destination could not accept a request and no success was observed.
    DestinationUnavailable,
    /// A request may have reached the server, so an exact keyed retry is safe.
    UncertainOutcome,
    /// The retry key was previously used for different request bytes.
    IdempotencyConflict,
    /// The destination board rejected the task under its configured policy.
    PolicyViolation,
    /// The request was rejected for another stable client-side reason.
    Rejected,
}

/// Why one occurrence was deliberately excluded from an export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KanbanSkipReason {
    /// The occurrence was already completed.
    Completed,
    /// The occurrence was explicitly skipped.
    Skipped,
}

/// One task-local export failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KanbanTaskFailure {
    /// The occurrence could not be mapped to the transfer contract.
    Mapping(KanbanMappingError),
    /// The gateway rejected or could not confirm the import.
    Gateway(KanbanGatewayFailure),
}

/// Result of exporting one occurrence from the selected date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KanbanTaskExportOutcome {
    /// kanbanTUI committed a destination-board change.
    Changed,
    /// kanbanTUI accepted an exact replay without another mutation.
    Unchanged,
    /// No request was made for this ineligible occurrence.
    Skipped(KanbanSkipReason),
    /// This task failed without stopping later tasks.
    Failed(KanbanTaskFailure),
}

/// Structured result for one occurrence in source display order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KanbanTaskExportResult {
    /// Source occurrence identity.
    pub occurrence_id: OccurrenceId,
    /// Chore name snapshot for a later result view.
    pub name: String,
    /// Import, skip, or failure outcome.
    pub outcome: KanbanTaskExportOutcome,
}

/// Aggregate counters for one selected-day export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KanbanDayExportSummary {
    /// Tasks that changed the destination board.
    pub changed: usize,
    /// Accepted exact replays or semantic no-ops.
    pub unchanged: usize,
    /// Completed or skipped source occurrences.
    pub skipped: usize,
    /// Mapping or gateway failures.
    pub failed: usize,
}

/// Complete ordered report for one selected-day export attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KanbanDayExportReport {
    /// Calendar date selected by the user.
    pub date: CalendarDate,
    /// One result per occurrence on that date, in input/display order.
    pub tasks: Vec<KanbanTaskExportResult>,
}

impl KanbanDayExportReport {
    /// Calculate aggregate counters without discarding task-local results.
    #[must_use]
    pub fn summary(&self) -> KanbanDayExportSummary {
        self.tasks
            .iter()
            .fold(KanbanDayExportSummary::default(), |mut summary, task| {
                match task.outcome {
                    KanbanTaskExportOutcome::Changed => summary.changed += 1,
                    KanbanTaskExportOutcome::Unchanged => summary.unchanged += 1,
                    KanbanTaskExportOutcome::Skipped(_) => summary.skipped += 1,
                    KanbanTaskExportOutcome::Failed(_) => summary.failed += 1,
                }
                summary
            })
    }
}

/// Export one materialized calendar day sequentially in the supplied order.
///
/// The immutable slice is the caller's already-loaded board snapshot. Items
/// from other dates are ignored. A task-local mapping or gateway failure is
/// recorded and does not prevent later eligible occurrences from being sent.
#[must_use]
pub fn export_day_to_kanban<G: KanbanGateway>(
    gateway: &G,
    destination: &KanbanDestinationScope,
    date: CalendarDate,
    occurrences: &[Occurrence],
) -> KanbanDayExportReport {
    let tasks = occurrences
        .iter()
        .filter(|occurrence| occurrence.due_date() == date)
        .map(|occurrence| {
            let outcome = match occurrence.state() {
                OccurrenceState::Completed { .. } => {
                    KanbanTaskExportOutcome::Skipped(KanbanSkipReason::Completed)
                }
                OccurrenceState::Skipped => {
                    KanbanTaskExportOutcome::Skipped(KanbanSkipReason::Skipped)
                }
                OccurrenceState::Pending => match map_occurrence_to_kanban(occurrence, destination)
                {
                    Ok(request) => match gateway.import_task(&request) {
                        Ok(result) => match result.outcome {
                            KanbanImportOutcome::Changed => KanbanTaskExportOutcome::Changed,
                            KanbanImportOutcome::Unchanged => KanbanTaskExportOutcome::Unchanged,
                        },
                        Err(error) => KanbanTaskExportOutcome::Failed(KanbanTaskFailure::Gateway(
                            G::classify_import_error(&error),
                        )),
                    },
                    Err(error) => {
                        KanbanTaskExportOutcome::Failed(KanbanTaskFailure::Mapping(error))
                    }
                },
            };
            KanbanTaskExportResult {
                occurrence_id: occurrence.id(),
                name: occurrence.name().as_str().to_owned(),
                outcome,
            }
        })
        .collect();
    KanbanDayExportReport { date, tasks }
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

    /// Normalize an adapter error for application reporting and retry guidance.
    fn classify_import_error(error: &Self::Error) -> KanbanGatewayFailure;
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeSet,
    };

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
        occurrence_with_id(
            id,
            CalendarDate::new(2026, 9, 16).expect("date should be valid"),
            state,
            name,
            description,
        )
    }

    fn occurrence_on(
        due_date: CalendarDate,
        state: OccurrenceState,
        name: &str,
        description: Option<&str>,
    ) -> Occurrence {
        occurrence_with_id(OccurrenceId::new(), due_date, state, name, description)
    }

    fn occurrence_with_id(
        id: OccurrenceId,
        due_date: CalendarDate,
        state: OccurrenceState,
        name: &str,
        description: Option<&str>,
    ) -> Occurrence {
        Occurrence::restore(
            OccurrenceSeed {
                id,
                chore_id: ChoreId::new(),
                schedule_id: ScheduleId::new(),
                nominal_date: due_date,
                due_date,
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

    #[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
    enum FakeGatewayError {
        #[error("policy rejection")]
        Policy,
        #[error("idempotency conflict")]
        Conflict,
        #[error("outcome unknown")]
        Uncertain,
    }

    #[derive(Default)]
    struct FakeGateway {
        calls: RefCell<Vec<(String, String)>>,
        receipts: RefCell<BTreeMap<String, Vec<u8>>>,
        mutations: Cell<usize>,
        policy_names: BTreeSet<String>,
        conflict_names: BTreeSet<String>,
        uncertain_once_names: RefCell<BTreeSet<String>>,
    }

    impl FakeGateway {
        fn with_failures(policy_names: &[&str], conflict_names: &[&str]) -> Self {
            Self {
                policy_names: policy_names.iter().map(ToString::to_string).collect(),
                conflict_names: conflict_names.iter().map(ToString::to_string).collect(),
                ..Self::default()
            }
        }

        fn with_uncertain_once(name: &str) -> Self {
            Self {
                uncertain_once_names: RefCell::new(BTreeSet::from([name.to_owned()])),
                ..Self::default()
            }
        }

        fn request_name(request: &KanbanImportRequest) -> String {
            let payload: Value =
                serde_json::from_slice(request.payload()).expect("fake should receive valid JSON");
            payload["active"][0]["text"]
                .as_str()
                .expect("mapped task should contain text")
                .to_owned()
        }
    }

    impl KanbanGateway for FakeGateway {
        type Error = FakeGatewayError;

        fn health(&self) -> Result<(), Self::Error> {
            Ok(())
        }

        fn import_task(
            &self,
            request: &KanbanImportRequest,
        ) -> Result<KanbanImportResult, Self::Error> {
            let name = Self::request_name(request);
            self.calls
                .borrow_mut()
                .push((name.clone(), request.idempotency_key().to_owned()));
            if self.policy_names.contains(&name) {
                return Err(FakeGatewayError::Policy);
            }
            if self.conflict_names.contains(&name) {
                return Err(FakeGatewayError::Conflict);
            }

            if let Some(existing) = self.receipts.borrow().get(request.idempotency_key()) {
                return if existing == request.payload() {
                    Ok(KanbanImportResult {
                        outcome: KanbanImportOutcome::Unchanged,
                        id_mapping: BTreeMap::new(),
                    })
                } else {
                    Err(FakeGatewayError::Conflict)
                };
            }

            self.receipts.borrow_mut().insert(
                request.idempotency_key().to_owned(),
                request.payload().to_vec(),
            );
            self.mutations.set(self.mutations.get() + 1);
            if self.uncertain_once_names.borrow_mut().remove(&name) {
                return Err(FakeGatewayError::Uncertain);
            }
            Ok(KanbanImportResult {
                outcome: KanbanImportOutcome::Changed,
                id_mapping: BTreeMap::new(),
            })
        }

        fn classify_import_error(error: &Self::Error) -> KanbanGatewayFailure {
            match error {
                FakeGatewayError::Policy => KanbanGatewayFailure::PolicyViolation,
                FakeGatewayError::Conflict => KanbanGatewayFailure::IdempotencyConflict,
                FakeGatewayError::Uncertain => KanbanGatewayFailure::UncertainOutcome,
            }
        }
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
    fn empty_and_fully_ineligible_days_perform_no_imports() {
        let selected = CalendarDate::new(2026, 9, 19).expect("date should be valid");
        let gateway = FakeGateway::default();
        let occurrences = vec![
            occurrence_on(
                CalendarDate::new(2026, 9, 18).expect("date should be valid"),
                OccurrenceState::Pending,
                "Other day",
                None,
            ),
            occurrence_on(
                selected,
                OccurrenceState::Completed { at: timestamp(4) },
                "Completed",
                None,
            ),
            occurrence_on(selected, OccurrenceState::Skipped, "Skipped", None),
        ];

        let report = export_day_to_kanban(
            &gateway,
            &destination("127.0.0.1:8765"),
            selected,
            &occurrences,
        );

        assert!(gateway.calls.borrow().is_empty());
        assert_eq!(gateway.mutations.get(), 0);
        assert_eq!(
            report.summary(),
            KanbanDayExportSummary {
                changed: 0,
                unchanged: 0,
                skipped: 2,
                failed: 0,
            }
        );
        assert_eq!(
            report
                .tasks
                .iter()
                .map(|task| task.outcome)
                .collect::<Vec<_>>(),
            vec![
                KanbanTaskExportOutcome::Skipped(KanbanSkipReason::Completed),
                KanbanTaskExportOutcome::Skipped(KanbanSkipReason::Skipped),
            ]
        );

        let empty = export_day_to_kanban(
            &gateway,
            &destination("127.0.0.1:8765"),
            CalendarDate::new(2026, 9, 20).expect("date should be valid"),
            &occurrences,
        );
        assert!(empty.tasks.is_empty());
        assert_eq!(empty.summary(), KanbanDayExportSummary::default());
    }

    #[test]
    fn export_is_sequential_and_continues_after_task_local_failures() {
        let selected = CalendarDate::new(2026, 9, 19).expect("date should be valid");
        let gateway = FakeGateway::with_failures(&["Policy rejected"], &["Conflicted"]);
        let occurrences = vec![
            occurrence_on(selected, OccurrenceState::Pending, "First", None),
            occurrence_on(selected, OccurrenceState::Pending, "Policy rejected", None),
            occurrence_on(
                selected,
                OccurrenceState::Completed { at: timestamp(4) },
                "Completed",
                None,
            ),
            occurrence_on(selected, OccurrenceState::Pending, "After rejection", None),
            occurrence_on(selected, OccurrenceState::Pending, "Conflicted", None),
        ];
        let source_snapshot = occurrences.clone();

        let report = export_day_to_kanban(
            &gateway,
            &destination("127.0.0.1:8765"),
            selected,
            &occurrences,
        );

        assert_eq!(occurrences, source_snapshot);
        assert_eq!(
            gateway
                .calls
                .borrow()
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["First", "Policy rejected", "After rejection", "Conflicted"]
        );
        assert_eq!(
            report
                .tasks
                .iter()
                .map(|task| task.outcome)
                .collect::<Vec<_>>(),
            vec![
                KanbanTaskExportOutcome::Changed,
                KanbanTaskExportOutcome::Failed(KanbanTaskFailure::Gateway(
                    KanbanGatewayFailure::PolicyViolation,
                )),
                KanbanTaskExportOutcome::Skipped(KanbanSkipReason::Completed),
                KanbanTaskExportOutcome::Changed,
                KanbanTaskExportOutcome::Failed(KanbanTaskFailure::Gateway(
                    KanbanGatewayFailure::IdempotencyConflict,
                )),
            ]
        );
        assert_eq!(
            report.summary(),
            KanbanDayExportSummary {
                changed: 2,
                unchanged: 0,
                skipped: 1,
                failed: 2,
            }
        );
        assert_eq!(gateway.mutations.get(), 2);
    }

    #[test]
    fn exact_retry_resolves_an_uncertain_result_without_duplicate_mutation() {
        let selected = CalendarDate::new(2026, 9, 19).expect("date should be valid");
        let gateway = FakeGateway::with_uncertain_once("May have arrived");
        let occurrences = vec![occurrence_on(
            selected,
            OccurrenceState::Pending,
            "May have arrived",
            None,
        )];

        let first = export_day_to_kanban(
            &gateway,
            &destination("127.0.0.1:8765"),
            selected,
            &occurrences,
        );
        let retry = export_day_to_kanban(
            &gateway,
            &destination("127.0.0.1:8765"),
            selected,
            &occurrences,
        );

        assert_eq!(
            first.tasks[0].outcome,
            KanbanTaskExportOutcome::Failed(KanbanTaskFailure::Gateway(
                KanbanGatewayFailure::UncertainOutcome,
            ))
        );
        assert_eq!(retry.tasks[0].outcome, KanbanTaskExportOutcome::Unchanged);
        assert_eq!(gateway.mutations.get(), 1);
        let calls = gateway.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, calls[1].1);
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
