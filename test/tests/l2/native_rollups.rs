//! Integration test for the NativeRollup contract on a real L1.
//!
//! Requires:
//!   1. Start L1: `NATIVE_ROLLUPS=1 make -C crates/l2 init-l1`
//!   2. Run: `cargo test -p ethrex-test --features native-rollups -- l2::native_rollups --nocapture`
//!
//! The test compiles and deploys NativeRollup.sol, builds an L2 state transition,
//! calls sendL1Message() + advance(), and verifies the contract state was updated.

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
    merkle_tree::{compute_merkle_proof, compute_merkle_root},
    types::{
        AccountState, Block, BlockBody, BlockHeader, ChainConfig, EIP1559Transaction,
        ELASTICITY_MULTIPLIER, Receipt, Transaction, TxKind, TxType,
        block_execution_witness::{ExecutionWitness, GuestProgramState},
        calculate_base_fee_per_gas,
    },
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_l2_sdk::{
    build_generic_tx, compile_contract, create_deploy, send_generic_transaction,
    wait_for_transaction_receipt,
};
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    db::guest_program_state_db::GuestProgramStateDb,
    environment::{EVMConfig, Environment},
    errors::TxResult,
    execute_precompile::{ExecutePrecompileInput, L1_ANCHOR, L2_BRIDGE},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::clients::Overrides;
use ethrex_rpc::clients::eth::EthClient;
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use ethrex_trie::Trie;
use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
use reqwest::Url;
use secp256k1::SecretKey;

use super::utils::{test_chain_config, workspace_root};

const L1_RPC_URL: &str = "http://localhost:8545";
/// Private key from crates/l2/Makefile (pre-funded in L1 genesis).
const L1_PRIVATE_KEY: &str = "385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924";

// ===== Helpers for building the L2 state transition =====
// These use manual signing because they build a raw L2 block for the witness,
// not an RPC transaction.

fn address_from_key(key: &SigningKey) -> Address {
    use k256::ecdsa::VerifyingKey;
    let verifying_key = VerifyingKey::from(key);
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let hash = keccak_hash(&pubkey_bytes.as_bytes()[1..]);
    Address::from_slice(&hash[12..])
}

fn sign_eip1559_tx(tx: &mut EIP1559Transaction, key: &SigningKey) {
    use ethrex_rlp::structs::Encoder;

    let mut buf = vec![0x02u8];
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

fn build_test_environment(
    origin: Address,
    gas_limit: u64,
    tx_nonce: u64,
    config: EVMConfig,
    block_number: u64,
    coinbase: Address,
    timestamp: u64,
    prev_randao: H256,
    chain_id: u64,
    base_fee: u64,
    gas_price: U256,
) -> Environment {
    Environment {
        origin,
        gas_limit,
        config,
        block_number: U256::from(block_number),
        coinbase,
        timestamp: U256::from(timestamp),
        prev_randao: Some(prev_randao),
        slot_number: U256::zero(),
        chain_id: U256::from(chain_id),
        base_fee_per_gas: U256::from(base_fee),
        base_blob_fee_per_gas: U256::zero(),
        gas_price,
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: Some(U256::from(1_000_000_000u64)),
        tx_max_fee_per_gas: Some(U256::from(2_000_000_000u64)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce,
        block_gas_limit: 30_000_000,
        difficulty: U256::zero(),
        is_privileged: false,
        fee_token: None,
    }
}

fn insert_account(trie: &mut Trie, address: Address, state: &AccountState) {
    let hashed_addr = keccak_hash(address.to_fixed_bytes()).to_vec();
    trie.insert(hashed_addr, state.encode_to_vec())
        .expect("trie insert failed");
}

fn get_trie_root_node(trie: &Trie) -> Option<ethrex_trie::Node> {
    trie.hash_no_commit();
    trie.root_node()
        .expect("root_node failed")
        .map(|arc_node| (*arc_node).clone())
}

/// Read the L2Bridge compiled runtime bytecode from solc output.
fn bridge_runtime_bytecode() -> Vec<u8> {
    let path = workspace_root().join("crates/vm/levm/contracts/solc_out/L2Bridge.bin-runtime");
    let hex_str = std::fs::read_to_string(&path)
        .expect("L2Bridge.bin-runtime not found — compile with solc first");
    hex::decode(hex_str.trim()).expect("invalid hex in bridge .bin-runtime")
}

/// Read the L1Anchor compiled runtime bytecode from solc output.
fn anchor_runtime_bytecode() -> Vec<u8> {
    let path = workspace_root().join("crates/vm/levm/contracts/solc_out/L1Anchor.bin-runtime");
    let hex_str = std::fs::read_to_string(&path)
        .expect("L1Anchor.bin-runtime not found — compile with solc first");
    hex::decode(hex_str.trim()).expect("invalid hex in anchor .bin-runtime")
}

/// Build the L2 state transition (relayer→L2Bridge processL1Message + Alice→Bob transfer).
///
/// The L2 genesis state includes the L2Bridge contract at `L2_BRIDGE` with preminted ETH
/// and a relayer account. Block 1 contains:
///   - TX0: relayer calls L2Bridge.processL1Message(l1_sender, charlie, 5 ETH, 100k, "", 0)
///   - TX1: alice sends 1 ETH to bob
///
/// Returns (input, transactions_rlp, witness_json, pre_state_root, post_state_root,
///          l1_anchor, gas_used_tx0, block).
#[allow(clippy::type_complexity)]
fn build_l2_state_transition(
    l1_sender: Address,
) -> (
    ExecutePrecompileInput,
    Vec<u8>,
    Vec<u8>,
    H256,
    H256,
    H256,
    u64,
    Block,
) {
    let alice_key = SigningKey::from_bytes(&[1u8; 32].into()).expect("valid key");
    let alice = address_from_key(&alice_key);
    let relayer_key = SigningKey::from_bytes(&[2u8; 32].into()).expect("valid key");
    let relayer = address_from_key(&relayer_key);
    let bob = Address::from_low_u64_be(0xB0B);
    let charlie = Address::from_low_u64_be(0xC4A);
    let coinbase = Address::from_low_u64_be(0xC01);
    let chain_id: u64 = 1;
    let base_fee: u64 = 1_000_000_000;

    // ===== L2Bridge setup =====
    let bridge_runtime = bridge_runtime_bytecode();
    let bridge_code_hash = H256(keccak_hash(&bridge_runtime));

    // Bridge storage: slot 0 = relayer address
    let mut bridge_storage_trie = Trie::new_temp();
    let slot0_key = keccak_hash(&[0u8; 32]).to_vec();
    let mut relayer_padded = [0u8; 32];
    relayer_padded[12..].copy_from_slice(relayer.as_bytes());
    let relayer_u256 = U256::from_big_endian(&relayer_padded);
    bridge_storage_trie
        .insert(slot0_key, relayer_u256.encode_to_vec())
        .expect("storage insert");
    let bridge_storage_root = bridge_storage_trie.hash_no_commit();

    // ===== Balances =====
    let alice_balance = U256::from(10) * U256::from(10).pow(U256::from(18)); // 10 ETH
    let relayer_balance = U256::from(1) * U256::from(10).pow(U256::from(18)); // 1 ETH for gas
    let bridge_premint = U256::from(100) * U256::from(10).pow(U256::from(18)); // 100 ETH
    let l1_msg_value = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH
    let l1_msg_gas_limit: u64 = 100_000; // subcall gas limit on L2

    // ===== Compute L1 message hash and Merkle root (l1_anchor) =====
    let empty_data_hash = H256::from(keccak_hash(&[]));
    let mut msg_preimage = Vec::with_capacity(168);
    msg_preimage.extend_from_slice(l1_sender.as_bytes());
    msg_preimage.extend_from_slice(charlie.as_bytes());
    msg_preimage.extend_from_slice(&l1_msg_value.to_big_endian());
    msg_preimage.extend_from_slice(&U256::from(l1_msg_gas_limit).to_big_endian());
    msg_preimage.extend_from_slice(empty_data_hash.as_bytes());
    msg_preimage.extend_from_slice(&U256::zero().to_big_endian());
    let l1_msg_hash = H256::from(keccak_hash(&msg_preimage));
    let l1_anchor = compute_merkle_root(&[l1_msg_hash]);
    let merkle_proof = compute_merkle_proof(&[l1_msg_hash], 0);

    // ===== L1Anchor setup =====
    let anchor_runtime = anchor_runtime_bytecode();
    let anchor_code_hash = H256(keccak_hash(&anchor_runtime));
    let mut anchor_storage_trie = Trie::new_temp();
    let anchor_slot0_key = keccak_hash(&[0u8; 32]).to_vec();
    let anchor_value = U256::from_big_endian(l1_anchor.as_bytes());
    anchor_storage_trie
        .insert(anchor_slot0_key, anchor_value.encode_to_vec())
        .expect("storage insert");
    let anchor_storage_root = anchor_storage_trie.hash_no_commit();

    // ===== Genesis state trie =====
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
    insert_account(
        &mut state_trie,
        relayer,
        &AccountState {
            nonce: 0,
            balance: relayer_balance,
            ..Default::default()
        },
    );
    insert_account(&mut state_trie, coinbase, &AccountState::default());
    insert_account(&mut state_trie, bob, &AccountState::default());
    insert_account(&mut state_trie, charlie, &AccountState::default());
    insert_account(
        &mut state_trie,
        L2_BRIDGE,
        &AccountState {
            nonce: 1,
            balance: bridge_premint,
            code_hash: bridge_code_hash,
            storage_root: bridge_storage_root,
        },
    );
    insert_account(
        &mut state_trie,
        L1_ANCHOR,
        &AccountState {
            nonce: 1,
            code_hash: anchor_code_hash,
            storage_root: anchor_storage_root,
            ..Default::default()
        },
    );
    let pre_state_root = state_trie.hash_no_commit();

    // gas_used = gas_limit / ELASTICITY_MULTIPLIER keeps base fee stable
    let parent_header = BlockHeader {
        number: 0,
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        gas_used: 15_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_000,
        ..Default::default()
    };

    // ===== Build transactions =====
    // TX0: relayer → L2Bridge.processL1Message(l1_sender, charlie, 5 ETH, 100k, "", 0, proof)
    let l1_msg_calldata = encode_calldata(
        "processL1Message(address,address,uint256,uint256,bytes,uint256,bytes32[])",
        &[
            Value::Address(l1_sender),
            Value::Address(charlie),
            Value::Uint(l1_msg_value),
            Value::Uint(U256::from(l1_msg_gas_limit)),
            Value::Bytes(Bytes::new()),
            Value::Uint(U256::zero()),
            Value::Array(
                merkle_proof
                    .iter()
                    .map(|h| Value::FixedBytes(Bytes::from(h.as_bytes().to_vec())))
                    .collect(),
            ),
        ],
    )
    .expect("encode processL1Message calldata failed");
    let mut tx0 = EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 2_000_000_000,
        gas_limit: 200_000,
        to: TxKind::Call(L2_BRIDGE),
        value: U256::zero(),
        data: Bytes::from(l1_msg_calldata),
        access_list: vec![],
        ..Default::default()
    };
    sign_eip1559_tx(&mut tx0, &relayer_key);
    let tx0 = Transaction::EIP1559Transaction(tx0);

    // TX1: alice → bob 1 ETH
    let transfer_value = U256::from(10).pow(U256::from(18));
    let mut tx1 = EIP1559Transaction {
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
    sign_eip1559_tx(&mut tx1, &alice_key);
    let tx1 = Transaction::EIP1559Transaction(tx1);

    let transactions = vec![tx0.clone(), tx1.clone()];
    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);

    // ===== Execute through LEVM to get exact gas, receipts, and post-state =====
    let temp_header = BlockHeader {
        parent_hash: parent_header.compute_block_hash(),
        number: 1,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_012,
        coinbase,
        ..Default::default()
    };

    let chain_config = test_chain_config();

    let mut storage_trie_roots = BTreeMap::new();
    storage_trie_roots.insert(
        L2_BRIDGE,
        get_trie_root_node(&bridge_storage_trie).expect("bridge storage root node"),
    );
    storage_trie_roots.insert(
        L1_ANCHOR,
        get_trie_root_node(&anchor_storage_trie).expect("anchor storage root node"),
    );

    let temp_witness = ExecutionWitness {
        codes: vec![bridge_runtime.clone(), anchor_runtime.clone()],
        block_headers_bytes: vec![parent_header.encode_to_vec(), temp_header.encode_to_vec()],
        first_block_number: 1,
        chain_config: chain_config.clone(),
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots: storage_trie_roots.clone(),
        keys: vec![],
    };

    let guest_state: GuestProgramState = temp_witness
        .try_into()
        .expect("Failed to build GuestProgramState");

    let db_inner = Arc::new(GuestProgramStateDb::new(guest_state));
    let db_dyn: Arc<dyn ethrex_levm::db::Database> = db_inner.clone();
    let mut gen_db = GeneralizedDatabase::new(db_dyn);

    let config = EVMConfig::new_from_chain_config(&chain_config, &temp_header);
    let effective_gas_price =
        U256::from(std::cmp::min(1_000_000_000u64 + base_fee, 2_000_000_000u64));

    // Execute TX0: processL1Message
    let env0 = build_test_environment(
        relayer,
        200_000,
        0,
        config,
        1,
        coinbase,
        1_000_012,
        temp_header.prev_randao,
        chain_id,
        base_fee,
        effective_gas_price,
    );

    let mut vm0 = VM::new(
        env0,
        &mut gen_db,
        &tx0,
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("VM creation failed");
    let report0 = vm0.execute().expect("TX0 execution failed");
    assert!(
        matches!(report0.result, TxResult::Success),
        "processL1Message transaction failed: {:?}",
        report0.result
    );
    let gas_used0 = report0.gas_used;

    // Execute TX1: transfer
    let env1 = build_test_environment(
        alice,
        21_000,
        0,
        config,
        1,
        coinbase,
        1_000_012,
        temp_header.prev_randao,
        chain_id,
        base_fee,
        effective_gas_price,
    );

    let mut vm1 = VM::new(
        env1,
        &mut gen_db,
        &tx1,
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("VM creation failed");
    let report1 = vm1.execute().expect("TX1 execution failed");
    assert!(
        matches!(report1.result, TxResult::Success),
        "Transfer transaction failed: {:?}",
        report1.result
    );
    let gas_used1 = report1.gas_used;
    let total_gas_used = gas_used0 + gas_used1;

    // Build receipts (cumulative gas)
    let receipt0 = Receipt::new(tx0.tx_type(), true, gas_used0, report0.logs.clone());
    let receipt1 = Receipt::new(tx1.tx_type(), true, total_gas_used, report1.logs.clone());
    let receipts_root = ethrex_common::types::compute_receipts_root(&[receipt0, receipt1]);

    // Compute post-state root
    let account_updates = gen_db.get_state_transitions().expect("state transitions");
    db_inner
        .state
        .lock()
        .expect("lock")
        .apply_account_updates(&account_updates)
        .expect("apply updates");
    let post_state_root = db_inner
        .state
        .lock()
        .expect("lock")
        .state_trie_root()
        .expect("state root");

    // ===== Build final block header (for witness block_headers_bytes) =====
    let final_header = BlockHeader {
        parent_hash: parent_header.compute_block_hash(),
        number: 1,
        gas_used: total_gas_used,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_012,
        coinbase,
        transactions_root,
        receipts_root,
        state_root: post_state_root,
        withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(&[])),
        ..Default::default()
    };

    // We still need a Block for build_l2_withdrawal_block (which reads block1.header)
    let block = Block {
        header: final_header.clone(),
        body: BlockBody {
            transactions: transactions.clone(),
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    // ===== Build final witness (with correct block header) =====
    let witness = ExecutionWitness {
        codes: vec![bridge_runtime.clone(), anchor_runtime],
        block_headers_bytes: vec![parent_header.encode_to_vec(), final_header.encode_to_vec()],
        first_block_number: 1,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots,
        keys: vec![],
    };

    // l1_anchor was computed earlier (before genesis setup) as compute_merkle_root(&[l1_msg_hash])

    let transactions_rlp = transactions.encode_to_vec();
    let witness_json = serde_json::to_vec(&witness).expect("witness JSON serialization failed");

    let input = ExecutePrecompileInput {
        pre_state_root,
        post_state_root,
        post_receipts_root: receipts_root,
        block_number: 1,
        block_gas_limit: 30_000_000,
        coinbase,
        prev_randao: H256::zero(),
        timestamp: 1_000_012,
        parent_base_fee: base_fee,
        parent_gas_limit: 30_000_000,
        parent_gas_used: 15_000_000,
        l1_anchor,
        transactions: block.body.transactions.clone(),
        execution_witness: witness,
    };

    (
        input,
        transactions_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
        l1_anchor,
        gas_used0,
        block,
    )
}

/// Build an L2 block containing a withdrawal transaction (block 2).
///
/// Executes Alice → L2Bridge.withdraw(receiver) with `withdrawal_amount` ETH.
/// Uses LEVM to compute exact gas_used and post-state root for the block.
///
/// Returns (transactions_rlp, witness_json, block2_post_state_root, receipts_root, base_fee, account_proof, storage_proof).
#[allow(clippy::type_complexity)]
fn build_l2_withdrawal_block(
    block1: &Block,
    block1_post_state_root: H256,
    withdrawal_receiver: Address,
    gas_used_tx0: u64,
    block1_l1_anchor: H256,
) -> (
    Vec<u8>,
    Vec<u8>,
    H256,
    H256,
    u64,
    Vec<Vec<u8>>,
    Vec<Vec<u8>>,
) {
    let alice_key = SigningKey::from_bytes(&[1u8; 32].into()).expect("valid key");
    let alice = address_from_key(&alice_key);
    let relayer_key = SigningKey::from_bytes(&[2u8; 32].into()).expect("valid key");
    let relayer = address_from_key(&relayer_key);
    let bob = Address::from_low_u64_be(0xB0B);
    let charlie = Address::from_low_u64_be(0xC4A);
    let coinbase = Address::from_low_u64_be(0xC01);

    // Compute block 2 base fee from block 1 header (EIP-1559)
    let base_fee: u64 = calculate_base_fee_per_gas(
        30_000_000, // block 2 gas_limit
        block1.header.gas_limit,
        block1.header.gas_used,
        block1.header.base_fee_per_gas.unwrap_or(1_000_000_000),
        ELASTICITY_MULTIPLIER,
    )
    .expect("base fee calculation failed");

    let bridge_runtime = bridge_runtime_bytecode();
    let bridge_code_hash = H256(keccak_hash(&bridge_runtime));
    let anchor_runtime = anchor_runtime_bytecode();
    let anchor_code_hash = H256(keccak_hash(&anchor_runtime));

    // Reconstruct block 1 post-state balances (must match build_l2_state_transition)
    let alice_initial = U256::from(10) * U256::from(10).pow(U256::from(18));
    let transfer_value = U256::from(10).pow(U256::from(18));
    let effective_gas_price: u64 = 2_000_000_000; // min(1gwei + 1gwei, 2gwei)
    let priority_fee: u64 = 1_000_000_000; // effective - base_fee (base_fee = 1gwei)
    let l1_msg_value = U256::from(5) * U256::from(10).pow(U256::from(18));
    let bridge_premint = U256::from(100) * U256::from(10).pow(U256::from(18));

    // Relayer: nonce=1, balance = 1 ETH - gas_used_tx0 * 2gwei
    let relayer_initial = U256::from(1) * U256::from(10).pow(U256::from(18));
    let relayer_gas_cost = U256::from(gas_used_tx0) * U256::from(effective_gas_price);
    // Alice: nonce=1, balance = 10 ETH - 1 ETH - 21000 * 2gwei
    let alice_gas_cost = U256::from(21_000u64) * U256::from(effective_gas_price);
    // Coinbase: balance = (gas_used_tx0 + 21000) * 1gwei (priority fee portion)
    let coinbase_reward = U256::from(gas_used_tx0 + 21_000u64) * U256::from(priority_fee);

    // Build bridge storage trie for block 2 pre-state (after processL1Message: slot0=relayer, slot1=l1MessageNonce=1)
    let mut bridge_storage_trie = Trie::new_temp();
    // slot 0: relayer address
    let slot0_key = keccak_hash(&[0u8; 32]).to_vec();
    let mut relayer_padded = [0u8; 32];
    relayer_padded[12..].copy_from_slice(relayer.as_bytes());
    let relayer_u256 = U256::from_big_endian(&relayer_padded);
    bridge_storage_trie
        .insert(slot0_key, relayer_u256.encode_to_vec())
        .expect("storage insert");
    // slot 1: l1MessageNonce = 1
    let mut slot1_raw = [0u8; 32];
    slot1_raw[31] = 1;
    let slot1_key = keccak_hash(&slot1_raw).to_vec();
    bridge_storage_trie
        .insert(slot1_key, U256::from(1).encode_to_vec())
        .expect("storage insert");
    let bridge_storage_root = bridge_storage_trie.hash_no_commit();

    // Build block 2 pre-state trie (= block 1 post-state including bridge)
    let mut pre_trie = Trie::new_temp();
    insert_account(
        &mut pre_trie,
        alice,
        &AccountState {
            nonce: 1,
            balance: alice_initial - transfer_value - alice_gas_cost,
            ..Default::default()
        },
    );
    insert_account(
        &mut pre_trie,
        relayer,
        &AccountState {
            nonce: 1,
            balance: relayer_initial - relayer_gas_cost,
            ..Default::default()
        },
    );
    insert_account(
        &mut pre_trie,
        bob,
        &AccountState {
            balance: transfer_value,
            ..Default::default()
        },
    );
    insert_account(
        &mut pre_trie,
        coinbase,
        &AccountState {
            balance: coinbase_reward,
            ..Default::default()
        },
    );
    insert_account(
        &mut pre_trie,
        charlie,
        &AccountState {
            balance: l1_msg_value,
            ..Default::default()
        },
    );
    insert_account(
        &mut pre_trie,
        L2_BRIDGE,
        &AccountState {
            nonce: 1,
            balance: bridge_premint - l1_msg_value,
            code_hash: bridge_code_hash,
            storage_root: bridge_storage_root,
        },
    );

    // L1Anchor post-state from block 1: slot 0 = block1_l1_anchor
    let mut anchor_storage_trie = Trie::new_temp();
    let anchor_slot0_key = keccak_hash(&[0u8; 32]).to_vec();
    let anchor_value = U256::from_big_endian(block1_l1_anchor.as_bytes());
    anchor_storage_trie
        .insert(anchor_slot0_key, anchor_value.encode_to_vec())
        .expect("storage insert");
    let anchor_storage_root = anchor_storage_trie.hash_no_commit();
    insert_account(
        &mut pre_trie,
        L1_ANCHOR,
        &AccountState {
            nonce: 1,
            code_hash: anchor_code_hash,
            storage_root: anchor_storage_root,
            ..Default::default()
        },
    );

    let pre_state_root = pre_trie.hash_no_commit();
    assert_eq!(
        pre_state_root, block1_post_state_root,
        "Block 2 pre-state root must match block 1 post-state root"
    );

    // Block 2 header fields (gas_used and state_root filled after execution)
    let block1_hash = block1.header.compute_block_hash();
    let block2_number: u64 = 2;
    let block2_timestamp: u64 = block1.header.timestamp + 12;
    let chain_config = test_chain_config();

    // Build withdrawal transaction: Alice → bridge.withdraw(receiver) with 1 ETH
    let withdrawal_amount = U256::from(10).pow(U256::from(18));
    let withdraw_selector = &keccak_hash(b"withdraw(address)")[..4];
    let mut withdraw_calldata = Vec::with_capacity(36);
    withdraw_calldata.extend_from_slice(withdraw_selector);
    let mut addr_bytes = [0u8; 32];
    addr_bytes[12..].copy_from_slice(withdrawal_receiver.as_bytes());
    withdraw_calldata.extend_from_slice(&addr_bytes);

    let gas_limit: u64 = 100_000;
    let mut tx = EIP1559Transaction {
        chain_id: 1,
        nonce: 1, // Alice's nonce after block 1
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 2_000_000_000,
        gas_limit,
        to: TxKind::Call(L2_BRIDGE),
        value: withdrawal_amount,
        data: Bytes::from(withdraw_calldata),
        access_list: vec![],
        ..Default::default()
    };
    sign_eip1559_tx(&mut tx, &alice_key);
    let transaction = Transaction::EIP1559Transaction(tx);

    // Execute through LEVM to get exact gas_used and post-state root.
    // Build a temporary witness to create a GuestProgramState for execution.
    let temp_header = BlockHeader {
        parent_hash: block1_hash,
        number: block2_number,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: block2_timestamp,
        coinbase,
        ..Default::default()
    };

    let temp_witness = ExecutionWitness {
        codes: vec![bridge_runtime.clone(), anchor_runtime.clone()],
        block_headers_bytes: vec![block1.header.encode_to_vec(), temp_header.encode_to_vec()],
        first_block_number: block2_number,
        chain_config: chain_config.clone(),
        state_trie_root: get_trie_root_node(&pre_trie),
        storage_trie_roots: {
            let mut m = BTreeMap::new();
            m.insert(
                L2_BRIDGE,
                get_trie_root_node(&bridge_storage_trie).expect("bridge storage root node"),
            );
            m.insert(
                L1_ANCHOR,
                get_trie_root_node(&anchor_storage_trie).expect("anchor storage root node"),
            );
            m
        },
        keys: vec![],
    };

    let guest_state: GuestProgramState = temp_witness
        .try_into()
        .expect("Failed to build GuestProgramState");

    let db_inner = Arc::new(GuestProgramStateDb::new(guest_state));

    // Simulate EXECUTE system write: L1Anchor slot 0 = block 2 l1_anchor (zero for no messages).
    // This mirrors what the EXECUTE precompile does before executing transactions.
    {
        use ethrex_common::types::AccountUpdate;
        let mut storage = rustc_hash::FxHashMap::default();
        storage.insert(H256::zero(), U256::zero());
        let anchor_update = AccountUpdate {
            address: L1_ANCHOR,
            added_storage: storage,
            ..Default::default()
        };
        db_inner
            .state
            .lock()
            .expect("lock")
            .apply_account_updates(&[anchor_update])
            .expect("system write");
    }

    let db_dyn: Arc<dyn ethrex_levm::db::Database> = db_inner.clone();
    let mut gen_db = GeneralizedDatabase::new(db_dyn);

    let config = EVMConfig::new_from_chain_config(&chain_config, &temp_header);
    let gas_price = U256::from(std::cmp::min(1_000_000_000u64 + base_fee, 2_000_000_000u64));

    let env = build_test_environment(
        alice,
        gas_limit,
        1,
        config,
        block2_number,
        coinbase,
        block2_timestamp,
        temp_header.prev_randao,
        1,
        base_fee,
        gas_price,
    );

    let mut vm = VM::new(
        env,
        &mut gen_db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("VM creation failed");

    let report = vm.execute().expect("Withdrawal tx execution failed");
    assert!(
        matches!(report.result, TxResult::Success),
        "Withdrawal transaction reverted: {:?}",
        report.result
    );

    let gas_used = report.gas_used;
    println!("  [block 2] Withdrawal tx gas_used: {gas_used}");
    println!(
        "  [block 2] Withdrawal tx logs: {} events",
        report.logs.len()
    );

    // Apply state transitions and compute post-state root
    let account_updates = gen_db
        .get_state_transitions()
        .expect("Failed to get state transitions");
    db_inner
        .state
        .lock()
        .expect("Lock poisoned")
        .apply_account_updates(&account_updates)
        .expect("Failed to apply account updates");
    let post_state_root = db_inner
        .state
        .lock()
        .expect("Lock poisoned")
        .state_trie_root()
        .expect("Failed to compute post-state root");

    // Generate MPT proofs for claimWithdrawal on L1
    // 1. Compute withdrawal hash: keccak256(abi.encodePacked(alice, receiver, amount, messageId))
    let mut withdrawal_preimage = Vec::with_capacity(104);
    withdrawal_preimage.extend_from_slice(alice.as_bytes());
    withdrawal_preimage.extend_from_slice(withdrawal_receiver.as_bytes());
    withdrawal_preimage.extend_from_slice(&withdrawal_amount.to_big_endian());
    withdrawal_preimage.extend_from_slice(&U256::zero().to_big_endian()); // messageId = 0
    let withdrawal_hash = keccak_hash(&withdrawal_preimage);

    // 2. Compute storage slot: keccak256(abi.encode(withdrawalHash, uint256(3)))
    //    sentMessages mapping is at slot 3 in L2Bridge
    let mut slot_preimage = [0u8; 64];
    slot_preimage[..32].copy_from_slice(&withdrawal_hash);
    slot_preimage[63] = 3; // sentMessages mapping base slot
    let storage_slot = keccak_hash(slot_preimage);

    // 3. Get account proof (state trie → L2Bridge account)
    let account_trie_key = keccak_hash(L2_BRIDGE.to_fixed_bytes());
    let state = db_inner.state.lock().expect("Lock poisoned");
    let account_proof = state
        .state_trie
        .get_proof(&account_trie_key)
        .expect("Failed to generate account proof");

    // 4. Get storage proof (L2Bridge storage trie → sentMessages[withdrawalHash])
    let storage_trie_key = keccak_hash(storage_slot);
    let storage_proof = state
        .storage_tries
        .get(&L2_BRIDGE)
        .expect("L2Bridge storage trie not found")
        .get_proof(&storage_trie_key)
        .expect("Failed to generate storage proof");
    drop(state);

    println!(
        "  [block 2] MPT proofs: account_proof={} nodes, storage_proof={} nodes",
        account_proof.len(),
        storage_proof.len()
    );

    // Build final block 2 with correct gas_used and state_root
    let transactions = vec![transaction.clone()];
    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);
    let receipt = Receipt::new(transaction.tx_type(), true, gas_used, report.logs);
    let receipts_root = ethrex_common::types::compute_receipts_root(&[receipt]);

    let block2_header = BlockHeader {
        parent_hash: block1_hash,
        number: block2_number,
        gas_used,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: block2_timestamp,
        coinbase,
        transactions_root,
        receipts_root,
        state_root: post_state_root,
        withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(&[])),
        ..Default::default()
    };

    // Build final witness with correct block 2 header
    let witness2 = ExecutionWitness {
        codes: vec![bridge_runtime, anchor_runtime],
        block_headers_bytes: vec![block1.header.encode_to_vec(), block2_header.encode_to_vec()],
        first_block_number: block2_number,
        chain_config,
        state_trie_root: get_trie_root_node(&pre_trie),
        storage_trie_roots: {
            let mut m = BTreeMap::new();
            m.insert(
                L2_BRIDGE,
                get_trie_root_node(&bridge_storage_trie).expect("bridge storage root node"),
            );
            m.insert(
                L1_ANCHOR,
                get_trie_root_node(&anchor_storage_trie).expect("anchor storage root node"),
            );
            m
        },
        keys: vec![],
    };

    let transactions_rlp = transactions.encode_to_vec();
    let witness2_json = serde_json::to_vec(&witness2).expect("witness JSON serialization failed");

    (
        transactions_rlp,
        witness2_json,
        post_state_root,
        receipts_root,
        base_fee,
        account_proof,
        storage_proof,
    )
}

/// Integration test: compile and deploy NativeRollup on a real L1, then advance
/// with two L2 blocks — block 1 (transfer + L1 message) and block 2 (withdrawal).
///
/// Flow:
///   1. Deploy NativeRollup contract
///   2. sendL1Message(charlie, 100k, "") with 5 ETH
///   3. advance(block 1) — transfer + L1 message
///   4. advance(block 2) — withdrawal: Alice withdraws 1 ETH to an L1 receiver
///   5. claimWithdrawal() — the L1 receiver claims the withdrawn ETH
///
/// Prerequisites:
///   1. Start L1: `NATIVE_ROLLUPS=1 make -C crates/l2 init-l1`
///   2. Run: `cargo test -p ethrex-test --features native-rollups -- l2::native_rollups --nocapture`
///
/// The `native-rollups` feature flag gates both compilation and execution.
#[tokio::test]
async fn test_native_rollup_on_l1() {
    // 1. Connect to L1
    let eth_client = EthClient::new(Url::parse(L1_RPC_URL).unwrap()).unwrap();
    let secret_key =
        SecretKey::from_slice(&hex::decode(L1_PRIVATE_KEY).expect("invalid hex")).expect("key");
    let signer: Signer = LocalSigner::new(secret_key).into();

    println!("Connected to L1 at {L1_RPC_URL}");
    println!("Deployer: {:?}", signer.address());

    // 2. Compile contracts (must happen before building L2 state, which reads bridge bytecode)
    let contracts_path = workspace_root().join("crates/vm/levm/contracts");
    compile_contract(
        &contracts_path,
        &contracts_path.join("L2Bridge.sol"),
        true, // runtime bytecode needed for L2 genesis state
        false,
        None,
        &[],
        None,
    )
    .expect("Failed to compile L2Bridge.sol");
    compile_contract(
        &contracts_path,
        &contracts_path.join("L1Anchor.sol"),
        true, // runtime bytecode needed for L2 genesis state
        false,
        None,
        &[],
        None,
    )
    .expect("Failed to compile L1Anchor.sol");
    compile_contract(
        &contracts_path,
        &contracts_path.join("NativeRollup.sol"),
        false,
        false,
        None,
        &[],
        Some(200),
    )
    .expect("Failed to compile NativeRollup.sol");

    // 3. Build L2 state transitions
    // Derive L1 sender address from the deployer key (matches msg.sender in sendL1Message)
    let l1_key_bytes: [u8; 32] = hex::decode(L1_PRIVATE_KEY).unwrap()[..32]
        .try_into()
        .unwrap();
    let l1_sender_key = SigningKey::from_bytes(&l1_key_bytes.into()).expect("valid l1 key");
    let l1_sender = address_from_key(&l1_sender_key);

    let (
        input,
        transactions_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
        l1_anchor,
        gas_used_tx0,
        block1,
    ) = build_l2_state_transition(l1_sender);
    let charlie = Address::from_low_u64_be(0xC4A);
    let l1_msg_value = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

    // The L1 receiver of the withdrawal (fresh address for easy balance verification)
    let l1_withdrawal_receiver = Address::from_low_u64_be(0xDEAD);

    // Build block 2 (withdrawal block)
    let (
        block2_txs_rlp,
        witness2_json,
        block2_post_state_root,
        block2_receipts_root,
        _block2_base_fee,
        account_proof,
        storage_proof,
    ) = build_l2_withdrawal_block(
        &block1,
        post_state_root,
        l1_withdrawal_receiver,
        gas_used_tx0,
        l1_anchor,
    );
    let alice_l2 = address_from_key(&SigningKey::from_bytes(&[1u8; 32].into()).unwrap());
    let withdrawal_amount = U256::from(10).pow(U256::from(18)); // 1 ETH

    // 4. Deploy NativeRollup
    let deploy_hex = std::fs::read_to_string(contracts_path.join("solc_out/NativeRollup.bin"))
        .expect("Failed to read compiled contract");
    let deploy_bytecode = hex::decode(deploy_hex.trim()).expect("invalid hex in .bin file");
    // Constructor args: (bytes32 _initialStateRoot, uint256 _blockGasLimit, uint256 _initialBaseFee)
    let mut constructor_args = Vec::with_capacity(96);
    constructor_args.extend_from_slice(pre_state_root.as_bytes()); // bytes32
    constructor_args.extend_from_slice(&U256::from(30_000_000u64).to_big_endian()); // uint256
    constructor_args.extend_from_slice(&U256::from(1_000_000_000u64).to_big_endian()); // uint256 (1 gwei)
    let init_code: Bytes = [deploy_bytecode, constructor_args].concat().into();

    let (deploy_tx_hash, contract_address) =
        create_deploy(&eth_client, &signer, init_code, Overrides::default())
            .await
            .expect("Failed to deploy NativeRollup");

    let deploy_receipt = wait_for_transaction_receipt(deploy_tx_hash, &eth_client, 30)
        .await
        .expect("Deploy receipt not found");
    assert!(
        deploy_receipt.receipt.status,
        "NativeRollup deployment failed"
    );

    println!("NativeRollup deployed at: {contract_address:?}");
    println!("  Deploy tx: {deploy_tx_hash:?}");

    // 5. Verify initial state: stateRoot = pre_state_root
    let stored_root = eth_client
        .get_storage_at(
            contract_address,
            U256::zero(),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        H256::from(stored_root.to_big_endian()),
        pre_state_root,
        "Initial stateRoot mismatch after deployment"
    );
    println!("  Initial stateRoot verified: {pre_state_root:?}");

    // 6. Call sendL1Message(charlie, 100_000, "") with 5 ETH
    let send_l1_msg_calldata = encode_calldata(
        "sendL1Message(address,uint256,bytes)",
        &[
            Value::Address(charlie),
            Value::Uint(U256::from(100_000u64)),
            Value::Bytes(Bytes::new()),
        ],
    )
    .expect("encode failed");

    let l1_msg_tx = build_generic_tx(
        &eth_client,
        TxType::EIP1559,
        contract_address,
        signer.address(),
        Bytes::from(send_l1_msg_calldata),
        Overrides {
            value: Some(l1_msg_value),
            ..Default::default()
        },
    )
    .await
    .expect("Failed to build sendL1Message tx");

    let l1_msg_tx_hash = send_generic_transaction(&eth_client, l1_msg_tx, &signer)
        .await
        .expect("Failed to send sendL1Message tx");

    let l1_msg_receipt = wait_for_transaction_receipt(l1_msg_tx_hash, &eth_client, 30)
        .await
        .expect("sendL1Message receipt not found");
    assert!(
        l1_msg_receipt.receipt.status,
        "NativeRollup.sendL1Message() reverted!"
    );
    println!("  sendL1Message() tx: {l1_msg_tx_hash:?}");

    // 7. Call advance(1, blockParams, transactionsRlp, witnessJson) — 1 L1 message consumed
    // BlockParams now has 5 fields; blockNumber, blockGasLimit, and parent gas params are tracked on-chain
    let advance_calldata = encode_calldata(
        "advance(uint256,(bytes32,bytes32,address,bytes32,uint256),bytes,bytes)",
        &[
            Value::Uint(U256::from(1)), // 1 L1 message
            Value::Tuple(vec![
                Value::FixedBytes(Bytes::from(input.post_state_root.as_bytes().to_vec())),
                Value::FixedBytes(Bytes::from(input.post_receipts_root.as_bytes().to_vec())),
                Value::Address(input.coinbase),
                Value::FixedBytes(Bytes::from(input.prev_randao.as_bytes().to_vec())),
                Value::Uint(U256::from(input.timestamp)),
            ]),
            Value::Bytes(Bytes::from(transactions_rlp)),
            Value::Bytes(Bytes::from(witness_json)),
        ],
    )
    .expect("encode failed");

    let advance_tx = build_generic_tx(
        &eth_client,
        TxType::EIP1559,
        contract_address,
        signer.address(),
        Bytes::from(advance_calldata),
        Overrides::default(),
    )
    .await
    .expect("Failed to build advance tx");

    let advance_tx_hash = send_generic_transaction(&eth_client, advance_tx, &signer)
        .await
        .expect("Failed to send advance tx");

    let advance_receipt = wait_for_transaction_receipt(advance_tx_hash, &eth_client, 30)
        .await
        .expect("Advance receipt not found");
    assert!(
        advance_receipt.receipt.status,
        "NativeRollup.advance() reverted!"
    );

    println!("  advance() tx: {advance_tx_hash:?}");
    println!(
        "  Gas used: {}",
        advance_receipt.receipt.cumulative_gas_used
    );

    // 8. Verify updated state
    let stored_root = eth_client
        .get_storage_at(
            contract_address,
            U256::zero(),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        H256::from(stored_root.to_big_endian()),
        post_state_root,
        "Post stateRoot mismatch"
    );

    let stored_block_num = eth_client
        .get_storage_at(
            contract_address,
            U256::from(1),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(stored_block_num, U256::from(1), "blockNumber mismatch");

    let stored_l1_msg_index = eth_client
        .get_storage_at(
            contract_address,
            U256::from(6),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        stored_l1_msg_index,
        U256::from(1),
        "l1MessageIndex mismatch"
    );

    // 9. Verify stateRootHistory[1] was stored (should equal post_state_root)
    // Storage slot for mapping(uint256 => bytes32) at slot 7, key 1:
    //   keccak256(abi.encode(uint256(1), uint256(7)))
    let mut slot_preimage = [0u8; 64];
    slot_preimage[31] = 1; // key = 1
    slot_preimage[63] = 7; // mapping base slot = 7
    let state_root_history_slot = U256::from_big_endian(&keccak_hash(slot_preimage));

    let stored_state_root_history = eth_client
        .get_storage_at(
            contract_address,
            state_root_history_slot,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        H256::from(stored_state_root_history.to_big_endian()),
        post_state_root,
        "stateRootHistory[1] should equal post_state_root"
    );

    println!("\n  Phase 1 passed: transfer + L1 message");
    println!("  Pre-state root:  {pre_state_root:?}");
    println!("  Post-state root: {post_state_root:?}");
    println!("  Block number:    1");
    println!("  L1 message index: 1");

    // ===== Phase 2: Withdrawal =====
    // Advance with block 2 (contains Alice → bridge.withdraw(l1_receiver) with 1 ETH)

    // 10. advance(0, blockParams2, block2_txs_rlp, witness2_json) — 0 L1 messages for block 2
    let block2_coinbase = Address::from_low_u64_be(0xC01);
    let block2_timestamp = block1.header.timestamp + 12;
    let advance2_calldata = encode_calldata(
        "advance(uint256,(bytes32,bytes32,address,bytes32,uint256),bytes,bytes)",
        &[
            Value::Uint(U256::from(0)), // no L1 messages consumed
            Value::Tuple(vec![
                Value::FixedBytes(Bytes::from(block2_post_state_root.as_bytes().to_vec())),
                Value::FixedBytes(Bytes::from(block2_receipts_root.as_bytes().to_vec())),
                Value::Address(block2_coinbase),
                Value::FixedBytes(Bytes::from(H256::zero().as_bytes().to_vec())),
                Value::Uint(U256::from(block2_timestamp)),
            ]),
            Value::Bytes(Bytes::from(block2_txs_rlp)),
            Value::Bytes(Bytes::from(witness2_json)),
        ],
    )
    .expect("encode advance2 failed");

    let advance2_tx = build_generic_tx(
        &eth_client,
        TxType::EIP1559,
        contract_address,
        signer.address(),
        Bytes::from(advance2_calldata),
        Overrides::default(),
    )
    .await
    .expect("Failed to build advance2 tx");

    let advance2_tx_hash = send_generic_transaction(&eth_client, advance2_tx, &signer)
        .await
        .expect("Failed to send advance2 tx");

    let advance2_receipt = wait_for_transaction_receipt(advance2_tx_hash, &eth_client, 30)
        .await
        .expect("Advance2 receipt not found");
    assert!(
        advance2_receipt.receipt.status,
        "NativeRollup.advance() for block 2 reverted!"
    );

    println!("\n  advance(block 2) tx: {advance2_tx_hash:?}");
    println!(
        "  Gas used: {}",
        advance2_receipt.receipt.cumulative_gas_used
    );

    // 11. Verify block 2 state
    let stored_root = eth_client
        .get_storage_at(
            contract_address,
            U256::zero(),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        H256::from(stored_root.to_big_endian()),
        block2_post_state_root,
        "Post stateRoot mismatch after block 2"
    );

    let stored_block_num = eth_client
        .get_storage_at(
            contract_address,
            U256::from(1),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        stored_block_num,
        U256::from(2),
        "blockNumber should be 2 after advance(block2)"
    );

    // 12. Verify stateRootHistory[2] was stored (should equal block2_post_state_root)
    let mut slot_preimage_b2 = [0u8; 64];
    slot_preimage_b2[31] = 2; // key = 2 (block number)
    slot_preimage_b2[63] = 7; // mapping base slot = 7
    let state_root_history_slot_b2 = U256::from_big_endian(&keccak_hash(slot_preimage_b2));

    let stored_state_root_history_b2 = eth_client
        .get_storage_at(
            contract_address,
            state_root_history_slot_b2,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        H256::from(stored_state_root_history_b2.to_big_endian()),
        block2_post_state_root,
        "stateRootHistory[2] should equal block2_post_state_root"
    );
    println!("  stateRootHistory[2]: {stored_state_root_history_b2:?}");

    // 13. Check l1_withdrawal_receiver balance before claim
    let receiver_balance_before = eth_client
        .get_balance(
            l1_withdrawal_receiver,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_balance failed");
    println!("  Receiver balance before claim: {receiver_balance_before}");

    // 14. Call claimWithdrawal(from, receiver, amount, messageId, blockNumber, accountProof, storageProof)
    // Uses MPT proofs against the L2 state root stored in stateRootHistory[2]
    let claim_calldata = encode_calldata(
        "claimWithdrawal(address,address,uint256,uint256,uint256,bytes[],bytes[])",
        &[
            Value::Address(alice_l2),               // _from (L2 sender)
            Value::Address(l1_withdrawal_receiver), // _receiver
            Value::Uint(withdrawal_amount),         // _amount (1 ETH)
            Value::Uint(U256::zero()),              // _messageId (first withdrawal)
            Value::Uint(U256::from(2)),             // _atBlockNumber (L2 block 2)
            Value::Array(
                account_proof
                    .iter()
                    .map(|node| Value::Bytes(Bytes::from(node.clone())))
                    .collect(),
            ), // _accountProof
            Value::Array(
                storage_proof
                    .iter()
                    .map(|node| Value::Bytes(Bytes::from(node.clone())))
                    .collect(),
            ), // _storageProof
        ],
    )
    .expect("encode claimWithdrawal failed");

    let claim_tx = build_generic_tx(
        &eth_client,
        TxType::EIP1559,
        contract_address,
        signer.address(),
        Bytes::from(claim_calldata),
        Overrides::default(),
    )
    .await
    .expect("Failed to build claim tx");

    let claim_tx_hash = send_generic_transaction(&eth_client, claim_tx, &signer)
        .await
        .expect("Failed to send claim tx");

    let claim_receipt = wait_for_transaction_receipt(claim_tx_hash, &eth_client, 30)
        .await
        .expect("Claim receipt not found");
    assert!(
        claim_receipt.receipt.status,
        "NativeRollup.claimWithdrawal() reverted!"
    );

    println!("  claimWithdrawal() tx: {claim_tx_hash:?}");

    // 15. Verify the receiver got the ETH
    let receiver_balance_after = eth_client
        .get_balance(
            l1_withdrawal_receiver,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_balance failed");
    assert_eq!(
        receiver_balance_after,
        receiver_balance_before + withdrawal_amount,
        "Receiver should have received {withdrawal_amount} wei"
    );
    println!("  Receiver balance after claim: {receiver_balance_after}");
    println!("  Withdrawal amount: {withdrawal_amount}");

    println!(
        "\nNativeRollup integration test passed (transfer + L1 message + withdrawal + claim)!"
    );
    println!("  Contract:        {contract_address:?}");
    println!("  L2 blocks:       2");
    println!("  L1 message:      5 ETH to charlie");
    println!("  Withdrawal:      1 ETH from alice to {l1_withdrawal_receiver:?}");

    // Clean up compiled contract artifacts
    clean_contracts_dir();
}

/// Removes the contracts/solc_out directory generated by the test.
fn clean_contracts_dir() {
    let solc_out = workspace_root().join("crates/vm/levm/contracts/solc_out");
    let _ = std::fs::remove_dir_all(&solc_out).inspect_err(|e| {
        println!("Failed to remove {}: {e}", solc_out.display());
    });
    println!("Cleaned up: {}", solc_out.display());
}
