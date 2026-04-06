use std::time::Duration;

use reqwest::Client;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::types::*;

const INITIAL_BACKOFF_MS: u64 = 1000;
const MAX_BACKOFF_MS: u64 = 30_000;

/// Error type for Heimdall client operations.
#[derive(Debug, thiserror::Error)]
pub enum HeimdallError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON deserialization failed: {0}")]
    Deserialize(String),
    #[error("Resource not found")]
    NotFound,
    #[error("Heimdall unreachable after retries")]
    Unavailable,
    #[error("Service unavailable (503)")]
    ServiceUnavailable,
    #[error("Operation cancelled")]
    Cancelled,
}

impl HeimdallError {
    /// Returns true if this error is retryable.
    ///
    /// Retryable: connection failures, timeouts, 5xx (except 503).
    /// NOT retryable: 4xx, 503, deserialization errors, not found.
    ///
    /// Bor does NOT retry 503 Service Unavailable — it returns immediately.
    fn is_retryable(&self) -> bool {
        match self {
            HeimdallError::Http(e) => {
                // Connection errors and timeouts are retryable
                if e.is_connect() || e.is_timeout() {
                    return true;
                }
                // 5xx server errors are retryable, EXCEPT 503
                if let Some(status) = e.status() {
                    if status.as_u16() == 503 {
                        return false;
                    }
                    return status.is_server_error();
                }
                // Other request errors (e.g., redirect) are retryable
                e.is_request()
            }
            HeimdallError::ServiceUnavailable | HeimdallError::Cancelled => false,
            HeimdallError::NotFound
            | HeimdallError::Deserialize(_)
            | HeimdallError::Unavailable => false,
        }
    }
}

/// HTTP client for the Heimdall REST API with retry logic.
///
/// Communicates with a Heimdall node to fetch spans, state sync events,
/// milestones, checkpoints, and status. Retries indefinitely on transient
/// failures with exponential backoff and jitter (matching Bor behavior —
/// Heimdall is essential for consensus).
pub struct HeimdallClient {
    base_url: String,
    http: Client,
    cancel_token: CancellationToken,
}

impl HeimdallClient {
    /// Creates a new client pointing at the given Heimdall base URL.
    ///
    /// Example: `HeimdallClient::new("http://localhost:1317", cancel_token)`
    pub fn new(base_url: &str, cancel_token: CancellationToken) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: Client::new(),
            cancel_token,
        }
    }

    /// GET /bor/spans/{id}
    pub async fn fetch_span(&self, span_id: u64) -> Result<Span, HeimdallError> {
        let url = format!("{}/bor/spans/{}", self.base_url, span_id);
        self.with_retry(|| self.get_data(&url, "span")).await
    }

    /// GET /bor/spans/{id} — single attempt, no retry.
    ///
    /// Used for pre-fetching future spans where a 500 means "doesn't exist yet"
    /// and should not block the caller with retries.
    pub async fn try_fetch_span(&self, span_id: u64) -> Result<Span, HeimdallError> {
        let url = format!("{}/bor/spans/{}", self.base_url, span_id);
        self.get_data(&url, "span").await
    }

    /// GET /bor/spans/latest
    pub async fn fetch_latest_span(&self) -> Result<Span, HeimdallError> {
        let url = format!("{}/bor/spans/latest", self.base_url);
        self.with_retry(|| self.get_data(&url, "span")).await
    }

    /// GET /clerk/time?from_id={}&to_time={RFC3339Nano}&pagination.limit={}
    ///
    /// `to_time` is a Unix timestamp (seconds) which is converted to
    /// RFC3339Nano format for the Heimdall API.
    ///
    /// Paginates automatically: keeps fetching pages until fewer than `limit`
    /// events are returned, matching Bor's behavior.
    pub async fn fetch_state_sync_events(
        &self,
        from_id: u64,
        to_time: u64,
        limit: u64,
    ) -> Result<Vec<EventRecord>, HeimdallError> {
        let to_time_rfc3339 = unix_to_rfc3339_nano(to_time);
        let mut all_events = Vec::new();
        let mut current_from_id = from_id;

        loop {
            let url = format!(
                "{}/clerk/time?from_id={}&to_time={}&pagination.limit={}",
                self.base_url, current_from_id, to_time_rfc3339, limit
            );
            let page = self.with_retry(|| self.get_event_list(&url)).await?;
            let page_len = page.len();

            if let Some(last) = page.last() {
                current_from_id = last.id + 1;
            }

            all_events.extend(page);

            if (page_len as u64) < limit {
                break;
            }
        }

        Ok(all_events)
    }

    /// GET /milestones/latest
    pub async fn fetch_latest_milestone(&self) -> Result<Milestone, HeimdallError> {
        let url = format!("{}/milestones/latest", self.base_url);
        self.with_retry(|| self.get_data(&url, "milestone")).await
    }

    /// GET /milestones/count
    pub async fn fetch_milestone_count(&self) -> Result<u64, HeimdallError> {
        let url = format!("{}/milestones/count", self.base_url);
        self.with_retry(|| self.get_count_flexible(&url)).await
    }

    /// GET /checkpoints/{number}
    pub async fn fetch_checkpoint(&self, number: u64) -> Result<Checkpoint, HeimdallError> {
        let url = format!("{}/checkpoints/{}", self.base_url, number);
        self.with_retry(|| self.get_data(&url, "checkpoint")).await
    }

    /// GET /checkpoints/latest
    pub async fn fetch_latest_checkpoint(&self) -> Result<Checkpoint, HeimdallError> {
        let url = format!("{}/checkpoints/latest", self.base_url);
        self.with_retry(|| self.get_data(&url, "checkpoint")).await
    }

    /// GET /checkpoints/count
    pub async fn fetch_checkpoint_count(&self) -> Result<u64, HeimdallError> {
        let url = format!("{}/checkpoints/count", self.base_url);
        self.with_retry(|| self.get_count_flexible(&url)).await
    }

    /// GET /status
    pub async fn fetch_status(&self) -> Result<HeimdallStatus, HeimdallError> {
        let url = format!("{}/status", self.base_url);
        self.with_retry(|| self.get_status(&url)).await
    }

    /// Execute a request function with retry logic.
    ///
    /// Retries indefinitely on transient errors (5xx except 503, connection,
    /// timeout) with exponential backoff and jitter. Bor retries indefinitely
    /// because Heimdall is essential for consensus. Logs every 5th retry to
    /// avoid spam.
    ///
    /// Returns `HeimdallError::Cancelled` if the cancellation token fires
    /// during a backoff sleep, allowing the node to shut down cleanly.
    async fn with_retry<T, F, Fut>(&self, f: F) -> Result<T, HeimdallError>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, HeimdallError>>,
    {
        let mut attempt: u32 = 0;

        loop {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if !e.is_retryable() {
                        return Err(e);
                    }

                    attempt = attempt.saturating_add(1);

                    // Cap the shift to avoid overflow (backoff maxes at MAX_BACKOFF_MS anyway)
                    let shift = attempt.min(20);
                    let backoff_ms = INITIAL_BACKOFF_MS.saturating_mul(1u64 << shift);
                    let backoff_ms = backoff_ms.min(MAX_BACKOFF_MS);
                    // Add jitter: 50% to 100% of backoff
                    let jitter = backoff_ms / 2 + (simple_random() % (backoff_ms / 2 + 1));
                    let delay = Duration::from_millis(jitter);

                    // Log every 5th retry to avoid spam
                    if attempt % 5 == 1 || attempt == 1 {
                        warn!(
                            "Heimdall request failed (attempt {attempt}): {e}, retrying in {delay:?}"
                        );
                    }

                    // Race the backoff sleep against the cancellation token
                    // so the node can shut down without waiting for the full delay.
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = self.cancel_token.cancelled() => {
                            warn!("Heimdall retry cancelled during shutdown");
                            return Err(HeimdallError::Cancelled);
                        }
                    }
                }
            }
        }
    }

    /// GET with flexible response parsing for both v1 and v2 Heimdall APIs.
    ///
    /// v1 wraps data as: `{"height": "0", "result": T}`
    /// v2 wraps data as: `{<v2_key>: T}` (e.g., `{"span": {...}}`)
    async fn get_data<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        v2_key: &str,
    ) -> Result<T, HeimdallError> {
        let body = self.get_body(url).await?;
        extract_from_body(&body, v2_key)
    }

    /// GET for event list — supports both v1 and v2 formats.
    ///
    /// v1: `{"result": [...]}` or `{"result": null}`
    /// v2: `{"event_records": [...]}` or `{"event_records": null}`
    async fn get_event_list(&self, url: &str) -> Result<Vec<EventRecord>, HeimdallError> {
        let body = self.get_body(url).await?;
        extract_events(&body)
    }

    /// GET for count endpoints — supports both v1 and v2 formats.
    ///
    /// v1: `{"result": {"count": "N"}}`
    /// v2 milestones: `{"count": "N"}`
    /// v2 checkpoints: `{"ack_count": "N"}`
    async fn get_count_flexible(&self, url: &str) -> Result<u64, HeimdallError> {
        let body = self.get_body(url).await?;
        extract_count(&body)
    }

    /// GET for status (same format in v1 and v2).
    async fn get_status(&self, url: &str) -> Result<HeimdallStatus, HeimdallError> {
        let body = self.get_body(url).await?;
        serde_json::from_str(&body).map_err(|e| HeimdallError::Deserialize(format!("{e}: {body}")))
    }

    /// Shared HTTP GET that handles status codes and returns the response body.
    async fn get_body(&self, url: &str) -> Result<String, HeimdallError> {
        let resp = self.http.get(url).send().await?;
        let status = resp.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(HeimdallError::NotFound);
        }
        if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
            return Err(HeimdallError::ServiceUnavailable);
        }
        if !status.is_success() {
            return Err(HeimdallError::Http(resp.error_for_status().unwrap_err()));
        }

        Ok(resp.text().await?)
    }
}

/// Extract data from a Heimdall response body, supporting both v1 and v2 formats.
///
/// Tries v2 key first (`{v2_key: T}`), then v1 (`{"result": T}`).
fn extract_from_body<T: serde::de::DeserializeOwned>(
    body: &str,
    v2_key: &str,
) -> Result<T, HeimdallError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| HeimdallError::Deserialize(format!("{e}")))?;

    // Try v2 format: {v2_key: T}
    if let Some(inner) = value.get(v2_key)
        && let Ok(result) = serde_json::from_value(inner.clone())
    {
        return Ok(result);
    }

    // Fall back to v1 format: {"result": T}
    if let Some(inner) = value.get("result") {
        return serde_json::from_value(inner.clone())
            .map_err(|e| HeimdallError::Deserialize(format!("{e}")));
    }

    Err(HeimdallError::Deserialize(format!(
        "no '{v2_key}' or 'result' key in response"
    )))
}

/// Extract event records from response body (v1 + v2).
///
/// v1: `{"result": [...]}` or `{"result": null}`
/// v2: `{"event_records": [...]}` or `{"event_records": null}`
fn extract_events(body: &str) -> Result<Vec<EventRecord>, HeimdallError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| HeimdallError::Deserialize(format!("{e}")))?;

    // Try v2: {"event_records": [...]}
    if let Some(inner) = value.get("event_records") {
        if inner.is_null() {
            return Ok(vec![]);
        }
        if let Ok(events) = serde_json::from_value(inner.clone()) {
            return Ok(events);
        }
    }

    // Fall back to v1: {"result": [...]}
    if let Some(inner) = value.get("result") {
        if inner.is_null() {
            return Ok(vec![]);
        }
        return serde_json::from_value(inner.clone())
            .map_err(|e| HeimdallError::Deserialize(format!("{e}")));
    }

    Err(HeimdallError::Deserialize(
        "no 'event_records' or 'result' key in response".to_string(),
    ))
}

/// Extract a count value from response body (v1 + v2).
///
/// v1: `{"result": {"count": "N"}}`
/// v2 milestones: `{"count": "N"}`
/// v2 checkpoints: `{"ack_count": "N"}`
fn extract_count(body: &str) -> Result<u64, HeimdallError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| HeimdallError::Deserialize(format!("{e}")))?;

    // Try v2 checkpoint count: {"ack_count": "N"}
    if let Some(v) = value.get("ack_count") {
        return parse_json_u64(v);
    }

    // Try v2 milestone count: {"count": "N"} (top-level)
    if let Some(v) = value.get("count") {
        // Distinguish from v1 where "count" is inside "result"
        if v.is_string() || v.is_number() {
            return parse_json_u64(v);
        }
    }

    // Fall back to v1: {"result": {"count": "N"}}
    if let Some(result) = value.get("result")
        && let Some(v) = result.get("count")
    {
        return parse_json_u64(v);
    }

    Err(HeimdallError::Deserialize(
        "no count found in response".to_string(),
    ))
}

/// Parse a JSON value (string or number) as u64.
fn parse_json_u64(value: &serde_json::Value) -> Result<u64, HeimdallError> {
    match value {
        serde_json::Value::String(s) => s
            .parse()
            .map_err(|e| HeimdallError::Deserialize(format!("invalid count: {e}"))),
        serde_json::Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| HeimdallError::Deserialize("count is not a u64".to_string())),
        _ => Err(HeimdallError::Deserialize(format!(
            "unexpected count type: {value}"
        ))),
    }
}

/// Simple pseudo-random number for jitter (no external dep needed).
fn simple_random() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
}

/// Converts a Unix timestamp (seconds) to RFC3339Nano format.
///
/// Example: `1705312200` → `"2024-01-15T10:30:00.000000000Z"`
///
/// Uses Howard Hinnant's civil_from_days algorithm.
fn unix_to_rfc3339_nano(unix_secs: u64) -> String {
    const SECS_PER_DAY: i64 = 86400;

    let s = unix_secs as i64;
    let mut days = s.div_euclid(SECS_PER_DAY);
    let rem_secs = s.rem_euclid(SECS_PER_DAY) as u64;

    let hours = rem_secs / 3600;
    let minutes = (rem_secs % 3600) / 60;
    let seconds = rem_secs % 60;

    // civil_from_days: convert day count (0 = 1970-01-01) to (year, month, day).
    days += 719468; // shift epoch from 1970-01-01 to 0000-03-01
    let era = (if days >= 0 { days } else { days - 146096 }) / 146097;
    let doe = (days - era * 146097) as u64; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.000000000Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_strips_trailing_slash() {
        let client = HeimdallClient::new("http://localhost:1317/", CancellationToken::new());
        assert_eq!(client.base_url, "http://localhost:1317");
    }

    #[test]
    fn parse_span_response() {
        let json = r#"{
            "height": "0",
            "result": {
                "id": "500",
                "start_block": "8000000",
                "end_block": "8006399",
                "selected_producers": [
                    {
                        "ID": "1",
                        "signer": "0x048cfedf907c4c9ddd11ff882380906e78e84bbe",
                        "voting_power": "10000",
                        "proposer_priority": "0"
                    },
                    {
                        "ID": "2",
                        "signer": "0x1efecb61a2f80aa34d3b9218b564a64d05946290",
                        "voting_power": "5000",
                        "proposer_priority": "-100"
                    }
                ],
                "validators": [
                    {
                        "ID": "1",
                        "signer": "0x048cfedf907c4c9ddd11ff882380906e78e84bbe",
                        "voting_power": "10000",
                        "proposer_priority": "0"
                    }
                ]
            }
        }"#;

        let resp: HeimdallResponse<Span> = serde_json::from_str(json).unwrap();
        let span = resp.result;
        assert_eq!(span.id, 500);
        assert_eq!(span.start_block, 8_000_000);
        assert_eq!(span.end_block, 8_006_399);
        assert_eq!(span.selected_producers.len(), 2);
        assert_eq!(span.validators.len(), 1);
    }

    #[test]
    fn parse_null_event_list() {
        let json = r#"{
            "height": "0",
            "result": null
        }"#;

        let resp: HeimdallResponse<Option<Vec<EventRecord>>> = serde_json::from_str(json).unwrap();
        assert!(resp.result.is_none());
    }

    #[test]
    fn parse_milestone_response() {
        let json = r#"{
            "height": "0",
            "result": {
                "ID": "42",
                "start_block": "60000000",
                "end_block": "60000100",
                "hash": "0xabcdef0000000000000000000000000000000000000000000000000000000000"
            }
        }"#;

        let resp: HeimdallResponse<Milestone> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.result.id, 42);
        assert_eq!(resp.result.start_block, 60_000_000);
        assert_eq!(resp.result.end_block, 60_000_100);
    }

    #[test]
    fn parse_checkpoint_response() {
        let json = r#"{
            "height": "0",
            "result": {
                "ID": "99",
                "start_block": "70000000",
                "end_block": "70000999",
                "root_hash": "0x1234560000000000000000000000000000000000000000000000000000000000"
            }
        }"#;

        let resp: HeimdallResponse<Checkpoint> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.result.id, 99);
        assert_eq!(resp.result.start_block, 70_000_000);
    }

    #[test]
    fn parse_count_response() {
        let json = r#"{
            "height": "0",
            "result": {
                "count": "150"
            }
        }"#;

        let resp: HeimdallResponse<CountResult> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.result.count, 150);
    }

    #[test]
    fn parse_status_response() {
        let json = r#"{
            "latest_block_height": "12345678",
            "catching_up": false
        }"#;

        let status: HeimdallStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.latest_block_height, "12345678");
        assert!(!status.catching_up);
    }

    #[test]
    fn error_retryability() {
        // NotFound is not retryable
        assert!(!HeimdallError::NotFound.is_retryable());
        // Deserialize is not retryable
        assert!(!HeimdallError::Deserialize("test".into()).is_retryable());
        // Unavailable is not retryable (already exhausted retries)
        assert!(!HeimdallError::Unavailable.is_retryable());
        // ServiceUnavailable (503) is NOT retryable per Bor behavior
        assert!(!HeimdallError::ServiceUnavailable.is_retryable());
        // Cancelled is not retryable
        assert!(!HeimdallError::Cancelled.is_retryable());
    }

    // ---- v2 extraction tests ----

    #[test]
    fn extract_span_v2_wrapper() {
        let json = r#"{
            "span": {
                "id": 100,
                "start_block": 640000,
                "end_block": 646399,
                "selected_producers": [],
                "validator_set": {
                    "validators": [
                        {
                            "val_id": 1,
                            "signer": "0x0000000000000000000000000000000000000001",
                            "voting_power": 100,
                            "proposer_priority": 0
                        }
                    ]
                }
            }
        }"#;

        let span: Span = extract_from_body(json, "span").expect("should parse v2 span");
        assert_eq!(span.id, 100);
        assert_eq!(span.start_block, 640_000);
        assert_eq!(span.validators.len(), 1);
        assert_eq!(span.validators[0].id, 1);
    }

    #[test]
    fn extract_span_v1_wrapper() {
        let json = r#"{
            "height": "0",
            "result": {
                "id": "100",
                "start_block": "640000",
                "end_block": "646399",
                "selected_producers": [],
                "validators": [
                    {
                        "ID": "1",
                        "signer": "0x0000000000000000000000000000000000000001",
                        "voting_power": "100",
                        "proposer_priority": "0"
                    }
                ]
            }
        }"#;

        let span: Span = extract_from_body(json, "span").expect("should parse v1 span");
        assert_eq!(span.id, 100);
        assert_eq!(span.validators.len(), 1);
    }

    #[test]
    fn extract_milestone_v2_wrapper() {
        let json = r#"{
            "milestone": {
                "id": 42,
                "start_block": 60000000,
                "end_block": 60000100,
                "hash": "0xabcdef0000000000000000000000000000000000000000000000000000000000"
            }
        }"#;

        let m: Milestone = extract_from_body(json, "milestone").expect("should parse v2 milestone");
        assert_eq!(m.id, 42);
    }

    #[test]
    fn extract_events_v2() {
        let json = r#"{
            "event_records": [
                {
                    "id": 1,
                    "contract": "0x0000000000000000000000000000000000001001",
                    "data": "0xaa",
                    "tx_hash": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "log_index": 0,
                    "bor_chain_id": "80002",
                    "record_time": "2023-01-01T00:00:00Z"
                }
            ]
        }"#;

        let events = extract_events(json).expect("should parse v2 events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, 1);
    }

    #[test]
    fn extract_events_v2_null() {
        let json = r#"{"event_records": null}"#;
        let events = extract_events(json).expect("should handle null events");
        assert!(events.is_empty());
    }

    #[test]
    fn extract_events_v1() {
        let json = r#"{
            "height": "0",
            "result": [
                {
                    "id": "1",
                    "contract": "0x0000000000000000000000000000000000001001",
                    "data": "0xaa",
                    "tx_hash": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "log_index": "0",
                    "bor_chain_id": "137",
                    "record_time": "2023-01-01T00:00:00Z"
                }
            ]
        }"#;

        let events = extract_events(json).expect("should parse v1 events");
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn extract_count_v2_ack_count() {
        let json = r#"{"ack_count": "150"}"#;
        assert_eq!(extract_count(json).unwrap(), 150);
    }

    #[test]
    fn extract_count_v2_milestone_count() {
        let json = r#"{"count": "42"}"#;
        assert_eq!(extract_count(json).unwrap(), 42);
    }

    #[test]
    fn extract_count_v1() {
        let json = r#"{"height": "0", "result": {"count": "99"}}"#;
        assert_eq!(extract_count(json).unwrap(), 99);
    }

    #[test]
    fn extract_count_numeric() {
        let json = r#"{"ack_count": 200}"#;
        assert_eq!(extract_count(json).unwrap(), 200);
    }

    #[test]
    fn unix_to_rfc3339_nano_known_values() {
        // Unix epoch
        assert_eq!(unix_to_rfc3339_nano(0), "1970-01-01T00:00:00.000000000Z");

        // 2024-01-15T09:50:00Z = 1705312200
        assert_eq!(
            unix_to_rfc3339_nano(1705312200),
            "2024-01-15T09:50:00.000000000Z"
        );

        // 2023-11-15T14:30:00Z = 1700058600
        assert_eq!(
            unix_to_rfc3339_nano(1700058600),
            "2023-11-15T14:30:00.000000000Z"
        );

        // Leap year: 2024-02-29T12:00:00Z = 1709208000
        assert_eq!(
            unix_to_rfc3339_nano(1709208000),
            "2024-02-29T12:00:00.000000000Z"
        );
    }
}
