//! Application-facing boundary for the optional kanbanTUI integration.

use std::{collections::BTreeMap, error::Error, fmt};

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
