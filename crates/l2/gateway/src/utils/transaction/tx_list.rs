use crate::utils::transaction::Transaction;
use ethrex_common::Address;
use std::{collections::VecDeque, ops::Deref, sync::Arc};

/// A nonce-sorted list of transactions from a single sender.
#[derive(Clone, Debug)]
pub struct TxList {
    sender: Address,
    txs: VecDeque<Arc<Transaction>>,
}

impl TxList {
    #[inline]
    pub fn empty_for_sender(sender: Address) -> Self {
        debug_assert_ne!(
            sender,
            Address::default(),
            "should never have sender {sender}"
        );
        Self {
            sender,
            txs: VecDeque::new(),
        }
    }

    #[inline]
    pub fn sender(&self) -> Address {
        self.sender
    }

    /// Inserts a new transaction into the ordered list, maintaining nonce order.
    /// If a transaction already exists with the same nonce, it's overwritten.
    #[inline]
    pub fn put(&mut self, new_tx: Arc<Transaction>) {
        let new_nonce = new_tx.nonce();

        if self.txs.is_empty() || self.txs[self.txs.len() - 1].nonce() < new_nonce {
            self.txs.push_back(new_tx);
            return;
        }

        match self.txs.binary_search_by_key(&new_nonce, |tx| tx.nonce()) {
            Ok(index) => self.txs[index] = new_tx,
            Err(index) => self.txs.insert(index, new_tx),
        }
    }

    /// Removes all transactions with nonce lower or equal than the provided threshold.
    /// Returns true if list becomes empty after removal.
    #[inline]
    pub fn forward(&mut self, nonce: u64) -> bool {
        if self.txs.front().is_some_and(|t| t.nonce() > nonce) {
            return false;
        }
        while let Some(t) = self.txs.pop_front() {
            if t.nonce() == nonce {
                break;
            }
        }
        self.is_empty()
    }

    /// Ready retrieves a sequentially increasing list of transactions starting at the
    /// provided nonce that is ready for processing. Only txs with gas_price > base_fee
    /// are included.
    #[inline]
    pub fn ready(&self, mut curr_nonce: u64, base_fee: u64) -> Option<Self> {
        if self.is_empty() || self.peek_nonce().unwrap() > curr_nonce {
            return None;
        }

        let mut ready_txs = Self::empty_for_sender(self.sender);
        for next_tx in self.iter() {
            if next_tx.nonce() != curr_nonce || !next_tx.valid_for_block(base_fee) {
                break;
            }

            curr_nonce += 1;
            ready_txs.push(next_tx.clone());
        }

        if ready_txs.is_empty() {
            return None;
        }

        Some(ready_txs)
    }

    /// Returns Some(tx) if the next tx is ready for processing.
    /// Checks ready for processing through:
    /// 1. nonce == curr_nonce
    /// 2. gas_price >= base_fee
    #[inline]
    pub fn first_ready(&self, curr_nonce: u64, base_fee: u64) -> Option<&Arc<Transaction>> {
        let next_tx = self.peek()?;
        if next_tx.nonce() != curr_nonce || !next_tx.valid_for_block(base_fee) {
            return None;
        }

        Some(next_tx)
    }

    /// Returns whether a specific target nonce can be processed this block.
    /// Checks for consecutive transactions from curr_nonce up to target_nonce,
    /// ensuring all transactions in between have sufficient gas price to cover the base fee.
    #[inline]
    pub fn nonce_ready(&self, mut curr_nonce: u64, base_fee: u64, target_nonce: u64) -> bool {
        if target_nonce < curr_nonce {
            return false;
        }

        for tx in self.iter() {
            if tx.nonce() != curr_nonce || !tx.valid_for_block(base_fee) {
                return false;
            }

            if curr_nonce == target_nonce {
                return true;
            }

            curr_nonce += 1;
        }

        false
    }

    /// Returns effective gas price at base_fee for tx with given nonce, or 0 if not found
    #[inline]
    pub fn get_effective_price_for_nonce(&self, nonce: &u64, base_fee: u64) -> u128 {
        self.get(nonce)
            .map_or(0, |tx| tx.effective_gas_price(Some(base_fee)))
    }

    /// Retrieves a transaction with the given nonce from the transaction list.
    /// Returns a reference to the transaction if it exists.
    #[inline]
    pub fn get(&self, nonce: &u64) -> Option<&Arc<Transaction>> {
        self.txs
            .binary_search_by_key(nonce, |tx| tx.nonce())
            .ok()
            .map(|index| &self.txs[index])
    }

    /// Pushes a transaction onto the end of the list.
    #[inline]
    pub fn push(&mut self, tx: Arc<Transaction>) {
        self.txs.push_back(tx);
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<Transaction>> {
        self.txs.iter()
    }

    /// Returns the nonce of the first transaction in the list
    pub fn peek_nonce(&self) -> Option<u64> {
        self.txs.front().map(|tx| tx.tx.nonce())
    }

    /// Returns a reference to the first transaction in the list
    pub fn peek(&self) -> Option<&Arc<Transaction>> {
        self.txs.front()
    }

    pub fn contains(&self, nonce: &u64) -> bool {
        self.txs
            .binary_search_by_key(nonce, |tx| tx.nonce())
            .is_ok()
    }

    pub(crate) fn pop_front(&mut self) -> Option<Arc<Transaction>> {
        self.txs.pop_front()
    }
}

impl From<Arc<Transaction>> for TxList {
    fn from(tx: Arc<Transaction>) -> Self {
        let sender = tx.sender();
        debug_assert_ne!(
            sender,
            Address::default(),
            "should never have sender {sender}"
        );
        Self {
            sender,
            txs: VecDeque::from(vec![tx]),
        }
    }
}

impl From<Vec<Arc<Transaction>>> for TxList {
    fn from(txs: Vec<Arc<Transaction>>) -> Self {
        let sender = txs[0].sender();
        debug_assert_ne!(
            sender,
            Address::default(),
            "should never have sender {sender}"
        );
        Self {
            sender,
            txs: VecDeque::from(txs),
        }
    }
}

impl Deref for TxList {
    type Target = Arc<Transaction>;

    fn deref(&self) -> &Self::Target {
        &self.txs[0]
    }
}
