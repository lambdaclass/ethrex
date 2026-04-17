use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::tracing::{PrePostState, PrestateAccountState, PrestateResult, PrestateTrace};
use ethrex_common::types::{Block, Transaction};
use ethrex_common::{tracing::CallTrace, types::BlockHeader};
use ethrex_crypto::Crypto;
use ethrex_levm::db::gen_db::CacheDB;
use ethrex_levm::vm::VMType;
use ethrex_levm::{db::gen_db::GeneralizedDatabase, tracing::LevmCallTracer, vm::VM};

use crate::{EvmError, backends::levm::LEVM};

impl LEVM {
    /// Execute all transactions of the block up until a certain transaction specified in `stop_index`.
    /// The goal is to just mutate the state up to that point, without needing to process transaction receipts or requests.
    pub fn rerun_block(
        db: &mut GeneralizedDatabase,
        block: &Block,
        stop_index: Option<usize>,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<(), EvmError> {
        Self::prepare_block(block, db, vm_type, crypto)?;

        // Executes transactions and stops when the index matches the stop index.
        for (index, (tx, sender)) in block
            .body
            .get_transactions_with_sender(crypto)
            .map_err(|error| EvmError::Transaction(error.to_string()))?
            .into_iter()
            .enumerate()
        {
            if stop_index.is_some_and(|stop| stop == index) {
                break;
            }

            Self::execute_tx(tx, sender, &block.header, db, vm_type, crypto)?;
        }

        // Process withdrawals only if the whole block has been executed.
        if stop_index.is_none()
            && let Some(withdrawals) = &block.body.withdrawals
        {
            Self::process_withdrawals(db, withdrawals)?;
        };

        Ok(())
    }

    /// Execute a transaction and capture the pre/post account state (prestateTracer).
    ///
    /// Captures a snapshot of all touched accounts before and after execution.
    /// The `diff_mode` flag controls whether to return both pre and post state or just pre state.
    ///
    /// Assumes the db already contains the state from all prior transactions in the block.
    pub fn trace_tx_prestate(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &Transaction,
        diff_mode: bool,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<PrestateResult, EvmError> {
        // Snapshot the current cache state before executing the tx.
        // This is the pre-tx state for all accounts already loaded in the cache.
        let pre_snapshot: CacheDB = db.current_accounts_state.clone();

        // Execute the transaction (updates current_accounts_state in place)
        let sender = tx
            .sender(crypto)
            .map_err(|e| EvmError::Transaction(format!("Couldn't recover sender: {e}")))?;
        let env = Self::setup_env(tx, sender, block_header, db, vm_type)?;
        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), vm_type, crypto)?;
        vm.execute()?;

        if diff_mode {
            let pre_map =
                build_account_state_map(&pre_snapshot, &db.current_accounts_state, db, true);
            let post_map =
                build_account_state_map(&pre_snapshot, &db.current_accounts_state, db, false);
            Ok(PrestateResult::Diff(PrePostState {
                pre: pre_map,
                post: post_map,
            }))
        } else {
            let pre_map =
                build_account_state_map(&pre_snapshot, &db.current_accounts_state, db, true);
            Ok(PrestateResult::Prestate(pre_map))
        }
    }

    /// Run transaction with callTracer activated.
    pub fn trace_tx_calls(
        db: &mut GeneralizedDatabase,
        block_header: &BlockHeader,
        tx: &Transaction,
        only_top_call: bool,
        with_log: bool,
        vm_type: VMType,
        crypto: &dyn Crypto,
    ) -> Result<CallTrace, EvmError> {
        let env = Self::setup_env(
            tx,
            tx.sender(crypto).map_err(|error| {
                EvmError::Transaction(format!("Couldn't recover addresses with error: {error}"))
            })?,
            block_header,
            db,
            vm_type,
        )?;
        let mut vm = VM::new(
            env,
            db,
            tx,
            LevmCallTracer::new(only_top_call, with_log),
            vm_type,
            crypto,
        )?;

        vm.execute()?;

        let callframe = vm.get_trace_result()?;

        // We only return the top call because a transaction only has one call with subcalls
        Ok(vec![callframe])
    }
}

/// Build a map of address -> `PrestateAccountState` for all accounts touched by a transaction.
///
/// `pre_snapshot` is a snapshot of `current_accounts_state` taken BEFORE the tx executed.
/// `post_cache` is `current_accounts_state` AFTER the tx executed.
/// `db` is the database (used to look up code bytes by hash for new accounts).
/// `use_pre` controls whether to use the pre-tx state (true) or post-tx state (false).
///
/// An account is "touched" if:
/// - It was newly loaded during this tx (present in `post_cache` but not in `pre_snapshot`)
/// - It was already cached and was modified (exists in both but differs)
fn build_account_state_map(
    pre_snapshot: &CacheDB,
    post_cache: &CacheDB,
    db: &GeneralizedDatabase,
    use_pre: bool,
) -> PrestateTrace {
    let mut result = PrestateTrace::new();

    for (addr, post_account) in post_cache {
        let (touched, pre_account_opt) = match pre_snapshot.get(addr) {
            None => {
                // Account was first loaded during this tx.
                // Pre-state comes from initial_accounts_state (the value loaded from DB before this tx).
                let pre_in_initial = db.initial_accounts_state.get(addr);
                // Consider touched only if the account changed (info or storage differ from initial).
                let changed = pre_in_initial.is_none_or(|pre| {
                    pre.info != post_account.info || pre.storage != post_account.storage
                });
                (changed, pre_in_initial)
            }
            Some(pre_account) => {
                // Account was already in cache. Only include if something changed.
                let changed = pre_account.info != post_account.info
                    || pre_account.storage != post_account.storage;
                (changed, Some(pre_account))
            }
        };

        if !touched {
            continue;
        }

        let source_account = if use_pre {
            match pre_account_opt {
                Some(a) => a,
                // If we can't find pre-state (shouldn't happen), skip this account
                None => continue,
            }
        } else {
            post_account
        };

        let address_hex = format!("0x{:x}", addr);

        // Look up code if account has non-empty code hash
        let code = if source_account.info.code_hash != *EMPTY_KECCACK_HASH {
            db.codes
                .get(&source_account.info.code_hash)
                .map(|c| format!("0x{}", hex::encode(&c.bytecode)))
        } else {
            None
        };

        // Build the storage map for the output.
        // When emitting the pre-state for an already-cached account, the pre_snapshot
        // only contains slots loaded by *previous* transactions. Any slot first accessed
        // during *this* transaction is missing from pre_snapshot but its original (pre-tx)
        // value is in `initial_accounts_state` (populated by `get_value_from_database`
        // when the VM loaded it from the store). We merge those original values so
        // the output includes every accessed slot.
        let storage: std::collections::HashMap<String, String> =
            if use_pre && pre_snapshot.contains_key(addr) {
                // Merge: start with pre_snapshot storage, then fill in any newly-loaded
                // slots from initial_accounts_state whose original values aren't in pre_snapshot.
                let mut merged = source_account.storage.clone();
                if let Some(initial) = db.initial_accounts_state.get(addr) {
                    for (k, v) in &initial.storage {
                        merged.entry(*k).or_insert(*v);
                    }
                }
                // Only include slots that are actually accessed in this tx
                // (i.e., present in the post_cache for this account).
                merged
                    .iter()
                    .filter(|(k, _)| post_account.storage.contains_key(k))
                    .filter(|(_, v)| !v.is_zero())
                    .map(|(k, v)| {
                        let key_hex = format!("0x{:x}", k);
                        let val_hex = format!("0x{:064x}", v);
                        (key_hex, val_hex)
                    })
                    .collect()
            } else {
                source_account
                    .storage
                    .iter()
                    .filter(|(_, v)| !v.is_zero())
                    .map(|(k, v)| {
                        let key_hex = format!("0x{:x}", k);
                        let val_hex = format!("0x{:064x}", v);
                        (key_hex, val_hex)
                    })
                    .collect()
            };

        let account_state = PrestateAccountState {
            balance: format!("0x{:x}", source_account.info.balance),
            nonce: source_account.info.nonce,
            code,
            storage,
        };

        result.insert(address_hex, account_state);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::constants::EMPTY_TRIE_HASH;
    use ethrex_common::types::{
        Account, AccountState, BlockHeader, ChainConfig, Code, CodeMetadata, EIP1559Transaction,
        Transaction, TxKind,
    };
    use ethrex_common::{Address, H256, U256};
    use ethrex_crypto::NativeCrypto;
    use ethrex_levm::db::Database;
    use ethrex_levm::errors::DatabaseError;
    use ethrex_levm::vm::VMType;
    use once_cell::sync::OnceCell;
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    // ── Test database ────────────────────────────────────────────────────

    struct TestDatabase {
        accounts: FxHashMap<Address, Account>,
    }

    impl Database for TestDatabase {
        fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
            Ok(self
                .accounts
                .get(&address)
                .map(|acc| AccountState {
                    nonce: acc.info.nonce,
                    balance: acc.info.balance,
                    storage_root: *EMPTY_TRIE_HASH,
                    code_hash: acc.info.code_hash,
                })
                .unwrap_or_default())
        }

        fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
            Ok(self
                .accounts
                .get(&address)
                .and_then(|acc| acc.storage.get(&key).copied())
                .unwrap_or_default())
        }

        fn get_block_hash(&self, _: u64) -> Result<H256, DatabaseError> {
            Ok(H256::zero())
        }

        fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
            Ok(ChainConfig {
                chain_id: 1,
                ..Default::default()
            })
        }

        fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(acc.code.clone());
                }
            }
            Ok(Code::default())
        }

        fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(CodeMetadata {
                        length: acc.code.bytecode.len() as u64,
                    });
                }
            }
            Ok(CodeMetadata { length: 0 })
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    /// Create an EIP-1559 tx that calls `contract` with 32-byte calldata encoding `slot`.
    fn call_contract_tx(contract: Address, sender: Address, slot: H256, nonce: u64) -> Transaction {
        let tx = EIP1559Transaction {
            chain_id: 1,
            nonce,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 10,
            gas_limit: 100_000,
            to: TxKind::Call(contract),
            value: U256::zero(),
            data: Bytes::from(slot.0.to_vec()),
            access_list: vec![],
            signature_y_parity: false,
            signature_r: U256::one(),
            signature_s: U256::one(),
            inner_hash: OnceCell::new(),
            sender_cache: {
                let cell = OnceCell::new();
                let _ = cell.set(sender);
                cell
            },
            cached_canonical: OnceCell::new(),
        };
        Transaction::EIP1559Transaction(tx)
    }

    fn default_header() -> BlockHeader {
        BlockHeader {
            coinbase: Address::from_low_u64_be(0xCCC),
            base_fee_per_gas: Some(1),
            gas_limit: 30_000_000,
            ..Default::default()
        }
    }

    /// Contract that reads the slot given in calldata[0..32] and writes 0xFF to it.
    ///
    /// ```text
    /// PUSH1 0xFF      60 FF
    /// PUSH1 0x00      60 00
    /// CALLDATALOAD    35
    /// DUP1            80
    /// SLOAD           54
    /// POP             50
    /// SSTORE          55
    /// STOP            00
    /// ```
    fn slot_readwrite_contract(storage: FxHashMap<H256, U256>) -> Account {
        let bytecode = Bytes::from(vec![
            0x60, 0xFF, 0x60, 0x00, 0x35, 0x80, 0x54, 0x50, 0x55, 0x00,
        ]);
        Account::new(
            U256::zero(),
            Code::from_bytecode(bytecode, &NativeCrypto),
            1,
            storage,
        )
    }

    // ── Tests ────────────────────────────────────────────────────────────

    /// Regression test: when tx A caches account C (loading only slot0), then
    /// tx B accesses a NEW slot (slot1) of the same account, the pre-state
    /// trace for tx B must include slot1's original value.
    ///
    /// The bug was that `build_account_state_map` used `pre_snapshot` as the
    /// source for pre-state storage, but `pre_snapshot` only contained slots
    /// loaded by previous txs — newly-loaded slots from `initial_accounts_state`
    /// were missing.
    #[test]
    fn prestate_trace_includes_newly_accessed_storage_slots() {
        let contract_addr = Address::from_low_u64_be(0xC000);
        let sender_addr = Address::from_low_u64_be(0x1000);

        let slot0 = H256::from_low_u64_be(0);
        let slot1 = H256::from_low_u64_be(1);

        // Contract has slot0=100, slot1=200 in the backing store
        let mut contract_storage = FxHashMap::default();
        contract_storage.insert(slot0, U256::from(100));
        contract_storage.insert(slot1, U256::from(200));

        let mut accounts = FxHashMap::default();
        accounts.insert(contract_addr, slot_readwrite_contract(contract_storage));
        accounts.insert(
            sender_addr,
            Account::new(
                U256::from(10u64) * U256::from(10u64).pow(U256::from(18)), // 10 ETH
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        );

        // Use GeneralizedDatabase::new (lazy loading) — NOT new_with_account_state
        let test_db = TestDatabase { accounts };
        let mut db = GeneralizedDatabase::new(Arc::new(test_db));

        let header = default_header();

        // Tx A: calls contract with slot0 → loads C into cache with only slot0
        let tx_a = call_contract_tx(contract_addr, sender_addr, slot0, 0);
        LEVM::execute_tx(
            &tx_a,
            sender_addr,
            &header,
            &mut db,
            VMType::L1,
            &NativeCrypto,
        )
        .expect("tx_a should succeed");

        // Verify: slot1 is NOT in current_accounts_state cache (lazy loading)
        assert!(
            !db.current_accounts_state[&contract_addr]
                .storage
                .contains_key(&slot1),
            "slot1 should not be cached yet after tx_a"
        );

        // Tx B: calls contract with slot1 → loads slot1 from DB, writes 0xFF
        let tx_b = call_contract_tx(contract_addr, sender_addr, slot1, 1);
        let result =
            LEVM::trace_tx_prestate(&mut db, &header, &tx_b, false, VMType::L1, &NativeCrypto)
                .expect("trace should succeed");

        let prestate = match result {
            PrestateResult::Prestate(p) => p,
            PrestateResult::Diff(_) => panic!("expected Prestate variant for non-diff mode"),
        };

        // The pre-state for the contract MUST include slot1's original value (200)
        let contract_hex = format!("0x{:x}", contract_addr);
        let contract_state = prestate
            .get(&contract_hex)
            .expect("contract should appear in prestate");

        let slot1_hex = format!("0x{:x}", slot1);
        let slot1_value = contract_state
            .storage
            .get(&slot1_hex)
            .expect("slot1 must be in prestate storage — its original value was 200");

        assert_eq!(
            slot1_value,
            &format!("0x{:064x}", U256::from(200)),
            "slot1 pre-state should be its original value (200), not the post-tx value"
        );
    }

    /// Same scenario as above but in diff mode: both pre and post maps
    /// must include the newly-accessed slot.
    #[test]
    fn prestate_diff_mode_includes_newly_accessed_storage_slots() {
        let contract_addr = Address::from_low_u64_be(0xC000);
        let sender_addr = Address::from_low_u64_be(0x1000);

        let slot0 = H256::from_low_u64_be(0);
        let slot1 = H256::from_low_u64_be(1);

        let mut contract_storage = FxHashMap::default();
        contract_storage.insert(slot0, U256::from(100));
        contract_storage.insert(slot1, U256::from(200));

        let mut accounts = FxHashMap::default();
        accounts.insert(contract_addr, slot_readwrite_contract(contract_storage));
        accounts.insert(
            sender_addr,
            Account::new(
                U256::from(10u64) * U256::from(10u64).pow(U256::from(18)),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        );

        let test_db = TestDatabase { accounts };
        let mut db = GeneralizedDatabase::new(Arc::new(test_db));
        let header = default_header();

        // Tx A: cache contract with slot0
        let tx_a = call_contract_tx(contract_addr, sender_addr, slot0, 0);
        LEVM::execute_tx(
            &tx_a,
            sender_addr,
            &header,
            &mut db,
            VMType::L1,
            &NativeCrypto,
        )
        .expect("tx_a should succeed");

        // Tx B: access slot1 (new slot) in diff mode
        let tx_b = call_contract_tx(contract_addr, sender_addr, slot1, 1);
        let result =
            LEVM::trace_tx_prestate(&mut db, &header, &tx_b, true, VMType::L1, &NativeCrypto)
                .expect("trace should succeed");

        let diff = match result {
            PrestateResult::Diff(d) => d,
            PrestateResult::Prestate(_) => panic!("expected Diff variant for diff mode"),
        };
        let contract_hex = format!("0x{:x}", contract_addr);
        let slot1_hex = format!("0x{:x}", slot1);

        // Pre-state must have slot1 = 200 (original)
        let pre_state = diff.pre.get(&contract_hex).expect("contract in pre");
        let pre_val = pre_state
            .storage
            .get(&slot1_hex)
            .expect("slot1 must be in pre storage");
        assert_eq!(pre_val, &format!("0x{:064x}", U256::from(200)));

        // Post-state must have slot1 = 0xFF (written by contract)
        let post_state = diff.post.get(&contract_hex).expect("contract in post");
        let post_val = post_state
            .storage
            .get(&slot1_hex)
            .expect("slot1 must be in post storage");
        assert_eq!(post_val, &format!("0x{:064x}", U256::from(0xFF)));
    }
}
