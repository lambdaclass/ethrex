//! Integration test for the EXECUTE precompile (Native Rollups EIP-8079 PoC).
//!
//! Demonstrates:
//! - Building an ExecutionWitness from a simple genesis state
//! - Re-executing a block with a transfer transaction via the EXECUTE precompile
//! - Applying a deposit (L1→L2 balance credit) via the anchor mechanism
//! - Verifying the final state root matches

#![cfg(feature = "native-rollups")]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountInfo, AccountState, Block, BlockBody, BlockHeader, ChainConfig, Code,
        CodeMetadata, EIP1559Transaction, EIP4844Transaction, Receipt, Transaction, TxKind,
        Withdrawal, block_execution_witness::ExecutionWitness,
    },
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    environment::Environment,
    errors::TxResult,
    execute_precompile::{Deposit, ExecutePrecompileInput, execute_inner},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Trie;
use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
use rustc_hash::FxHashMap;

/// Helper: derive Ethereum address from a k256 signing key.
fn address_from_key(key: &SigningKey) -> Address {
    use k256::ecdsa::VerifyingKey;
    let verifying_key = VerifyingKey::from(key);
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    // Skip the 0x04 prefix byte
    let hash = keccak_hash(&pubkey_bytes.as_bytes()[1..]);
    Address::from_slice(&hash[12..])
}

/// Helper: sign an EIP-1559 transaction.
fn sign_eip1559_tx(tx: &mut EIP1559Transaction, key: &SigningKey) {
    use ethrex_rlp::structs::Encoder;

    // Build the unsigned message: tx_type || RLP(chain_id, nonce, max_priority_fee, max_fee, gas_limit, to, value, data, access_list)
    let mut buf = vec![0x02u8]; // EIP-1559 type
    Encoder::new(&mut buf)
        .encode_field(&tx.chain_id)
        .encode_field(&tx.nonce)
        .encode_field(&tx.max_priority_fee_per_gas)
        .encode_field(&tx.max_fee_per_gas)
        .encode_field(&tx.gas_limit)
        .encode_field(&tx.to)
        .encode_field(&tx.value)
        .encode_field(&tx.data)
        .encode_field(&tx.access_list)
        .finish();

    let msg_hash = keccak_hash(&buf);

    let (sig, recid) = key.sign_prehash(&msg_hash).expect("signing failed");

    let sig_bytes = sig.to_bytes();
    tx.signature_r = U256::from_big_endian(&sig_bytes[..32]);
    tx.signature_s = U256::from_big_endian(&sig_bytes[32..64]);
    tx.signature_y_parity = recid.to_byte() != 0;
}

/// Helper: insert an account into the state trie.
fn insert_account(trie: &mut Trie, address: Address, state: &AccountState) {
    let hashed_addr = keccak_hash(address.to_fixed_bytes()).to_vec();
    trie.insert(hashed_addr, state.encode_to_vec())
        .expect("trie insert failed");
}

/// Helper: get the root node from a trie for use in ExecutionWitness.
fn get_trie_root_node(trie: &Trie) -> Option<ethrex_trie::Node> {
    trie.hash_no_commit();
    trie.root_node()
        .expect("root_node failed")
        .map(|arc_node| (*arc_node).clone())
}

/// The main integration test: execute a simple transfer + deposit via the EXECUTE precompile.
#[test]
fn test_execute_precompile_transfer_and_deposit() {
    // ===== Setup: Keys and Addresses =====
    let alice_key = SigningKey::from_bytes(&[1u8; 32].into()).expect("valid key");
    let alice = address_from_key(&alice_key);

    let bob = Address::from_low_u64_be(0xB0B);
    let charlie = Address::from_low_u64_be(0xC4A);
    let coinbase = Address::from_low_u64_be(0xC01);

    let chain_id: u64 = 1;
    let base_fee: u64 = 1_000_000_000; // 1 gwei

    // ===== Genesis State =====
    let alice_balance = U256::from(10) * U256::from(10).pow(U256::from(18)); // 10 ETH
    let alice_state = AccountState {
        nonce: 0,
        balance: alice_balance,
        ..Default::default()
    };

    let coinbase_state = AccountState::default();

    let mut state_trie = Trie::new_temp();
    insert_account(&mut state_trie, alice, &alice_state);
    insert_account(&mut state_trie, coinbase, &coinbase_state);
    // Bob and Charlie don't need to be in the trie initially (they're empty accounts)
    // But we need them for the witness to work — the precompile will try to look them up
    insert_account(&mut state_trie, bob, &AccountState::default());
    insert_account(&mut state_trie, charlie, &AccountState::default());

    let pre_state_root = state_trie.hash_no_commit();

    // ===== Parent Block Header (genesis) =====
    let parent_header = BlockHeader {
        number: 0,
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_000,
        ..Default::default()
    };

    // ===== Build Transfer Transaction: Alice → Bob, 1 ETH =====
    let transfer_value = U256::from(10).pow(U256::from(18)); // 1 ETH
    let gas_limit: u64 = 21_000; // Simple transfer

    let mut tx = EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1_000_000_000, // 1 gwei priority
        max_fee_per_gas: 2_000_000_000,          // 2 gwei max fee
        gas_limit,
        to: TxKind::Call(bob),
        value: transfer_value,
        data: Bytes::new(),
        access_list: vec![],
        ..Default::default()
    };
    sign_eip1559_tx(&mut tx, &alice_key);

    let transaction = Transaction::EIP1559Transaction(tx);

    // Verify we can recover Alice's address from the signed transaction
    let recovered_sender = transaction.sender().expect("sender recovery failed");
    assert_eq!(recovered_sender, alice, "Sender recovery mismatch");

    // ===== Compute Block Fields =====
    // Gas used for a simple transfer
    let gas_used: u64 = 21_000;
    // Effective gas price: min(max_priority_fee + base_fee, max_fee_per_gas)
    let effective_gas_price: u64 = std::cmp::min(1_000_000_000 + base_fee, 2_000_000_000);
    let gas_cost = U256::from(gas_used) * U256::from(effective_gas_price);

    // Priority fee goes to coinbase
    let priority_fee_per_gas: u64 = effective_gas_price.saturating_sub(base_fee);
    let coinbase_reward = U256::from(gas_used) * U256::from(priority_fee_per_gas);

    // Compute transactions root
    let transactions = vec![transaction.clone()];
    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);

    // Compute receipts root (successful transfer)
    let receipt = Receipt::new(transaction.tx_type(), true, gas_used, vec![]);
    let receipts_root = ethrex_common::types::compute_receipts_root(&[receipt]);

    // ===== Block Header =====
    let block_header = BlockHeader {
        parent_hash: parent_header.compute_block_hash(),
        number: 1,
        gas_used,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_012, // 12 seconds after parent
        coinbase,
        transactions_root,
        receipts_root,
        // State root will be computed after we know the post-state
        // For now set a placeholder — we'll compute it properly
        state_root: H256::zero(),
        withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(&[])),
        ..Default::default()
    };

    let block = Block {
        header: block_header,
        body: BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    // ===== Compute Expected Post-State (after transfer + deposit) =====
    // After the transfer:
    // - Alice: alice_balance - transfer_value - gas_cost
    // - Bob: transfer_value
    // - Coinbase: coinbase_reward
    // After the deposit:
    // - Charlie: 5 ETH
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

    let alice_post = AccountState {
        nonce: 1,
        balance: alice_balance - transfer_value - gas_cost,
        ..Default::default()
    };
    let bob_post = AccountState {
        balance: transfer_value,
        ..Default::default()
    };
    let coinbase_post = AccountState {
        balance: coinbase_reward,
        ..Default::default()
    };
    let charlie_post = AccountState {
        balance: deposit_amount,
        ..Default::default()
    };

    let mut post_trie = Trie::new_temp();
    insert_account(&mut post_trie, alice, &alice_post);
    insert_account(&mut post_trie, bob, &bob_post);
    insert_account(&mut post_trie, coinbase, &coinbase_post);
    insert_account(&mut post_trie, charlie, &charlie_post);
    let post_state_root = post_trie.hash_no_commit();

    // ===== Update block header with correct state root =====
    let mut block = block;
    block.header.state_root = post_state_root;

    // ===== Build ExecutionWitness =====
    let chain_config = ChainConfig {
        chain_id,
        // Activate all pre-merge forks at block 0
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        // Post-merge
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        // Activate Shanghai at timestamp 0 (for withdrawals support)
        shanghai_time: Some(0),
        ..Default::default()
    };

    let parent_header_bytes = parent_header.encode_to_vec();
    let block_header_bytes = block.header.encode_to_vec();

    let witness = ExecutionWitness {
        codes: vec![],
        block_headers_bytes: vec![parent_header_bytes, block_header_bytes],
        first_block_number: 1,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots: BTreeMap::new(),
        keys: vec![],
    };

    // ===== Deposits =====
    let deposits = vec![Deposit {
        address: charlie,
        amount: deposit_amount,
    }];

    // ===== Execute! =====
    let input = ExecutePrecompileInput {
        pre_state_root,
        post_state_root,
        deposits,
        execution_witness: witness,
        blocks: vec![block],
    };

    let result = execute_inner(input);
    match &result {
        Ok(output) => {
            assert_eq!(output.as_ref(), &[0x01], "Expected success byte 0x01");
            println!("EXECUTE precompile succeeded!");
            println!("  Pre-state root:  {pre_state_root:?}");
            println!("  Post-state root: {post_state_root:?}");
            println!("  Alice sent 1 ETH to Bob");
            println!("  Charlie received 5 ETH deposit");
        }
        Err(e) => {
            panic!("EXECUTE precompile failed: {e}");
        }
    }
}

/// Helper: build a minimal ExecutePrecompileInput for rejection tests.
///
/// Creates a genesis state with a single account, builds an ExecutionWitness,
/// and wraps the given block in an ExecutePrecompileInput.
fn build_rejection_test_input(block: Block) -> ExecutePrecompileInput {
    let account = Address::from_low_u64_be(0xA);
    let chain_id: u64 = 1;

    let mut state_trie = Trie::new_temp();
    insert_account(
        &mut state_trie,
        account,
        &AccountState {
            balance: U256::from(10) * U256::from(10).pow(U256::from(18)),
            ..Default::default()
        },
    );
    let pre_state_root = state_trie.hash_no_commit();

    let parent_header = BlockHeader {
        number: block.header.number.saturating_sub(1),
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        timestamp: block.header.timestamp.saturating_sub(12),
        ..Default::default()
    };

    let chain_config = ChainConfig {
        chain_id,
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        shanghai_time: Some(0),
        ..Default::default()
    };

    let witness = ExecutionWitness {
        codes: vec![],
        block_headers_bytes: vec![parent_header.encode_to_vec(), block.header.encode_to_vec()],
        first_block_number: block.header.number,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots: BTreeMap::new(),
        keys: vec![],
    };

    ExecutePrecompileInput {
        pre_state_root,
        post_state_root: pre_state_root, // Doesn't matter — will fail before state check
        deposits: vec![],
        execution_witness: witness,
        blocks: vec![block],
    }
}

/// Test that blob transactions (EIP-4844) are rejected in native rollup blocks.
#[test]
fn test_execute_precompile_rejects_blob_transactions() {
    let blob_tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        to: Address::from_low_u64_be(0xB0B),
        blob_versioned_hashes: vec![H256::zero()],
        ..Default::default()
    });

    let block = Block {
        header: BlockHeader {
            number: 1,
            gas_limit: 30_000_000,
            timestamp: 1_000_012,
            ..Default::default()
        },
        body: BlockBody {
            transactions: vec![blob_tx],
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    let result = execute_inner(build_rejection_test_input(block));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Blob transactions"),
        "Expected blob transaction rejection"
    );
}

/// Test that blocks with non-empty withdrawals are rejected in native rollups.
#[test]
fn test_execute_precompile_rejects_withdrawals() {
    let block = Block {
        header: BlockHeader {
            number: 1,
            gas_used: 0,
            gas_limit: 30_000_000,
            timestamp: 1_000_012,
            ..Default::default()
        },
        body: BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(vec![Withdrawal {
                index: 0,
                validator_index: 0,
                address: Address::from_low_u64_be(0xA),
                amount: 1000,
            }]),
        },
    };

    let result = execute_inner(build_rejection_test_input(block));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("must not contain withdrawals"),
        "Expected withdrawal rejection"
    );
}

// ===== Contract-based demo test infrastructure =====

/// Minimal Database implementation for L1 VM tests.
///
/// Provides chain config and default values for everything else.
/// Actual account state is loaded into GeneralizedDatabase's cache.
struct TestDb {
    chain_config: ChainConfig,
}

impl ethrex_levm::db::Database for TestDb {
    fn get_account_state(
        &self,
        _address: Address,
    ) -> Result<AccountState, ethrex_levm::errors::DatabaseError> {
        Ok(AccountState::default())
    }
    fn get_storage_value(
        &self,
        _address: Address,
        _key: H256,
    ) -> Result<U256, ethrex_levm::errors::DatabaseError> {
        Ok(U256::zero())
    }
    fn get_block_hash(
        &self,
        _block_number: u64,
    ) -> Result<H256, ethrex_levm::errors::DatabaseError> {
        Ok(H256::zero())
    }
    fn get_chain_config(&self) -> Result<ChainConfig, ethrex_levm::errors::DatabaseError> {
        Ok(self.chain_config)
    }
    fn get_account_code(
        &self,
        _code_hash: H256,
    ) -> Result<Code, ethrex_levm::errors::DatabaseError> {
        Ok(Code::default())
    }
    fn get_code_metadata(
        &self,
        _code_hash: H256,
    ) -> Result<CodeMetadata, ethrex_levm::errors::DatabaseError> {
        Ok(CodeMetadata { length: 0 })
    }
}

/// Proxy contract bytecode: forwards all calldata to the EXECUTE precompile at 0x0101.
///
/// ```text
/// CALLDATASIZE PUSH1 0x00 PUSH1 0x00 CALLDATACOPY   // mem[0..] = calldata
/// PUSH1 0x01 PUSH1 0x00                              // retSize=1, retOffset=0
/// CALLDATASIZE PUSH1 0x00 PUSH1 0x00                 // argsSize, argsOffset=0, value=0
/// PUSH2 0x0101 GAS CALL                              // call EXECUTE precompile
/// PUSH1 0x1C JUMPI                                   // jump to RETURN on success
/// PUSH1 0x00 PUSH1 0x00 REVERT                       // revert on failure
/// JUMPDEST PUSH1 0x01 PUSH1 0x00 RETURN              // return mem[0..1]
/// ```
const PROXY_BYTECODE_HEX: &str =
    "3660006000376001600036600060006101015AF1601C5760006000FD5B60016000F3";

/// Build the L2 state transition used by both the direct test and the contract test.
///
/// Returns the ExecutePrecompileInput along with pre/post state roots for verification.
fn build_l2_state_transition() -> (ExecutePrecompileInput, H256, H256) {
    let alice_key = SigningKey::from_bytes(&[1u8; 32].into()).expect("valid key");
    let alice = address_from_key(&alice_key);
    let bob = Address::from_low_u64_be(0xB0B);
    let charlie = Address::from_low_u64_be(0xC4A);
    let coinbase = Address::from_low_u64_be(0xC01);
    let chain_id: u64 = 1;
    let base_fee: u64 = 1_000_000_000;

    // Genesis state
    let alice_balance = U256::from(10) * U256::from(10).pow(U256::from(18));
    let mut state_trie = Trie::new_temp();
    insert_account(
        &mut state_trie,
        alice,
        &AccountState {
            nonce: 0,
            balance: alice_balance,
            ..Default::default()
        },
    );
    insert_account(&mut state_trie, coinbase, &AccountState::default());
    insert_account(&mut state_trie, bob, &AccountState::default());
    insert_account(&mut state_trie, charlie, &AccountState::default());
    let pre_state_root = state_trie.hash_no_commit();

    // Parent block
    let parent_header = BlockHeader {
        number: 0,
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_000,
        ..Default::default()
    };

    // Transfer: Alice → Bob, 1 ETH
    let transfer_value = U256::from(10).pow(U256::from(18));
    let mut tx = EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 2_000_000_000,
        gas_limit: 21_000,
        to: TxKind::Call(bob),
        value: transfer_value,
        data: Bytes::new(),
        access_list: vec![],
        ..Default::default()
    };
    sign_eip1559_tx(&mut tx, &alice_key);
    let transaction = Transaction::EIP1559Transaction(tx);
    let transactions = vec![transaction.clone()];

    // Compute block fields
    let gas_used: u64 = 21_000;
    let effective_gas_price: u64 = std::cmp::min(1_000_000_000 + base_fee, 2_000_000_000);
    let gas_cost = U256::from(gas_used) * U256::from(effective_gas_price);
    let priority_fee_per_gas: u64 = effective_gas_price.saturating_sub(base_fee);
    let coinbase_reward = U256::from(gas_used) * U256::from(priority_fee_per_gas);

    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);
    let receipt = Receipt::new(transaction.tx_type(), true, gas_used, vec![]);
    let receipts_root = ethrex_common::types::compute_receipts_root(&[receipt]);

    // Post-state (after transfer + deposit)
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18));
    let mut post_trie = Trie::new_temp();
    insert_account(
        &mut post_trie,
        alice,
        &AccountState {
            nonce: 1,
            balance: alice_balance - transfer_value - gas_cost,
            ..Default::default()
        },
    );
    insert_account(
        &mut post_trie,
        bob,
        &AccountState {
            balance: transfer_value,
            ..Default::default()
        },
    );
    insert_account(
        &mut post_trie,
        coinbase,
        &AccountState {
            balance: coinbase_reward,
            ..Default::default()
        },
    );
    insert_account(
        &mut post_trie,
        charlie,
        &AccountState {
            balance: deposit_amount,
            ..Default::default()
        },
    );
    let post_state_root = post_trie.hash_no_commit();

    // Block
    let block = Block {
        header: BlockHeader {
            parent_hash: parent_header.compute_block_hash(),
            number: 1,
            gas_used,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(base_fee),
            timestamp: 1_000_012,
            coinbase,
            transactions_root,
            receipts_root,
            state_root: post_state_root,
            withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(&[])),
            ..Default::default()
        },
        body: BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    // Witness
    let chain_config = ChainConfig {
        chain_id,
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        shanghai_time: Some(0),
        ..Default::default()
    };

    let witness = ExecutionWitness {
        codes: vec![],
        block_headers_bytes: vec![parent_header.encode_to_vec(), block.header.encode_to_vec()],
        first_block_number: 1,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots: BTreeMap::new(),
        keys: vec![],
    };

    let input = ExecutePrecompileInput {
        pre_state_root,
        post_state_root,
        deposits: vec![Deposit {
            address: charlie,
            amount: deposit_amount,
        }],
        execution_witness: witness,
        blocks: vec![block],
    };

    (input, pre_state_root, post_state_root)
}

/// Demo: an L1 contract calls the EXECUTE precompile to verify an L2 state transition.
///
/// This test shows the full end-to-end flow:
///   L1 transaction → proxy contract → CALL to 0x0101 → EXECUTE precompile
///     → deserialize calldata → re-execute L2 block → verify state roots → success
#[test]
fn test_execute_precompile_via_contract() {
    let (input, pre_state_root, post_state_root) = build_l2_state_transition();

    // Serialize the EXECUTE input as calldata
    let calldata = serde_json::to_vec(&input).expect("JSON serialization failed");
    println!("Serialized EXECUTE calldata: {} bytes", calldata.len());

    // Deploy proxy contract on "L1"
    let proxy_address = Address::from_low_u64_be(0xFFFF);
    let sender = Address::from_low_u64_be(0x1234);

    let proxy_bytecode = Bytes::from(hex::decode(PROXY_BYTECODE_HEX).expect("invalid hex"));
    let proxy_code_hash = H256(keccak_hash(proxy_bytecode.as_ref()));
    let proxy_code = Code::from_bytecode(proxy_bytecode);

    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    accounts.insert(
        proxy_address,
        Account {
            info: AccountInfo {
                code_hash: proxy_code_hash,
                balance: U256::zero(),
                nonce: 1,
            },
            code: proxy_code,
            storage: FxHashMap::default(),
        },
    );
    accounts.insert(
        sender,
        Account {
            info: AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
            code: Code::default(),
            storage: FxHashMap::default(),
        },
    );

    let l1_chain_config = ChainConfig {
        chain_id: 1,
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        shanghai_time: Some(0),
        ..Default::default()
    };

    let store: Arc<dyn ethrex_levm::db::Database> = Arc::new(TestDb {
        chain_config: l1_chain_config,
    });
    let mut db = GeneralizedDatabase::new_with_account_state(store, accounts);

    // Build L1 transaction: call the proxy contract with the serialized EXECUTE input
    let l1_tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000_000, // 1B gas — plenty for the precompile
        to: TxKind::Call(proxy_address),
        value: U256::zero(),
        data: Bytes::from(calldata),
        access_list: vec![],
        ..Default::default()
    });

    let env = Environment {
        origin: sender,
        gas_limit: 1_000_000_000,
        block_gas_limit: 1_000_000_000,
        tx_nonce: 0,
        chain_id: U256::from(1),
        ..Default::default()
    };

    let mut vm = VM::new(env, &mut db, &l1_tx, LevmCallTracer::disabled(), VMType::L1)
        .expect("VM creation failed");

    let report = vm.execute().expect("VM execution failed");

    assert!(
        matches!(report.result, TxResult::Success),
        "L1 transaction reverted: {:?}",
        report.result
    );
    assert_eq!(
        report.output.as_ref(),
        &[0x01],
        "Expected success byte 0x01 from EXECUTE precompile"
    );

    println!("Contract demo succeeded!");
    println!("  L2 state transition verified via L1 contract call:");
    println!("    Pre-state root:  {pre_state_root:?}");
    println!("    Post-state root: {post_state_root:?}");
    println!("  Gas used: {}", report.gas_used);
}
