//! Bounded client for kanbanTUI's authenticated loopback HTTP adapter.

use std::{
    collections::BTreeMap,
    env, fmt,
    io::{self, Read, Write},
    net::{SocketAddr, SocketAddrV4, TcpStream},
    str::FromStr,
    time::Duration,
};

use std::fmt::Write as _;

use serde::{Deserialize, Deserializer, de};
use thiserror::Error;

use crate::{
    app::kanban::{
        KanbanGateway, KanbanGatewayFailure, KanbanImportOutcome, KanbanImportRequest,
        KanbanImportResult,
    },
    config::KanbanConfig,
};

const IMPORT_PATH: &str = "/v1/board/import?mode=merge";
const HEALTH_PATH: &str = "/health";
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 128;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// A validated numeric IPv4 loopback HTTP endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopbackEndpoint(SocketAddrV4);

impl LoopbackEndpoint {
    /// Return the socket address used by the client.
    #[must_use]
    pub const fn socket_addr(self) -> SocketAddrV4 {
        self.0
    }
}

impl fmt::Display for LoopbackEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "http://{}", self.0)
    }
}

/// Invalid kanbanTUI endpoint configuration.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("endpoint must be http:// followed by a numeric IPv4 loopback address and nonzero port")]
pub struct EndpointError;

impl FromStr for LoopbackEndpoint {
    type Err = EndpointError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let authority = value.strip_prefix("http://").ok_or(EndpointError)?;
        if authority.is_empty()
            || authority.chars().any(|character| {
                matches!(character, '/' | '?' | '#' | '@') || character.is_whitespace()
            })
        {
            return Err(EndpointError);
        }
        let address = authority.parse::<SocketAddr>().map_err(|_| EndpointError)?;
        match address {
            SocketAddr::V4(address) if address.ip().is_loopback() && address.port() != 0 => {
                Ok(Self(address))
            }
            SocketAddr::V4(_) | SocketAddr::V6(_) => Err(EndpointError),
        }
    }
}

impl<'de> Deserialize<'de> for LoopbackEndpoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

/// A validated environment-variable name containing the bearer token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenEnvironmentVariable(String);

impl TokenEnvironmentVariable {
    /// Return the environment-variable name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for TokenEnvironmentVariable {
    fn default() -> Self {
        Self("KANBAN_TUI_API_TOKEN".to_owned())
    }
}

impl fmt::Display for TokenEnvironmentVariable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Invalid token environment-variable configuration.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("token_env must be an ASCII environment-variable name")]
pub struct TokenEnvironmentVariableError;

impl FromStr for TokenEnvironmentVariable {
    type Err = TokenEnvironmentVariableError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut characters = value.chars();
        let Some(first) = characters.next() else {
            return Err(TokenEnvironmentVariableError);
        };
        if !(first == '_' || first.is_ascii_alphabetic())
            || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
        {
            return Err(TokenEnvironmentVariableError);
        }
        Ok(Self(value.to_owned()))
    }
}

impl<'de> Deserialize<'de> for TokenEnvironmentVariable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

/// Classified, sanitized kanbanTUI client failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum KanbanClientError {
    /// The configured token environment variable is absent or non-Unicode.
    #[error("kanbanTUI API token environment variable `{variable}` is unavailable")]
    MissingToken {
        /// Configured environment-variable name, never its value.
        variable: String,
    },
    /// The token does not satisfy kanbanTUI's header contract.
    #[error("kanbanTUI API token must be nonempty printable ASCII without whitespace")]
    InvalidToken,
    /// The supplied retry key does not satisfy kanbanTUI's header contract.
    #[error("kanbanTUI idempotency key is invalid")]
    InvalidIdempotencyKey,
    /// The serialized request exceeds kanbanTUI's documented limit.
    #[error("kanbanTUI import payload exceeds 1 MiB")]
    RequestTooLarge,
    /// A connection could not be established.
    #[error("cannot connect to the configured kanbanTUI loopback endpoint")]
    ConnectionFailed,
    /// A bounded socket operation timed out.
    #[error("kanbanTUI request timed out")]
    Timeout,
    /// The connection failed after it was established.
    #[error("kanbanTUI connection ended before a valid response was received")]
    TransportFailure,
    /// The server response exceeded the local safety limit.
    #[error("kanbanTUI response exceeds the supported size")]
    ResponseTooLarge,
    /// The HTTP or JSON response violated the documented contract.
    #[error("kanbanTUI returned an invalid response")]
    MalformedResponse,
    /// The configured token was rejected.
    #[error("kanbanTUI rejected the configured API token")]
    Unauthorized,
    /// The retry identity belongs to a different request.
    #[error("kanbanTUI rejected the retry identity because it belongs to another request")]
    IdempotencyConflict,
    /// The task violates destination board policy.
    #[error("kanbanTUI rejected the task by policy ({rule})")]
    PolicyViolation {
        /// Stable policy rule returned by kanbanTUI.
        rule: String,
        /// Configured limit, when present.
        limit: Option<u64>,
        /// Actual value, when present.
        actual: Option<u64>,
        /// Source task ID, when present.
        task_id: Option<u64>,
    },
    /// kanbanTUI could not access its selected datastore.
    #[error("kanbanTUI board storage is temporarily unavailable")]
    StoreUnavailable,
    /// A bounded client-side request was rejected.
    #[error("kanbanTUI rejected the request ({code}, HTTP {status})")]
    Rejected {
        /// HTTP status.
        status: u16,
        /// Stable server error code.
        code: String,
    },
    /// The server reported an unexpected failure.
    #[error("kanbanTUI failed to process the request ({code}, HTTP {status})")]
    ServerFailure {
        /// HTTP status.
        status: u16,
        /// Stable server error code.
        code: String,
    },
}

/// Production implementation of the kanbanTUI gateway.
#[derive(Clone, Debug)]
pub struct KanbanClient {
    endpoint: LoopbackEndpoint,
    token_environment: TokenEnvironmentVariable,
    timeout: Duration,
}

impl KanbanClient {
    /// Build a client from validated application configuration.
    #[must_use]
    pub fn new(config: &KanbanConfig) -> Self {
        Self {
            endpoint: config.endpoint,
            token_environment: config.token_env.clone(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    fn token(&self) -> Result<String, KanbanClientError> {
        let token = env::var(self.token_environment.as_str()).map_err(|_| {
            KanbanClientError::MissingToken {
                variable: self.token_environment.as_str().to_owned(),
            }
        })?;
        validate_token(&token)?;
        Ok(token)
    }

    fn health_with_token(&self, token: &str) -> Result<(), KanbanClientError> {
        let response = self.send("GET", HEALTH_PATH, token, None, &[])?;
        if response.status != 200 {
            return Err(classify_error(response.status, &response.body)?);
        }
        let health: HealthResponse = serde_json::from_slice(&response.body)
            .map_err(|_| KanbanClientError::MalformedResponse)?;
        if health.status != "ok" {
            return Err(KanbanClientError::MalformedResponse);
        }
        Ok(())
    }

    fn import_with_token(
        &self,
        request: &KanbanImportRequest,
        token: &str,
    ) -> Result<KanbanImportResult, KanbanClientError> {
        validate_idempotency_key(request.idempotency_key())?;
        let response = self.send(
            "POST",
            IMPORT_PATH,
            token,
            Some(request.idempotency_key()),
            request.payload(),
        )?;
        if response.status != 200 {
            return Err(classify_error(response.status, &response.body)?);
        }
        let wire: ImportResponse = serde_json::from_slice(&response.body)
            .map_err(|_| KanbanClientError::MalformedResponse)?;
        if wire.mode != "merge" {
            return Err(KanbanClientError::MalformedResponse);
        }
        let outcome = match wire.outcome.as_str() {
            "changed" => KanbanImportOutcome::Changed,
            "unchanged" => KanbanImportOutcome::Unchanged,
            _ => return Err(KanbanClientError::MalformedResponse),
        };
        let mut id_mapping = BTreeMap::new();
        for (source, destination) in wire.id_mapping {
            let source = source
                .parse::<u64>()
                .map_err(|_| KanbanClientError::MalformedResponse)?;
            id_mapping.insert(source, destination);
        }
        Ok(KanbanImportResult {
            outcome,
            id_mapping,
        })
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        token: &str,
        idempotency_key: Option<&str>,
        body: &[u8],
    ) -> Result<HttpResponse, KanbanClientError> {
        validate_token(token)?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(KanbanClientError::RequestTooLarge);
        }
        let address = SocketAddr::V4(self.endpoint.socket_addr());
        let mut stream = TcpStream::connect_timeout(&address, self.timeout)
            .map_err(|error| map_io_error(&error, true))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.timeout)))
            .map_err(|error| map_io_error(&error, false))?;

        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.endpoint.socket_addr()
        );
        if method == "POST" {
            request.push_str("Content-Type: application/json\r\n");
            write!(&mut request, "Content-Length: {}\r\n", body.len())
                .expect("writing to a string should not fail");
        }
        if let Some(key) = idempotency_key {
            write!(&mut request, "Idempotency-Key: {key}\r\n")
                .expect("writing to a string should not fail");
        }
        request.push_str("\r\n");

        stream
            .write_all(request.as_bytes())
            .and_then(|()| stream.write_all(body))
            .and_then(|()| stream.flush())
            .map_err(|error| map_io_error(&error, false))?;

        let mut response = Vec::new();
        stream
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut response)
            .map_err(|error| map_io_error(&error, false))?;
        if response.is_empty() {
            return Err(KanbanClientError::TransportFailure);
        }
        if response.len() > MAX_RESPONSE_BYTES {
            return Err(KanbanClientError::ResponseTooLarge);
        }
        parse_http_response(&response)
    }
}

impl KanbanGateway for KanbanClient {
    type Error = KanbanClientError;

    fn health(&self) -> Result<(), Self::Error> {
        let token = self.token()?;
        self.health_with_token(&token)
    }

    fn import_task(
        &self,
        request: &KanbanImportRequest,
    ) -> Result<KanbanImportResult, Self::Error> {
        let token = self.token()?;
        self.import_with_token(request, &token)
    }

    fn classify_import_error(error: &Self::Error) -> KanbanGatewayFailure {
        match error {
            KanbanClientError::MissingToken { .. }
            | KanbanClientError::InvalidToken
            | KanbanClientError::Unauthorized => KanbanGatewayFailure::Authentication,
            KanbanClientError::ConnectionFailed | KanbanClientError::StoreUnavailable => {
                KanbanGatewayFailure::DestinationUnavailable
            }
            KanbanClientError::Timeout
            | KanbanClientError::TransportFailure
            | KanbanClientError::ResponseTooLarge
            | KanbanClientError::MalformedResponse
            | KanbanClientError::ServerFailure { .. } => KanbanGatewayFailure::UncertainOutcome,
            KanbanClientError::IdempotencyConflict => KanbanGatewayFailure::IdempotencyConflict,
            KanbanClientError::PolicyViolation { .. } => KanbanGatewayFailure::PolicyViolation,
            KanbanClientError::InvalidIdempotencyKey
            | KanbanClientError::RequestTooLarge
            | KanbanClientError::Rejected { .. } => KanbanGatewayFailure::Rejected,
        }
    }
}

fn validate_token(token: &str) -> Result<(), KanbanClientError> {
    if token.is_empty() || token.bytes().any(|byte| !(33..=126).contains(&byte)) {
        return Err(KanbanClientError::InvalidToken);
    }
    Ok(())
}

fn validate_idempotency_key(key: &str) -> Result<(), KanbanClientError> {
    if key.is_empty()
        || key.len() > MAX_IDEMPOTENCY_KEY_BYTES
        || key.bytes().any(|byte| !(33..=126).contains(&byte))
    {
        return Err(KanbanClientError::InvalidIdempotencyKey);
    }
    Ok(())
}

fn map_io_error(error: &io::Error, connecting: bool) -> KanbanClientError {
    if matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        KanbanClientError::Timeout
    } else if connecting {
        KanbanClientError::ConnectionFailed
    } else {
        KanbanClientError::TransportFailure
    }
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn parse_http_response(response: &[u8]) -> Result<HttpResponse, KanbanClientError> {
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(KanbanClientError::MalformedResponse)?;
    let header_bytes = &response[..separator];
    let body = &response[separator + 4..];
    let headers =
        std::str::from_utf8(header_bytes).map_err(|_| KanbanClientError::MalformedResponse)?;
    let mut lines = headers.split("\r\n");
    let status_line = lines.next().ok_or(KanbanClientError::MalformedResponse)?;
    let mut status_parts = status_line.split_whitespace();
    let version = status_parts
        .next()
        .ok_or(KanbanClientError::MalformedResponse)?;
    let status = status_parts
        .next()
        .ok_or(KanbanClientError::MalformedResponse)?
        .parse::<u16>()
        .map_err(|_| KanbanClientError::MalformedResponse)?;
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") || !(100..=599).contains(&status) {
        return Err(KanbanClientError::MalformedResponse);
    }

    let mut content_length = None;
    let mut json_content_type = false;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or(KanbanClientError::MalformedResponse)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(KanbanClientError::MalformedResponse);
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(KanbanClientError::MalformedResponse);
            }
            content_length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| KanbanClientError::MalformedResponse)?,
            );
        }
        if name.eq_ignore_ascii_case("content-type") {
            json_content_type = value
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"));
        }
    }
    if !json_content_type || content_length.is_some_and(|length| length != body.len()) {
        return Err(KanbanClientError::MalformedResponse);
    }
    Ok(HttpResponse {
        status,
        body: body.to_vec(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HealthResponse {
    status: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportResponse {
    outcome: String,
    mode: String,
    id_mapping: BTreeMap<String, u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorBody {
    code: String,
    #[serde(default, rename = "message")]
    _message: Option<String>,
    #[serde(default)]
    rule: Option<String>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    actual: Option<u64>,
    #[serde(default)]
    task_id: Option<u64>,
    #[serde(default, rename = "request_id")]
    _request_id: Option<String>,
}

fn classify_error(status: u16, body: &[u8]) -> Result<KanbanClientError, KanbanClientError> {
    let envelope: ErrorEnvelope =
        serde_json::from_slice(body).map_err(|_| KanbanClientError::MalformedResponse)?;
    let error = envelope.error;
    if error.code.is_empty() {
        return Err(KanbanClientError::MalformedResponse);
    }
    Ok(match (status, error.code.as_str()) {
        (401, "unauthorized") => KanbanClientError::Unauthorized,
        (409, "idempotency_conflict") => KanbanClientError::IdempotencyConflict,
        (422, "policy_violation") => KanbanClientError::PolicyViolation {
            rule: error.rule.unwrap_or_else(|| "policy_violation".to_owned()),
            limit: error.limit,
            actual: error.actual,
            task_id: error.task_id,
        },
        (503, "store_unavailable") => KanbanClientError::StoreUnavailable,
        (400..=499, _) => KanbanClientError::Rejected {
            status,
            code: error.code,
        },
        _ => KanbanClientError::ServerFailure {
            status,
            code: error.code,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        io::{Read, Write},
        net::{Ipv4Addr, TcpListener},
        sync::mpsc,
        thread,
    };

    use super::*;
    use crate::{
        app::kanban::{KanbanDestinationScope, KanbanTaskExportOutcome, export_day_to_kanban},
        domain::{
            CalendarDate, ChoreId, ChoreName, Occurrence, OccurrenceId, OccurrenceSeed,
            OccurrenceState, ScheduleId, Timestamp,
        },
    };

    fn client_for(address: SocketAddrV4) -> KanbanClient {
        KanbanClient {
            endpoint: LoopbackEndpoint(address),
            token_environment: TokenEnvironmentVariable::default(),
            timeout: Duration::from_millis(500),
        }
    }

    fn serve_once(response: &'static [u8]) -> (SocketAddrV4, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("listener should bind");
        let address = match listener.local_addr().expect("address should resolve") {
            SocketAddr::V4(address) => address,
            SocketAddr::V6(_) => unreachable!("test listener is IPv4"),
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let request = read_request(&mut stream);
            let _ = sender.send(request);
            stream.write_all(response).expect("response should write");
        });
        (address, receiver)
    }

    fn read_request(stream: &mut TcpStream) -> Vec<u8> {
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout should configure");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let count = stream.read(&mut buffer).expect("request should read");
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            let Some(separator) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..separator]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("valid length"))
                    })
                })
                .unwrap_or(0);
            if request.len() >= separator + 4 + length {
                break;
            }
        }
        request
    }

    fn json_response(status: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[test]
    fn endpoint_accepts_only_numeric_ipv4_loopback_with_nonzero_port() {
        let endpoint: LoopbackEndpoint = "http://127.0.0.1:8765"
            .parse()
            .expect("endpoint should parse");
        assert_eq!(endpoint.to_string(), "http://127.0.0.1:8765");
        for invalid in [
            "https://127.0.0.1:8765",
            "http://localhost:8765",
            "http://192.168.1.2:8765",
            "http://127.0.0.1:0",
            "http://user@127.0.0.1:8765",
            "http://127.0.0.1:8765/path",
        ] {
            assert!(invalid.parse::<LoopbackEndpoint>().is_err(), "{invalid}");
        }
    }

    #[test]
    fn token_environment_names_are_bounded_to_portable_ascii_identifiers() {
        assert!(
            "KANBAN_TUI_API_TOKEN"
                .parse::<TokenEnvironmentVariable>()
                .is_ok()
        );
        for invalid in ["", "9TOKEN", "TOKEN-NAME", "TOKEN NAME", "TÖKEN"] {
            assert!(
                invalid.parse::<TokenEnvironmentVariable>().is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn request_debug_output_redacts_payload_and_retry_identity() {
        let request = KanbanImportRequest::new(
            b"{\"private\":\"Wash bedroom\"}".to_vec(),
            "secret-key".to_owned(),
        );
        let debug = format!("{request:?}");
        assert!(!debug.contains("Wash bedroom"));
        assert!(!debug.contains("secret-key"));
        assert!(debug.contains("payload_bytes"));
    }

    #[test]
    fn import_uses_exact_headers_path_and_normalizes_success() {
        let body = "{\"outcome\":\"changed\",\"mode\":\"merge\",\"id_mapping\":{\"1\":7}}";
        let response = json_response("200 OK", body);
        let response: &'static [u8] = Box::leak(response.into_boxed_slice());
        let (address, captured) = serve_once(response);
        let client = client_for(address);
        let request = KanbanImportRequest::new(
            b"{\"format\":\"kanbanTUI-board\"}".to_vec(),
            "retry-1".to_owned(),
        );

        let result = client
            .import_with_token(&request, "token-value")
            .expect("import should succeed");

        assert_eq!(result.outcome, KanbanImportOutcome::Changed);
        assert_eq!(result.id_mapping.get(&1), Some(&7));
        let sent = String::from_utf8(captured.recv().expect("request should arrive"))
            .expect("request should be UTF-8");
        assert!(sent.starts_with("POST /v1/board/import?mode=merge HTTP/1.1\r\n"));
        assert!(sent.contains("\r\nAuthorization: Bearer token-value\r\n"));
        assert!(sent.contains("\r\nContent-Type: application/json\r\n"));
        assert!(sent.contains("\r\nIdempotency-Key: retry-1\r\n"));
        assert!(sent.ends_with("{\"format\":\"kanbanTUI-board\"}"));
    }

    #[test]
    fn health_is_non_mutating_and_requires_the_expected_body() {
        let body = "{\"status\":\"ok\"}";
        let response = json_response("200 OK", body);
        let response: &'static [u8] = Box::leak(response.into_boxed_slice());
        let (address, captured) = serve_once(response);
        client_for(address)
            .health_with_token("token-value")
            .expect("health should succeed");
        let sent = String::from_utf8(captured.recv().expect("request should arrive"))
            .expect("request should be UTF-8");
        assert!(sent.starts_with("GET /health HTTP/1.1\r\n"));
        assert!(!sent.contains("Content-Type:"));
        assert!(!sent.contains("Idempotency-Key:"));
    }

    #[test]
    fn remote_errors_are_classified_without_messages_or_request_ids() {
        let policy = br#"{"error":{"code":"policy_violation","message":"private text","rule":"task_text_limit","limit":40,"actual":60,"task_id":1}}"#;
        assert_eq!(
            classify_error(422, policy),
            Ok(KanbanClientError::PolicyViolation {
                rule: "task_text_limit".to_owned(),
                limit: Some(40),
                actual: Some(60),
                task_id: Some(1),
            })
        );
        let server = br#"{"error":{"code":"internal_error","request_id":"abc"}}"#;
        assert_eq!(
            classify_error(500, server),
            Ok(KanbanClientError::ServerFailure {
                status: 500,
                code: "internal_error".to_owned(),
            })
        );
    }

    #[test]
    fn client_failures_have_stable_application_categories() {
        let cases = [
            (
                KanbanClientError::MissingToken {
                    variable: "TOKEN".to_owned(),
                },
                KanbanGatewayFailure::Authentication,
            ),
            (
                KanbanClientError::Unauthorized,
                KanbanGatewayFailure::Authentication,
            ),
            (
                KanbanClientError::ConnectionFailed,
                KanbanGatewayFailure::DestinationUnavailable,
            ),
            (
                KanbanClientError::StoreUnavailable,
                KanbanGatewayFailure::DestinationUnavailable,
            ),
            (
                KanbanClientError::Timeout,
                KanbanGatewayFailure::UncertainOutcome,
            ),
            (
                KanbanClientError::TransportFailure,
                KanbanGatewayFailure::UncertainOutcome,
            ),
            (
                KanbanClientError::IdempotencyConflict,
                KanbanGatewayFailure::IdempotencyConflict,
            ),
            (
                KanbanClientError::PolicyViolation {
                    rule: "task_text_limit".to_owned(),
                    limit: Some(40),
                    actual: Some(60),
                    task_id: Some(1),
                },
                KanbanGatewayFailure::PolicyViolation,
            ),
            (
                KanbanClientError::Rejected {
                    status: 400,
                    code: "invalid_import_format".to_owned(),
                },
                KanbanGatewayFailure::Rejected,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(
                <KanbanClient as KanbanGateway>::classify_import_error(&error),
                expected
            );
        }
    }

    #[test]
    fn invalid_headers_tokens_and_responses_fail_before_leaking_data() {
        assert_eq!(validate_token(""), Err(KanbanClientError::InvalidToken));
        assert_eq!(
            validate_token("token with space"),
            Err(KanbanClientError::InvalidToken)
        );
        assert_eq!(
            validate_idempotency_key("bad key"),
            Err(KanbanClientError::InvalidIdempotencyKey)
        );
        assert!(matches!(
            parse_http_response(b"not HTTP"),
            Err(KanbanClientError::MalformedResponse)
        ));
        assert_eq!(
            map_io_error(&io::Error::from(io::ErrorKind::TimedOut), false),
            KanbanClientError::Timeout
        );
    }

    #[test]
    fn transport_and_contract_failures_cover_recovery_categories() {
        let request = KanbanImportRequest::new(b"{}".to_vec(), "recovery-test".to_owned());
        let cases = [
            (
                "401 Unauthorized",
                r#"{"error":{"code":"unauthorized"}}"#,
                KanbanClientError::Unauthorized,
            ),
            (
                "409 Conflict",
                r#"{"error":{"code":"idempotency_conflict"}}"#,
                KanbanClientError::IdempotencyConflict,
            ),
            (
                "422 Unprocessable Entity",
                r#"{"error":{"code":"policy_violation","rule":"task_text_limit"}}"#,
                KanbanClientError::PolicyViolation {
                    rule: "task_text_limit".to_owned(),
                    limit: None,
                    actual: None,
                    task_id: None,
                },
            ),
            (
                "503 Service Unavailable",
                r#"{"error":{"code":"store_unavailable"}}"#,
                KanbanClientError::StoreUnavailable,
            ),
        ];
        for (status, body, expected) in cases {
            let response = Box::leak(json_response(status, body).into_boxed_slice());
            let (address, _) = serve_once(response);
            assert_eq!(
                client_for(address).import_with_token(&request, "token-value"),
                Err(expected)
            );
        }

        let malformed = Box::leak(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\n\r\n{"
                .to_vec()
                .into_boxed_slice(),
        );
        let (address, _) = serve_once(malformed);
        assert_eq!(
            client_for(address).import_with_token(&request, "token-value"),
            Err(KanbanClientError::MalformedResponse)
        );

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("listener should bind");
        let address = match listener.local_addr().expect("address should resolve") {
            SocketAddr::V4(address) => address,
            SocketAddr::V6(_) => unreachable!("test listener is IPv4"),
        };
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let _ = read_request(&mut stream);
        });
        assert_eq!(
            client_for(address).import_with_token(&request, "token-value"),
            Err(KanbanClientError::TransportFailure)
        );

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("listener should bind");
        let address = match listener.local_addr().expect("address should resolve") {
            SocketAddr::V4(address) => address,
            SocketAddr::V6(_) => unreachable!("test listener is IPv4"),
        };
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let _ = read_request(&mut stream);
            thread::sleep(Duration::from_millis(100));
        });
        let mut client = client_for(address);
        client.timeout = Duration::from_millis(20);
        assert_eq!(
            client.import_with_token(&request, "token-value"),
            Err(KanbanClientError::Timeout)
        );

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("listener should bind");
        let unavailable = match listener.local_addr().expect("address should resolve") {
            SocketAddr::V4(address) => address,
            SocketAddr::V6(_) => unreachable!("test listener is IPv4"),
        };
        drop(listener);
        assert_eq!(
            client_for(unavailable).import_with_token(&request, "token-value"),
            Err(KanbanClientError::ConnectionFailed)
        );
    }

    struct ContractGateway(KanbanClient);

    impl KanbanGateway for ContractGateway {
        type Error = KanbanClientError;

        fn health(&self) -> Result<(), Self::Error> {
            self.0.health_with_token("contract-token")
        }

        fn import_task(
            &self,
            request: &KanbanImportRequest,
        ) -> Result<KanbanImportResult, Self::Error> {
            self.0.import_with_token(request, "contract-token")
        }

        fn classify_import_error(error: &Self::Error) -> KanbanGatewayFailure {
            <KanbanClient as KanbanGateway>::classify_import_error(error)
        }
    }

    fn contract_occurrence(date: CalendarDate, name: &str, state: OccurrenceState) -> Occurrence {
        let created = Timestamp::from_unix_timestamp(1).expect("timestamp should be valid");
        Occurrence::restore(
            OccurrenceSeed {
                id: OccurrenceId::new(),
                chore_id: ChoreId::new(),
                schedule_id: ScheduleId::new(),
                nominal_date: date,
                due_date: date,
                name: ChoreName::new(name).expect("name should be valid"),
                description: None,
                created_at: created,
            },
            state,
            created,
        )
    }

    type ContractCapture = (Vec<String>, Vec<String>, usize);

    fn serve_contract() -> (SocketAddrV4, mpsc::Receiver<ContractCapture>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("listener should bind");
        let address = match listener.local_addr().expect("address should resolve") {
            SocketAddr::V4(address) => address,
            SocketAddr::V6(_) => unreachable!("test listener is IPv4"),
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || run_contract_server(&listener, &sender));
        (address, receiver)
    }

    fn run_contract_server(listener: &TcpListener, sender: &mpsc::Sender<ContractCapture>) {
        let mut accepted_keys = BTreeSet::new();
        let mut calls_by_name = HashMap::<String, usize>::new();
        let mut ordered_names = Vec::new();
        let mut ordered_keys = Vec::new();
        let mut mutations = 0;
        for _ in 0..6 {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let request = read_request(&mut stream);
            let separator = request
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .expect("request should have headers");
            let headers =
                String::from_utf8(request[..separator].to_vec()).expect("headers should be UTF-8");
            assert!(headers.starts_with("POST /v1/board/import?mode=merge HTTP/1.1\r\n"));
            assert!(headers.contains("\r\nAuthorization: Bearer contract-token\r\n"));
            assert!(headers.contains("\r\nContent-Type: application/json\r\n"));
            let key = headers
                .lines()
                .find_map(|line| line.strip_prefix("Idempotency-Key: "))
                .expect("idempotency key should be present")
                .to_owned();
            let body: serde_json::Value = serde_json::from_slice(&request[separator + 4..])
                .expect("payload should be valid JSON");
            assert_eq!(body["format"], "kanbanTUI-board");
            assert_eq!(body["version"], 1);
            assert_eq!(body["active"].as_array().map(Vec::len), Some(1));
            assert_eq!(body["archived"].as_array().map(Vec::len), Some(0));
            let name = body["active"][0]["text"]
                .as_str()
                .expect("task text should be present")
                .to_owned();
            ordered_names.push(name.clone());
            ordered_keys.push(key.clone());
            let call = calls_by_name.entry(name.clone()).or_default();
            *call += 1;

            if name == "Blocked by policy" {
                let response = json_response(
                    "422 Unprocessable Entity",
                    r#"{"error":{"code":"policy_violation","rule":"task_text_limit"}}"#,
                );
                stream.write_all(&response).expect("response should write");
                continue;
            }
            let changed = accepted_keys.insert(key);
            mutations += usize::from(changed);
            if name == "Uncertain delivery" && *call == 1 {
                continue;
            }
            let outcome = if changed { "changed" } else { "unchanged" };
            let response = json_response(
                "200 OK",
                &format!(r#"{{"outcome":"{outcome}","mode":"merge","id_mapping":{{}}}}"#),
            );
            stream.write_all(&response).expect("response should write");
        }
        sender
            .send((ordered_names, ordered_keys, mutations))
            .expect("capture should send");
    }

    #[test]
    fn selected_day_workflow_matches_v1_contract_and_retries_without_duplicates() {
        let (address, capture_receiver) = serve_contract();

        let date = CalendarDate::new(2026, 9, 19).expect("date should be valid");
        let completed_at = Timestamp::from_unix_timestamp(2).expect("timestamp should be valid");
        let occurrences = vec![
            contract_occurrence(date, "Laundry", OccurrenceState::Pending),
            contract_occurrence(date, "Blocked by policy", OccurrenceState::Pending),
            contract_occurrence(date, "Uncertain delivery", OccurrenceState::Pending),
            contract_occurrence(
                date,
                "Already done",
                OccurrenceState::Completed { at: completed_at },
            ),
        ];
        let gateway = ContractGateway(client_for(address));
        let scope = KanbanDestinationScope::new(address.to_string())
            .expect("destination scope should be valid");

        let first = export_day_to_kanban(&gateway, &scope, date, &occurrences);
        assert_eq!(first.summary().changed, 1);
        assert_eq!(first.summary().skipped, 1);
        assert_eq!(first.summary().failed, 2);
        assert!(matches!(
            first.tasks[1].outcome,
            KanbanTaskExportOutcome::Failed(_)
        ));
        assert!(matches!(
            first.tasks[2].outcome,
            KanbanTaskExportOutcome::Failed(_)
        ));

        let retry = export_day_to_kanban(&gateway, &scope, date, &occurrences);
        assert_eq!(retry.summary().changed, 0);
        assert_eq!(retry.summary().unchanged, 2);
        assert_eq!(retry.summary().skipped, 1);
        assert_eq!(retry.summary().failed, 1);

        let (names, keys, mutations) = capture_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("contract server should finish");
        assert_eq!(
            names,
            [
                "Laundry",
                "Blocked by policy",
                "Uncertain delivery",
                "Laundry",
                "Blocked by policy",
                "Uncertain delivery",
            ]
        );
        assert_eq!(keys[..3], keys[3..]);
        assert_eq!(mutations, 2, "retry must not duplicate accepted tasks");
    }

    #[test]
    fn missing_runtime_token_names_only_the_environment_variable() {
        let client = KanbanClient {
            endpoint: "http://127.0.0.1:8765"
                .parse()
                .expect("endpoint should parse"),
            token_environment: "CHORETUI_TEST_TOKEN_THAT_IS_NOT_SET_41"
                .parse()
                .expect("name should parse"),
            timeout: DEFAULT_TIMEOUT,
        };
        assert_eq!(
            client.token(),
            Err(KanbanClientError::MissingToken {
                variable: "CHORETUI_TEST_TOKEN_THAT_IS_NOT_SET_41".to_owned(),
            })
        );
    }
}
