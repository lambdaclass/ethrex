//! Integration test: verify BinaryTrieState + BinaryTrieVmDb work together
//! for EIP-7864 block execution.
//!
//! Tests the full state lifecycle:
//! 1. Genesis initialization
//! 2. VmDatabase reads from binary trie state
//! 3. AccountUpdate application (simulating post-execution writes)
//! 4. State root computation and determinism

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use ethrex_binary_trie::state::BinaryTrieState;
use ethrex_blockchain::binary_trie_db::BinaryTrieVmDb;
use ethrex_common::{
    Address, H256, U256,
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountInfo, AccountUpdate, ChainConfig, Code, GenesisAccount},
};
use ethrex_vm::VmDatabase;

fn addr(b: u8) -> Address {
    let mut a = [0u8; 20];
    a[19] = b;
    Address::from(a)
}

fn slot(n: u64) -> H256 {
    H256(U256::from(n).to_big_endian())
}

/// Full lifecycle: genesis → VM reads → updates → root changes
#[test]
fn test_full_state_lifecycle() {
    // 1. Genesis with two accounts.
    let mut state = BinaryTrieState::new();
    let alice = addr(0x01);
    let bob = addr(0x02);

    let mut accounts = BTreeMap::new();
    accounts.insert(
        alice,
        GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance: U256::from(10_000_000u64),
            nonce: 0,
        },
    );
    accounts.insert(
        bob,
        GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance: U256::zero(),
            nonce: 0,
        },
    );
    state.apply_genesis(&accounts).unwrap();
    let root_after_genesis = state.state_root();

    // 2. Wrap in VmDb and verify reads.
    let state_arc = Arc::new(RwLock::new(state));
    let vm_db = BinaryTrieVmDb::new(state_arc.clone(), ChainConfig::default());

    let alice_state = vm_db.get_account_state(alice).unwrap().unwrap();
    assert_eq!(alice_state.balance, U256::from(10_000_000u64));
    assert_eq!(alice_state.nonce, 0);
    assert_eq!(alice_state.code_hash, *EMPTY_KECCACK_HASH);

    let bob_state = vm_db.get_account_state(bob).unwrap().unwrap();
    assert_eq!(bob_state.balance, U256::zero());

    // 3. Simulate a transfer: Alice sends 1M to Bob, nonce increments.
    let transfer_amount = U256::from(1_000_000u64);
    let gas_cost = U256::from(21_000u64); // simplified

    let mut alice_update = AccountUpdate::new(alice);
    alice_update.info = Some(AccountInfo {
        code_hash: *EMPTY_KECCACK_HASH,
        balance: alice_state.balance - transfer_amount - gas_cost,
        nonce: 1,
    });

    let mut bob_update = AccountUpdate::new(bob);
    bob_update.info = Some(AccountInfo {
        code_hash: *EMPTY_KECCACK_HASH,
        balance: transfer_amount,
        nonce: 0,
    });

    // 4. Apply updates.
    {
        let mut state = state_arc.write().unwrap();
        state.apply_account_update(&alice_update).unwrap();
        state.apply_account_update(&bob_update).unwrap();
    }

    // 5. Verify state changed.
    let state = state_arc.read().unwrap();
    let root_after_transfer = state.state_root();
    assert_ne!(root_after_genesis, root_after_transfer);

    let alice_after = state.get_account_state(&alice).unwrap();
    assert_eq!(
        alice_after.balance,
        U256::from(10_000_000u64) - transfer_amount - gas_cost
    );
    assert_eq!(alice_after.nonce, 1);

    let bob_after = state.get_account_state(&bob).unwrap();
    assert_eq!(bob_after.balance, transfer_amount);
}

/// Contract deployment via AccountUpdate
#[test]
fn test_contract_deployment_lifecycle() {
    let mut state = BinaryTrieState::new();
    let deployer = addr(0x10);
    let contract = addr(0x11);

    let mut accounts = BTreeMap::new();
    accounts.insert(
        deployer,
        GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance: U256::from(10_000_000u64),
            nonce: 0,
        },
    );
    state.apply_genesis(&accounts).unwrap();

    // Simulate contract creation.
    let bytecode = Bytes::from(vec![
        0x60u8, 0x00, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xF3,
    ]); // PUSH 0, PUSH 0, MSTORE, PUSH 32, PUSH 0, RETURN
    let code = Code::from_bytecode(bytecode.clone());
    let code_hash = code.hash;

    // Deployer update: nonce++, balance decreases.
    let mut deployer_update = AccountUpdate::new(deployer);
    deployer_update.info = Some(AccountInfo {
        code_hash: *EMPTY_KECCACK_HASH,
        balance: U256::from(9_000_000u64),
        nonce: 1,
    });

    // Contract update: new account with code.
    let mut contract_update = AccountUpdate::new(contract);
    contract_update.info = Some(AccountInfo {
        code_hash,
        balance: U256::zero(),
        nonce: 1,
    });
    contract_update.code = Some(code);

    state.apply_account_update(&deployer_update).unwrap();
    state.apply_account_update(&contract_update).unwrap();

    // Verify via VmDb.
    let state_arc = Arc::new(RwLock::new(state));
    let vm_db = BinaryTrieVmDb::new(state_arc, ChainConfig::default());

    let contract_state = vm_db.get_account_state(contract).unwrap().unwrap();
    assert_eq!(contract_state.code_hash, code_hash);

    let retrieved_code = vm_db.get_account_code(code_hash).unwrap();
    assert_eq!(retrieved_code.bytecode, bytecode);

    let meta = vm_db.get_code_metadata(code_hash).unwrap();
    assert_eq!(meta.length, bytecode.len() as u64);
}

/// Storage operations via AccountUpdate + VmDb reads
#[test]
fn test_storage_lifecycle() {
    let mut state = BinaryTrieState::new();
    let contract = addr(0x20);

    let bytecode = Bytes::from(vec![0x00; 10]);
    let mut accounts = BTreeMap::new();
    accounts.insert(
        contract,
        GenesisAccount {
            code: bytecode,
            storage: BTreeMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );
    state.apply_genesis(&accounts).unwrap();

    // Write storage slots.
    let mut update = AccountUpdate::new(contract);
    update.added_storage.insert(slot(0), U256::from(42u64));
    update.added_storage.insert(slot(1), U256::from(99u64));
    update.added_storage.insert(slot(100), U256::from(777u64)); // main storage area (slot >= 64)
    state.apply_account_update(&update).unwrap();

    // Read via VmDb.
    let state_arc = Arc::new(RwLock::new(state));
    let vm_db = BinaryTrieVmDb::new(state_arc.clone(), ChainConfig::default());

    assert_eq!(
        vm_db.get_storage_slot(contract, slot(0)).unwrap(),
        Some(U256::from(42u64))
    );
    assert_eq!(
        vm_db.get_storage_slot(contract, slot(1)).unwrap(),
        Some(U256::from(99u64))
    );
    assert_eq!(
        vm_db.get_storage_slot(contract, slot(100)).unwrap(),
        Some(U256::from(777u64))
    );
    assert_eq!(vm_db.get_storage_slot(contract, slot(999)).unwrap(), None);

    // Verify storage_root is non-empty.
    let contract_state = vm_db.get_account_state(contract).unwrap().unwrap();
    assert_ne!(contract_state.storage_root, *EMPTY_TRIE_HASH);

    // Delete a slot (write zero).
    {
        let mut state = state_arc.write().unwrap();
        let mut delete_update = AccountUpdate::new(contract);
        delete_update.added_storage.insert(slot(0), U256::zero());
        state.apply_account_update(&delete_update).unwrap();
    }

    assert_eq!(vm_db.get_storage_slot(contract, slot(0)).unwrap(), None);
    // Other slots still there.
    assert_eq!(
        vm_db.get_storage_slot(contract, slot(1)).unwrap(),
        Some(U256::from(99u64))
    );
}

/// State root determinism: same operations → same root regardless of construction path
#[test]
fn test_state_root_determinism_across_updates() {
    let alice = addr(0x30);
    let bob = addr(0x31);

    let build_state = || {
        let mut state = BinaryTrieState::new();
        let mut accounts = BTreeMap::new();
        accounts.insert(
            alice,
            GenesisAccount {
                code: Bytes::new(),
                storage: BTreeMap::new(),
                balance: U256::from(1000u64),
                nonce: 0,
            },
        );
        state.apply_genesis(&accounts).unwrap();

        // Apply same updates.
        let mut u1 = AccountUpdate::new(alice);
        u1.info = Some(AccountInfo {
            code_hash: *EMPTY_KECCACK_HASH,
            balance: U256::from(500u64),
            nonce: 1,
        });

        let mut u2 = AccountUpdate::new(bob);
        u2.info = Some(AccountInfo {
            code_hash: *EMPTY_KECCACK_HASH,
            balance: U256::from(500u64),
            nonce: 0,
        });

        state.apply_account_update(&u1).unwrap();
        state.apply_account_update(&u2).unwrap();
        state.state_root()
    };

    assert_eq!(build_state(), build_state());
}
