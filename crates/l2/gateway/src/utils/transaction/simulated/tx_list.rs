use ethrex_common::{Address, H256, U256};

use crate::utils::transaction::{simulated::transaction::SimulatedTx, Transaction, TxList};
use std::sync::Arc;

/// Current contains the current active tx for this sender.
/// i.e., current.nonce = state[address].nonce.
/// Pending contains all other txs for this sender in nonce order.
#[derive(Clone, Debug)]
pub struct SimulatedTxList {
    pub current: Option<SimulatedTx>,
    pub pending: TxList,
}

impl SimulatedTxList {
    /// Takes a TxList containing all txs for a sender and the simulated tx of the first tx in pending
    /// and returns a SimulatedTxList.
    ///
    /// Will optionally trim the current tx from the pending list.
    pub fn new(current: Option<SimulatedTx>, pending: &TxList) -> SimulatedTxList {
        let mut pending = pending.clone();

        // Remove current from pending, if it exists
        if let Some(ref current) = current {
            if pending
                .peek_nonce()
                .is_some_and(|nonce| current.nonce() == nonce)
            {
                pending.pop_front();
            }

            debug_assert!(
                pending
                    .peek_nonce()
                    .map_or(true, |nonce| current.nonce() == nonce + 1),
                "pending tx list nonce must be consecutive from current"
            );
        }

        SimulatedTxList { current, pending }
    }

    /// Updates the pending tx list.
    /// If current is the same as the first pending we advance the pending.
    #[inline]
    pub fn new_pending(&mut self, mut pending: TxList) {
        if let Some(current) = &self.current {
            if pending
                .peek_nonce()
                .is_some_and(|nonce| current.nonce() == nonce)
            {
                pending.pop_front();
            }
        }
        self.pending = pending;
    }

    pub fn len(&self) -> usize {
        self.pending.len() + self.current.is_some() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn hash(&self) -> H256 {
        self.current
            .as_ref()
            .map(|t| t.tx_hash())
            .unwrap_or_else(|| self.pending.tx_hash())
    }

    /// Removes the active transaction for the sender from the list.
    /// Returns true if all transactions for this sender have now been applied.
    pub fn pop(&mut self, base_fee: u64) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        if let Some(nonce) = self.current.take().map(|t| t.nonce()) {
            self.pending.first_ready(nonce + 1, base_fee).is_none()
        } else {
            self.pending
                .peek()
                .is_none_or(|t| base_fee > t.max_fee_per_gas().unwrap())
        }
    }

    pub fn put(&mut self, tx: SimulatedTx) {
        self.current = Some(tx);
    }

    pub fn next_to_sim(&mut self) -> Option<Arc<Transaction>> {
        self.current
            .as_ref()
            .map(|t| t.tx.clone())
            .or_else(|| self.pending.pop_front())
    }

    pub fn sender(&self) -> Address {
        self.pending.sender()
    }

    pub fn nonce(&self) -> u64 {
        if let Some(tx) = &self.current {
            tx.nonce()
        } else {
            self.pending.peek_nonce().unwrap_or_default()
        }
    }

    pub fn push(&mut self, tx: Arc<Transaction>) {
        debug_assert_eq!(
            tx.sender(),
            self.sender(),
            "shouldn't ever push a tx from a different sender: current {} vs pushed {}. txhash: {}",
            self.sender(),
            tx.sender(),
            tx.tx_hash()
        );
        self.pending.push(tx);
    }

    #[inline]
    pub fn weight(&self) -> U256 {
        if let Some(tx) = &self.current {
            return tx.payment;
        }
        if let Some(tx) = self.pending.peek() {
            debug_assert!(!tx.is_deposit(), "tx should never be deposit");
            return U256::from(tx.priority_fee_or_price());
        }
        U256::ZERO
    }

    pub fn payment(&self) -> U256 {
        self.current.as_ref().map(|c| c.payment).unwrap_or_default()
    }

    pub fn gas_limit(&self) -> Option<u64> {
        self.current
            .as_ref()
            .map(|c| c.gas_limit())
            .or_else(|| self.pending.peek().map(|t| t.gas_limit()))
    }
}

impl From<SimulatedTx> for SimulatedTxList {
    fn from(current: SimulatedTx) -> Self {
        let sender = current.sender();
        Self {
            current: Some(current),
            pending: TxList::empty_for_sender(sender),
        }
    }
}
