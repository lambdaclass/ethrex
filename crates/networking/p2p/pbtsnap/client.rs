//! `pbtsnap/1` — client side.
//!
//! Two things live here: the [`PbtSnapProvider`] seam the sync driver is
//! written against, and the one implementation of it that talks to real peers.
//!
//! The seam exists because the driver's job is *rejection*, not download. A
//! driver tested only against an honest server proves nothing about the case it
//! is written for — a peer that answers, promptly and in the right shape, with
//! a lie. Putting the transport behind a trait lets the byzantine cases be
//! written as ordinary unit tests rather than as a network fixture, and it is
//! what the adversarial suite in `sync::pbt_snap` is built on.

use std::future::Future;

use bytes::Bytes;
use ethrex_common::H256;
use tracing::debug;

use crate::peer_handler::PeerHandler;
use crate::peer_table::PeerTableServerProtocol as _;
use crate::rlpx::message::Message as RLPxMessage;
use crate::rlpx::p2p::SUPPORTED_PBTSNAP_CAPABILITIES;
use crate::rlpx::pbtsnap::{GetPbtLeafRange, PbtLeafRange};
use crate::snap::constants::PEER_REPLY_TIMEOUT;

/// How many peers one leaf-range request is offered to before it gives up.
///
/// Attempts are *across* peers: a peer that fails is scored down and the next
/// selection is unlikely to return it, so this is closer to "three peers" than
/// "three tries at one peer". Verification failures are retried by the driver
/// on top of this, and deliberately so — this counts silence, the driver counts
/// lies, and collapsing the two would let a lying peer spend a budget meant for
/// a flaky one.
const LEAF_RANGE_ATTEMPTS: u32 = 3;

/// Why a request to the network did not produce an answer.
///
/// Deliberately thin. Everything a *dishonest* answer can be wrong about is the
/// driver's business, not the transport's; this type only says that no answer
/// arrived.
#[derive(Debug, thiserror::Error)]
pub enum PbtProviderError {
    #[error("no peer answered a pbtsnap leaf range request after {0} attempts")]
    NoAnswer(u32),
    #[error("peer table error: {0}")]
    PeerTable(String),
    #[error("bytecode request failed: {0}")]
    Bytecodes(String),
}

/// The transport the PBT sync driver runs over.
///
/// Bytecodes ride `snap/1 GetByteCodes` rather than a `pbtsnap` message of
/// their own — code is content-addressed and self-verifying, so it needs no
/// binary-tree-specific framing. That does couple PBT sync to the peer also
/// advertising `snap/1`, which is true of every ethrex node today; see the
/// plan's open question 6 for when it stops being.
pub trait PbtSnapProvider {
    /// One `GetPbtLeafRange` round trip. The answer is *unverified*: a
    /// well-formed response from a hostile peer is a successful call.
    fn get_leaf_range(
        &self,
        request: GetPbtLeafRange,
    ) -> impl Future<Output = Result<PbtLeafRange, PbtProviderError>> + Send;

    /// Bytecodes by hash. A short, reordered or substituted answer is legal
    /// here and is the driver's problem, not the transport's.
    fn get_bytecodes(
        &self,
        hashes: &[H256],
    ) -> impl Future<Output = Result<Vec<Bytes>, PbtProviderError>> + Send;
}

/// The real provider: `pbtsnap/1` requests to peers that advertise it.
#[derive(Debug, Clone)]
pub struct PeerPbtSnapProvider {
    peers: PeerHandler,
}

impl PeerPbtSnapProvider {
    pub fn new(peers: PeerHandler) -> Self {
        Self { peers }
    }

    fn record(&self, peer_id: H256, success: bool) -> Result<(), PbtProviderError> {
        let result = if success {
            self.peers.peer_table.record_success(peer_id)
        } else {
            self.peers.peer_table.record_failure(peer_id)
        };
        result.map_err(|e| PbtProviderError::PeerTable(e.to_string()))
    }
}

impl PbtSnapProvider for PeerPbtSnapProvider {
    async fn get_leaf_range(
        &self,
        request: GetPbtLeafRange,
    ) -> Result<PbtLeafRange, PbtProviderError> {
        for attempt in 1..=LEAF_RANGE_ATTEMPTS {
            // Selection filters on the peer's *advertised* capability list.
            // `negotiated_pbtsnap_capability` is written and never read, which
            // is the shape `snap/1` already has here — see the note in
            // `pbtsnap::live_tests`.
            let Some((peer_id, mut connection, _permit)) = self
                .peers
                .peer_table
                .get_best_peer(SUPPORTED_PBTSNAP_CAPABILITIES.to_vec())
                .await
                .map_err(|e| PbtProviderError::PeerTable(e.to_string()))?
            else {
                debug!("No pbtsnap-capable peer available (attempt {attempt})");
                continue;
            };

            match connection
                .outgoing_request(
                    RLPxMessage::GetPbtLeafRange(request.clone()),
                    PEER_REPLY_TIMEOUT,
                )
                .await
            {
                Ok(RLPxMessage::PbtLeafRange(response)) if response.id == request.id => {
                    self.record(peer_id, true)?;
                    return Ok(response);
                }
                // The id is mirrored by the protocol, so a mismatched one is a
                // violation rather than an unlucky round trip. It is also the
                // one thing the response-routing layer cannot have got right by
                // accident, since it keys the in-flight map on it.
                Ok(other) => {
                    debug!(%peer_id, "Unexpected reply to a pbtsnap leaf range: {other}");
                    self.record(peer_id, false)?;
                }
                Err(error) => {
                    debug!(%peer_id, %error, "A pbtsnap leaf range request failed");
                    self.record(peer_id, false)?;
                }
            }
        }
        Err(PbtProviderError::NoAnswer(LEAF_RANGE_ATTEMPTS))
    }

    async fn get_bytecodes(&self, hashes: &[H256]) -> Result<Vec<Bytes>, PbtProviderError> {
        let mut peers = self.peers.clone();
        crate::snap::request_bytecodes(&mut peers, hashes)
            .await
            .map_err(|e| PbtProviderError::Bytecodes(e.to_string()))?
            .ok_or_else(|| PbtProviderError::Bytecodes("no peer returned bytecodes".to_string()))
    }
}
