use ethrex_common::{Address, H256};
use rustc_hash::{FxHashMap, FxHashSet};

/// EIP-8037 per-frame state-diff journal.
///
/// Tracks all state-growth events that occurred within a single call frame.
/// On successful return, the child's diff is merged into the parent via
/// [`StateDiff::merge_from_child`]. On revert, the diff is discarded.
/// Block-level state gas is computed from the tx-level finalized diff
/// (`VM::state_diff_finalized.bytes() * cost_per_state_byte`).
#[derive(Debug, Clone, Default)]
pub struct StateDiff {
    /// Addresses that this frame has created (CREATE/CREATE2/CALL-with-value to empty).
    pub new_accounts: FxHashSet<Address>,
    /// Storage slots whose original value was 0 and were set to nonzero (charged STATE_BYTES_PER_STORAGE_SET).
    pub new_storage_slots: FxHashSet<(Address, H256)>,
    /// Per-address deployed-code byte counts (charged at code-deposit step in CREATE).
    pub code_deposits: FxHashMap<Address, u64>,
    /// EIP-7702 auth-total entries: one entry per auth tuple that passed ecrecover.
    /// Each charges `STATE_BYTES_PER_NEW_ACCOUNT + STATE_BYTES_PER_AUTH_BASE` bytes
    /// (worst-case full new-account + auth-base). Duplicates are intentional: EIP-8037
    /// block accounting charges per tuple, even when multiple tuples share an authority.
    /// User-side refunds (pre-existing authority, etc.) are applied via `state_gas_reservoir`,
    /// not by removing entries here.
    pub auth_total: Vec<Address>,
    /// EIP-7702 auth-only entries: authority address → `STATE_BYTES_PER_AUTH_BASE` bytes
    /// (downgraded — authority pre-existed). Currently unused in production code; the
    /// downgrade is handled via `state_gas_reservoir` so block accounting stays at the
    /// worst case. Kept for the unit-test coverage of `record_auth_downgrade_to_only`.
    pub auth_only: Vec<Address>,

    /// Cross-frame cancellations: storage slots cleared (N→0) but created in an ancestor.
    /// Resolved on merge_from_child by removing from parent/ancestor's new_storage_slots.
    pub cancellations_storage: FxHashSet<(Address, H256)>,
    /// Cross-frame cancellations: accounts selfdestructed but created in an ancestor.
    /// Resolved on merge_from_child by removing from parent/ancestor's new_accounts (and slots/code).
    pub cancellations_account: FxHashSet<Address>,
}

impl StateDiff {
    /// Compute the total state bytes this diff represents.
    #[expect(
        clippy::as_conversions,
        reason = "HashSet::len() returns usize; saturating_mul caps the result — safe narrowing"
    )]
    pub fn bytes(&self) -> u64 {
        use crate::gas_cost::{
            STATE_BYTES_PER_AUTH_BASE, STATE_BYTES_PER_NEW_ACCOUNT, STATE_BYTES_PER_STORAGE_SET,
        };
        // EIP-8037 ethereum/EIPs#11573: a fresh 7702 authorization charges
        // STATE_BYTES_PER_NEW_ACCOUNT + STATE_BYTES_PER_AUTH_BASE per tuple. The block
        // header reports this worst-case sum; user-side refunds flow via
        // `state_gas_reservoir` rather than by removing entries here.
        let auth_total_bytes =
            STATE_BYTES_PER_NEW_ACCOUNT.saturating_add(STATE_BYTES_PER_AUTH_BASE);

        (self.new_accounts.len() as u64)
            .saturating_mul(STATE_BYTES_PER_NEW_ACCOUNT)
            .saturating_add(
                (self.new_storage_slots.len() as u64).saturating_mul(STATE_BYTES_PER_STORAGE_SET),
            )
            .saturating_add(
                self.code_deposits
                    .values()
                    .copied()
                    .fold(0u64, u64::saturating_add),
            )
            .saturating_add((self.auth_total.len() as u64).saturating_mul(auth_total_bytes))
            .saturating_add((self.auth_only.len() as u64).saturating_mul(STATE_BYTES_PER_AUTH_BASE))
    }

    // -------------------------------------------------------------------------
    // Recording API
    // -------------------------------------------------------------------------

    pub fn record_new_account(&mut self, addr: Address) {
        self.new_accounts.insert(addr);
    }

    pub fn record_new_storage_slot(&mut self, addr: Address, key: H256) {
        self.new_storage_slots.insert((addr, key));
    }

    pub fn record_code_deposit(&mut self, addr: Address, code_len: u64) {
        self.code_deposits
            .entry(addr)
            .and_modify(|n| *n = n.saturating_add(code_len))
            .or_insert(code_len);
    }

    pub fn record_auth_total(&mut self, authority: Address) {
        self.auth_total.push(authority);
    }

    /// Move one occurrence of `authority` from `auth_total` to `auth_only`.
    pub fn record_auth_downgrade_to_only(&mut self, authority: Address) {
        if let Some(idx) = self.auth_total.iter().position(|a| *a == authority) {
            self.auth_total.swap_remove(idx);
            self.auth_only.push(authority);
        }
    }

    // -------------------------------------------------------------------------
    // Cancellation API
    // -------------------------------------------------------------------------

    /// Cancel a slot creation. If created in this frame's diff, remove directly.
    /// Otherwise queue for cross-frame resolution at merge_from_child.
    pub fn cancel_storage_slot(&mut self, addr: Address, key: H256) {
        if !self.new_storage_slots.remove(&(addr, key)) {
            self.cancellations_storage.insert((addr, key));
        }
    }

    /// Cancel an account creation. If created in this frame's diff, remove account + its slots + code_deposit.
    /// Otherwise queue for cross-frame resolution at merge_from_child.
    pub fn cancel_new_account(&mut self, addr: Address) {
        if self.new_accounts.remove(&addr) {
            self.new_storage_slots.retain(|(a, _)| *a != addr);
            self.code_deposits.remove(&addr);
        } else {
            self.cancellations_account.insert(addr);
        }
    }

    // -------------------------------------------------------------------------
    // Merge
    // -------------------------------------------------------------------------

    /// Merge a successful child frame's diff into self (parent).
    ///
    /// `ancestors` is the slice of older frames in the call stack (call_frames[0..parent]),
    /// passed in stack order (oldest first). The cancellation search iterates ancestors
    /// in REVERSE order so the youngest ancestor is checked first — required for cross-frame
    /// cancellation resolution. Pass an empty slice if self is the only frame above child.
    ///
    /// Algorithm:
    ///   1. Apply child's cancellations: search self FIRST, then ancestors youngest-first.
    ///      Idempotent: if not found anywhere, propagate up so a higher merge can resolve.
    ///   2. Set-union the rest: child's new_accounts/new_storage_slots/auth_total/auth_only into self.
    ///   3. Sum-merge code_deposits.
    pub fn merge_from_child(&mut self, mut child: StateDiff, ancestors: &mut [StateDiff]) -> u64 {
        use crate::gas_cost::{STATE_BYTES_PER_NEW_ACCOUNT, STATE_BYTES_PER_STORAGE_SET};
        // Bytes whose state-gas charges are now stranded — the corresponding records
        // are being removed by this merge — and should be refunded to the reservoir
        // by the caller. Same-frame cancellations don't go through here (they remove
        // locally and refund inline at the SSTORE/CREATE handler), so this only
        // counts cross-frame resolutions.
        let mut refundable_bytes: u64 = 0;

        // 1a. Storage cancellations
        for (addr, key) in child.cancellations_storage.iter() {
            if self.new_storage_slots.remove(&(*addr, *key)) {
                refundable_bytes = refundable_bytes.saturating_add(STATE_BYTES_PER_STORAGE_SET);
                continue;
            }
            let mut handled = false;
            for ancestor in ancestors.iter_mut().rev() {
                if ancestor.new_storage_slots.remove(&(*addr, *key)) {
                    refundable_bytes = refundable_bytes.saturating_add(STATE_BYTES_PER_STORAGE_SET);
                    handled = true;
                    break;
                }
            }
            if !handled {
                // Propagate up so a higher merge can resolve.
                self.cancellations_storage.insert((*addr, *key));
            }
        }

        // 1b. Account cancellations
        for addr in child.cancellations_account.iter() {
            let found_in_self = self.new_accounts.remove(addr);
            if found_in_self {
                refundable_bytes = refundable_bytes.saturating_add(STATE_BYTES_PER_NEW_ACCOUNT);
                #[expect(clippy::as_conversions, reason = "filter().count() bounded")]
                let slot_count = self
                    .new_storage_slots
                    .iter()
                    .filter(|(a, _)| a == addr)
                    .count() as u64;
                refundable_bytes = refundable_bytes
                    .saturating_add(slot_count.saturating_mul(STATE_BYTES_PER_STORAGE_SET));
                if let Some(code_len) = self.code_deposits.remove(addr) {
                    refundable_bytes = refundable_bytes.saturating_add(code_len);
                }
                self.new_storage_slots.retain(|(a, _)| a != addr);
                continue;
            }
            let mut handled = false;
            for ancestor in ancestors.iter_mut().rev() {
                if ancestor.new_accounts.remove(addr) {
                    refundable_bytes = refundable_bytes.saturating_add(STATE_BYTES_PER_NEW_ACCOUNT);
                    #[expect(clippy::as_conversions, reason = "filter().count() bounded")]
                    let slot_count = ancestor
                        .new_storage_slots
                        .iter()
                        .filter(|(a, _)| a == addr)
                        .count() as u64;
                    refundable_bytes = refundable_bytes
                        .saturating_add(slot_count.saturating_mul(STATE_BYTES_PER_STORAGE_SET));
                    if let Some(code_len) = ancestor.code_deposits.remove(addr) {
                        refundable_bytes = refundable_bytes.saturating_add(code_len);
                    }
                    ancestor.new_storage_slots.retain(|(a, _)| a != addr);
                    handled = true;
                    break;
                }
            }
            if !handled {
                // Propagate up so a higher merge can resolve.
                self.cancellations_account.insert(*addr);
            }
        }

        // 1c. Scrub the child's own slots/code_deposits for cancelled accounts so the
        // set-union below cannot reintroduce them. Without this, a same-tx
        // CREATE+SSTORE-in-init+SELFDESTRUCT pattern leaks the init's slots back into
        // the parent after the cancellation removed the new_account record. Bytes
        // dropped here also need to be refunded to the reservoir (the state-gas was
        // drawn at SSTORE / code-deposit time but the state effect no longer exists).
        let cancelled_accounts: Vec<Address> =
            child.cancellations_account.iter().copied().collect();
        for addr in &cancelled_accounts {
            #[expect(clippy::as_conversions, reason = "filter().count() bounded")]
            let slot_count = child
                .new_storage_slots
                .iter()
                .filter(|(a, _)| a == addr)
                .count() as u64;
            refundable_bytes = refundable_bytes
                .saturating_add(slot_count.saturating_mul(STATE_BYTES_PER_STORAGE_SET));
            if let Some(code_len) = child.code_deposits.remove(addr) {
                refundable_bytes = refundable_bytes.saturating_add(code_len);
            }
            child.new_storage_slots.retain(|(a, _)| a != addr);
        }

        // 2. Set-union for HashSets, Vec-extend for auth lists (per-tuple worst case).
        self.new_accounts.extend(child.new_accounts);
        self.new_storage_slots.extend(child.new_storage_slots);
        self.auth_total.extend(child.auth_total);
        self.auth_only.extend(child.auth_only);
        // Mutual-exclusion sweep: for each occurrence in `auth_only`, drop one matching
        // occurrence from `auth_total` so a downgrade isn't double-counted.
        for addr in self.auth_only.clone() {
            if let Some(idx) = self.auth_total.iter().position(|a| *a == addr) {
                self.auth_total.swap_remove(idx);
            }
        }

        // 3. Sum-merge code_deposits
        for (addr, len) in child.code_deposits {
            self.code_deposits
                .entry(addr)
                .and_modify(|n| *n = n.saturating_add(len))
                .or_insert(len);
        }

        refundable_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H256;

    fn addr(n: u64) -> Address {
        Address::from_low_u64_be(n)
    }
    fn key(n: u64) -> H256 {
        H256::from_low_u64_be(n)
    }

    #[test]
    fn record_dedup() {
        let mut d = StateDiff::default();
        d.record_new_account(addr(1));
        d.record_new_account(addr(1));
        assert_eq!(d.new_accounts.len(), 1);
    }

    #[test]
    fn bytes_empty() {
        assert_eq!(StateDiff::default().bytes(), 0);
    }

    #[test]
    fn bytes_one_account() {
        let mut d = StateDiff::default();
        d.record_new_account(addr(1));
        assert_eq!(d.bytes(), 112);
    }

    #[test]
    fn bytes_one_slot() {
        let mut d = StateDiff::default();
        d.record_new_storage_slot(addr(1), key(1));
        assert_eq!(d.bytes(), 32);
    }

    #[test]
    fn bytes_auth_total_then_downgrade() {
        let mut d = StateDiff::default();
        d.record_auth_total(addr(1));
        assert_eq!(d.bytes(), 135);
        d.record_auth_downgrade_to_only(addr(1));
        assert_eq!(d.bytes(), 23);
    }

    #[test]
    fn cancel_local_account_clears_slots_and_code() {
        let mut d = StateDiff::default();
        d.record_new_account(addr(1));
        d.record_new_storage_slot(addr(1), key(1));
        d.record_new_storage_slot(addr(1), key(2));
        d.record_code_deposit(addr(1), 100);
        d.cancel_new_account(addr(1));
        assert_eq!(d.bytes(), 0);
        assert!(d.new_accounts.is_empty());
        assert!(d.new_storage_slots.is_empty());
        assert!(d.code_deposits.is_empty());
    }

    #[test]
    fn cancel_local_slot() {
        let mut d = StateDiff::default();
        d.record_new_storage_slot(addr(1), key(1));
        d.cancel_storage_slot(addr(1), key(1));
        assert_eq!(d.bytes(), 0);
    }

    #[test]
    fn cancel_unknown_slot_queues_for_cross_frame() {
        let mut d = StateDiff::default();
        d.cancel_storage_slot(addr(1), key(1));
        assert_eq!(d.cancellations_storage.len(), 1);
    }

    #[test]
    fn merge_direct_parent_storage_cancellation() {
        // Parent owns slot. Child cancels it. After merge, parent slot gone.
        let mut parent = StateDiff::default();
        parent.record_new_storage_slot(addr(1), key(1));
        let mut child = StateDiff::default();
        child.cancel_storage_slot(addr(1), key(1));
        parent.merge_from_child(child, &mut []);
        assert_eq!(parent.bytes(), 0);
    }

    #[test]
    fn merge_deep_ancestor_storage_cancellation() {
        // grandparent owns slot, child cancels via merge into parent's diff with ancestors=[grandparent].
        let mut grandparent = StateDiff::default();
        grandparent.record_new_storage_slot(addr(1), key(1));
        let mut parent = StateDiff::default();
        let mut ancestors = [grandparent];
        parent.merge_from_child(
            {
                let mut c = StateDiff::default();
                c.cancel_storage_slot(addr(1), key(1));
                c
            },
            &mut ancestors,
        );
        assert!(ancestors[0].new_storage_slots.is_empty());
    }

    #[test]
    fn merge_idempotent_absent_key() {
        // Child cancels a slot that doesn't exist anywhere.
        let mut parent = StateDiff::default();
        let mut child = StateDiff::default();
        child.cancel_storage_slot(addr(1), key(1));
        parent.merge_from_child(child, &mut []);
        // Cancellation propagates up so a higher merge can resolve.
        assert_eq!(parent.cancellations_storage.len(), 1);
    }

    #[test]
    fn merge_account_cancellation_clears_slots_in_ancestor() {
        let mut grandparent = StateDiff::default();
        grandparent.record_new_account(addr(1));
        grandparent.record_new_storage_slot(addr(1), key(1));
        grandparent.record_new_storage_slot(addr(1), key(2));
        grandparent.record_code_deposit(addr(1), 50);
        let mut parent = StateDiff::default();
        let mut child = StateDiff::default();
        child.cancel_new_account(addr(1));
        let mut ancestors = [grandparent];
        parent.merge_from_child(child, &mut ancestors);
        assert!(ancestors[0].new_accounts.is_empty());
        assert!(ancestors[0].new_storage_slots.is_empty());
        assert!(ancestors[0].code_deposits.is_empty());
    }

    #[test]
    fn bytes_code_deposit() {
        let mut d = StateDiff::default();
        d.record_code_deposit(addr(1), 100);
        assert_eq!(d.bytes(), 100);
        // Idempotent sum-merge: a second deposit on the same address adds.
        d.record_code_deposit(addr(1), 50);
        assert_eq!(d.bytes(), 150);
    }

    #[test]
    fn merge_auth_only_takes_precedence_over_auth_total() {
        // Parent has authority in auth_only (downgraded). Child re-records in auth_total.
        // After merge, auth_only must win (monotonic downgrade) — no double-count.
        let mut parent = StateDiff::default();
        parent.record_auth_total(addr(1));
        parent.record_auth_downgrade_to_only(addr(1));
        assert_eq!(parent.bytes(), 23);
        let mut child = StateDiff::default();
        child.record_auth_total(addr(1));
        parent.merge_from_child(child, &mut []);
        // Must remain 23, not 23 + 135 = 158.
        assert_eq!(parent.bytes(), 23);
        assert!(!parent.auth_total.contains(&addr(1)));
        assert!(parent.auth_only.contains(&addr(1)));
    }

    #[test]
    fn merge_set_union() {
        let mut parent = StateDiff::default();
        parent.record_new_account(addr(1));
        let mut child = StateDiff::default();
        child.record_new_account(addr(2));
        child.record_new_storage_slot(addr(2), key(1));
        parent.merge_from_child(child, &mut []);
        assert_eq!(parent.new_accounts.len(), 2);
        assert_eq!(parent.new_storage_slots.len(), 1);
    }
}
