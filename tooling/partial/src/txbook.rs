//! Transactions this node submitted: pending, located in a verified block, and their
//! verified receipts. A plain value owned by [`crate::state::State`]; no I/O.

use std::collections::{HashMap, VecDeque};

use ethrex_common::types::{Receipt, Transaction};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;

use crate::state::Store;
use crate::verify::{BlockNumHash, VerifiedHeader};

/// Most located transactions remembered.
pub const LOCATED_CAP: usize = 1024;

/// A submitted transaction not yet seen in a verified block.
#[derive(Clone, Debug)]
pub struct PendingTx {
    /// Transaction hash.
    pub hash: H256,
    /// Recovered sender.
    pub sender: Address,
    /// Sender nonce.
    pub nonce: u64,
    /// The transaction.
    pub tx: Transaction,
    /// The anchor at submission. Inclusion can only be in a later block.
    pub since: BlockNumHash,
}

/// A submitted transaction found in a verified block.
#[derive(Clone, Debug)]
pub struct Located {
    /// The block it was found in.
    pub block: BlockNumHash,
    /// Its index in that block.
    pub index: u64,
    /// Recovered sender.
    pub sender: Address,
    /// Sender nonce.
    pub nonce: u64,
    /// The transaction.
    pub tx: Transaction,
    /// The anchor at submission.
    pub since: BlockNumHash,
    /// Its receipt, once fetched and verified.
    pub receipt: Option<VerifiedReceipt>,
}

/// A receipt taken from a receipt list that matched its header's `receipts_root`, with the
/// per-transaction fields derived from its neighbours.
#[derive(Clone, Debug)]
pub struct VerifiedReceipt {
    block: BlockNumHash,
    block_timestamp: u64,
    index: u64,
    tx_hash: H256,
    receipt: Receipt,
    gas_used: u64,
    effective_gas_price: U256,
    first_log_index: u64,
}

impl VerifiedReceipt {
    pub(crate) fn new(
        header: &VerifiedHeader,
        index: u64,
        tx: &Transaction,
        receipt: &Receipt,
        gas_used: u64,
        first_log_index: u64,
    ) -> Self {
        Self {
            block: header.into(),
            block_timestamp: header.header().timestamp,
            index,
            tx_hash: tx.hash(&NativeCrypto),
            receipt: receipt.clone(),
            gas_used,
            effective_gas_price: tx
                .effective_gas_price(header.header().base_fee_per_gas)
                .unwrap_or_default(),
            first_log_index,
        }
    }

    /// The block the receipt belongs to.
    pub const fn block(&self) -> BlockNumHash {
        self.block
    }

    /// The block's timestamp.
    pub const fn block_timestamp(&self) -> u64 {
        self.block_timestamp
    }

    /// Transaction index in the block.
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// Transaction hash.
    pub const fn tx_hash(&self) -> H256 {
        self.tx_hash
    }

    /// The receipt as the block carries it.
    pub const fn receipt(&self) -> &Receipt {
        &self.receipt
    }

    /// Gas used by this transaction alone.
    pub const fn gas_used(&self) -> u64 {
        self.gas_used
    }

    /// The price paid per gas.
    pub const fn effective_gas_price(&self) -> U256 {
        self.effective_gas_price
    }

    /// Index of the first log of this transaction within the block.
    pub const fn first_log_index(&self) -> u64 {
        self.first_log_index
    }
}

/// A submitted transaction, wherever it is.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum SubmittedTx {
    /// Not seen in a verified block yet.
    Pending(PendingTx),
    /// Found in a verified block.
    Located(Located),
}

impl SubmittedTx {
    /// The transaction.
    pub fn tx(&self) -> &Transaction {
        match self {
            Self::Pending(p) => &p.tx,
            Self::Located(l) => &l.tx,
        }
    }

    /// The sender.
    pub fn sender(&self) -> Address {
        match self {
            Self::Pending(p) => p.sender,
            Self::Located(l) => l.sender,
        }
    }
}

/// How a submission changed the book.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submitted {
    /// A new pending transaction.
    New,
    /// It replaced a pending transaction with the same sender and nonce.
    Replaced,
    /// The same hash was already in the book.
    Duplicate,
}

/// Submitted transactions.
#[derive(Clone, Debug, Default)]
pub struct TxBook {
    pending: Vec<PendingTx>,
    located: HashMap<H256, Located>,
    /// Insertion order of `located`, oldest first, to cap it.
    located_order: VecDeque<H256>,
}

impl TxBook {
    /// Adds a pending transaction. A same-sender, same-nonce pending entry is replaced; a
    /// resubmission with the same hash is ignored. An older `since` is kept, since it only
    /// widens the search.
    pub fn submit(&mut self, tx: Transaction, sender: Address, since: BlockNumHash) -> Submitted {
        let hash = tx.hash(&NativeCrypto);
        if self.located.contains_key(&hash) {
            return Submitted::Duplicate;
        }
        let nonce = tx.nonce();
        if let Some(existing) = self.pending.iter_mut().find(|p| p.hash == hash) {
            if since.number < existing.since.number {
                existing.since = since;
            }
            return Submitted::Duplicate;
        }
        let mut outcome = Submitted::New;
        let mut since = since;
        self.pending.retain(|p| {
            let same_slot = p.sender == sender && p.nonce == nonce;
            if same_slot {
                outcome = Submitted::Replaced;
                if p.since.number < since.number {
                    since = p.since;
                }
            }
            !same_slot
        });
        self.pending.push(PendingTx {
            hash,
            sender,
            nonce,
            tx,
            since,
        });
        outcome
    }

    /// Pending transactions, oldest submission first.
    pub fn pending(&self) -> &[PendingTx] {
        &self.pending
    }

    /// Whether anything is pending.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// A submitted transaction by hash.
    pub fn get(&self, hash: H256) -> Option<SubmittedTx> {
        if let Some(located) = self.located.get(&hash) {
            return Some(SubmittedTx::Located(located.clone()));
        }
        self.pending
            .iter()
            .find(|p| p.hash == hash)
            .cloned()
            .map(SubmittedTx::Pending)
    }

    /// The verified receipt of a located transaction, if fetched.
    pub fn receipt(&self, hash: H256) -> Option<&VerifiedReceipt> {
        self.located.get(&hash).and_then(|l| l.receipt.as_ref())
    }

    /// The next nonce `sender` could use counting consecutive pending transactions from
    /// `anchor_nonce`, the nonce at the anchor.
    pub fn pending_nonce(&self, sender: Address, anchor_nonce: u64) -> u64 {
        let mut next = anchor_nonce;
        loop {
            let has_next = self
                .pending
                .iter()
                .any(|p| p.sender == sender && p.nonce == next);
            if !has_next {
                return next;
            }
            next = next.saturating_add(1);
        }
    }

    /// Matches pending transactions against the transactions of a verified block. Returns
    /// the hashes located.
    pub fn locate(&mut self, block: BlockNumHash, transactions: &[Transaction]) -> Vec<H256> {
        let mut found = Vec::new();
        for (index, tx) in transactions.iter().enumerate() {
            let hash = tx.hash(&NativeCrypto);
            let Some(pos) = self.pending.iter().position(|p| p.hash == hash) else {
                continue;
            };
            let pending = self.pending.remove(pos);
            let Ok(index) = u64::try_from(index) else {
                continue;
            };
            self.insert_located(Located {
                block,
                index,
                sender: pending.sender,
                nonce: pending.nonce,
                tx: pending.tx,
                since: pending.since,
                receipt: None,
            });
            found.push(hash);
        }
        found
    }

    fn insert_located(&mut self, located: Located) {
        let hash = located.tx.hash(&NativeCrypto);
        if self.located.insert(hash, located).is_none() {
            self.located_order.push_back(hash);
        }
        while self.located.len() > LOCATED_CAP {
            let Some(oldest) = self.located_order.pop_front() else {
                break;
            };
            self.located.remove(&oldest);
        }
    }

    /// Located transactions whose receipt has not been fetched, grouped by block.
    pub fn blocks_awaiting_receipts(&self) -> Vec<BlockNumHash> {
        let mut blocks: Vec<BlockNumHash> = self
            .located
            .values()
            .filter(|l| l.receipt.is_none())
            .map(|l| l.block)
            .collect();
        blocks.sort_by_key(|b| b.number);
        blocks.dedup();
        blocks
    }

    /// Located transactions in `block`, with their indices.
    pub fn located_in(&self, block: H256) -> Vec<(H256, u64)> {
        self.located
            .values()
            .filter(|l| l.block.hash == block)
            .map(|l| (l.tx.hash(&NativeCrypto), l.index))
            .collect()
    }

    /// Records the verified receipt of a located transaction.
    pub fn set_receipt(&mut self, hash: H256, receipt: VerifiedReceipt) {
        if let Some(located) = self.located.get_mut(&hash)
            && located.block == receipt.block()
        {
            located.receipt = Some(receipt);
        }
    }

    /// Drops pending transactions whose nonce the sender has passed at the anchor. The
    /// caller supplies `anchor_nonce` per sender for senders it holds; others are kept.
    pub fn prune_replaced(&mut self, anchor_nonce: impl Fn(Address) -> Option<u64>) -> Vec<H256> {
        let mut dropped = Vec::new();
        self.pending.retain(|p| {
            let replaced = anchor_nonce(p.sender).is_some_and(|nonce| nonce > p.nonce);
            if replaced {
                dropped.push(p.hash);
            }
            !replaced
        });
        dropped
    }

    /// Located transactions whose block is above the anchor, or whose block number holds
    /// a different header, return to pending without a receipt. Blocks older than the held
    /// window are kept, since the store cannot judge them.
    pub fn revalidate(&mut self, store: &Store) {
        let anchor = store.anchor().number();
        let orphaned: Vec<H256> = self
            .located
            .iter()
            .filter(|(_, l)| {
                l.block.number > anchor
                    || store
                        .header(l.block.number)
                        .is_some_and(|held| held.hash() != l.block.hash)
            })
            .map(|(hash, _)| *hash)
            .collect();
        for hash in orphaned {
            let Some(located) = self.located.remove(&hash) else {
                continue;
            };
            self.located_order.retain(|h| *h != hash);
            self.pending.push(PendingTx {
                hash,
                sender: located.sender,
                nonce: located.nonce,
                tx: located.tx,
                since: located.since,
            });
        }
    }
}
