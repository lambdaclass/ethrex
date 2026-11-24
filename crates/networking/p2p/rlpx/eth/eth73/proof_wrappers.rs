//! The wrapper exchange of EIP-8504.
//!
//! A dependency transaction, a frame transaction with at least one proof frame,
//! never travels in `Transactions` or `PooledTransactions`: it is propagated only
//! inside its wrapper, which also carries one witness for each of its
//! dependencies. A node announces the wrappers it has admitted by hash and sends
//! one only when a peer asks for it:
//!
//! - `NewProofWrapperHashes (0x16)`: `[[wrapper-hash: B_32, tx-hash: B_32, length: P], ...]`
//! - `GetProofWrappers (0x17)`: `[request-id: P, [wrapper-hash: B_32, ...]]`
//! - `ProofWrappers (0x18)`: `[request-id: P, [wrapper, ...]]`
//!
//! The transaction hash in an announcement lets a peer skip a wrapper for a
//! transaction it already holds, since a transaction can have more than one valid
//! wrapper. `length` is the size in bytes of the wrapper's RLP serialization.

use std::collections::HashSet;

use bytes::BufMut;
use ethrex_blockchain::{Blockchain, mempool::ProofWrapperAnnouncement};
use ethrex_common::{H256, types::ProofWrapper};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use ethrex_storage::error::StoreError;
use lru::LruCache;
use std::num::NonZeroUsize;

use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress_bounded},
};

/// Most wrappers announced in one `NewProofWrapperHashes` message. We never send
/// more, and a longer announcement is rejected when decoded.
pub const MAX_PROOF_WRAPPER_ANNOUNCEMENTS: usize = 4096;

/// Most wrapper hashes in one `GetProofWrappers` request, as in the other
/// requests by hash of the protocol.
pub const MAX_PROOF_WRAPPER_REQUEST_HASHES: usize = 256;

/// Soft limit on the wrappers served in one `ProofWrappers` response, and on the
/// announced sizes we ask for in one request. A response always carries at least
/// one wrapper, so a single wrapper above the limit still goes out.
pub const PROOF_WRAPPERS_RESPONSE_SOFT_LIMIT: usize = 2 * 1024 * 1024;

/// Upper bound on the decompressed size of a `ProofWrappers` response, enforced
/// before decompression. Twice the soft limit: room for a peer that overshoots by
/// one wrapper.
const MAX_PROOF_WRAPPERS_BYTES: usize = 2 * PROOF_WRAPPERS_RESPONSE_SOFT_LIMIT;

/// Upper bound on the decompressed size of an announcement: two hashes and a
/// length per entry (33 + 33 + 9 bytes once RLP-tagged, plus the entry's list
/// header), with slack for the outer header.
const MAX_ANNOUNCEMENT_BYTES: usize = MAX_PROOF_WRAPPER_ANNOUNCEMENTS * 80 + 64;

/// Upper bound on the decompressed size of a request: a request id and at most
/// [`MAX_PROOF_WRAPPER_REQUEST_HASHES`] hashes.
const MAX_REQUEST_BYTES: usize = MAX_PROOF_WRAPPER_REQUEST_HASHES * 33 + 64;

/// Most transactions a connection remembers as already requested from its peer.
/// Past it the oldest are forgotten, so the memory per connection stays bounded.
pub const MAX_REQUESTED_FROM_PEER: usize = 4 * MAX_PROOF_WRAPPER_ANNOUNCEMENTS;

/// Most wrappers a peer may announce and then not deliver, over the life of a
/// connection, before it is dropped. An honest peer misses a request only when
/// the transaction left its pool in between. The limit sits well below
/// [`MAX_REQUESTED_FROM_PEER`], so a peer cannot push a transaction out of that
/// memory by announcing wrappers it never sends.
pub const MAX_UNDELIVERED_FROM_PEER: usize = MAX_REQUESTED_FROM_PEER / 4;

/// The transactions whose wrapper a connection has already requested from its
/// peer. A transaction is requested from a given peer at most once. A peer that
/// announces a wrapper and then withholds it, or sends one the node does not
/// admit, therefore cannot be made to cost a second fetch and a second witness
/// verification, and cannot take the request again ahead of the other peers that
/// announced the same transaction.
#[derive(Debug)]
pub struct RequestedFromPeer(LruCache<H256, ()>);

impl Default for RequestedFromPeer {
    fn default() -> Self {
        Self(LruCache::new(
            NonZeroUsize::new(MAX_REQUESTED_FROM_PEER).unwrap_or(NonZeroUsize::MIN),
        ))
    }
}

impl RequestedFromPeer {
    /// Drops from `announced` the wrappers of transactions already requested
    /// from this peer.
    pub fn retain_unrequested(&self, announced: &mut Vec<ProofWrapperAnnouncement>) {
        announced.retain(|wrapper| !self.0.contains(&wrapper.tx_hash));
    }

    /// Records that these wrappers were requested from the peer. When the memory
    /// is full, the transactions requested longest ago make room.
    pub fn record(&mut self, requested: &[ProofWrapperAnnouncement]) {
        for wrapper in requested {
            self.0.put(wrapper.tx_hash, ());
        }
    }
}

/// Announces wrappers the sender has admitted and can serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProofWrapperHashes {
    pub wrappers: Vec<ProofWrapperAnnouncement>,
}

impl NewProofWrapperHashes {
    pub fn new(wrappers: Vec<ProofWrapperAnnouncement>) -> Self {
        Self { wrappers }
    }
}

impl RLPxMessage for NewProofWrapperHashes {
    const CODE: u8 = 0x16;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let entries: Vec<(H256, H256, u64)> = self
            .wrappers
            .iter()
            .map(|wrapper| {
                (
                    wrapper.wrapper_hash,
                    wrapper.tx_hash,
                    wrapper.wrapper_len as u64,
                )
            })
            .collect();
        let msg_data = snappy_compress(entries.encode_to_vec())?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress_bounded(msg_data, MAX_ANNOUNCEMENT_BYTES)?;
        let entries = Vec::<(H256, H256, u64)>::decode(&decompressed_data)?;
        if entries.len() > MAX_PROOF_WRAPPER_ANNOUNCEMENTS {
            return Err(RLPDecodeError::Custom(
                "NewProofWrapperHashes: too many wrappers".to_string(),
            ));
        }
        let wrappers = entries
            .into_iter()
            .map(|(wrapper_hash, tx_hash, length)| {
                Ok(ProofWrapperAnnouncement {
                    wrapper_hash,
                    tx_hash,
                    wrapper_len: usize::try_from(length)
                        .map_err(|_| RLPDecodeError::InvalidLength)?,
                })
            })
            .collect::<Result<Vec<_>, RLPDecodeError>>()?;
        Ok(Self { wrappers })
    }
}

/// Requests wrappers by hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetProofWrappers {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    pub id: u64,
    pub wrapper_hashes: Vec<H256>,
}

impl GetProofWrappers {
    pub fn new(id: u64, wrapper_hashes: Vec<H256>) -> Self {
        Self { id, wrapper_hashes }
    }

    /// Builds the response from the pool. A wrapper that is no longer pooled, or
    /// whose transaction is private, is left out, as the protocol allows for
    /// every request by hash; so is everything past the response size limit.
    pub fn handle(&self, blockchain: &Blockchain) -> Result<ProofWrappers, StoreError> {
        let mut wrappers = Vec::new();
        let mut seen = HashSet::with_capacity(self.wrapper_hashes.len());
        let mut bytes_used = 0usize;
        for hash in &self.wrapper_hashes {
            // Serve each requested hash at most once.
            if !seen.insert(*hash) {
                continue;
            }
            let Some(wrapper) = blockchain.mempool.get_proof_wrapper(*hash)? else {
                continue;
            };
            let wrapper_len = wrapper.length();
            if !wrappers.is_empty() && bytes_used + wrapper_len > PROOF_WRAPPERS_RESPONSE_SOFT_LIMIT
            {
                break;
            }
            bytes_used += wrapper_len;
            wrappers.push(wrapper);
        }
        Ok(ProofWrappers {
            id: self.id,
            wrappers,
        })
    }
}

impl RLPxMessage for GetProofWrappers {
    const CODE: u8 = 0x17;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.wrapper_hashes)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress_bounded(msg_data, MAX_REQUEST_BYTES)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (wrapper_hashes, decoder): (Vec<H256>, _) = decoder.decode_field("wrapperHashes")?;
        decoder.finish()?;
        if wrapper_hashes.len() > MAX_PROOF_WRAPPER_REQUEST_HASHES {
            return Err(RLPDecodeError::Custom(
                "GetProofWrappers: too many wrapper hashes".to_string(),
            ));
        }
        Ok(Self { id, wrapper_hashes })
    }
}

/// Returns the requested wrappers the sender still has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofWrappers {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    pub id: u64,
    pub wrappers: Vec<ProofWrapper>,
}

impl ProofWrappers {
    pub fn new(id: u64, wrappers: Vec<ProofWrapper>) -> Self {
        Self { id, wrappers }
    }
}

impl RLPxMessage for ProofWrappers {
    const CODE: u8 = 0x18;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.wrappers)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress_bounded(msg_data, MAX_PROOF_WRAPPERS_BYTES)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (wrappers, decoder): (Vec<ProofWrapper>, _) = decoder.decode_field("wrappers")?;
        decoder.finish()?;
        Ok(Self { id, wrappers })
    }
}
