//! Regression test: phantom empty account from DestroyedModified status leak
//!
//! `restore_cache_state` restores `info` on revert but not `status`. This allows
//! an account that was `Destroyed` to keep `DestroyedModified` status after a
//! reverted inner call modified it. `get_state_transitions` then emits an
//! `AccountUpdate { removed: false, removed_storage: true }` for an account that
//! was empty before and is empty after, causing a phantom empty `AccountState` to
//! be inserted into the state trie (violating EIP-161).

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    evm::calculate_create_address,
    types::{
        Account, AccountState, Code, CodeMetadata, EIP1559Transaction, Fork, Transaction, TxKind,
    },
    utils::keccak,
};
use ethrex_levm::{
    account::AccountStatus,
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::DatabaseError,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::Trie;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Test Database Implementation ====================

struct TestDatabase {
    accounts: FxHashMap<Address, Account>,
}

impl TestDatabase {
    fn new() -> Self {
        Self {
            accounts: FxHashMap::default(),
        }
    }
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

    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        Ok(ethrex_common::types::ChainConfig::default())
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

// ==================== Constants ====================

const SENDER: u64 = 0x1000;
const BENEFICIARY: u64 = 0x4000;
const C_CONTRACT: u64 = 0x5000;
const D_CONTRACT: u64 = 0x6000;
const GAS_LIMIT: u64 = 1_000_000;

// ==================== Bytecode Helpers ====================

/// Init code for B: deploys runtime code `PUSH20 <beneficiary> SELFDESTRUCT`.
/// Uses PUSH + MSTORE + RETURN pattern: stores runtime bytecode in memory
/// (right-aligned in a 32-byte word) and RETURNs the relevant slice.
fn b_init_code(beneficiary: Address) -> Bytes {
    // Runtime code: PUSH20 <beneficiary> SELFDESTRUCT (22 bytes)
    let mut runtime = Vec::new();
    runtime.push(0x73); // PUSH20
    runtime.extend_from_slice(beneficiary.as_bytes());
    runtime.push(0xFF); // SELFDESTRUCT
    let runtime_len = runtime.len(); // 22

    let mut init = Vec::new();
    // PUSH22 <runtime_code>
    init.push(0x60 + runtime_len as u8 - 1); // 0x75 = PUSH22
    init.extend_from_slice(&runtime);
    // PUSH1 0
    init.extend_from_slice(&[0x60, 0x00]);
    // MSTORE — stores 32-byte word at offset 0, runtime code right-aligned
    init.push(0x52);
    // PUSH1 <runtime_len> — return length
    init.extend_from_slice(&[0x60, runtime_len as u8]);
    // PUSH1 <32 - runtime_len> — return offset (skip left padding)
    init.extend_from_slice(&[0x60, (32 - runtime_len) as u8]);
    // RETURN
    init.push(0xF3);

    Bytes::from(init)
}

/// D's runtime code (reverter): sends 1 wei to B via CALL, then REVERTs.
/// This marks B as DestroyedModified (via mark_modified on a Destroyed account),
/// then the REVERT restores B's info but not its status.
fn d_runtime_code(b_addr: Address) -> Bytes {
    let mut code = Vec::new();
    // CALL stack (bottom to top): retSize, retOffset, argsSize, argsOffset, value, to, gas
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]);
    // value = 1 wei
    code.push(0x7F); // PUSH32
    code.extend_from_slice(&U256::from(1).to_big_endian());
    // to = B_ADDR
    code.push(0x73); // PUSH20
    code.extend_from_slice(b_addr.as_bytes());
    // GAS, CALL, POP
    code.extend_from_slice(&[0x5A, 0xF1, 0x50]);
    // REVERT(0, 0) — revert all state changes within D's call frame
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0xFD]);

    Bytes::from(code)
}

/// C's runtime code (wrapper): CALLs D with 2 wei (enough for D to forward 1 to B), then STOPs.
fn c_runtime_code(d_addr: Address) -> Bytes {
    let mut code = Vec::new();
    // CALL stack (bottom to top): retSize, retOffset, argsSize, argsOffset, value, to, gas
    code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]);
    // value = 2 wei
    code.push(0x7F); // PUSH32
    code.extend_from_slice(&U256::from(2).to_big_endian());
    // to = D_ADDR
    code.push(0x73); // PUSH20
    code.extend_from_slice(d_addr.as_bytes());
    // GAS, CALL, POP
    code.extend_from_slice(&[0x5A, 0xF1, 0x50]);
    // STOP
    code.push(0x00);

    Bytes::from(code)
}

fn make_env(sender: Address, fork: Fork, tx_nonce: u64) -> Environment {
    let blob_schedule = EVMConfig::canonical_values(fork);
    Environment {
        origin: sender,
        gas_limit: GAS_LIMIT,
        config: EVMConfig::new(fork, blob_schedule),
        block_number: U256::from(1),
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: U256::from(1000),
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(1000),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(1000),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: Some(U256::from(1000)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce,
        block_gas_limit: GAS_LIMIT * 2,
        is_privileged: false,
        fee_token: None,
    }
}

// ==================== Test ====================

/// Regression test: a reverted inner call can leak `DestroyedModified` status
/// onto an empty account, causing `get_state_transitions` to emit a spurious
/// `AccountUpdate { removed: false, removed_storage: true }`. When applied to
/// the state trie this inserts a phantom empty `AccountState`, violating EIP-161.
///
/// Setup (pre-Cancun, Shanghai fork — SELFDESTRUCT unconditional):
///   - tx1: CREATE deploys contract B (runtime = SELFDESTRUCT to BENEFICIARY)
///   - tx2: CALL B → B self-destructs, end-of-tx cleanup resets B to default + Destroyed
///   - tx3: CALL C → C calls D with 2 wei → D sends 1 wei to B (Destroyed → DestroyedModified)
///          → D REVERTs → restore_cache_state restores B's info (empty) but NOT status
///
/// After tx3, B is empty with DestroyedModified status. `get_state_transitions`
/// then emits an update for B that, when applied to the trie, inserts a phantom
/// empty account.
#[test]
fn test_phantom_empty_account_from_destroyed_modified_status_leak() {
    let sender = Address::from_low_u64_be(SENDER);
    let beneficiary = Address::from_low_u64_be(BENEFICIARY);
    let c_addr = Address::from_low_u64_be(C_CONTRACT);
    let d_addr = Address::from_low_u64_be(D_CONTRACT);
    // B is deployed by sender with nonce 0
    let b_addr = calculate_create_address(sender, 0);

    // Pre-deploy contracts C and D with the needed bytecode
    let c_code = c_runtime_code(d_addr);
    let d_code = d_runtime_code(b_addr);

    let mut initial_accounts: FxHashMap<Address, Account> = FxHashMap::default();
    initial_accounts.insert(
        sender,
        Account::new(
            U256::from(10u128.pow(18)), // 1 ETH — plenty for gas
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    initial_accounts.insert(
        beneficiary,
        Account::new(U256::zero(), Code::default(), 0, FxHashMap::default()),
    );
    initial_accounts.insert(
        c_addr,
        Account::new(
            U256::from(100), // enough to send 2 wei to D
            Code::from_bytecode(c_code),
            0,
            FxHashMap::default(),
        ),
    );
    initial_accounts.insert(
        d_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(d_code),
            0,
            FxHashMap::default(),
        ),
    );

    let test_db = TestDatabase::new();
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(test_db), initial_accounts);

    let fork = Fork::Shanghai;

    // ---- tx1: Deploy B (CREATE transaction) ----
    let tx1 = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Create,
        data: b_init_code(beneficiary),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        nonce: 0,
        ..Default::default()
    });
    let env1 = make_env(sender, fork, 0);
    let mut vm1 = VM::new(env1, &mut db, &tx1, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report1 = vm1.execute().unwrap();
    assert!(report1.is_success(), "tx1 (deploy B) should succeed");

    // ---- tx2: Call B → B self-destructs to BENEFICIARY ----
    let tx2 = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(b_addr),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        nonce: 1,
        ..Default::default()
    });
    let env2 = make_env(sender, fork, 1);
    let mut vm2 = VM::new(env2, &mut db, &tx2, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report2 = vm2.execute().unwrap();
    assert!(
        report2.is_success(),
        "tx2 (call B → selfdestruct) should succeed"
    );

    // Verify B is now empty with Destroyed status after tx2
    let b_after_tx2 = db.current_accounts_state.get(&b_addr).unwrap();
    assert!(
        b_after_tx2.is_empty(),
        "B should be empty after selfdestruct cleanup"
    );
    assert_eq!(
        b_after_tx2.status,
        AccountStatus::Destroyed,
        "B should have Destroyed status after tx2"
    );

    // ---- tx3: Call C → C calls D → D sends 1 wei to B → D reverts ----
    let tx3 = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(c_addr),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        nonce: 2,
        ..Default::default()
    });
    let env3 = make_env(sender, fork, 2);
    let mut vm3 = VM::new(env3, &mut db, &tx3, LevmCallTracer::disabled(), VMType::L1).unwrap();
    let report3 = vm3.execute().unwrap();
    assert!(
        report3.is_success(),
        "tx3 (call C → D → B, D reverts) should succeed"
    );

    // After the fix, B's status should be properly restored to Destroyed (not DestroyedModified)
    let b_after_tx3 = db.current_accounts_state.get(&b_addr).unwrap();
    assert!(
        b_after_tx3.is_empty(),
        "B should be empty (info restored by revert)"
    );
    assert_eq!(
        b_after_tx3.status,
        AccountStatus::Destroyed,
        "B should have Destroyed status (status correctly restored on revert)"
    );

    // ---- Get state transitions and apply to trie ----
    let account_updates = db.get_state_transitions().unwrap();

    // Apply account updates to a fresh trie (simulating apply_account_updates_from_trie_batch)
    let mut state_trie = Trie::new_temp();
    for update in &account_updates {
        let hashed = keccak(update.address.to_fixed_bytes());
        if update.removed {
            state_trie.remove(hashed.as_bytes()).unwrap();
            continue;
        }
        let mut state = match state_trie.get(hashed.as_bytes()).unwrap() {
            Some(encoded) => AccountState::decode(&encoded).unwrap(),
            None => AccountState::default(),
        };
        if update.removed_storage {
            state.storage_root = *EMPTY_TRIE_HASH;
        }
        if let Some(info) = &update.info {
            state.nonce = info.nonce;
            state.balance = info.balance;
            state.code_hash = info.code_hash;
        }
        state_trie
            .insert(hashed.as_bytes().to_vec(), state.encode_to_vec())
            .unwrap();
    }

    // B should NOT be in the trie. Before the fix, the spurious AccountUpdate
    // with removed_storage=true caused a default AccountState to be inserted.
    let b_hashed = keccak(b_addr.to_fixed_bytes());
    assert!(
        state_trie.get(b_hashed.as_bytes()).unwrap().is_none(),
        "B should NOT be in the state trie — phantom empty account detected (EIP-161 violation)"
    );
}
