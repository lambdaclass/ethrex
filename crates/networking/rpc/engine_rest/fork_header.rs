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
use ethrex_common::types::{ChainConfig, Fork};

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

/// True when `timestamp` falls inside the active range of the engine-REST fork
/// selected by `Eth-Execution-Version`.
///
/// Per execution-apis #793 the header selects a *container shape*, and the
/// per-fork catalogue defines one entry per shape (Paris..Amsterdam). Each value
/// therefore covers the span from its own activation up to the next fork that
/// introduces a new shape — it is NOT a single `ChainConfig::get_fork` value.
///
/// This distinction is load-bearing for the BPO forks: BPO1..BPO5 introduce no
/// engine containers of their own and ride the Osaka shapes, so `osaka` MUST
/// accept BPO-era timestamps. An exact `get_fork(ts) == fork` test would reject
/// every block after BPO1 activation (i.e. all of current mainnet, sepolia and
/// hoodi). Hegota likewise has no catalogue entry yet and rides Amsterdam.
///
/// Used by `GET /payloads/{id}` (mismatch → 400 unsupported-fork) and by the
/// `/bodies` handlers (mismatch → `available = false`, per the spec's rule that
/// a body is unavailable when its timestamp "falls outside the header fork's
/// active range"). Both callers share this predicate so they cannot drift.
pub fn fork_covers_timestamp(chain_config: &ChainConfig, fork: Fork, timestamp: u64) -> bool {
    match fork {
        Fork::Paris => !chain_config.is_shanghai_activated(timestamp),
        Fork::Shanghai => {
            chain_config.is_shanghai_activated(timestamp)
                && !chain_config.is_cancun_activated(timestamp)
        }
        Fork::Cancun => {
            chain_config.is_cancun_activated(timestamp)
                && !chain_config.is_prague_activated(timestamp)
        }
        Fork::Prague => {
            chain_config.is_prague_activated(timestamp)
                && !chain_config.is_osaka_activated(timestamp)
        }
        // Spans Osaka + BPO1..BPO5: none of the BPO forks change the engine
        // container shapes, so they are served under the Osaka header value.
        Fork::Osaka => {
            chain_config.is_osaka_activated(timestamp)
                && !chain_config.is_amsterdam_activated(timestamp)
        }
        Fork::Amsterdam => chain_config.is_amsterdam_activated(timestamp),
        // `parse_fork_segment` restricts the header to the 6 catalogue forks.
        _ => false,
    }
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
