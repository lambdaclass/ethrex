//! Dual-execution validation for JIT-compiled code.
//!
//! When validation mode is active, the VM runs both JIT and interpreter on the
//! same input state and compares their outcomes. Mismatches trigger cache
//! invalidation and fallback to the interpreter result.

use crate::db::gen_db::CacheDB;
use crate::errors::{ContextResult, TxResult};

/// Result of comparing JIT execution against interpreter execution.
#[derive(Debug)]
pub enum DualExecutionResult {
    /// JIT and interpreter produced identical results.
    Match,
    /// JIT and interpreter diverged.
    Mismatch { reason: String },
}

/// Compare a JIT execution outcome against an interpreter execution outcome.
///
/// Checks status, gas_used, output bytes, refunded gas, logs, and **DB state
/// changes** (account status, balances, nonces, code_hash, and storage for all
/// modified accounts).
#[allow(clippy::too_many_arguments)]
pub fn validate_dual_execution(
    jit_result: &ContextResult,
    interp_result: &ContextResult,
    jit_refunded_gas: u64,
    interp_refunded_gas: u64,
    jit_logs: &[ethrex_common::types::Log],
    interp_logs: &[ethrex_common::types::Log],
    jit_accounts: &CacheDB,
    interp_accounts: &CacheDB,
) -> DualExecutionResult {
    // 1. Compare status (success vs revert)
    let jit_success = matches!(jit_result.result, TxResult::Success);
    let interp_success = matches!(interp_result.result, TxResult::Success);
    if jit_success != interp_success {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "status mismatch: JIT={}, interpreter={}",
                if jit_success { "success" } else { "revert" },
                if interp_success { "success" } else { "revert" },
            ),
        };
    }

    // 2. Compare gas_used
    if jit_result.gas_used != interp_result.gas_used {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "gas_used mismatch: JIT={}, interpreter={}",
                jit_result.gas_used, interp_result.gas_used,
            ),
        };
    }

    // 3. Compare output bytes
    if jit_result.output != interp_result.output {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "output mismatch: JIT len={}, interpreter len={}",
                jit_result.output.len(),
                interp_result.output.len(),
            ),
        };
    }

    // 4. Compare refunded gas
    if jit_refunded_gas != interp_refunded_gas {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "refunded_gas mismatch: JIT={jit_refunded_gas}, interpreter={interp_refunded_gas}",
            ),
        };
    }

    // 5. Compare logs (count + ordered equality)
    if jit_logs.len() != interp_logs.len() {
        return DualExecutionResult::Mismatch {
            reason: format!(
                "log count mismatch: JIT={}, interpreter={}",
                jit_logs.len(),
                interp_logs.len(),
            ),
        };
    }
    for (i, (jit_log, interp_log)) in jit_logs.iter().zip(interp_logs.iter()).enumerate() {
        if jit_log != interp_log {
            return DualExecutionResult::Mismatch {
                reason: format!("log mismatch at index {i}"),
            };
        }
    }

    // 6. Compare DB state changes (balance, nonce, storage for modified accounts)
    if let Some(reason) = compare_account_states(jit_accounts, interp_accounts) {
        return DualExecutionResult::Mismatch { reason };
    }

    DualExecutionResult::Match
}

/// Compare modified account states between JIT and interpreter DB snapshots.
///
/// Checks account status (Modified/Destroyed/DestroyedModified), balance, nonce,
/// code_hash, and storage for all non-Unmodified accounts.
/// Returns `Some(reason)` on first mismatch, `None` if all modified accounts match.
fn compare_account_states(jit_accounts: &CacheDB, interp_accounts: &CacheDB) -> Option<String> {
    // Check every address present in either DB
    // Collect all addresses that were modified in either
    for (address, jit_account) in jit_accounts {
        if jit_account.is_unmodified() {
            continue;
        }
        let Some(interp_account) = interp_accounts.get(address) else {
            return Some(format!(
                "state mismatch: account {address:?} modified by JIT but absent in interpreter DB"
            ));
        };

        // Compare account status (e.g., Modified vs Destroyed)
        if jit_account.status != interp_account.status {
            return Some(format!(
                "state mismatch: account {address:?} status JIT={:?} interpreter={:?}",
                jit_account.status, interp_account.status,
            ));
        }

        // Compare balance
        if jit_account.info.balance != interp_account.info.balance {
            return Some(format!(
                "state mismatch: account {address:?} balance JIT={} interpreter={}",
                jit_account.info.balance, interp_account.info.balance,
            ));
        }

        // Compare nonce
        if jit_account.info.nonce != interp_account.info.nonce {
            return Some(format!(
                "state mismatch: account {address:?} nonce JIT={} interpreter={}",
                jit_account.info.nonce, interp_account.info.nonce,
            ));
        }

        // Compare code_hash (CREATE/CREATE2 may deploy different code)
        if jit_account.info.code_hash != interp_account.info.code_hash {
            return Some(format!(
                "state mismatch: account {address:?} code_hash JIT={:?} interpreter={:?}",
                jit_account.info.code_hash, interp_account.info.code_hash,
            ));
        }

        // Compare storage slots
        for (slot, jit_value) in &jit_account.storage {
            let interp_value = interp_account
                .storage
                .get(slot)
                .copied()
                .unwrap_or_default();
            if *jit_value != interp_value {
                return Some(format!(
                    "state mismatch: account {address:?} storage slot {slot:?} \
                     JIT={jit_value} interpreter={interp_value}",
                ));
            }
        }
        // Check slots in interpreter but not in JIT
        for (slot, interp_value) in &interp_account.storage {
            if !jit_account.storage.contains_key(slot) && !interp_value.is_zero() {
                return Some(format!(
                    "state mismatch: account {address:?} storage slot {slot:?} \
                     JIT=0 interpreter={interp_value}",
                ));
            }
        }
    }

    // Check accounts modified by interpreter but absent in JIT DB
    for (address, interp_account) in interp_accounts {
        if interp_account.is_unmodified() {
            continue;
        }
        if !jit_accounts.contains_key(address) {
            return Some(format!(
                "state mismatch: account {address:?} modified by interpreter but absent in JIT DB"
            ));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{AccountStatus, LevmAccount};
    use bytes::Bytes;
    use ethrex_common::types::{AccountInfo, Log};
    use ethrex_common::{Address, H256, U256};
    use rustc_hash::FxHashMap;

    fn success_result(gas_used: u64, output: &[u8]) -> ContextResult {
        ContextResult {
            result: TxResult::Success,
            gas_used,
            gas_spent: gas_used,
            output: Bytes::copy_from_slice(output),
        }
    }

    fn revert_result(gas_used: u64, output: &[u8]) -> ContextResult {
        use crate::errors::VMError;
        ContextResult {
            result: TxResult::Revert(VMError::RevertOpcode),
            gas_used,
            gas_spent: gas_used,
            output: Bytes::copy_from_slice(output),
        }
    }

    fn make_log(addr: Address, topics: Vec<H256>, data: Vec<u8>) -> Log {
        Log {
            address: addr,
            topics,
            data: Bytes::from(data),
        }
    }

    fn empty_accounts() -> CacheDB {
        FxHashMap::default()
    }

    fn make_account(balance: u64, nonce: u64, storage: Vec<(H256, U256)>) -> LevmAccount {
        LevmAccount {
            info: AccountInfo {
                code_hash: H256::zero(),
                balance: U256::from(balance),
                nonce,
            },
            storage: storage.into_iter().collect(),
            has_storage: false,
            status: AccountStatus::Modified,
        }
    }

    // ---- Basic comparison tests (unchanged behavior) ----

    #[test]
    fn test_matching_success_outcomes() {
        let jit = success_result(21000, &[0x01, 0x02]);
        let interp = success_result(21000, &[0x01, 0x02]);
        let db = empty_accounts();
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &db, &db);
        assert!(matches!(result, DualExecutionResult::Match));
    }

    #[test]
    fn test_gas_mismatch() {
        let jit = success_result(21000, &[]);
        let interp = success_result(21500, &[]);
        let db = empty_accounts();
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &db, &db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("gas_used"));
        }
    }

    #[test]
    fn test_output_mismatch() {
        let jit = success_result(21000, &[0x01]);
        let interp = success_result(21000, &[0x02]);
        let db = empty_accounts();
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &db, &db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("output"));
        }
    }

    #[test]
    fn test_status_mismatch_success_vs_revert() {
        let jit = success_result(21000, &[]);
        let interp = revert_result(21000, &[]);
        let db = empty_accounts();
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &db, &db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("status"));
        }
    }

    #[test]
    fn test_log_count_mismatch() {
        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let log = make_log(Address::zero(), vec![], vec![0x42]);
        let db = empty_accounts();
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[log], &[], &db, &db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("log count"));
        }
    }

    #[test]
    fn test_refunded_gas_mismatch() {
        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let db = empty_accounts();
        let result = validate_dual_execution(&jit, &interp, 100, 200, &[], &[], &db, &db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("refunded_gas"));
        }
    }

    #[test]
    fn test_matching_with_logs() {
        let jit = success_result(30000, &[0xAA]);
        let interp = success_result(30000, &[0xAA]);
        let log1 = make_log(Address::zero(), vec![H256::zero()], vec![1, 2, 3]);
        let log2 = make_log(Address::zero(), vec![H256::zero()], vec![1, 2, 3]);
        let db = empty_accounts();
        let result = validate_dual_execution(&jit, &interp, 50, 50, &[log1], &[log2], &db, &db);
        assert!(matches!(result, DualExecutionResult::Match));
    }

    #[test]
    fn test_log_content_mismatch() {
        let jit = success_result(30000, &[]);
        let interp = success_result(30000, &[]);
        let jit_log = make_log(Address::zero(), vec![], vec![1]);
        let interp_log = make_log(Address::zero(), vec![], vec![2]);
        let db = empty_accounts();
        let result =
            validate_dual_execution(&jit, &interp, 0, 0, &[jit_log], &[interp_log], &db, &db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("log mismatch at index"));
        }
    }

    // ---- DB state comparison tests (Fix 1) ----

    #[test]
    fn test_matching_db_state_with_storage() {
        let addr = Address::from_low_u64_be(0x42);
        let slot = H256::from_low_u64_be(1);
        let value = U256::from(999);

        let mut jit_db: CacheDB = FxHashMap::default();
        jit_db.insert(addr, make_account(100, 1, vec![(slot, value)]));

        let mut interp_db: CacheDB = FxHashMap::default();
        interp_db.insert(addr, make_account(100, 1, vec![(slot, value)]));

        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &jit_db, &interp_db);
        assert!(matches!(result, DualExecutionResult::Match));
    }

    #[test]
    fn test_balance_mismatch() {
        let addr = Address::from_low_u64_be(0x42);

        let mut jit_db: CacheDB = FxHashMap::default();
        jit_db.insert(addr, make_account(100, 1, vec![]));

        let mut interp_db: CacheDB = FxHashMap::default();
        interp_db.insert(addr, make_account(200, 1, vec![]));

        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &jit_db, &interp_db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("balance"));
        }
    }

    #[test]
    fn test_nonce_mismatch() {
        let addr = Address::from_low_u64_be(0x42);

        let mut jit_db: CacheDB = FxHashMap::default();
        jit_db.insert(addr, make_account(100, 1, vec![]));

        let mut interp_db: CacheDB = FxHashMap::default();
        interp_db.insert(addr, make_account(100, 2, vec![]));

        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &jit_db, &interp_db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("nonce"));
        }
    }

    #[test]
    fn test_storage_slot_mismatch() {
        let addr = Address::from_low_u64_be(0x42);
        let slot = H256::from_low_u64_be(1);

        let mut jit_db: CacheDB = FxHashMap::default();
        jit_db.insert(addr, make_account(100, 1, vec![(slot, U256::from(10))]));

        let mut interp_db: CacheDB = FxHashMap::default();
        interp_db.insert(addr, make_account(100, 1, vec![(slot, U256::from(20))]));

        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &jit_db, &interp_db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("storage slot"));
        }
    }

    #[test]
    fn test_unmodified_accounts_ignored() {
        let addr = Address::from_low_u64_be(0x42);

        let mut jit_db: CacheDB = FxHashMap::default();
        let mut jit_acct = make_account(100, 1, vec![]);
        jit_acct.status = AccountStatus::Unmodified;
        jit_db.insert(addr, jit_acct);

        let mut interp_db: CacheDB = FxHashMap::default();
        let mut interp_acct = make_account(200, 2, vec![]);
        interp_acct.status = AccountStatus::Unmodified;
        interp_db.insert(addr, interp_acct);

        // Different values but both unmodified — should be Match
        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &jit_db, &interp_db);
        assert!(matches!(result, DualExecutionResult::Match));
    }

    #[test]
    fn test_account_status_mismatch_destroyed_vs_modified() {
        let addr = Address::from_low_u64_be(0x42);

        let mut jit_db: CacheDB = FxHashMap::default();
        let mut jit_acct = make_account(100, 1, vec![]);
        jit_acct.status = AccountStatus::Destroyed;
        jit_db.insert(addr, jit_acct);

        let mut interp_db: CacheDB = FxHashMap::default();
        interp_db.insert(addr, make_account(100, 1, vec![])); // Modified by default

        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &jit_db, &interp_db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("status"));
        }
    }

    #[test]
    fn test_code_hash_mismatch() {
        let addr = Address::from_low_u64_be(0x42);

        let mut jit_db: CacheDB = FxHashMap::default();
        let mut jit_acct = make_account(100, 1, vec![]);
        jit_acct.info.code_hash = H256::from_low_u64_be(0xAA);
        jit_db.insert(addr, jit_acct);

        let mut interp_db: CacheDB = FxHashMap::default();
        let mut interp_acct = make_account(100, 1, vec![]);
        interp_acct.info.code_hash = H256::from_low_u64_be(0xBB);
        interp_db.insert(addr, interp_acct);

        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &jit_db, &interp_db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("code_hash"));
        }
    }

    #[test]
    fn test_extra_storage_slot_in_interpreter() {
        let addr = Address::from_low_u64_be(0x42);
        let slot1 = H256::from_low_u64_be(1);
        let slot2 = H256::from_low_u64_be(2);

        let mut jit_db: CacheDB = FxHashMap::default();
        jit_db.insert(addr, make_account(100, 1, vec![(slot1, U256::from(10))]));

        let mut interp_db: CacheDB = FxHashMap::default();
        interp_db.insert(
            addr,
            make_account(
                100,
                1,
                vec![(slot1, U256::from(10)), (slot2, U256::from(5))],
            ),
        );

        let jit = success_result(21000, &[]);
        let interp = success_result(21000, &[]);
        let result = validate_dual_execution(&jit, &interp, 0, 0, &[], &[], &jit_db, &interp_db);
        assert!(matches!(result, DualExecutionResult::Mismatch { .. }));
        if let DualExecutionResult::Mismatch { reason } = result {
            assert!(reason.contains("storage slot"));
        }
    }
}
