//! Mempool-driven state pre-warming (PoC).
//!
//! After each imported block, speculatively executes top-of-mempool
//! transactions against the new head state during the idle inter-slot gap, so
//! the next block's state reads hit warm persistent caches (RocksDB block
//! cache, code cache). Read-only and throwaway: speculative results are
//! discarded; a wrong prediction costs wasted I/O, never incorrect state.
//! The pass is cancelled the moment the next block arrives and never runs
//! past the next slot boundary.

use ethrex_common::types::{Block, MempoolTransaction, Transaction};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;

/// The next block's slot starts exactly one slot after its parent's; it cannot
/// arrive before that. Warming must end at this boundary.
pub fn next_slot_deadline_unix(parent_timestamp: u64, slot_duration_secs: u64) -> u64 {
    parent_timestamp.saturating_add(slot_duration_secs)
}

/// Picks the warm set from a mempool snapshot: whole sender groups (nonce
/// order preserved), groups ordered by their head tx's effective tip, until
/// `gas_budget` (sum of gas limits) is crossed. Ordering fidelity does not
/// matter for warming, only membership — hence whole groups, no interleaving.
pub fn select_warm_set(
    txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>>,
    base_fee: Option<u64>,
    gas_budget: u64,
) -> Vec<(Transaction, Address)> {
    let mut groups: Vec<(U256, Address, Vec<MempoolTransaction>)> = txs_by_sender
        .into_iter()
        .filter_map(|(sender, txs)| {
            let tip = txs.first()?.transaction().effective_gas_tip(base_fee)?;
            Some((tip, sender, txs))
        })
        .collect();
    groups.sort_by(|a, b| b.0.cmp(&a.0));

    let mut out = Vec::new();
    let mut gas_acc: u64 = 0;
    'outer: for (_, sender, txs) in groups {
        for mtx in txs {
            gas_acc = gas_acc.saturating_add(mtx.transaction().gas_limit());
            out.push((mtx.transaction().clone(), sender));
            if gas_acc >= gas_budget {
                break 'outer;
            }
        }
    }
    out
}

/// How much of an arrived block the last warming pass covered. `warmed` maps
/// warmed tx hash -> gas limit.
#[derive(Debug)]
pub struct OverlapStats {
    pub matched_txs: usize,
    pub block_txs: usize,
    pub matched_gas: u64,
    pub block_gas: u64,
}

pub fn compute_overlap(warmed: &FxHashMap<H256, u64>, block: &Block) -> OverlapStats {
    let mut stats = OverlapStats {
        matched_txs: 0,
        block_txs: block.body.transactions.len(),
        matched_gas: 0,
        block_gas: 0,
    };
    for tx in &block.body.transactions {
        let gas = tx.gas_limit();
        stats.block_gas = stats.block_gas.saturating_add(gas);
        if warmed.contains_key(&tx.hash(&NativeCrypto)) {
            stats.matched_txs += 1;
            stats.matched_gas = stats.matched_gas.saturating_add(gas);
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::Address;
    use ethrex_common::types::{EIP1559Transaction, MempoolTransaction, Transaction};
    use rustc_hash::FxHashMap;

    fn make_tx(
        sender_byte: u8,
        nonce: u64,
        max_fee: u64,
        tip: u64,
        gas_limit: u64,
    ) -> (Address, MempoolTransaction) {
        let sender = Address::repeat_byte(sender_byte);
        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            nonce,
            gas_limit,
            max_fee_per_gas: max_fee,
            max_priority_fee_per_gas: tip,
            ..Default::default()
        });
        (sender, MempoolTransaction::new(tx, sender))
    }

    #[test]
    fn deadline_is_next_slot_boundary() {
        assert_eq!(next_slot_deadline_unix(1_700_000_000, 12), 1_700_000_012);
    }

    #[test]
    fn select_prefers_higher_tip_sender_and_respects_budget() {
        let (a, tx_a) = make_tx(0xaa, 0, 100, 50, 21_000); // high tip
        let (b, tx_b) = make_tx(0xbb, 0, 100, 10, 21_000); // low tip
        let mut map = FxHashMap::default();
        map.insert(a, vec![tx_a]);
        map.insert(b, vec![tx_b]);
        // Budget only fits one tx: the high-tip sender must win.
        let set = select_warm_set(map, Some(1), 21_000);
        assert_eq!(set.len(), 1);
        assert_eq!(set[0].1, a);
    }

    #[test]
    fn select_keeps_nonce_order_within_sender() {
        let (a, tx0) = make_tx(0xaa, 0, 100, 50, 21_000);
        let (_, tx1) = make_tx(0xaa, 1, 100, 50, 21_000);
        let mut map = FxHashMap::default();
        map.insert(a, vec![tx0, tx1]);
        let set = select_warm_set(map, Some(1), 1_000_000);
        assert_eq!(set.len(), 2);
        assert_eq!(set[0].0.nonce(), 0);
        assert_eq!(set[1].0.nonce(), 1);
    }

    #[test]
    fn select_includes_budget_crossing_tx_then_stops() {
        let (a, tx0) = make_tx(0xaa, 0, 100, 50, 30_000);
        let (_, tx1) = make_tx(0xaa, 1, 100, 50, 30_000);
        let (_, tx2) = make_tx(0xaa, 2, 100, 50, 30_000);
        let mut map = FxHashMap::default();
        map.insert(a, vec![tx0, tx1, tx2]);
        // Budget 40k: tx0 (30k) is under, tx1 crosses to 60k and is included, tx2 is not.
        let set = select_warm_set(map, Some(1), 40_000);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn overlap_counts_matched_txs_and_gas() {
        use ethrex_common::types::{Block, BlockBody, BlockHeader};
        use ethrex_crypto::NativeCrypto;
        let (_, mtx) = make_tx(0xaa, 0, 100, 50, 21_000);
        let tx = mtx.transaction().clone();
        let mut warmed = FxHashMap::default();
        warmed.insert(tx.hash(&NativeCrypto), tx.gas_limit());
        let block = Block::new(
            BlockHeader::default(),
            BlockBody {
                transactions: vec![tx],
                ommers: vec![],
                withdrawals: None,
            },
        );
        let s = compute_overlap(&warmed, &block);
        assert_eq!(s.matched_txs, 1);
        assert_eq!(s.block_txs, 1);
        assert_eq!(s.matched_gas, 21_000);
        assert_eq!(s.block_gas, 21_000);
    }
}
