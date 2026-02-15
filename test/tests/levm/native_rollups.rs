//! Tests for the EXECUTE precompile (Native Rollups EIP-8079 PoC).
//!
//! Includes:
//! - Unit tests that call execute_precompile()/execute_inner() directly
//! - Contract-based test that exercises NativeRollup.sol via the in-process VM

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    dead_code
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
    execute_precompile::{Deposit, ExecutePrecompileInput, execute_inner, execute_precompile},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Trie;
use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
use rustc_hash::FxHashMap;

// ===== Helpers =====

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

/// Build ABI-encoded calldata for the EXECUTE precompile.
///
/// Format: abi.encode(bytes32 preStateRoot, bytes blockRlp, bytes witnessJson, bytes deposits)
fn build_precompile_calldata(
    pre_state_root: H256,
    deposits: &[Deposit],
    block_rlp: &[u8],
    witness_json: &[u8],
) -> Vec<u8> {
    // Build packed deposits bytes
    let mut deposits_data = Vec::new();
    for deposit in deposits {
        deposits_data.extend_from_slice(deposit.address.as_bytes());
        deposits_data.extend_from_slice(&deposit.amount.to_big_endian());
    }

    // Helper: pad to 32-byte boundary
    fn pad32(len: usize) -> usize {
        len + ((32 - (len % 32)) % 32)
    }

    // Calculate offsets (relative to start of calldata, after the 4 static words = 128 bytes)
    let block_offset: usize = 128; // 4 * 32
    let block_padded = pad32(block_rlp.len());
    let witness_offset: usize = block_offset + 32 + block_padded;
    let witness_padded = pad32(witness_json.len());
    let deposits_offset: usize = witness_offset + 32 + witness_padded;

    let mut data = Vec::new();

    // 1. preStateRoot (bytes32)
    data.extend_from_slice(pre_state_root.as_bytes());

    // 2. offset to blockRlp
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(block_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // 3. offset to witnessJson
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(witness_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // 4. offset to deposits
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(deposits_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // 5. blockRlp: length + data + padding
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(block_rlp.len() as u64).to_be_bytes());
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(block_rlp);
    data.resize(data.len() + (block_padded - block_rlp.len()), 0);

    // 6. witnessJson: length + data + padding
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(witness_json.len() as u64).to_be_bytes());
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(witness_json);
    data.resize(data.len() + (witness_padded - witness_json.len()), 0);

    // 7. deposits: length + data + padding
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(deposits_data.len() as u64).to_be_bytes());
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(&deposits_data);
    let deposits_padded = pad32(deposits_data.len());
    data.resize(data.len() + (deposits_padded - deposits_data.len()), 0);

    data
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
        deposits: vec![],
        execution_witness: witness,
        block,
    }
}

/// Build the L2 state transition used by both the direct test and the contract test.
///
/// Returns:
///   - ExecutePrecompileInput (for direct execute_inner calls)
///   - block RLP bytes (for binary calldata / contract call)
///   - witness JSON bytes (for binary calldata / contract call)
///   - pre_state_root
///   - post_state_root
fn build_l2_state_transition() -> (ExecutePrecompileInput, Vec<u8>, Vec<u8>, H256, H256) {
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

    let block_rlp = block.encode_to_vec();
    let witness_json = serde_json::to_vec(&witness).expect("witness JSON serialization failed");

    let input = ExecutePrecompileInput {
        pre_state_root,
        deposits: vec![Deposit {
            address: charlie,
            amount: deposit_amount,
        }],
        execution_witness: witness,
        block,
    };

    (
        input,
        block_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
    )
}

// ===== Unit Tests =====

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

    // ===== Build ABI-encoded calldata and execute via execute_precompile() =====
    let block_rlp = block.encode_to_vec();
    let witness_json = serde_json::to_vec(&witness).expect("witness JSON serialization failed");
    let calldata = build_precompile_calldata(pre_state_root, &deposits, &block_rlp, &witness_json);
    println!("ABI-encoded EXECUTE calldata: {} bytes", calldata.len());

    let mut gas_remaining: u64 = 1_000_000;
    let result = execute_precompile(
        &Bytes::from(calldata),
        &mut gas_remaining,
        ethrex_common::types::Fork::Prague,
    );
    match &result {
        Ok(output) => {
            assert_eq!(output.len(), 96, "Expected 96-byte ABI-encoded return");
            let returned_root = H256::from_slice(&output[..32]);
            let returned_block_num = U256::from_big_endian(&output[32..64]);
            let returned_withdrawal_root = H256::from_slice(&output[64..96]);
            assert_eq!(
                returned_root, post_state_root,
                "Returned state root mismatch"
            );
            assert_eq!(
                returned_block_num,
                U256::from(1),
                "Returned block number mismatch"
            );
            // No withdrawals in this block, so the withdrawal root should be zero
            assert_eq!(
                returned_withdrawal_root,
                H256::zero(),
                "Withdrawal root should be zero when no withdrawals"
            );
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

// ===== Contract-based test infrastructure =====

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

/// NativeRollup.sol runtime bytecode (compiled with solc 0.8.31).
///
/// Source: crates/vm/levm/contracts/NativeRollup.sol
/// Compile: cd crates/vm/levm/contracts && solc --bin-runtime NativeRollup.sol -o solc_out --overwrite
const NATIVE_ROLLUP_RUNTIME_HEX: &str = "608060405260043610610089575f3560e01c80639588eca2116100585780639588eca21461027f578063a623f02e146102a9578063a7932794146102e5578063ed3133f214610322578063f340fa011461034a576101c3565b806307132c05146101c75780630a0454441461020357806357e871e71461022b5780637b89893914610255576101c3565b366101c3575f34116100d0576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100c790610cb5565b60405180910390fd5b600260405180604001604052803373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550503373ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516101b99190610ceb565b60405180910390a2005b5f5ffd5b3480156101d2575f5ffd5b506101ed60048036038101906101e89190610d3f565b610366565b6040516101fa9190610d84565b60405180910390f35b34801561020e575f5ffd5b5061022960048036038101906102249190610e82565b610383565b005b348015610236575f5ffd5b5061023f61075e565b60405161024c9190610ceb565b60405180910390f35b348015610260575f5ffd5b50610269610764565b6040516102769190610ceb565b60405180910390f35b34801561028a575f5ffd5b5061029361076a565b6040516102a09190610f3b565b60405180910390f35b3480156102b4575f5ffd5b506102cf60048036038101906102ca9190610f54565b61076f565b6040516102dc9190610f3b565b60405180910390f35b3480156102f0575f5ffd5b5061030b60048036038101906103069190610f54565b610784565b604051610319929190610f8e565b60405180910390f35b34801561032d575f5ffd5b506103486004803603810190610343919061100a565b6107d2565b005b610364600480360381019061035f919061109b565b610a64565b005b6005602052805f5260405f205f915054906101000a900460ff1681565b60065f9054906101000a900460ff16156103d2576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016103c990611110565b60405180910390fd5b600160065f6101000a81548160ff021916908315150217905550600154831115610431576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161042890611178565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168673ffffffffffffffffffffffffffffffffffffffff160361049f576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610496906111e0565b60405180910390fd5b5f85116104e1576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104d890611248565b60405180910390fd5b5f878787876040516020016104f994939291906112cb565b60405160208183030381529060405280519060200120905060055f8281526020019081526020015f205f9054906101000a900460ff161561056f576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161056690611362565b60405180910390fd5b5f60045f8681526020019081526020015f205490505f5f1b81036105c8576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016105bf906113ca565b60405180910390fd5b5f6105d585858486610b9a565b905080610617576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161060e90611432565b60405180910390fd5b600160055f8581526020019081526020015f205f6101000a81548160ff0219169083151502179055505f8973ffffffffffffffffffffffffffffffffffffffff16896040516106659061147d565b5f6040518083038185875af1925050503d805f811461069f576040519150601f19603f3d011682016040523d82523d5f602084013e6106a4565b606091505b50509050806106e8576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016106df906114db565b60405180910390fd5b87878b73ffffffffffffffffffffffffffffffffffffffff167f1113af8a2f367ad0f39a44a9985b12833c5e9dcb54532dd60575fc4ccbd5f9818c6040516107309190610ceb565b60405180910390a4505050505f60065f6101000a81548160ff02191690831515021790555050505050505050565b60015481565b60035481565b5f5481565b6004602052805f5260405f205f915090505481565b60028181548110610793575f80fd5b905f5260205f2090600202015f91509050805f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16908060010154905082565b5f600354905060028054905086826107ea9190611526565b111561082b576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610822906115a3565b60405180910390fd5b60605f5f90505b878110156108e1575f600282856108499190611526565b8154811061085a576108596115c1565b5b905f5260205f209060020201905082815f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1682600101546040516020016108a29291906115ee565b6040516020818303038152906040526040516020016108c2929190611661565b6040516020818303038152906040529250508080600101915050610832565b5086826108ee9190611526565b6003819055505f5f54878787878660405160200161091196959493929190611716565b60405160208183030381529060405290505f5f61010173ffffffffffffffffffffffffffffffffffffffff168360405161094b9190611772565b5f604051808303815f865af19150503d805f8114610984576040519150601f19603f3d011682016040523d82523d5f602084013e610989565b606091505b509150915081801561099c575060608151145b6109db576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016109d2906117f8565b60405180910390fd5b5f5f5f838060200190518101906109f2919061183e565b925092509250825f81905550816001819055508060045f8481526020019081526020015f2081905550817ff043fac73e6b482de32fb49fc68e40396eba14ca2d2c494f5a795a2dd317c5e88483604051610a4d92919061188e565b60405180910390a250505050505050505050505050565b5f3411610aa6576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610a9d90610cb5565b60405180910390fd5b600260405180604001604052808373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550508073ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e34604051610b8f9190610ceb565b60405180910390a250565b5f5f8290505f5f90505b86869050811015610be157610bd282888884818110610bc657610bc56115c1565b5b90506020020135610bf0565b91508080600101915050610ba4565b50838114915050949350505050565b5f81831015610c29578282604051602001610c0c9291906118d5565b604051602081830303815290604052805190602001209050610c55565b8183604051602001610c3c9291906118d5565b6040516020818303038152906040528051906020012090505b92915050565b5f82825260208201905092915050565b7f4d7573742073656e6420455448000000000000000000000000000000000000005f82015250565b5f610c9f600d83610c5b565b9150610caa82610c6b565b602082019050919050565b5f6020820190508181035f830152610ccc81610c93565b9050919050565b5f819050919050565b610ce581610cd3565b82525050565b5f602082019050610cfe5f830184610cdc565b92915050565b5f5ffd5b5f5ffd5b5f819050919050565b610d1e81610d0c565b8114610d28575f5ffd5b50565b5f81359050610d3981610d15565b92915050565b5f60208284031215610d5457610d53610d04565b5b5f610d6184828501610d2b565b91505092915050565b5f8115159050919050565b610d7e81610d6a565b82525050565b5f602082019050610d975f830184610d75565b92915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610dc682610d9d565b9050919050565b610dd681610dbc565b8114610de0575f5ffd5b50565b5f81359050610df181610dcd565b92915050565b610e0081610cd3565b8114610e0a575f5ffd5b50565b5f81359050610e1b81610df7565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f840112610e4257610e41610e21565b5b8235905067ffffffffffffffff811115610e5f57610e5e610e25565b5b602083019150836020820283011115610e7b57610e7a610e29565b5b9250929050565b5f5f5f5f5f5f5f60c0888a031215610e9d57610e9c610d04565b5b5f610eaa8a828b01610de3565b9750506020610ebb8a828b01610de3565b9650506040610ecc8a828b01610e0d565b9550506060610edd8a828b01610e0d565b9450506080610eee8a828b01610e0d565b93505060a088013567ffffffffffffffff811115610f0f57610f0e610d08565b5b610f1b8a828b01610e2d565b925092505092959891949750929550565b610f3581610d0c565b82525050565b5f602082019050610f4e5f830184610f2c565b92915050565b5f60208284031215610f6957610f68610d04565b5b5f610f7684828501610e0d565b91505092915050565b610f8881610dbc565b82525050565b5f604082019050610fa15f830185610f7f565b610fae6020830184610cdc565b9392505050565b5f5f83601f840112610fca57610fc9610e21565b5b8235905067ffffffffffffffff811115610fe757610fe6610e25565b5b60208301915083600182028301111561100357611002610e29565b5b9250929050565b5f5f5f5f5f6060868803121561102357611022610d04565b5b5f61103088828901610e0d565b955050602086013567ffffffffffffffff81111561105157611050610d08565b5b61105d88828901610fb5565b9450945050604086013567ffffffffffffffff8111156110805761107f610d08565b5b61108c88828901610fb5565b92509250509295509295909350565b5f602082840312156110b0576110af610d04565b5b5f6110bd84828501610de3565b91505092915050565b7f5265656e7472616e637947756172643a207265656e7472616e742063616c6c005f82015250565b5f6110fa601f83610c5b565b9150611105826110c6565b602082019050919050565b5f6020820190508181035f830152611127816110ee565b9050919050565b7f426c6f636b206e6f74207965742066696e616c697a65640000000000000000005f82015250565b5f611162601783610c5b565b915061116d8261112e565b602082019050919050565b5f6020820190508181035f83015261118f81611156565b9050919050565b7f496e76616c6964207265636569766572000000000000000000000000000000005f82015250565b5f6111ca601083610c5b565b91506111d582611196565b602082019050919050565b5f6020820190508181035f8301526111f7816111be565b9050919050565b7f416d6f756e74206d75737420626520706f7369746976650000000000000000005f82015250565b5f611232601783610c5b565b915061123d826111fe565b602082019050919050565b5f6020820190508181035f83015261125f81611226565b9050919050565b5f8160601b9050919050565b5f61127c82611266565b9050919050565b5f61128d82611272565b9050919050565b6112a56112a082610dbc565b611283565b82525050565b5f819050919050565b6112c56112c082610cd3565b6112ab565b82525050565b5f6112d68287611294565b6014820191506112e68286611294565b6014820191506112f682856112b4565b60208201915061130682846112b4565b60208201915081905095945050505050565b7f5769746864726177616c20616c726561647920636c61696d65640000000000005f82015250565b5f61134c601a83610c5b565b915061135782611318565b602082019050919050565b5f6020820190508181035f83015261137981611340565b9050919050565b7f4e6f207769746864726177616c7320666f72207468697320626c6f636b0000005f82015250565b5f6113b4601d83610c5b565b91506113bf82611380565b602082019050919050565b5f6020820190508181035f8301526113e1816113a8565b9050919050565b7f496e76616c6964204d65726b6c652070726f6f660000000000000000000000005f82015250565b5f61141c601483610c5b565b9150611427826113e8565b602082019050919050565b5f6020820190508181035f83015261144981611410565b9050919050565b5f81905092915050565b50565b5f6114685f83611450565b91506114738261145a565b5f82019050919050565b5f6114878261145d565b9150819050919050565b7f455448207472616e73666572206661696c6564000000000000000000000000005f82015250565b5f6114c5601383610c5b565b91506114d082611491565b602082019050919050565b5f6020820190508181035f8301526114f2816114b9565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61153082610cd3565b915061153b83610cd3565b9250828201905080821115611553576115526114f9565b5b92915050565b7f4e6f7420656e6f756768206465706f73697473000000000000000000000000005f82015250565b5f61158d601383610c5b565b915061159882611559565b602082019050919050565b5f6020820190508181035f8301526115ba81611581565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f6115f98285611294565b60148201915061160982846112b4565b6020820191508190509392505050565b5f81519050919050565b8281835e5f83830152505050565b5f61163b82611619565b6116458185611450565b9350611655818560208601611623565b80840191505092915050565b5f61166c8285611631565b91506116788284611631565b91508190509392505050565b5f82825260208201905092915050565b828183375f83830152505050565b5f601f19601f8301169050919050565b5f6116bd8385611684565b93506116ca838584611694565b6116d3836116a2565b840190509392505050565b5f6116e882611619565b6116f28185611684565b9350611702818560208601611623565b61170b816116a2565b840191505092915050565b5f6080820190506117295f830189610f2c565b818103602083015261173c8187896116b2565b905081810360408301526117518185876116b2565b9050818103606083015261176581846116de565b9050979650505050505050565b5f61177d8284611631565b915081905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f6117e2602683610c5b565b91506117ed82611788565b604082019050919050565b5f6020820190508181035f83015261180f816117d6565b9050919050565b5f8151905061182481610d15565b92915050565b5f8151905061183881610df7565b92915050565b5f5f5f6060848603121561185557611854610d04565b5b5f61186286828701611816565b93505060206118738682870161182a565b925050604061188486828701611816565b9150509250925092565b5f6040820190506118a15f830185610f2c565b6118ae6020830184610f2c565b9392505050565b5f819050919050565b6118cf6118ca82610d0c565b6118b5565b82525050565b5f6118e082856118be565b6020820191506118f082846118be565b602082019150819050939250505056fea2646970667358221220f0c240ed44c6267c9e9732d109754c761b375c2811c9533cc35664938782f41264736f6c634300081f0033";

/// Encode a call to NativeRollup.deposit(address).
fn encode_deposit_call(recipient: Address) -> Vec<u8> {
    let mut encoded = Vec::new();
    // Function selector: deposit(address) = 0xf340fa01
    encoded.extend_from_slice(&[0xf3, 0x40, 0xfa, 0x01]);
    let mut addr_bytes = [0u8; 32];
    addr_bytes[12..].copy_from_slice(recipient.as_bytes());
    encoded.extend_from_slice(&addr_bytes);
    encoded
}

/// Encode a call to NativeRollup.advance(uint256, bytes, bytes).
fn encode_advance_call(deposits_count: u64, block_rlp: &[u8], witness_json: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::new();

    // Function selector: advance(uint256,bytes,bytes) = 0xed3133f2
    encoded.extend_from_slice(&[0xed, 0x31, 0x33, 0xf2]);

    // _depositsCount (uint256)
    let mut deposits_count_bytes = [0u8; 32];
    deposits_count_bytes[24..].copy_from_slice(&deposits_count.to_be_bytes());
    encoded.extend_from_slice(&deposits_count_bytes);

    // Offset to _block: 3 static params * 32 = 96 = 0x60
    let mut block_offset = [0u8; 32];
    block_offset[31] = 0x60;
    encoded.extend_from_slice(&block_offset);

    // Offset to _witness: 0x60 + 32 (block length) + padded block data
    let padded_block_len = block_rlp.len() + ((32 - (block_rlp.len() % 32)) % 32);
    let witness_offset: u64 = 96 + 32 + padded_block_len as u64;
    let mut witness_offset_bytes = [0u8; 32];
    witness_offset_bytes[24..].copy_from_slice(&witness_offset.to_be_bytes());
    encoded.extend_from_slice(&witness_offset_bytes);

    // _block: length + data (padded to 32-byte boundary)
    let mut block_len = [0u8; 32];
    block_len[24..].copy_from_slice(&(block_rlp.len() as u64).to_be_bytes());
    encoded.extend_from_slice(&block_len);
    encoded.extend_from_slice(block_rlp);
    let block_padding = (32 - (block_rlp.len() % 32)) % 32;
    encoded.resize(encoded.len() + block_padding, 0);

    // _witness: length + data (padded to 32-byte boundary)
    let mut witness_len = [0u8; 32];
    witness_len[24..].copy_from_slice(&(witness_json.len() as u64).to_be_bytes());
    encoded.extend_from_slice(&witness_len);
    encoded.extend_from_slice(witness_json);
    let witness_padding = (32 - (witness_json.len() % 32)) % 32;
    encoded.resize(encoded.len() + witness_padding, 0);

    encoded
}

/// NativeRollup contract with deposit + advance to verify an L2 state transition.
///
/// This test shows the full end-to-end flow:
///   1. deposit(charlie) with 5 ETH → records pending deposit
///   2. advance(1, blockRlp, witnessJson)
///      → NativeRollup builds binary calldata → CALL to 0x0101 → EXECUTE precompile
///      → parse binary calldata → re-execute L2 block → verify state roots
///      → success → contract updates stateRoot, blockNumber, depositIndex
#[test]
fn test_native_rollup_contract() {
    let (_input, block_rlp, witness_json, pre_state_root, post_state_root) =
        build_l2_state_transition();

    let charlie = Address::from_low_u64_be(0xC4A);
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

    // Deploy NativeRollup contract on "L1" (pre-loaded with runtime bytecode + initial storage)
    let contract_address = Address::from_low_u64_be(0xFFFF);
    let sender = Address::from_low_u64_be(0x1234);

    let runtime_bytecode =
        Bytes::from(hex::decode(NATIVE_ROLLUP_RUNTIME_HEX).expect("invalid hex"));
    let contract_code_hash = H256(keccak_hash(runtime_bytecode.as_ref()));
    let contract_code = Code::from_bytecode(runtime_bytecode);

    // Pre-populate contract storage: slot 0 = stateRoot (pre_state_root)
    let mut contract_storage: FxHashMap<H256, U256> = FxHashMap::default();
    contract_storage.insert(
        H256::zero(),
        U256::from_big_endian(pre_state_root.as_bytes()),
    );

    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    accounts.insert(
        contract_address,
        Account {
            info: AccountInfo {
                code_hash: contract_code_hash,
                balance: U256::zero(),
                nonce: 1,
            },
            code: contract_code,
            storage: contract_storage,
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

    // === TX 1: deposit(charlie) with 5 ETH ===
    let deposit_calldata = encode_deposit_call(charlie);

    let deposit_tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000_000,
        to: TxKind::Call(contract_address),
        value: deposit_amount,
        data: Bytes::from(deposit_calldata),
        access_list: vec![],
        ..Default::default()
    });

    let deposit_env = Environment {
        origin: sender,
        gas_limit: 1_000_000_000,
        block_gas_limit: 1_000_000_000,
        tx_nonce: 0,
        chain_id: U256::from(1),
        ..Default::default()
    };

    let mut vm = VM::new(
        deposit_env,
        &mut db,
        &deposit_tx,
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("VM creation failed");

    let deposit_report = vm.execute().expect("VM execution failed");
    assert!(
        matches!(deposit_report.result, TxResult::Success),
        "Deposit transaction reverted: {:?}",
        deposit_report.result
    );
    println!("Deposit TX succeeded (5 ETH for charlie)");

    // === TX 2: advance(1, blockRlp, witnessJson) ===
    let advance_calldata = encode_advance_call(1, &block_rlp, &witness_json);

    let advance_tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 1,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000_000,
        to: TxKind::Call(contract_address),
        value: U256::zero(),
        data: Bytes::from(advance_calldata),
        access_list: vec![],
        ..Default::default()
    });

    let advance_env = Environment {
        origin: sender,
        gas_limit: 1_000_000_000,
        block_gas_limit: 1_000_000_000,
        tx_nonce: 1,
        chain_id: U256::from(1),
        ..Default::default()
    };

    let mut vm = VM::new(
        advance_env,
        &mut db,
        &advance_tx,
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("VM creation failed");

    let report = vm.execute().expect("VM execution failed");

    assert!(
        matches!(report.result, TxResult::Success),
        "L1 advance transaction reverted: {:?}",
        report.result
    );

    // Verify the contract updated its storage
    let contract_account = db.get_account(contract_address).expect("account not found");
    let stored_state_root = contract_account
        .storage
        .get(&H256::zero())
        .copied()
        .unwrap_or_default();
    let stored_block_number = contract_account
        .storage
        .get(&H256::from_low_u64_be(1))
        .copied()
        .unwrap_or_default();
    let stored_deposit_index = contract_account
        .storage
        .get(&H256::from_low_u64_be(3))
        .copied()
        .unwrap_or_default();

    // Convert stored U256 back to H256 for comparison
    let root_bytes = stored_state_root.to_big_endian();
    assert_eq!(
        H256::from(root_bytes),
        post_state_root,
        "Contract stateRoot mismatch"
    );
    assert_eq!(
        stored_block_number,
        U256::from(1),
        "Contract blockNumber mismatch"
    );
    assert_eq!(
        stored_deposit_index,
        U256::from(1),
        "Contract depositIndex mismatch"
    );

    println!("NativeRollup contract demo succeeded!");
    println!("  L2 state transition verified via deposit() + advance():");
    println!("    Pre-state root:  {pre_state_root:?}");
    println!("    Post-state root: {post_state_root:?}");
    println!("    Block number:    1");
    println!("    Deposit index:   1");
    println!("  Gas used: {}", report.gas_used);
}
