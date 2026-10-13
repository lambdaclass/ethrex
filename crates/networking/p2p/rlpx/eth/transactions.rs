use std::collections::HashSet;

use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress, snappy_decompress_bounded},
};
use crate::types::Node;
use bytes::BufMut;
use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_blockchain::error::MempoolError;
use ethrex_common::types::Fork;
use ethrex_common::types::P2PTransaction;
use ethrex_common::types::WrappedEIP4844Transaction;
use ethrex_common::{H256, types::Transaction};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{
    decode::{RLPDecode, decode_rlp_item, get_item_with_prefix},
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};
use ethrex_storage::error::StoreError;
use tracing::debug;

/// Allowed skew between a peer's announced pooled-transaction size and the size we compute
/// from the received transaction. The announced size is a soft hint that varies slightly
/// between clients (e.g. geth's `Transaction.Size()` omits the v1 blob-sidecar wrapper
/// version byte), so we tolerate a few bytes before treating it as a protocol violation.
/// Matches go-ethereum's tx fetcher (`eth/fetcher/tx_fetcher.go`).
pub(crate) const POOLED_TX_SIZE_TOLERANCE: usize = 8;

/// Upper bound on the decompressed size of a `PooledTransactions` response, enforced before
/// decompression so an oversized reply is rejected without materializing it. Tighter than the
/// global frame/snappy cap (`MAX_SNAPPY_DECOMPRESSED_LEN`, ~16 MiB): go-ethereum soft-limits a
/// response to `softResponseLimit` (2 MiB) and stops after the first tx that crosses it, so a
/// well-behaved reply is at most ~2 MiB plus one max-size tx (~1 MiB blob wrapper). 4 MiB clears
/// that with margin while staying 4× below the frame cap.
pub(crate) const MAX_POOLED_TRANSACTIONS_BYTES: usize = 4 * 1024 * 1024;

/// Target maximum size of a `PooledTransactions` response we *serve*, matching go-ethereum's
/// `softResponseLimit` (2 MiB). We stop before a transaction would push the response over this,
/// so what we emit stays well under any peer's inbound decode cap
/// ([`MAX_POOLED_TRANSACTIONS_BYTES`]). Partial responses are protocol-legal — the requester
/// re-requests whatever it still needs.
pub(crate) const MAX_POOLED_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#transactions-0x02
// Broadcast message
#[derive(Debug, Clone)]
pub struct Transactions {
    pub transactions: Vec<Transaction>,
}

impl Transactions {
    pub fn new(transactions: Vec<Transaction>) -> Self {
        Self { transactions }
    }
}

impl RLPxMessage for Transactions {
    const CODE: u8 = 0x02;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        let mut encoder = Encoder::new(&mut encoded_data);
        let txs_iter = self.transactions.iter();
        for tx in txs_iter {
            encoder = encoder.encode_field(tx)
        }
        encoder.finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let mut decoder = Decoder::new(&decompressed_data)?;
        let mut transactions: Vec<Transaction> = vec![];
        // This is done like this because the blanket Vec<T> implementation
        // gets confused since a legacy transaction is actually a list,
        // or so it seems.
        while let Ok((tx, updated_decoder)) = decoder.decode_field::<Transaction>("p2p transaction")
        {
            decoder = updated_decoder;
            transactions.push(tx);
        }
        Ok(Self::new(transactions))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#newpooledtransactionhashes-0x08
// Broadcast message
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NewPooledTransactionHashes {
    pub transaction_types: Bytes,
    pub transaction_sizes: Vec<usize>,
    pub transaction_hashes: Vec<H256>,
}

impl NewPooledTransactionHashes {
    /// Build from pre-computed raw fields (used when constructing trimmed announcements).
    pub fn from_raw(
        transaction_types: Bytes,
        transaction_sizes: Vec<usize>,
        transaction_hashes: Vec<H256>,
    ) -> Self {
        Self {
            transaction_types,
            transaction_sizes,
            transaction_hashes,
        }
    }

    pub fn new(
        transactions: Vec<Transaction>,
        blockchain: &Blockchain,
    ) -> Result<Self, StoreError> {
        let transactions_len = transactions.len();
        let mut transaction_types = Vec::with_capacity(transactions_len);
        let mut transaction_sizes = Vec::with_capacity(transactions_len);
        let mut transaction_hashes = Vec::with_capacity(transactions_len);
        for transaction in transactions {
            let transaction_type = transaction.tx_type();
            let transaction_hash = transaction.hash(&NativeCrypto);
            // size is defined as the len of the canonical encoding of the transaction
            // as it would appear in a PooledTransactions response.
            // https://eips.ethereum.org/EIPS/eip-2718
            let transaction_size = match transaction {
                // Blob transactions use the network (wrapped) representation
                // which includes the blobs bundle.
                // https://eips.ethereum.org/EIPS/eip-4844#networking
                Transaction::EIP4844Transaction(eip4844_tx) => {
                    // If the bundle is gone (the tx was evicted or pulled into a payload
                    // between the broadcaster's snapshot and now) we cannot serve this tx
                    // either: `Blockchain::get_p2p_transaction_by_hash` errors out on a
                    // blob tx with no bundle, so `GetPooledTransactions` skips it. Skip it
                    // here too. Substituting an empty bundle would announce ~162 bytes for
                    // a ~137 KB transaction, and peers that check the announced size
                    // against the delivered one (geth's tx fetcher, and our own
                    // `validate_requested` below, both with an 8-byte tolerance) will
                    // disconnect us for the mismatch.
                    let Some(tx_blobs_bundle) =
                        blockchain.mempool.get_blobs_bundle(transaction_hash)?
                    else {
                        continue;
                    };
                    // A tx ingested over eth/72 may hold an elided bundle (commitments
                    // and proofs, no blobs; cells travel separately). This pre-72
                    // announcement promises a full-blob delivery, but the serve path
                    // drops elided txs from pre-72 responses (`GetPooledTransactions`
                    // arm in `connection/server.rs`), so we would announce a hash we
                    // never deliver — and with a size no full-blob delivery can match.
                    // geth's tx fetcher checks a delivered tx against every peer that
                    // announced its hash, so when another peer delivers the full tx the
                    // mismatch is charged to us. Skip it, mirroring the serve path.
                    if tx_blobs_bundle.blobs.len() != tx_blobs_bundle.commitments.len() {
                        continue;
                    }
                    let p2p_tx =
                        P2PTransaction::EIP4844TransactionWithBlobs(WrappedEIP4844Transaction {
                            tx: eip4844_tx,
                            wrapper_version: (tx_blobs_bundle.version != 0)
                                .then_some(tx_blobs_bundle.version),
                            blobs_bundle: tx_blobs_bundle,
                        });
                    p2p_tx.encode_canonical_len()
                }
                _ => transaction.encode_canonical_len(),
            };
            transaction_types.push(transaction_type as u8);
            transaction_hashes.push(transaction_hash);
            transaction_sizes.push(transaction_size);
        }
        Ok(Self {
            transaction_types: transaction_types.into(),
            transaction_sizes,
            transaction_hashes,
        })
    }

    pub fn get_transactions_to_request(
        &self,
        blockchain: &Blockchain,
        announcer: H256,
    ) -> Result<Vec<H256>, StoreError> {
        blockchain.mempool.reserve_unknown_hashes(
            &self.transaction_hashes,
            &self.transaction_types,
            &self.transaction_sizes,
            announcer,
        )
    }

    /// Extract only the entries for the given `requested` hashes from this announcement.
    /// Returns a trimmed announcement containing just those hashes with their types and sizes,
    /// in announcement order, taking the first entry of a hash announced twice.
    pub fn filter_to(&self, requested: &[H256]) -> NewPooledTransactionHashes {
        let (types, sizes, hashes) = filter_announced(
            &self.transaction_types,
            &self.transaction_sizes,
            &self.transaction_hashes,
            requested,
        );
        NewPooledTransactionHashes {
            transaction_types: types.into(),
            transaction_sizes: sizes,
            transaction_hashes: hashes,
        }
    }
}

/// The entries of an announcement whose hash is in `requested`, in announcement order and
/// once per hash. One pass over the announcement, so trimming a large one stays linear.
pub(crate) fn filter_announced(
    types: &[u8],
    sizes: &[usize],
    hashes: &[H256],
    requested: &[H256],
) -> (Vec<u8>, Vec<usize>, Vec<H256>) {
    let mut wanted: HashSet<H256> = requested.iter().copied().collect();
    let mut kept_types = Vec::with_capacity(wanted.len());
    let mut kept_sizes = Vec::with_capacity(wanted.len());
    let mut kept_hashes = Vec::with_capacity(wanted.len());
    for ((hash, tx_type), size) in hashes.iter().zip(types).zip(sizes) {
        if wanted.remove(hash) {
            kept_types.push(*tx_type);
            kept_sizes.push(*size);
            kept_hashes.push(*hash);
        }
    }
    (kept_types, kept_sizes, kept_hashes)
}

/// Decodes the `types`, `sizes` and `hashes` fields that every `NewPooledTransactionHashes`
/// version starts with. The three must announce the same number of transactions, and that is
/// checked from the RLP headers alone, before any of them is decoded, so a malformed
/// announcement is rejected without building its vectors.
pub(crate) fn decode_announced_txs(
    decoder: Decoder<'_>,
) -> Result<(Bytes, Vec<usize>, Vec<H256>, Decoder<'_>), RLPDecodeError> {
    let (types, decoder) = decoder.get_encoded_item_ref()?;
    let (sizes, decoder) = decoder.get_encoded_item_ref()?;
    let (hashes, decoder) = decoder.get_encoded_item_ref()?;

    let types_count = match decode_rlp_item(types)? {
        (false, payload, _) => payload.len(),
        (true, _, _) => return Err(RLPDecodeError::UnexpectedList),
    };
    if rlp_list_len(sizes)? != types_count || rlp_list_len(hashes)? != types_count {
        return Err(RLPDecodeError::Custom(
            "transaction_hashes, transaction_sizes and transaction_types must have the same length"
                .to_string(),
        ));
    }

    Ok((
        Bytes::decode(types)?,
        Vec::<usize>::decode(sizes)?,
        Vec::<H256>::decode(hashes)?,
        decoder,
    ))
}

/// Number of items in the RLP list `rlp`, walking their headers without decoding them.
fn rlp_list_len(rlp: &[u8]) -> Result<usize, RLPDecodeError> {
    let (is_list, mut items, _) = decode_rlp_item(rlp)?;
    if !is_list {
        return Err(RLPDecodeError::UnexpectedString);
    }
    let mut count = 0;
    while !items.is_empty() {
        (_, items) = get_item_with_prefix(items)?;
        count += 1;
    }
    Ok(count)
}

impl RLPxMessage for NewPooledTransactionHashes {
    const CODE: u8 = 0x08;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.transaction_types)
            .encode_field(&self.transaction_sizes)
            .encode_field(&self.transaction_hashes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (transaction_types, transaction_sizes, transaction_hashes, decoder) =
            decode_announced_txs(decoder)?;
        // The announcement is exactly three fields until eth/72 adds `cell_mask`
        // (EIP-8070). Reject a longer list instead of ignoring the extra fields,
        // so a peer encoding a newer shape on this session is caught here.
        decoder.finish()?;

        Ok(Self {
            transaction_types,
            transaction_sizes,
            transaction_hashes,
        })
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#getpooledtransactions-0x09
#[derive(Debug, Clone)]
pub struct GetPooledTransactions {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub transaction_hashes: Vec<H256>,
}

impl GetPooledTransactions {
    pub fn new(id: u64, transaction_hashes: Vec<H256>) -> Self {
        Self {
            transaction_hashes,
            id,
        }
    }

    pub fn handle(&self, blockchain: &Blockchain) -> Result<PooledTransactions, StoreError> {
        // TODO(#1615): get transactions in batch instead of iterating over them.
        let mut pooled_transactions = Vec::new();
        let mut seen = HashSet::with_capacity(self.transaction_hashes.len());
        let mut bytes_used = 0usize;
        for hash in &self.transaction_hashes {
            // Serve each requested hash at most once: a peer padding the request with duplicates
            // must not amplify the response or force repeated mempool lookups (the lookup is
            // skipped for a repeat, so an all-duplicate request costs one probe, not N).
            if !seen.insert(*hash) {
                continue;
            }
            // As per the spec, skipping unavailable transactions is perfectly acceptable,
            // for example if a transaction was taken out of the mempool due to payload
            // building after being advertised.
            let Ok(tx) = blockchain.get_p2p_transaction_by_hash(hash) else {
                continue;
            };
            // Cap the response size (geth `softResponseLimit`): stop before a tx would push the
            // response over the budget so we never emit more than a peer's inbound cap accepts.
            // Always serve at least one tx so a lone oversized (blob) tx still goes out.
            let tx_size = tx.encode_canonical_len();
            if !pooled_transactions.is_empty() && bytes_used + tx_size > MAX_POOLED_RESPONSE_BYTES {
                break;
            }
            bytes_used += tx_size;
            pooled_transactions.push(tx);
        }

        Ok(PooledTransactions {
            id: self.id,
            pooled_transactions,
        })
    }
}

impl RLPxMessage for GetPooledTransactions {
    const CODE: u8 = 0x09;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.transaction_hashes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (transaction_hashes, _): (Vec<H256>, _) = decoder.decode_field("transactionHashes")?;

        Ok(Self::new(id, transaction_hashes))
    }
}

// https://github.com/ethereum/devp2p/blob/master/caps/eth.md#pooledtransactions-0x0a
#[derive(Debug, Clone)]
pub struct PooledTransactions {
    // id is a u64 chosen by the requesting peer, the responding peer must mirror the value for the response
    // https://github.com/ethereum/devp2p/blob/master/caps/eth.md#protocol-messages
    pub id: u64,
    pub pooled_transactions: Vec<P2PTransaction>,
}

impl PooledTransactions {
    pub fn new(id: u64, pooled_transactions: Vec<P2PTransaction>) -> Self {
        Self {
            pooled_transactions,
            id,
        }
    }

    /// validates if the received TXs match the request
    pub fn validate_requested(
        &self,
        requested: &NewPooledTransactionHashes,
        fork: Fork,
    ) -> Result<(), MempoolError> {
        // A well-formed response contains each requested tx at most once. Track seen hashes so
        // a peer can't send duplicates (or more txs than requested) — each would otherwise pass
        // the per-tx checks below, letting a response balloon to the frame cap regardless of
        // how little we asked for.
        let mut seen = HashSet::with_capacity(self.pooled_transactions.len());
        for tx in &self.pooled_transactions {
            // Reject duplicates before any per-tx work (blob validation, hash lookup) so a peer
            // echoing the same tx N times can't force N rounds of that work — fail fast.
            let tx_hash = tx.compute_hash();
            if !seen.insert(tx_hash) {
                return Err(MempoolError::DuplicatePooledTx);
            }
            if let P2PTransaction::EIP4844TransactionWithBlobs(itx) = tx {
                itx.blobs_bundle.validate_cheap(&itx.tx, fork)?;
            }
            let Some(pos) = requested
                .transaction_hashes
                .iter()
                .position(|&hash| hash == tx_hash)
            else {
                return Err(MempoolError::RequestedPooledTxNotFound);
            };

            let expected_type = requested.transaction_types[pos];
            let expected_size = requested.transaction_sizes[pos];
            if tx.tx_type() as u8 != expected_type {
                return Err(MempoolError::InvalidPooledTxType(expected_type));
            }
            // The announced size is a soft hint, not an exact value. geth's
            // `Transaction.Size()` under-counts a v1 (EIP-7594 cell-proof) blob sidecar by
            // one byte — it omits the wrapper version byte — so a strict equality check
            // would disconnect geth peers on every v1 blob-tx announcement. Match geth's tx
            // fetcher, which tolerates up to 8 bytes of skew before treating it as a
            // violation (go-ethereum eth/fetcher/tx_fetcher.go).
            let tx_size = tx.encode_canonical_len();
            if tx_size.abs_diff(expected_size) > POOLED_TX_SIZE_TOLERANCE {
                return Err(MempoolError::InvalidPooledTxSize);
            }
        }
        Ok(())
    }

    /// Saves every incoming pooled transaction to the mempool.
    pub async fn handle(
        self,
        node: &Node,
        blockchain: &Blockchain,
        is_l2_mode: bool,
    ) -> Result<(), MempoolError> {
        for tx in self.pooled_transactions {
            if let P2PTransaction::EIP4844TransactionWithBlobs(itx) = tx {
                if is_l2_mode {
                    debug!(
                        peer=%node,
                        "Rejecting blob transaction in L2 mode - blob transactions are not supported in L2",
                    );
                    continue;
                }
                if let Err(e) = blockchain
                    .add_blob_transaction_to_pool(itx.tx, itx.blobs_bundle)
                    .await
                {
                    if matches!(e, MempoolError::BlobsBundleError(_)) {
                        return Err(e);
                    }
                    debug!(
                        peer=%node,
                        error=%e,
                        "Error adding transaction"
                    );
                    continue;
                }
            } else {
                let regular_tx = tx
                    .try_into()
                    .map_err(|error| MempoolError::StoreError(StoreError::Custom(error)))?;
                if let Err(e) = blockchain.add_transaction_to_pool(regular_tx).await {
                    debug!(
                        peer=%node,
                        error=%e,
                        "Error adding transaction"
                    );
                    continue;
                }
            }
        }
        Ok(())
    }
}

impl RLPxMessage for PooledTransactions {
    const CODE: u8 = 0x0A;
    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.pooled_transactions)
            .finish();
        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        // Reject an oversized response by its declared decompressed length before allocating and
        // materializing the transaction list (which for blob txs pulls large sidecars into
        // memory). See `MAX_POOLED_TRANSACTIONS_BYTES`.
        let decompressed_data = snappy_decompress_bounded(msg_data, MAX_POOLED_TRANSACTIONS_BYTES)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder): (u64, _) = decoder.decode_field("request-id")?;
        let (pooled_transactions, _): (Vec<P2PTransaction>, _) =
            decoder.decode_field("pooledTransactions")?;

        Ok(Self::new(id, pooled_transactions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rlpx::eth::eth72::transactions::NewPooledTransactionHashes72;
    use ethrex_common::types::EIP1559Transaction;

    /// A compressed announcement with `types`, `sizes` and `hashes` as given, so the field
    /// lengths can disagree, plus an empty `cell_mask` when `eth72` is set. Each hash is
    /// `hash_len` bytes long: a wrong length can only be caught by decoding the item.
    fn raw_announcement(
        types: usize,
        sizes: usize,
        hashes: usize,
        hash_len: usize,
        eth72: bool,
    ) -> Vec<u8> {
        let hashes: Vec<Bytes> = (0..hashes)
            .map(|i| Bytes::from(vec![i as u8 + 1; hash_len]))
            .collect();
        let mut encoded = vec![];
        let encoder = Encoder::new(&mut encoded)
            .encode_field(&Bytes::from(vec![2u8; types]))
            .encode_field(&vec![100usize; sizes])
            .encode_field(&hashes);
        if eth72 {
            encoder.encode_field(&Bytes::new()).finish();
        } else {
            encoder.finish();
        }
        snappy_compress(encoded).unwrap()
    }

    fn decode_announcement(msg: &[u8], eth72: bool) -> Result<usize, RLPDecodeError> {
        if eth72 {
            NewPooledTransactionHashes72::decode(msg).map(|a| a.transaction_hashes.len())
        } else {
            NewPooledTransactionHashes::decode(msg).map(|a| a.transaction_hashes.len())
        }
    }

    #[test]
    fn announcements_with_matching_field_lengths_decode() {
        for eth72 in [false, true] {
            assert_eq!(
                decode_announcement(&raw_announcement(3, 3, 3, 32, eth72), eth72),
                Ok(3)
            );
        }
    }

    #[test]
    fn announcements_with_mismatched_field_lengths_are_rejected() {
        let mismatch = || {
            RLPDecodeError::Custom(
                "transaction_hashes, transaction_sizes and transaction_types must have the same length"
                    .to_string(),
            )
        };
        for eth72 in [false, true] {
            for (types, sizes, hashes) in [(2, 3, 3), (3, 2, 3), (3, 3, 2), (0, 0, 1)] {
                assert_eq!(
                    decode_announcement(&raw_announcement(types, sizes, hashes, 32, eth72), eth72),
                    Err(mismatch()),
                    "types {types}, sizes {sizes}, hashes {hashes}, eth72 {eth72}"
                );
            }
            // The lengths are compared before any hash is decoded: 31-byte hashes are only
            // caught by decoding them, so a mismatch must be reported first.
            assert_eq!(
                decode_announcement(&raw_announcement(2, 2, 3, 31, eth72), eth72),
                Err(mismatch())
            );
            assert_ne!(
                decode_announcement(&raw_announcement(3, 3, 3, 31, eth72), eth72),
                Err(mismatch())
            );
        }
    }

    #[test]
    fn filter_to_keeps_announcement_order_and_first_entries() {
        let h = |n| H256::from_low_u64_be(n);
        let announcement = NewPooledTransactionHashes::from_raw(
            vec![0u8, 1, 2, 3].into(),
            vec![10, 11, 12, 13],
            vec![h(1), h(2), h(3), h(2)],
        );
        let trimmed = announcement.filter_to(&[h(3), h(2), h(9)]);
        assert_eq!(trimmed.transaction_hashes, vec![h(2), h(3)]);
        assert_eq!(trimmed.transaction_types.as_ref(), &[1, 2]);
        assert_eq!(trimmed.transaction_sizes, vec![11, 12]);
    }

    fn p2p_tx(nonce: u64) -> P2PTransaction {
        P2PTransaction::EIP1559Transaction(EIP1559Transaction {
            nonce,
            ..Default::default()
        })
    }

    fn announcement_for(txs: &[P2PTransaction]) -> NewPooledTransactionHashes {
        NewPooledTransactionHashes::from_raw(
            txs.iter()
                .map(|t| t.tx_type() as u8)
                .collect::<Vec<_>>()
                .into(),
            txs.iter().map(|t| t.encode_canonical_len()).collect(),
            txs.iter().map(|t| t.compute_hash()).collect(),
        )
    }

    #[test]
    fn validate_requested_accepts_each_requested_tx_once() {
        let txs = vec![p2p_tx(0), p2p_tx(1)];
        let announced = announcement_for(&txs);
        let response = PooledTransactions::new(0, txs);
        assert!(
            response
                .validate_requested(&announced, Fork::Cancun)
                .is_ok()
        );
    }

    #[test]
    fn validate_requested_rejects_duplicate_tx() {
        // We requested one tx; the peer echoes it twice. Both copies pass the per-tx type/size
        // checks, so without the duplicate guard the response could balloon past what we asked.
        let announced = announcement_for(&[p2p_tx(0)]);
        let response = PooledTransactions::new(0, vec![p2p_tx(0), p2p_tx(0)]);
        assert!(matches!(
            response.validate_requested(&announced, Fork::Cancun),
            Err(MempoolError::DuplicatePooledTx)
        ));
    }
}
