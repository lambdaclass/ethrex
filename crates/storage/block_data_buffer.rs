use std::collections::VecDeque;
use std::sync::Arc;

use ethrex_common::H256;
use ethrex_common::types::{Block, BlockBody, BlockHash, BlockHeader, BlockNumber, Code, Receipt};
use rustc_hash::{FxHashMap, FxHashSet};

/// Block data held in memory after `newPayload` returns, before the background
/// flusher writes it to RocksDB. One per block hash (holds reorg siblings).
#[derive(Debug, Clone)]
pub struct BufferedBlock {
    pub header: BlockHeader,
    pub body: BlockBody,
    pub number: BlockNumber,
    pub receipts: Vec<Receipt>,
}

/// In-memory overlay for not-yet-flushed block data. Consulted before disk by
/// every read of headers/bodies/numbers/receipts/codes/tx-locations.
///
/// Updated by RCU swap behind `Arc<RwLock<Arc<BlockDataBuffer>>>` in `Store`,
/// mirroring `TrieLayerCache`: a reader clones the inner `Arc` under a brief
/// read lock and works on the snapshot lock-free, while the single writer
/// mutates a clone and swaps the `Arc` in. Both critical sections are O(1)
/// pointer operations, so contention between readers and the writer is minimal.
#[derive(Debug, Clone)]
pub struct BlockDataBuffer {
    by_hash: FxHashMap<BlockHash, Arc<BufferedBlock>>,
    by_number: FxHashMap<BlockNumber, Vec<BlockHash>>,
    tx_index: FxHashMap<H256, Vec<(BlockNumber, BlockHash, u64)>>,
    /// Content-addressed code introduced by buffered blocks, with the block
    /// number that introduced it (for eviction).
    codes: FxHashMap<H256, (Code, BlockNumber)>,
    /// Hashes not yet known to be on disk, oldest first.
    unflushed: VecDeque<BlockHash>,
    /// Highest block number whose data is durably on disk.
    flushed_upto: BlockNumber,
}

impl Default for BlockDataBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockDataBuffer {
    pub fn new() -> Self {
        Self {
            by_hash: FxHashMap::default(),
            by_number: FxHashMap::default(),
            tx_index: FxHashMap::default(),
            codes: FxHashMap::default(),
            unflushed: VecDeque::new(),
            flushed_upto: 0,
        }
    }

    /// Takes ownership of `block` and moves its header/body into the buffer (the
    /// caller hands ownership over the flush channel and drops it right after, so
    /// cloning would be pure waste).
    pub fn insert(&mut self, block: Block, receipts: Vec<Receipt>, codes: Vec<(H256, Code)>) {
        let hash = block.hash();
        let number = block.header.number;
        if self.by_hash.contains_key(&hash) {
            return; // duplicate guard
        }
        for (index, tx) in block.body.transactions.iter().enumerate() {
            self.tx_index
                .entry(tx.hash())
                .or_default()
                .push((number, hash, index as u64));
        }
        for (code_hash, code) in codes {
            self.codes.entry(code_hash).or_insert((code, number));
        }
        self.by_number.entry(number).or_default().push(hash);
        let Block { header, body } = block;
        self.by_hash.insert(
            hash,
            Arc::new(BufferedBlock {
                header,
                body,
                number,
                receipts,
            }),
        );
        self.unflushed.push_back(hash);
    }

    pub fn get_header(&self, hash: &BlockHash) -> Option<BlockHeader> {
        self.by_hash.get(hash).map(|b| b.header.clone())
    }

    pub fn get_body(&self, hash: &BlockHash) -> Option<BlockBody> {
        self.by_hash.get(hash).map(|b| b.body.clone())
    }

    pub fn get_number(&self, hash: &BlockHash) -> Option<BlockNumber> {
        self.by_hash.get(hash).map(|b| b.number)
    }

    pub fn get_receipt(&self, hash: &BlockHash, index: u64) -> Option<Receipt> {
        self.by_hash
            .get(hash)
            .and_then(|b| b.receipts.get(index as usize).cloned())
    }

    pub fn get_receipts(&self, hash: &BlockHash) -> Option<Vec<Receipt>> {
        self.by_hash.get(hash).map(|b| b.receipts.clone())
    }

    pub fn get_code(&self, code_hash: &H256) -> Option<Code> {
        self.codes.get(code_hash).map(|(c, _)| c.clone())
    }

    pub fn get_tx_locations(&self, tx_hash: &H256) -> Vec<(BlockNumber, BlockHash, u64)> {
        self.tx_index.get(tx_hash).cloned().unwrap_or_default()
    }

    /// Number of buffered block hashes not yet known to be on disk (may exceed unique block numbers under reorgs).
    pub fn unflushed_len(&self) -> usize {
        self.unflushed.len()
    }

    pub fn flushed_upto(&self) -> BlockNumber {
        self.flushed_upto
    }

    pub fn set_flushed_upto(&mut self, n: BlockNumber) {
        self.flushed_upto = self.flushed_upto.max(n);
    }

    /// All unflushed blocks, ascending by number (deterministic flush order).
    pub fn flushable(&self) -> Vec<Arc<BufferedBlock>> {
        let mut out: Vec<Arc<BufferedBlock>> = self
            .unflushed
            .iter()
            .filter_map(|h| self.by_hash.get(h).cloned())
            .collect();
        out.sort_by_key(|b| b.number);
        out
    }

    /// Codes introduced by the given block hashes (for the flush write tx).
    pub fn codes_for(&self, hashes: &[BlockHash]) -> Vec<(H256, Code)> {
        let numbers: FxHashSet<BlockNumber> =
            hashes.iter().filter_map(|h| self.get_number(h)).collect();
        self.codes
            .iter()
            .filter(|(_, (_, n))| numbers.contains(n))
            .map(|(h, (c, _))| (*h, c.clone()))
            .collect()
    }

    /// Drop all buffered data for blocks with number <= `new_flushed_upto` and
    /// advance the durable marker. Called only after the disk write committed.
    pub fn evict_flushed(&mut self, new_flushed_upto: BlockNumber) {
        self.set_flushed_upto(new_flushed_upto);
        let drop_hashes: Vec<BlockHash> = self
            .by_hash
            .iter()
            .filter(|(_, b)| b.number <= new_flushed_upto)
            .map(|(h, _)| *h)
            .collect();
        for h in &drop_hashes {
            if let Some(b) = self.by_hash.remove(h) {
                if let Some(v) = self.by_number.get_mut(&b.number) {
                    v.retain(|x| x != h);
                    if v.is_empty() {
                        self.by_number.remove(&b.number);
                    }
                }
                for tx in b.body.transactions.iter() {
                    if let Some(v) = self.tx_index.get_mut(&tx.hash()) {
                        v.retain(|(_, bh, _)| bh != h);
                        if v.is_empty() {
                            self.tx_index.remove(&tx.hash());
                        }
                    }
                }
            }
        }
        self.unflushed.retain(|h| self.by_hash.contains_key(h));
        self.codes.retain(|_, (_, n)| *n > new_flushed_upto);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::types::{Block, BlockBody, BlockHeader};

    fn block(number: u64) -> Block {
        let header = BlockHeader {
            number,
            ..Default::default()
        };
        Block::new(header, BlockBody::default())
    }

    #[test]
    fn insert_then_read_back() {
        let mut buf = BlockDataBuffer::new();
        let b = block(7);
        let hash = b.hash();
        buf.insert(b, vec![], vec![]);

        assert_eq!(buf.get_header(&hash).map(|h| h.number), Some(7));
        assert_eq!(buf.get_number(&hash), Some(7));
        assert!(buf.get_body(&hash).is_some());
        assert_eq!(buf.unflushed_len(), 1);
    }

    #[test]
    fn evict_advances_flushed_upto_and_drops_blocks() {
        let mut buf = BlockDataBuffer::new();
        let b = block(7);
        let hash = b.hash();
        buf.insert(b, vec![], vec![]);
        buf.set_flushed_upto(7);
        buf.evict_flushed(7);

        assert_eq!(buf.flushed_upto(), 7);
        assert_eq!(buf.unflushed_len(), 0);
        assert!(buf.get_header(&hash).is_none());
    }

    #[test]
    fn codes_for_returns_codes_for_given_block() {
        let mut buf = BlockDataBuffer::new();
        let b = block(9);
        let hash = b.hash();
        let code_hash = H256::repeat_byte(0xab);
        let code = Code::from_bytecode_unchecked(Bytes::from_static(&[0x60, 0x00]), code_hash);
        buf.insert(b, vec![], vec![(code_hash, code)]);

        assert_eq!(buf.get_code(&code_hash).map(|c| c.hash), Some(code_hash));
        let codes = buf.codes_for(&[hash]);
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].0, code_hash);

        buf.evict_flushed(9);
        assert!(buf.get_code(&code_hash).is_none());
    }

    #[test]
    fn reorg_siblings_coexist() {
        let mut buf = BlockDataBuffer::new();
        let mut a = block(5);
        let mut b = block(5);
        // Vary a header field so the two blocks at the same number hash differently.
        a.header.extra_data = Bytes::from_static(b"a");
        b.header.extra_data = Bytes::from_static(b"b");
        let hash_a = a.hash();
        let hash_b = b.hash();
        assert_ne!(hash_a, hash_b);

        buf.insert(a, vec![], vec![]);
        buf.insert(b, vec![], vec![]);

        assert!(buf.get_header(&hash_a).is_some());
        assert!(buf.get_header(&hash_b).is_some());
        assert_eq!(buf.unflushed_len(), 2);
    }
}
