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
        Account, AccountInfo, AccountState, BlockHeader, ChainConfig, Code, CodeMetadata,
        EIP1559Transaction, EIP4844Transaction, Receipt, Transaction, TxKind,
        block_execution_witness::{ExecutionWitness, GuestProgramState},
    },
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    db::guest_program_state_db::GuestProgramStateDb,
    environment::{EVMConfig, Environment},
    errors::TxResult,
    execute_precompile::{
        ExecutePrecompileInput, L1_ANCHOR, L2_BRIDGE, compute_merkle_proof, compute_merkle_root,
        execute_inner, execute_precompile,
    },
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

/// Build a standard ChainConfig with all forks enabled at genesis.
fn test_chain_config() -> ChainConfig {
    ChainConfig {
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
    }
}

/// Helper: encode L2Bridge.processL1Message(address,address,uint256,uint256,bytes,uint256,bytes32[]) calldata.
///
/// Function signature: processL1Message(address from, address to, uint256 value, uint256 gasLimit, bytes data, uint256 nonce, bytes32[] merkleProof)
/// The `bytes data` and `bytes32[] merkleProof` parameters are dynamic, requiring ABI encoding with offset pointers.
fn encode_process_l1_message_call(
    from: Address,
    to: Address,
    value: U256,
    gas_limit: U256,
    data: &[u8],
    nonce: u64,
    merkle_proof: &[H256],
) -> Vec<u8> {
    // selector = keccak256("processL1Message(address,address,uint256,uint256,bytes,uint256,bytes32[])")[0:4]
    let selector =
        &keccak_hash(b"processL1Message(address,address,uint256,uint256,bytes,uint256,bytes32[])")
            [..4];

    // ABI encoding: 7 params, where params 4 (bytes data) and 6 (bytes32[] merkleProof) are dynamic.
    // Head: 7 * 32 = 224 bytes
    // Param 0: from (address, left-padded)
    // Param 1: to (address, left-padded)
    // Param 2: value (uint256)
    // Param 3: gasLimit (uint256)
    // Param 4: offset to data (uint256)
    // Param 5: nonce (uint256)
    // Param 6: offset to merkleProof (uint256)
    // Tail: [data_length][data_bytes][padding] [proof_length][proof_elements...]
    let mut calldata = Vec::new();
    calldata.extend_from_slice(selector);

    // from (address)
    let mut from_bytes = [0u8; 32];
    from_bytes[12..].copy_from_slice(from.as_bytes());
    calldata.extend_from_slice(&from_bytes);

    // to (address)
    let mut to_bytes = [0u8; 32];
    to_bytes[12..].copy_from_slice(to.as_bytes());
    calldata.extend_from_slice(&to_bytes);

    // value (uint256)
    calldata.extend_from_slice(&value.to_big_endian());

    // gasLimit (uint256)
    calldata.extend_from_slice(&gas_limit.to_big_endian());

    // Compute offsets for dynamic params
    let head_size: usize = 7 * 32; // 224 bytes
    let data_padded = data.len() + ((32 - (data.len() % 32)) % 32);
    let data_offset = head_size; // offset to bytes data
    let proof_offset = data_offset + 32 + data_padded; // offset to bytes32[] proof

    // offset to data (dynamic param 4)
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(data_offset as u64).to_be_bytes());
    calldata.extend_from_slice(&offset_bytes);

    // nonce (uint256)
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes[24..].copy_from_slice(&nonce.to_be_bytes());
    calldata.extend_from_slice(&nonce_bytes);

    // offset to merkleProof (dynamic param 6)
    let mut proof_offset_bytes = [0u8; 32];
    proof_offset_bytes[24..].copy_from_slice(&(proof_offset as u64).to_be_bytes());
    calldata.extend_from_slice(&proof_offset_bytes);

    // Tail: data length + data + padding
    let mut data_len_bytes = [0u8; 32];
    data_len_bytes[24..].copy_from_slice(&(data.len() as u64).to_be_bytes());
    calldata.extend_from_slice(&data_len_bytes);
    calldata.extend_from_slice(data);
    let padding = (32 - (data.len() % 32)) % 32;
    calldata.resize(calldata.len() + padding, 0);

    // Tail: merkleProof length + elements
    let mut proof_len_bytes = [0u8; 32];
    proof_len_bytes[24..].copy_from_slice(&(merkle_proof.len() as u64).to_be_bytes());
    calldata.extend_from_slice(&proof_len_bytes);
    for hash in merkle_proof {
        calldata.extend_from_slice(hash.as_bytes());
    }

    calldata
}

/// Build ABI-encoded calldata for the EXECUTE precompile (14-slot ABI).
///
/// Format: abi.encode(
///     bytes32 preStateRoot,           // slot 0
///     bytes32 postStateRoot,          // slot 1
///     bytes32 postReceiptsRoot,       // slot 2
///     uint256 blockNumber,            // slot 3
///     uint256 blockGasLimit,          // slot 4
///     address coinbase,               // slot 5 (left-padded to 32 bytes)
///     bytes32 prevRandao,             // slot 6
///     uint256 timestamp,              // slot 7
///     uint256 parentBaseFee,          // slot 8
///     uint256 parentGasLimit,         // slot 9
///     uint256 parentGasUsed,          // slot 10
///     bytes32 l1Anchor,               // slot 11
///     bytes   transactions,           // slot 12 (dynamic offset pointer)
///     bytes   witnessJson             // slot 13 (dynamic offset pointer)
/// )
#[allow(clippy::too_many_arguments)]
fn build_precompile_calldata(
    pre_state_root: H256,
    post_state_root: H256,
    post_receipts_root: H256,
    block_number: u64,
    block_gas_limit: u64,
    coinbase: Address,
    prev_randao: H256,
    timestamp: u64,
    parent_base_fee: u64,
    parent_gas_limit: u64,
    parent_gas_used: u64,
    l1_anchor: H256,
    transactions_rlp: &[u8],
    witness_json: &[u8],
) -> Vec<u8> {
    // Helper: pad to 32-byte boundary
    fn pad32(len: usize) -> usize {
        len + ((32 - (len % 32)) % 32)
    }

    // Head is 14 * 32 = 448 bytes. Dynamic data starts after the head.
    let head_size: usize = 448;
    let txs_offset: usize = head_size;
    let txs_padded = pad32(transactions_rlp.len());
    let witness_offset: usize = txs_offset + 32 + txs_padded;

    let mut data = Vec::new();

    // slot 0: preStateRoot (bytes32)
    data.extend_from_slice(pre_state_root.as_bytes());

    // slot 1: postStateRoot (bytes32)
    data.extend_from_slice(post_state_root.as_bytes());

    // slot 2: postReceiptsRoot (bytes32)
    data.extend_from_slice(post_receipts_root.as_bytes());

    // slot 3: blockNumber (uint256)
    let mut num_bytes = [0u8; 32];
    num_bytes[24..].copy_from_slice(&block_number.to_be_bytes());
    data.extend_from_slice(&num_bytes);

    // slot 4: blockGasLimit (uint256)
    let mut gas_bytes = [0u8; 32];
    gas_bytes[24..].copy_from_slice(&block_gas_limit.to_be_bytes());
    data.extend_from_slice(&gas_bytes);

    // slot 5: coinbase (address, left-padded to 32 bytes)
    let mut coinbase_bytes = [0u8; 32];
    coinbase_bytes[12..].copy_from_slice(coinbase.as_bytes());
    data.extend_from_slice(&coinbase_bytes);

    // slot 6: prevRandao (bytes32)
    data.extend_from_slice(prev_randao.as_bytes());

    // slot 7: timestamp (uint256)
    let mut ts_bytes = [0u8; 32];
    ts_bytes[24..].copy_from_slice(&timestamp.to_be_bytes());
    data.extend_from_slice(&ts_bytes);

    // slot 8: parentBaseFee (uint256)
    let mut pbf_bytes = [0u8; 32];
    pbf_bytes[24..].copy_from_slice(&parent_base_fee.to_be_bytes());
    data.extend_from_slice(&pbf_bytes);

    // slot 9: parentGasLimit (uint256)
    let mut pgl_bytes = [0u8; 32];
    pgl_bytes[24..].copy_from_slice(&parent_gas_limit.to_be_bytes());
    data.extend_from_slice(&pgl_bytes);

    // slot 10: parentGasUsed (uint256)
    let mut pgu_bytes = [0u8; 32];
    pgu_bytes[24..].copy_from_slice(&parent_gas_used.to_be_bytes());
    data.extend_from_slice(&pgu_bytes);

    // slot 11: l1Anchor (bytes32)
    data.extend_from_slice(l1_anchor.as_bytes());

    // slot 12: offset to transactions
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(txs_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // slot 13: offset to witnessJson
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(witness_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // tail: transactions (length + data + padding)
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(transactions_rlp.len() as u64).to_be_bytes());
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(transactions_rlp);
    data.resize(data.len() + (txs_padded - transactions_rlp.len()), 0);

    // tail: witnessJson (length + data + padding)
    let witness_padded = pad32(witness_json.len());
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(witness_json.len() as u64).to_be_bytes());
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(witness_json);
    data.resize(data.len() + (witness_padded - witness_json.len()), 0);

    data
}

/// Helper: build a minimal ExecutePrecompileInput for rejection tests.
///
/// Creates a genesis state with a single account, builds an ExecutionWitness,
/// and wraps the given transactions in an ExecutePrecompileInput with individual fields.
fn build_rejection_test_input(
    transactions: Vec<Transaction>,
    block_number: u64,
    timestamp: u64,
) -> ExecutePrecompileInput {
    let account = Address::from_low_u64_be(0xA);

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
        number: block_number.saturating_sub(1),
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        gas_used: 15_000_000,
        base_fee_per_gas: Some(1_000_000_000),
        timestamp: timestamp.saturating_sub(12),
        ..Default::default()
    };

    // Build a temporary header for the witness block_headers_bytes
    let temp_header = BlockHeader {
        parent_hash: parent_header.compute_block_hash(),
        number: block_number,
        gas_limit: 30_000_000,
        timestamp,
        ..Default::default()
    };

    let chain_config = test_chain_config();

    let witness = ExecutionWitness {
        codes: vec![],
        block_headers_bytes: vec![parent_header.encode_to_vec(), temp_header.encode_to_vec()],
        first_block_number: block_number,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots: BTreeMap::new(),
        keys: vec![],
    };

    ExecutePrecompileInput {
        pre_state_root,
        post_state_root: H256::zero(), // doesn't matter for rejection tests
        post_receipts_root: H256::zero(),
        block_number,
        block_gas_limit: 30_000_000,
        coinbase: Address::zero(),
        prev_randao: H256::zero(),
        timestamp,
        parent_base_fee: 1_000_000_000,
        parent_gas_limit: 30_000_000,
        parent_gas_used: 15_000_000,
        l1_anchor: H256::zero(),
        transactions,
        execution_witness: witness,
    }
}

/// Convenience wrapper using the default L1 sender address (0x1A1).
#[allow(clippy::type_complexity)]
fn build_l2_state_transition() -> (ExecutePrecompileInput, Vec<u8>, Vec<u8>, H256, H256, H256) {
    build_l2_state_transition_with_sender(Address::from_low_u64_be(0x1A1))
}

/// Build the L2 state transition: processL1Message (relayer->L2Bridge) + transfer (Alice->Bob).
///
/// The L2 genesis includes the L2Bridge predeploy at `L2_BRIDGE` with preminted ETH and
/// a relayer account with gas budget. The block contains two transactions:
///   1. Relayer calls L2Bridge.processL1Message(l1_sender, charlie, 5 ETH, 100000, "", 0)
///   2. Alice sends 1 ETH to Bob
///
/// Returns:
///   - ExecutePrecompileInput (for direct execute_inner calls)
///   - transactions RLP bytes (for binary calldata / contract call)
///   - witness JSON bytes (for binary calldata / contract call)
///   - pre_state_root
///   - post_state_root
///   - l1_anchor
#[allow(clippy::type_complexity)]
fn build_l2_state_transition_with_sender(
    l1_sender: Address,
) -> (ExecutePrecompileInput, Vec<u8>, Vec<u8>, H256, H256, H256) {
    // ===== Keys and Addresses =====
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
    let bridge_runtime = hex::decode(L2_BRIDGE_RUNTIME_HEX).expect("valid bridge hex");
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
    let l1_message_value = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH
    let l1_message_gas_limit = U256::from(100_000u64);

    // ===== Compute L1 message hash and Merkle root (l1_anchor) =====
    // This must be done before genesis setup because L1Anchor needs l1_anchor in storage.
    // message_hash = keccak256(abi.encodePacked(from[20], to[20], value[32], gasLimit[32], keccak256(data)[32], nonce[32]))
    let data_hash = H256::from(keccak_hash(b"")); // empty data
    let mut message_preimage = Vec::with_capacity(168);
    message_preimage.extend_from_slice(l1_sender.as_bytes()); // from: 20 bytes
    message_preimage.extend_from_slice(charlie.as_bytes()); // to: 20 bytes
    message_preimage.extend_from_slice(&l1_message_value.to_big_endian()); // value: 32 bytes
    message_preimage.extend_from_slice(&l1_message_gas_limit.to_big_endian()); // gasLimit: 32 bytes
    message_preimage.extend_from_slice(data_hash.as_bytes()); // keccak256(data): 32 bytes
    message_preimage.extend_from_slice(&U256::zero().to_big_endian()); // nonce=0: 32 bytes
    let message_hash = H256::from(keccak_hash(&message_preimage));
    let l1_anchor = compute_merkle_root(&[message_hash]);
    let merkle_proof = compute_merkle_proof(&[message_hash], 0);

    // ===== L1Anchor setup =====
    let anchor_runtime = hex::decode(L1_ANCHOR_RUNTIME_HEX).expect("valid anchor hex");
    let anchor_code_hash = H256(keccak_hash(&anchor_runtime));

    // L1Anchor storage: slot 0 = l1MessagesRoot (the Merkle root anchored by EXECUTE precompile)
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
    insert_account(&mut state_trie, bob, &AccountState::default());
    insert_account(&mut state_trie, charlie, &AccountState::default());
    insert_account(&mut state_trie, coinbase, &AccountState::default());
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

    // ===== Parent header (genesis) =====
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
    // TX0: relayer -> L2Bridge.processL1Message(l1_sender, charlie, 5 ETH, 100000, "", 0, proof)
    let l1_message_calldata = encode_process_l1_message_call(
        l1_sender,
        charlie,
        l1_message_value,
        l1_message_gas_limit,
        b"", // empty data for simple ETH transfer
        0,
        &merkle_proof,
    );
    let mut tx0 = EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 2_000_000_000,
        gas_limit: 200_000,
        to: TxKind::Call(L2_BRIDGE),
        value: U256::zero(),
        data: Bytes::from(l1_message_calldata),
        access_list: vec![],
        ..Default::default()
    };
    sign_eip1559_tx(&mut tx0, &relayer_key);
    let tx0 = Transaction::EIP1559Transaction(tx0);

    // TX1: alice -> bob 1 ETH
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

    let db = Arc::new(GuestProgramStateDb::new(guest_state));
    let db_dyn: Arc<dyn ethrex_levm::db::Database> = db.clone();
    let mut gen_db = GeneralizedDatabase::new(db_dyn);

    let config = EVMConfig::new_from_chain_config(&chain_config, &temp_header);
    let effective_gas_price =
        U256::from(std::cmp::min(1_000_000_000u64 + base_fee, 2_000_000_000u64));

    // Execute TX0: processL1Message
    let env0 = Environment {
        origin: relayer,
        gas_limit: 200_000,
        config,
        block_number: U256::from(1),
        coinbase,
        timestamp: U256::from(1_000_012u64),
        prev_randao: Some(temp_header.prev_randao),
        slot_number: U256::zero(),
        chain_id: U256::from(chain_id),
        base_fee_per_gas: U256::from(base_fee),
        base_blob_fee_per_gas: U256::zero(),
        gas_price: effective_gas_price,
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: Some(U256::from(1_000_000_000u64)),
        tx_max_fee_per_gas: Some(U256::from(2_000_000_000u64)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: 30_000_000,
        difficulty: U256::zero(),
        is_privileged: false,
        fee_token: None,
    };

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
    let env1 = Environment {
        origin: alice,
        gas_limit: 21_000,
        config,
        block_number: U256::from(1),
        coinbase,
        timestamp: U256::from(1_000_012u64),
        prev_randao: Some(temp_header.prev_randao),
        slot_number: U256::zero(),
        chain_id: U256::from(chain_id),
        base_fee_per_gas: U256::from(base_fee),
        base_blob_fee_per_gas: U256::zero(),
        gas_price: effective_gas_price,
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: Some(U256::from(1_000_000_000u64)),
        tx_max_fee_per_gas: Some(U256::from(2_000_000_000u64)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: 30_000_000,
        difficulty: U256::zero(),
        is_privileged: false,
        fee_token: None,
    };

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
    db.state
        .lock()
        .expect("lock")
        .apply_account_updates(&account_updates)
        .expect("apply updates");
    let post_state_root = db
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

    // ===== Build final witness (with correct block header) =====
    let witness = ExecutionWitness {
        codes: vec![bridge_runtime, anchor_runtime],
        block_headers_bytes: vec![parent_header.encode_to_vec(), final_header.encode_to_vec()],
        first_block_number: 1,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots,
        keys: vec![],
    };

    // l1_anchor was computed earlier (before genesis setup) as compute_merkle_root(&[message_hash])

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
        transactions,
        execution_witness: witness,
    };

    (
        input,
        transactions_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
        l1_anchor,
    )
}

// ===== Unit Tests =====

/// The main test: execute a processL1Message + transfer via the EXECUTE precompile.
///
/// Flow:
///   1. Build L2 genesis with L2Bridge (preminted) + relayer
///   2. L2 block has 2 txs: processL1Message(l1_sender, charlie, 5 ETH, 100000, "", 0) + Alice->Bob 1 ETH
///   3. Call EXECUTE precompile with individual fields, transactions, witness, and l1Anchor (Merkle root)
///   4. Verify returned state root, block number, and gas used
#[test]
fn test_execute_precompile_transfer_and_l1_message() {
    let (input, transactions_rlp, witness_json, pre_state_root, post_state_root, l1_anchor) =
        build_l2_state_transition();

    // Build ABI-encoded calldata and execute via execute_precompile()
    let calldata = build_precompile_calldata(
        pre_state_root,
        input.post_state_root,
        input.post_receipts_root,
        input.block_number,
        input.block_gas_limit,
        input.coinbase,
        input.prev_randao,
        input.timestamp,
        input.parent_base_fee,
        input.parent_gas_limit,
        input.parent_gas_used,
        l1_anchor,
        &transactions_rlp,
        &witness_json,
    );
    println!("ABI-encoded EXECUTE calldata: {} bytes", calldata.len());

    let mut gas_remaining: u64 = 1_000_000;
    let result = execute_precompile(
        &Bytes::from(calldata),
        &mut gas_remaining,
        ethrex_common::types::Fork::Prague,
    );
    match &result {
        Ok(output) => {
            assert_eq!(output.len(), 160, "Expected 160-byte ABI-encoded return");
            let returned_root = H256::from_slice(&output[..32]);
            let returned_block_num = U256::from_big_endian(&output[32..64]);
            let returned_gas_used = U256::from_big_endian(&output[64..96]);
            let returned_burned_fees = U256::from_big_endian(&output[96..128]);
            let returned_base_fee = U256::from_big_endian(&output[128..160]);
            assert_eq!(
                returned_root, post_state_root,
                "Returned state root mismatch"
            );
            assert_eq!(
                returned_block_num,
                U256::from(1),
                "Returned block number mismatch"
            );
            assert!(
                returned_gas_used > U256::zero(),
                "Gas used should be positive"
            );
            // burnedFees = base_fee_per_gas * total_gas_used
            let base_fee = U256::from(1_000_000_000u64); // 1 gwei
            let expected_burned_fees = base_fee * returned_gas_used;
            assert_eq!(
                returned_burned_fees, expected_burned_fees,
                "Burned fees mismatch: expected base_fee * gas_used = {expected_burned_fees}, got {returned_burned_fees}"
            );
            assert_eq!(
                returned_base_fee, base_fee,
                "Returned baseFeePerGas mismatch"
            );
            println!("EXECUTE precompile succeeded!");
            println!("  Pre-state root:  {pre_state_root:?}");
            println!("  Post-state root: {post_state_root:?}");
            println!("  Relayer processed L1 message: 5 ETH to charlie");
            println!("  Alice sent 1 ETH to Bob");
            println!("  Gas used: {returned_gas_used}");
            println!("  Burned fees: {returned_burned_fees}");
            println!("  Base fee per gas: {returned_base_fee}");
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

    let result = execute_inner(build_rejection_test_input(vec![blob_tx], 1, 1_000_012));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Blob transactions"),
        "Expected blob transaction rejection"
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

/// NativeRollup.sol runtime bytecode (compiled with solc 0.8.31 --via-ir --optimize).
/// Uses state root proofs (MPT) for L2→L1 withdrawals and Merkle root for L1→L2 messages.
///
/// Source: crates/vm/levm/contracts/NativeRollup.sol
/// Compile: cd crates/vm/levm/contracts && solc --bin-runtime --via-ir --optimize NativeRollup.sol -o /tmp/solc_native_out --overwrite
const NATIVE_ROLLUP_RUNTIME_HEX: &str = "60806040526004361015610160575b3615610018575f80fd5b341561012b5760405161002c602082610c76565b5f90526005546040513360601b6bffffffffffffffffffffffff1916602082018181526034830191909152346048830152620186a060688301527fc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a4706088830181905260a88084018590528352916100a460c882610c76565b519020600160401b83101561011757600183016005556100c383610bea565b819291549060031b91821b915f19901b191617905560405190348252620186a06020830152604082015233907fb42098fed77baca4d18a86b35f7bbfc750de2275ad10e0837fd550faf3c9d56f60603392a4005b634e487b7160e01b5f52604160045260245ffd5b60405162461bcd60e51b815260206004820152600d60248201526c09aeae6e840e6cadcc8408aa89609b1b6044820152606490fd5b5f3560e01c8063069c7eee146108cf57806307132c05146108a057806317413d09146108765780631ed873c71461085957806336d9e47c1461083c5780633e67267f1461080457806357e871e7146107e75780635eaadcc7146106d25780637877a797146106b55780639588eca21461069957806395c570bd1461067c5763ace421140361000e57346106785760e036600319011261067857610201610c02565b6024356001600160a01b0381168082036106785760443590606435906084359060a43567ffffffffffffffff811161067857610241903690600401610c18565b909660c43567ffffffffffffffff811161067857610263903690600401610c18565b91909260095460ff81166106335760ff19166001176009555f868152600760205260409020549182156105fe5760405160208101916001600160601b03199060601b1682526001600160601b03198b60601b166034820152896048820152886068820152606881526102d6608882610c76565b519020998a5f52600860205260ff60405f2054166105c75761033a92610334604051602081019061fffd60601b825260148152610314603482610c76565b5190206040519060208201526020815261032f604082610c76565b610f37565b9061102e565b915f928051156105b357602081015160f890811c908110610559578060f6198101116105455760f51901806001116105455761038d61038761038161039393602098610c49565b846116f0565b836116f0565b82611848565b94909403610500576103e59360209101015161033460405160208101908c825260036040820152604081526103c9606082610c76565b5190206040516020810191825260208152610314604082610c76565b935f965f975b8651891015610412576001906104018a89610f26565b5160f81c9060081b179801976103eb565b60018891036104bb575f808481949382948352600860205260408320600160ff198254161790555af1610443610cb4565b50156104805760207f1113af8a2f367ad0f39a44a9985b12833c5e9dcb54532dd60575fc4ccbd5f98191604051908152a46009805460ff19169055005b60405162461bcd60e51b8152602060048201526013602482015272115512081d1c985b9cd9995c8819985a5b1959606a1b6044820152606490fd5b60405162461bcd60e51b815260206004820152601a60248201527f5769746864726177616c206e6f7420696e204c322073746174650000000000006044820152606490fd5b60405162461bcd60e51b815260206004820152601d60248201527f4d50543a2073746f72616765526f6f74206e6f742033322062797465730000006044820152606490fd5b634e487b7160e01b5f52601160045260245ffd5b90935060c0116105765760209261039361038d6103876001610381565b60405162461bcd60e51b81526020600482015260156024820152741354150e881858d8dbdd5b9d081b9bdd081b1a5cdd605a1b6044820152606490fd5b634e487b7160e01b5f52603260045260245ffd5b60405162461bcd60e51b815260206004820152600f60248201526e105b1c9958591e4818db185a5b5959608a1b6044820152606490fd5b60405162461bcd60e51b815260206004820152600d60248201526c556e6b6e6f776e20626c6f636b60981b6044820152606490fd5b60405162461bcd60e51b815260206004820152601f60248201527f5265656e7472616e637947756172643a207265656e7472616e742063616c6c006044820152606490fd5b5f80fd5b34610678575f366003190112610678576020600454604051908152f35b34610678575f3660031901126106785760205f54604051908152f35b34610678575f366003190112610678576020600254604051908152f35b6060366003190112610678576106e6610c02565b60243560443567ffffffffffffffff81116106785761070c610713913690600401610bbc565b3691610ce3565b91600554926020815191012060405160208101906001600160601b03193360601b1682526001600160601b03198460601b1660348201523460488201528460688201528260888201528560a882015260a8815261077160c882610c76565b519020600160401b851015610117576001850160055561079085610bea565b819291549060031b91821b915f19901b1916179055604051923484526020840152604083015260018060a01b0316907fb42098fed77baca4d18a86b35f7bbfc750de2275ad10e0837fd550faf3c9d56f60603392a4005b34610678575f366003190112610678576020600154604051908152f35b34610678576020366003190112610678576004356005548110156106785761082d602091610bea565b90549060031b1c604051908152f35b34610678575f366003190112610678576020600354604051908152f35b34610678575f366003190112610678576020600654604051908152f35b34610678576020366003190112610678576004355f526007602052602060405f2054604051908152f35b34610678576020366003190112610678576004355f526008602052602060ff60405f2054166040519015158152f35b34610678576101003660031901126106785760043560a03660231901126106785760c43567ffffffffffffffff811161067857610910903690600401610bbc565b919060e43567ffffffffffffffff811161067857610932903690600401610bbc565b600654936109408186610c49565b60055410610b7e578061095661095c9287610d84565b95610c49565b60065560015492600184018094116105455760025495600354956004545f54986064359860018060a01b038a16809a0361067857604051998a9960208b019c8d5260243560408c015260443560608c015260808b01528260a08b015260c08a015260843560e08a015260a4356101008a01526101208901526101408801526101608701526101808601526101a085016101c090526101e08501906109ff92610c56565b90601f19848303016101c0850152610a1692610c56565b03601f1981018252610a289082610c76565b51905a915f9291836101018194f1610a3e610cb4565b9080610b73575b15610b1f5760a081805181010312610678576020810151604082015191606081015160a0608083015192015190835f5584600155600455600355825f5260076020528160405f205580610ac5575b7f3f4f02c37d640d53005d93fae1fede1a8eedaf8eab546e433b17f30035dc69d39160409182519182526020820152a2005b5f80808084335af1610ad5610cb4565b50610a935760405162461bcd60e51b815260206004820152601b60248201527f4275726e65642066656573207472616e73666572206661696c656400000000006044820152606490fd5b60405162461bcd60e51b815260206004820152602660248201527f4558454355544520707265636f6d70696c6520766572696669636174696f6e2060448201526519985a5b195960d21b6064820152608490fd5b5060a0815114610a45565b60405162461bcd60e51b81526020600482015260166024820152754e6f7420656e6f756768204c31206d6573736167657360501b6044820152606490fd5b9181601f840112156106785782359167ffffffffffffffff8311610678576020838186019501011161067857565b6005548110156105b35760055f5260205f2001905f90565b600435906001600160a01b038216820361067857565b9181601f840112156106785782359167ffffffffffffffff8311610678576020808501948460051b01011161067857565b9190820180921161054557565b908060209392818452848401375f828201840152601f01601f1916010190565b90601f8019910116810190811067ffffffffffffffff82111761011757604052565b67ffffffffffffffff811161011757601f01601f191660200190565b3d15610cde573d90610cc582610c98565b91610cd36040519384610c76565b82523d5f602084013e565b606090565b929192610cef82610c98565b91610cfd6040519384610c76565b829481845281830111610678578281602093845f960137010152565b67ffffffffffffffff81116101175760051b60200190565b90610d3b82610d19565b610d486040519182610c76565b8281528092610d59601f1991610d19565b0190602036910137565b80518210156105b35760209160051b010190565b9190820391821161054557565b91908015610ed25760018114610ebd57610d9d81610d31565b925f5b828110610e90575050505b81516001811115610e8057600181018091116105455760011c90610dce82610d31565b905f5b845160011c811015610e3257600181901b906001600160ff1b038116810361054557610dfd8287610d63565b51916001810180911161054557600192610e1a610e219289610d63565b5190610ed8565b610e2b8286610d63565b5201610dd1565b509291909160018082511614610e4b575b505090610dab565b80515f19810190811161054557610e6191610d63565b51905f19810190811161054557610e789083610d63565b525f80610e43565b50908051156105b3576020015190565b80610ea5610ea060019385610c49565b610bea565b90549060031b1c610eb68288610d63565b5201610da0565b5090610ec890610bea565b90549060031b1c90565b505f9150565b9080821015610f06576040519060208201928352604082015260408152610f00606082610c76565b51902090565b906040519060208201928352604082015260408152610f00606082610c76565b9081518110156105b3570160200190565b9081518060011b908082046002149015171561054557610f5681610c98565b90610f646040519283610c76565b808252610f73601f1991610c98565b01366020830137805f5b8451811015610ff8576001600160f81b0319600f60f81b610f9e8388610f26565b5160041c1616908060011b9181830460021482151715610545575f1a610fc48386610f26565b53600f60f81b610fd48288610f26565b5116916001810180911161054557610ff16001935f1a9186610f26565b5301610f7d565b50925050565b5f1981146105455760010190565b602081519101519060208110611020575090565b5f199060200360031b1b1690565b9293915f90815b86831015611449578260051b820135601e198336030181121561067857820180359067ffffffffffffffff8211610678576020019681360388136106785761107e36838a610ce3565b602081519101200361140b5761109f61109782896114b8565b919082610c49565b915f818a845b8681106113e3575050506011810361115e57508651841461114857908392916110de6110d46110e4968a610f26565b5160f81c94610ffe565b99611689565b506020815103611103576110f960019161100c565b925b019194611035565b60405162461bcd60e51b815260206004820152601a60248201527f4d50543a206272616e6368206368696c64206e6f7420686173680000000000006044820152606490fd5b94509450949650505061115a9361163e565b5090565b9398936002036113aa5761117483838387611551565b50938451156105b357602085015160fc1c946002861495861561139f575b60018114908115611394575b501561138b5760ff60015b1681518060011b908082046002149015171561054557816111cc91939293610d77565b5f925b8184106112985750505050906111e69392916115f3565b50906112445760208151036112065761120060019161100c565b926110fb565b60405162461bcd60e51b815260206004820152601660248201527509aa0a87440caf0e840dccaf0e840dcdee840d0c2e6d60531b6044820152606490fd5b94939550505051036112535790565b60405162461bcd60e51b815260206004820152601960248201527f4d50543a206c656166207061746820696e636f6d706c657465000000000000006044820152606490fd5b9091929c8b51811015611350578b60ff8f838160016112c66112bd6112df958a610c49565b821c9289610c49565b16611339576112d59089610f26565b5160fc1c93610f26565b5160f81c9116036112ff576112f5600191610ffe565b9d019291906111cf565b60405162461bcd60e51b815260206004820152601260248201527109aa0a87440e0c2e8d040dad2e6dac2e8c6d60731b6044820152606490fd5b611345600f918a610f26565b5160f81c1693610f26565b60405162461bcd60e51b81526020600482015260136024820152721354150e881c185d1a081d1bdbc81cda1bdc9d606a1b6044820152606490fd5b60ff60026111a9565b60039150145f61119e565b600381149650611192565b60405162461bcd60e51b81526020600482015260116024820152704d50543a20696e76616c6964206e6f646560781b6044820152606490fd5b6113f481611402946113fc946118fe565b919050610c49565b91610ffe565b908a83916110a5565b60405162461bcd60e51b815260206004820152601660248201527509aa0a87440d2dcecc2d8d2c840dcdec8ca40d0c2e6d60531b6044820152606490fd5b60405162461bcd60e51b81526020600482015260156024820152744d50543a2070726f6f6620696e636f6d706c65746560581b6044820152606490fd5b908210156105b3570190565b60ff60bf199116019060ff821161054557565b60ff60f6199116019060ff821161054557565b919080156105b357823560f81c60c081101580611546575b611530576114e260ff919492946114a5565b16905f935f915b83831061150157505050600101806001116105455790565b9091948560010190816001116105455761151e6001928585611486565b3560f81c9060081b17950191906114e9565b60ff92935061153f9150611492565b1690600190565b5060f78111156114d0565b9091939293915f925b8581106115815750505050604051611573602082610c76565b5f8082523660208301379190565b61158c8183856118fe565b919085156115ae57506115a8916115a291610c49565b93610ffe565b9261155a565b945095506115cc6115d2916115c68887969596610d77565b90610d77565b84610c49565b9182841161067857821161067857826115ef933693039101610ce3565b9190565b9091939293915f925b8581106116155750505050604051611573602082610c76565b6116208183856118fe565b9190600186146115ae5750611638916115a291610c49565b926115fc565b9091939293915f925b8581106116605750505050604051611573602082610c76565b61166b8183856118fe565b9190601086146115ae5750611683916115a291610c49565b92611647565b9294935f9391925b8681106116ab575050505050604051611573602082610c76565b6116b68185856118fe565b91908387146116d957506116d3916116cd91610c49565b94610ffe565b93611691565b955096506115d291506115cc906115c68887610d77565b6116fa8282610f26565b5160f81c91608083106118365760b78311156118135760bf8311156117b35760f78311156117905760f6198301928311610545575f915f905b84821061175c57505060018101809111610545576117599261175491610c49565b610c49565b90565b909260018301908184116105455761177f61177986600194610c49565b84610f26565b5160f81c9060081b17930190611733565b919050600182018092116105455760bf1981019081116105455761175991610c49565b60b6198301928311610545575f915f905b8482106117e557505060018101809111610545576117599261175491610c49565b909260018301908184116105455761180261177986600194610c49565b5160f81c9060081b179301906117c4565b9190506001820180921161054557607f1981019081116105455761175991610c49565b80925060019150018091116105455790565b906118538183610f26565b5160f81c608081106118e55760b78111156118c75760b6198101908111610545575f925f905b828210611899575050600182018092116105455761189691610c49565b91565b90936001840190818511610545576118b661177987600194610c49565b5160f81c9060081b17940190611879565b9291506001810180911161054557607f198301928311610545579190565b509160019150565b60ff166001019060ff821161054557565b9092919261190d848284611486565b3560f81c91608083101561192357505050600190565b60b783116119535750506001830180931161054557607f190160ff81116105455761194f60ff916118ed565b1690565b9093909160bf81116119e15760b6190160ff81116105455760ff16935f928391905b8683106119aa5750505060018101809111610545578361199491610c49565b9260010190816001116105455761175991610c49565b9091936001840190818511610545576119cf6119c887600194610c49565b8585611486565b3560f81c9060081b1794019190611975565b9093909160f78311611a0d575050600183018093116105455761194f611a0860ff92611492565b6118ed565b939091611a1b60ff916114a5565b16935f925f915b868310611a415750505060018101809111610545578361199491610c49565b909193600184019081851161054557611a5f6119c887600194610c49565b3560f81c9060081b1794019190611a2256fea264697066735822122027f0e1676c4092a13e858a8daa28f4763c3ed5b08dbfd38437df27264889033c64736f6c634300081f0033";

/// L2Bridge.sol runtime bytecode (compiled with solc 0.8.31).
/// Includes Merkle proof verification against L1Anchor predeploy,
/// and sentMessages mapping for proof-based L2→L1 withdrawals.
///
/// Source: crates/vm/levm/contracts/L2Bridge.sol
/// Compile: cd crates/vm/levm/contracts && solc --bin-runtime L2Bridge.sol -o /tmp/solc_native_out --overwrite
const L2_BRIDGE_RUNTIME_HEX: &str = "608060405260043610610054575f3560e01c806331baf74a1461005857806351cff8d9146100805780637b259db41461009c57806382e3702d146100c65780638406c07914610102578063b060182d1461012c575b5f5ffd5b348015610063575f5ffd5b5061007e6004803603810190610079919061082b565b610156565b005b61009a60048036038101906100959190610907565b610445565b005b3480156100a7575f5ffd5b506100b06105d2565b6040516100bd9190610941565b60405180910390f35b3480156100d1575f5ffd5b506100ec60048036038101906100e7919061098d565b6105d8565b6040516100f991906109d2565b60405180910390f35b34801561010d575f5ffd5b506101166105f5565b60405161012391906109fa565b60405180910390f35b348015610137575f5ffd5b50610140610619565b60405161014d9190610941565b60405180910390f35b5f5f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff16146101e4576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101db90610a6d565b60405180910390fd5b6001548314610228576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161021f90610ad5565b60405180910390fd5b5f600154905060018161023b9190610b20565b6001819055505f8a8a8a8a8a8a604051610256929190610b8f565b60405180910390208660405160200161027496959493929190610c2c565b6040516020818303038152906040528051906020012090505f61fffe73ffffffffffffffffffffffffffffffffffffffff1663193e6dfe6040518163ffffffff1660e01b8152600401602060405180830381865afa1580156102d8573d5f5f3e3d5ffd5b505050506040513d601f19601f820116820180604052508101906102fc9190610caf565b905061030a8585838561061f565b610349576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161034090610d24565b60405180910390fd5b8a73ffffffffffffffffffffffffffffffffffffffff168a8a908a8a604051610373929190610b8f565b5f60405180830381858888f193505050503d805f81146103ae576040519150601f19603f3d011682016040523d82523d5f602084013e6103b3565b606091505b505050828b73ffffffffffffffffffffffffffffffffffffffff168d73ffffffffffffffffffffffffffffffffffffffff167f481690f24f8062803168a6eea64e8fda33ee03ae29d4fbc1e5b1e76629e13e218d8d8d8d604051610418929190610b8f565b604051809103902060405161042f93929190610d51565b60405180910390a4505050505050505050505050565b5f3411610487576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161047e90610df6565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168173ffffffffffffffffffffffffffffffffffffffff16036104f5576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104ec90610e5e565b60405180910390fd5b5f60025490506001816105089190610b20565b6002819055505f338334846040516020016105269493929190610e7c565b604051602081830303815290604052805190602001209050600160035f8381526020019081526020015f205f6101000a81548160ff021916908315150217905550818373ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167f2e16c360bf25f9193c8e78b0fcdf02bacfd34fd98ec9fe4aa2549e15346dafd2346040516105c59190610941565b60405180910390a4505050565b60025481565b6003602052805f5260405f205f915054906101000a900460ff1681565b5f5f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1681565b60015481565b5f5f8290505f5f90505b86869050811015610666576106578288888481811061064b5761064a610ec9565b5b90506020020135610675565b91508080600101915050610629565b50838114915050949350505050565b5f818310156106ae578282604051602001610691929190610ef6565b6040516020818303038152906040528051906020012090506106da565b81836040516020016106c1929190610ef6565b6040516020818303038152906040528051906020012090505b92915050565b5f5ffd5b5f5ffd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610711826106e8565b9050919050565b61072181610707565b811461072b575f5ffd5b50565b5f8135905061073c81610718565b92915050565b5f819050919050565b61075481610742565b811461075e575f5ffd5b50565b5f8135905061076f8161074b565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261079657610795610775565b5b8235905067ffffffffffffffff8111156107b3576107b2610779565b5b6020830191508360018202830111156107cf576107ce61077d565b5b9250929050565b5f5f83601f8401126107eb576107ea610775565b5b8235905067ffffffffffffffff81111561080857610807610779565b5b6020830191508360208202830111156108245761082361077d565b5b9250929050565b5f5f5f5f5f5f5f5f5f60e08a8c031215610848576108476106e0565b5b5f6108558c828d0161072e565b99505060206108668c828d0161072e565b98505060406108778c828d01610761565b97505060606108888c828d01610761565b96505060808a013567ffffffffffffffff8111156108a9576108a86106e4565b5b6108b58c828d01610781565b955095505060a06108c88c828d01610761565b93505060c08a013567ffffffffffffffff8111156108e9576108e86106e4565b5b6108f58c828d016107d6565b92509250509295985092959850929598565b5f6020828403121561091c5761091b6106e0565b5b5f6109298482850161072e565b91505092915050565b61093b81610742565b82525050565b5f6020820190506109545f830184610932565b92915050565b5f819050919050565b61096c8161095a565b8114610976575f5ffd5b50565b5f8135905061098781610963565b92915050565b5f602082840312156109a2576109a16106e0565b5b5f6109af84828501610979565b91505092915050565b5f8115159050919050565b6109cc816109b8565b82525050565b5f6020820190506109e55f8301846109c3565b92915050565b6109f481610707565b82525050565b5f602082019050610a0d5f8301846109eb565b92915050565b5f82825260208201905092915050565b7f4c324272696467653a206e6f742072656c6179657200000000000000000000005f82015250565b5f610a57601583610a13565b9150610a6282610a23565b602082019050919050565b5f6020820190508181035f830152610a8481610a4b565b9050919050565b7f4c324272696467653a206e6f6e6365206d69736d6174636800000000000000005f82015250565b5f610abf601883610a13565b9150610aca82610a8b565b602082019050919050565b5f6020820190508181035f830152610aec81610ab3565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f610b2a82610742565b9150610b3583610742565b9250828201905080821115610b4d57610b4c610af3565b5b92915050565b5f81905092915050565b828183375f83830152505050565b5f610b768385610b53565b9350610b83838584610b5d565b82840190509392505050565b5f610b9b828486610b6b565b91508190509392505050565b5f8160601b9050919050565b5f610bbd82610ba7565b9050919050565b5f610bce82610bb3565b9050919050565b610be6610be182610707565b610bc4565b82525050565b5f819050919050565b610c06610c0182610742565b610bec565b82525050565b5f819050919050565b610c26610c218261095a565b610c0c565b82525050565b5f610c378289610bd5565b601482019150610c478288610bd5565b601482019150610c578287610bf5565b602082019150610c678286610bf5565b602082019150610c778285610c15565b602082019150610c878284610bf5565b602082019150819050979650505050505050565b5f81519050610ca981610963565b92915050565b5f60208284031215610cc457610cc36106e0565b5b5f610cd184828501610c9b565b91505092915050565b7f4c324272696467653a20696e76616c69642070726f6f660000000000000000005f82015250565b5f610d0e601783610a13565b9150610d1982610cda565b602082019050919050565b5f6020820190508181035f830152610d3b81610d02565b9050919050565b610d4b8161095a565b82525050565b5f606082019050610d645f830186610932565b610d716020830185610932565b610d7e6040830184610d42565b949350505050565b7f5769746864726177616c20616d6f756e74206d75737420626520706f736974695f8201527f7665000000000000000000000000000000000000000000000000000000000000602082015250565b5f610de0602283610a13565b9150610deb82610d86565b604082019050919050565b5f6020820190508181035f830152610e0d81610dd4565b9050919050565b7f496e76616c6964207265636569766572000000000000000000000000000000005f82015250565b5f610e48601083610a13565b9150610e5382610e14565b602082019050919050565b5f6020820190508181035f830152610e7581610e3c565b9050919050565b5f610e878287610bd5565b601482019150610e978286610bd5565b601482019150610ea78285610bf5565b602082019150610eb78284610bf5565b60208201915081905095945050505050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f610f018285610c15565b602082019150610f118284610c15565b602082019150819050939250505056fea2646970667358221220dd25933b04a53134355c292466c0c72033dd3425e13b14d3b09ebd1c78c42a9764736f6c634300081f0033";

/// L1Anchor.sol runtime bytecode (compiled with solc 0.8.31).
///
/// Source: crates/vm/levm/contracts/L1Anchor.sol
/// Compile: cd crates/vm/levm/contracts && solc --bin-runtime L1Anchor.sol -o solc_out --overwrite
const L1_ANCHOR_RUNTIME_HEX: &str = "6080604052348015600e575f5ffd5b50600436106026575f3560e01c8063193e6dfe14602a575b5f5ffd5b60306044565b604051603b9190605f565b60405180910390f35b5f5481565b5f819050919050565b6059816049565b82525050565b5f60208201905060705f8301846052565b9291505056fea26469706673582212207697c8bc66d07fd2c3c8a919faad2c0acbd79972c963889054adb0c82738e20764736f6c634300081f0033";

/// Encode a call to NativeRollup.sendL1Message(address,uint256,bytes).
fn encode_send_l1_message_call(to: Address, data: &[u8]) -> Vec<u8> {
    // Function signature: sendL1Message(address,uint256,bytes)
    let selector = &keccak_hash(b"sendL1Message(address,uint256,bytes)")[..4];
    let mut encoded = Vec::new();
    encoded.extend_from_slice(selector);

    // _to (address)
    let mut addr_bytes = [0u8; 32];
    addr_bytes[12..].copy_from_slice(to.as_bytes());
    encoded.extend_from_slice(&addr_bytes);

    // _gasLimit (uint256) = DEFAULT_GAS_LIMIT = 100000
    let mut gas_limit_bytes = [0u8; 32];
    gas_limit_bytes[24..].copy_from_slice(&100_000u64.to_be_bytes());
    encoded.extend_from_slice(&gas_limit_bytes);

    // offset to _data (dynamic param 2 -> offset = 3 * 32 = 96 = 0x60)
    let mut offset_bytes = [0u8; 32];
    offset_bytes[31] = 0x60;
    encoded.extend_from_slice(&offset_bytes);

    // _data: length + data + padding
    let mut data_len_bytes = [0u8; 32];
    data_len_bytes[24..].copy_from_slice(&(data.len() as u64).to_be_bytes());
    encoded.extend_from_slice(&data_len_bytes);
    encoded.extend_from_slice(data);
    let padding = (32 - (data.len() % 32)) % 32;
    encoded.resize(encoded.len() + padding, 0);

    encoded
}

/// Parameters for the BlockParams struct in NativeRollup.advance().
/// The contract now tracks blockNumber, blockGasLimit, parentBaseFee, parentGasLimit,
/// and parentGasUsed on-chain, so BlockParams only has 5 fields.
struct BlockParams {
    post_state_root: H256,
    post_receipts_root: H256,
    coinbase: Address,
    prev_randao: H256,
    timestamp: u64,
}

/// Encode a call to NativeRollup.advance(uint256,(bytes32,bytes32,address,bytes32,uint256),bytes,bytes).
///
/// Function selector: advance(uint256,(bytes32,bytes32,address,bytes32,uint256),bytes,bytes) = 0x069c7eee
fn encode_advance_call(
    l1_messages_count: u64,
    block_params: &BlockParams,
    transactions_rlp: &[u8],
    witness_json: &[u8],
) -> Vec<u8> {
    fn pad32(len: usize) -> usize {
        len + ((32 - (len % 32)) % 32)
    }

    let mut encoded = Vec::new();

    // Function selector: 0x069c7eee
    encoded.extend_from_slice(&[0x06, 0x9c, 0x7e, 0xee]);

    // Param 0: _l1MessagesCount (uint256)
    let mut count_bytes = [0u8; 32];
    count_bytes[24..].copy_from_slice(&l1_messages_count.to_be_bytes());
    encoded.extend_from_slice(&count_bytes);

    // Param 1: _blockParams (struct — 5 fields encoded inline as a tuple)
    // All fields are value types, so they are encoded in-place (no offset pointer)
    // postStateRoot (bytes32)
    encoded.extend_from_slice(block_params.post_state_root.as_bytes());
    // postReceiptsRoot (bytes32)
    encoded.extend_from_slice(block_params.post_receipts_root.as_bytes());
    // coinbase (address, left-padded to 32 bytes)
    let mut coinbase_bytes = [0u8; 32];
    coinbase_bytes[12..].copy_from_slice(block_params.coinbase.as_bytes());
    encoded.extend_from_slice(&coinbase_bytes);
    // prevRandao (bytes32)
    encoded.extend_from_slice(block_params.prev_randao.as_bytes());
    // timestamp (uint256)
    let mut ts_bytes = [0u8; 32];
    ts_bytes[24..].copy_from_slice(&block_params.timestamp.to_be_bytes());
    encoded.extend_from_slice(&ts_bytes);

    // Param 2: offset to _transactions (dynamic)
    // Head for params is: 32 (_l1MessagesCount) + 5*32 (struct inline) + 32 (tx offset) + 32 (witness offset) = 256
    let head_size: usize = 32 + 160 + 32 + 32; // = 256
    let txs_offset = head_size;
    let txs_padded = pad32(transactions_rlp.len());
    let witness_offset = txs_offset + 32 + txs_padded;

    let mut txs_offset_bytes = [0u8; 32];
    txs_offset_bytes[24..].copy_from_slice(&(txs_offset as u64).to_be_bytes());
    encoded.extend_from_slice(&txs_offset_bytes);

    // Param 3: offset to _witness (dynamic)
    let mut witness_offset_bytes = [0u8; 32];
    witness_offset_bytes[24..].copy_from_slice(&(witness_offset as u64).to_be_bytes());
    encoded.extend_from_slice(&witness_offset_bytes);

    // Tail: _transactions (length + data + padding)
    let mut txs_len = [0u8; 32];
    txs_len[24..].copy_from_slice(&(transactions_rlp.len() as u64).to_be_bytes());
    encoded.extend_from_slice(&txs_len);
    encoded.extend_from_slice(transactions_rlp);
    let txs_padding = (32 - (transactions_rlp.len() % 32)) % 32;
    encoded.resize(encoded.len() + txs_padding, 0);

    // Tail: _witness (length + data + padding)
    let mut witness_len = [0u8; 32];
    witness_len[24..].copy_from_slice(&(witness_json.len() as u64).to_be_bytes());
    encoded.extend_from_slice(&witness_len);
    encoded.extend_from_slice(witness_json);
    let witness_padding = (32 - (witness_json.len() % 32)) % 32;
    encoded.resize(encoded.len() + witness_padding, 0);

    encoded
}

/// NativeRollup contract with sendL1Message + advance to verify an L2 state transition.
///
/// This test shows the full end-to-end flow:
///   1. sendL1Message(charlie, 100000, "") with 5 ETH -> records pending L1 message hash
///   2. advance(1, blockRlp, witnessJson)
///      -> NativeRollup computes Merkle root -> builds EXECUTE calldata -> CALL to 0x0101
///      -> precompile re-executes L2 block -> verifies state roots + anchors l1MessagesRoot
///      -> success -> contract updates stateRoot, blockNumber, l1MessageIndex
#[test]
fn test_native_rollup_contract() {
    let charlie = Address::from_low_u64_be(0xC4A);
    let l1_message_value = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

    // Deploy NativeRollup contract on "L1" (pre-loaded with runtime bytecode + initial storage)
    let contract_address = Address::from_low_u64_be(0xFFFF);
    let sender = Address::from_low_u64_be(0x1234);

    // Build L2 state with sender as the L1 message originator (must match msg.sender on L1)
    let (input, transactions_rlp, witness_json, pre_state_root, post_state_root, _l1_anchor) =
        build_l2_state_transition_with_sender(sender);

    let runtime_bytecode =
        Bytes::from(hex::decode(NATIVE_ROLLUP_RUNTIME_HEX).expect("invalid hex"));
    let contract_code_hash = H256(keccak_hash(runtime_bytecode.as_ref()));
    let contract_code = Code::from_bytecode(runtime_bytecode);

    // Pre-populate contract storage:
    //   slot 0 = stateRoot (pre_state_root)
    //   slot 2 = blockGasLimit (30_000_000)
    //   slot 3 = lastBaseFeePerGas (1_000_000_000 = 1 gwei)
    //   slot 4 = lastGasUsed (15_000_000 = blockGasLimit / 2, keeps base fee stable)
    let mut contract_storage: FxHashMap<H256, U256> = FxHashMap::default();
    contract_storage.insert(
        H256::zero(),
        U256::from_big_endian(pre_state_root.as_bytes()),
    );
    contract_storage.insert(H256::from_low_u64_be(2), U256::from(30_000_000u64));
    contract_storage.insert(H256::from_low_u64_be(3), U256::from(1_000_000_000u64));
    contract_storage.insert(H256::from_low_u64_be(4), U256::from(15_000_000u64));

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

    let l1_chain_config = test_chain_config();

    let store: Arc<dyn ethrex_levm::db::Database> = Arc::new(TestDb {
        chain_config: l1_chain_config,
    });
    let mut db = GeneralizedDatabase::new_with_account_state(store, accounts);

    // === TX 1: sendL1Message(charlie, 100000, "") with 5 ETH ===
    let send_l1_msg_calldata = encode_send_l1_message_call(charlie, b"");

    let send_l1_msg_tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000_000,
        to: TxKind::Call(contract_address),
        value: l1_message_value,
        data: Bytes::from(send_l1_msg_calldata),
        access_list: vec![],
        ..Default::default()
    });

    let send_l1_msg_env = Environment {
        origin: sender,
        gas_limit: 1_000_000_000,
        block_gas_limit: 1_000_000_000,
        tx_nonce: 0,
        chain_id: U256::from(1),
        ..Default::default()
    };

    let mut vm = VM::new(
        send_l1_msg_env,
        &mut db,
        &send_l1_msg_tx,
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("VM creation failed");

    let send_report = vm.execute().expect("VM execution failed");
    assert!(
        matches!(send_report.result, TxResult::Success),
        "sendL1Message transaction reverted: {:?}",
        send_report.result
    );
    println!("sendL1Message TX succeeded (5 ETH to charlie)");

    // Record sender balance before advance (to verify burned fees transfer)
    let sender_balance_before = db.get_account(sender).expect("sender account").info.balance;

    // === TX 2: advance(1, blockParams, transactionsRlp, witnessJson) ===
    let advance_block_params = BlockParams {
        post_state_root: input.post_state_root,
        post_receipts_root: input.post_receipts_root,
        coinbase: input.coinbase,
        prev_randao: input.prev_randao,
        timestamp: input.timestamp,
    };
    let advance_calldata =
        encode_advance_call(1, &advance_block_params, &transactions_rlp, &witness_json);

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
    let stored_l1_message_index = contract_account
        .storage
        .get(&H256::from_low_u64_be(6))
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
        stored_l1_message_index,
        U256::from(1),
        "Contract l1MessageIndex mismatch"
    );

    // Verify sender received burned fees (gas_price=0 so no gas costs on L1 side)
    let sender_balance_after = db.get_account(sender).expect("sender account").info.balance;
    let burned_fees_received = sender_balance_after - sender_balance_before;
    assert!(
        burned_fees_received > U256::zero(),
        "Sender should have received burned fees from advance()"
    );
    println!("  Burned fees sent to relayer: {burned_fees_received}");

    println!("NativeRollup contract demo succeeded!");
    println!("  L2 state transition verified via sendL1Message() + advance():");
    println!("    Pre-state root:  {pre_state_root:?}");
    println!("    Post-state root: {post_state_root:?}");
    println!("    Block number:    1");
    println!("    L1 message index: 1");
    println!("  Gas used: {}", report.gas_used);
}
