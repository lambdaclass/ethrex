//! `Eth-Execution-Version` header extractor for the fork-scoped engine REST
//! routes (`/payloads`, `/forkchoice`, `/bodies`).
//!
//! Per the latest spec (execution-apis #793, `refactor.md § Versioning model`)
//! the fork is selected by the `Eth-Execution-Version: <fork>` request header
//! rather than a URL path segment, keeping URLs stable across forks. The header
//! value maps to `ethrex_common::types::Fork`. A missing header, a value not in
//! the engine REST fork table (Paris..Amsterdam), or a non-ASCII value is
//! rejected with `400 /engine-api/errors/unsupported-fork` — this covers
//! pre-Merge forks (Frontier..London) and BPO forks that have no body schema of
//! their own (they ride the Osaka shapes).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use ethrex_common::types::Fork;

use crate::engine_rest::error::ProblemJson;

/// Canonical fork-selector request header (matches the spec and prysm #16901).
pub const EXECUTION_VERSION_HEADER: &str = "eth-execution-version";

/// Parse an `Eth-Execution-Version` value into a `Fork`. The accepted tokens are
/// the lowercase fork names the spec and CLs use.
pub fn parse_fork_segment(s: &str) -> Result<Fork, ProblemJson> {
    let fork = match s {
        "paris" => Fork::Paris,
        "shanghai" => Fork::Shanghai,
        "cancun" => Fork::Cancun,
        "prague" => Fork::Prague,
        "osaka" => Fork::Osaka,
        "amsterdam" => Fork::Amsterdam,
        _ => {
            return Err(ProblemJson::unsupported_fork(&format!(
                "unsupported Eth-Execution-Version: {s}"
            )));
        }
    };
    Ok(fork)
}

/// Axum extractor that reads and validates the `Eth-Execution-Version` header.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionVersion(pub Fork);

impl<S> FromRequestParts<S> for ExecutionVersion
where
    S: Send + Sync,
{
    type Rejection = ProblemJson;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let raw = parts
            .headers
            .get(EXECUTION_VERSION_HEADER)
            .ok_or_else(|| ProblemJson::unsupported_fork("missing Eth-Execution-Version header"))?;
        let value = raw.to_str().map_err(|_| {
            ProblemJson::unsupported_fork("Eth-Execution-Version header is not valid ASCII")
        })?;
        parse_fork_segment(value).map(ExecutionVersion)
    }
}
