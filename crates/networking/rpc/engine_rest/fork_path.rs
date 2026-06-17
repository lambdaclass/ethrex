//! Fork path-segment extractor for `/{fork}/...` engine REST routes.
//!
//! Maps the URL path segment ("paris" .. "amsterdam") to `ethrex_common::types::Fork`.
//! Any segment not in the engine REST spec's fork table is rejected with
//! `400 Bad Request` — this covers both pre-Merge forks (Frontier..London) and
//! forks that have no REST routes of their own (e.g. the BPO forks that sit
//! between Osaka and Amsterdam).

use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;
use ethrex_common::types::Fork;

use crate::engine_rest::error::ProblemJson;

/// Parse a URL fork segment into a `Fork`.
pub fn parse_fork_segment(s: &str) -> Result<Fork, ProblemJson> {
    let fork = match s {
        "paris" => Fork::Paris,
        "shanghai" => Fork::Shanghai,
        "cancun" => Fork::Cancun,
        "prague" => Fork::Prague,
        "osaka" => Fork::Osaka,
        "amsterdam" => Fork::Amsterdam,
        _ => {
            return Err(ProblemJson::bad_request(&format!(
                "unsupported fork segment: {s}"
            )));
        }
    };
    Ok(fork)
}

/// Axum extractor that pulls the first `{fork}` URL segment and validates it.
#[derive(Debug, Clone, Copy)]
pub struct ForkPath(pub Fork);

impl<S> FromRequestParts<S> for ForkPath
where
    S: Send + Sync,
{
    type Rejection = ProblemJson;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(fork): Path<String> = Path::from_request_parts(parts, state)
            .await
            .map_err(|err| ProblemJson::bad_request(&format!("missing fork segment: {err}")))?;
        parse_fork_segment(&fork).map(ForkPath)
    }
}
