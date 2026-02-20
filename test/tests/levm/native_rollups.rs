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
    execute_precompile::{ExecutePrecompileInput, L2_BRIDGE, execute_inner, execute_precompile},
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

/// Helper: encode L2Bridge.processL1Message(address,address,uint256,uint256,bytes,uint256) calldata.
///
/// Function signature: processL1Message(address from, address to, uint256 value, uint256 gasLimit, bytes data, uint256 nonce)
/// The `bytes data` parameter is dynamic, requiring ABI encoding with offset pointer.
fn encode_process_l1_message_call(
    from: Address,
    to: Address,
    value: U256,
    gas_limit: U256,
    data: &[u8],
    nonce: u64,
) -> Vec<u8> {
    // selector = keccak256("processL1Message(address,address,uint256,uint256,bytes,uint256)")[0:4]
    let selector =
        &keccak_hash(b"processL1Message(address,address,uint256,uint256,bytes,uint256)")[..4];

    // ABI encoding: 6 params, where param 5 (bytes data) is dynamic.
    // Head: 6 * 32 = 192 bytes
    // Param 0: from (address, left-padded)
    // Param 1: to (address, left-padded)
    // Param 2: value (uint256)
    // Param 3: gasLimit (uint256)
    // Param 4: offset to data (uint256, points to 192 = 0xC0)
    // Param 5: nonce (uint256)
    // Tail: [length of data][data bytes][padding]
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

    // offset to data (dynamic param 4 → offset = 6 * 32 = 192 = 0xC0)
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&192u64.to_be_bytes());
    calldata.extend_from_slice(&offset_bytes);

    // nonce (uint256)
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes[24..].copy_from_slice(&nonce.to_be_bytes());
    calldata.extend_from_slice(&nonce_bytes);

    // Tail: data length + data + padding
    let mut data_len_bytes = [0u8; 32];
    data_len_bytes[24..].copy_from_slice(&(data.len() as u64).to_be_bytes());
    calldata.extend_from_slice(&data_len_bytes);
    calldata.extend_from_slice(data);
    // Pad to 32-byte boundary
    let padding = (32 - (data.len() % 32)) % 32;
    calldata.resize(calldata.len() + padding, 0);

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
///     bytes32 l1MessagesRollingHash,  // slot 11
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
    l1_messages_rolling_hash: H256,
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

    // slot 11: l1MessagesRollingHash (bytes32)
    data.extend_from_slice(l1_messages_rolling_hash.as_bytes());

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
        l1_messages_rolling_hash: H256::zero(),
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
///   - l1_messages_rolling_hash
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
    // TX0: relayer -> L2Bridge.processL1Message(l1_sender, charlie, 5 ETH, 100000, "", 0)
    let l1_message_calldata = encode_process_l1_message_call(
        l1_sender,
        charlie,
        l1_message_value,
        l1_message_gas_limit,
        b"", // empty data for simple ETH transfer
        0,
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

    let temp_witness = ExecutionWitness {
        codes: vec![bridge_runtime.clone()],
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
        codes: vec![bridge_runtime],
        block_headers_bytes: vec![parent_header.encode_to_vec(), final_header.encode_to_vec()],
        first_block_number: 1,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots,
        keys: vec![],
    };

    // ===== Compute L1 messages rolling hash =====
    // message_hash = keccak256(abi.encodePacked(from[20], to[20], value[32], gasLimit[32], keccak256(data)[32], nonce[32]))
    // = 168 bytes preimage
    let data_hash = H256::from(keccak_hash(b"")); // empty data
    let mut message_preimage = Vec::with_capacity(168);
    message_preimage.extend_from_slice(l1_sender.as_bytes()); // from: 20 bytes
    message_preimage.extend_from_slice(charlie.as_bytes()); // to: 20 bytes
    message_preimage.extend_from_slice(&l1_message_value.to_big_endian()); // value: 32 bytes
    message_preimage.extend_from_slice(&l1_message_gas_limit.to_big_endian()); // gasLimit: 32 bytes
    message_preimage.extend_from_slice(data_hash.as_bytes()); // keccak256(data): 32 bytes
    message_preimage.extend_from_slice(&U256::zero().to_big_endian()); // nonce=0: 32 bytes
    let message_hash = H256::from(keccak_hash(&message_preimage));

    // rolling = keccak256(abi.encodePacked(H256::zero(), message_hash))
    let mut rolling_preimage = [0u8; 64];
    rolling_preimage[..32].copy_from_slice(H256::zero().as_bytes());
    rolling_preimage[32..].copy_from_slice(message_hash.as_bytes());
    let l1_messages_rolling_hash = H256::from(keccak_hash(rolling_preimage));

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
        l1_messages_rolling_hash,
        transactions,
        execution_witness: witness,
    };

    (
        input,
        transactions_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
        l1_messages_rolling_hash,
    )
}

// ===== Unit Tests =====

/// The main test: execute a processL1Message + transfer via the EXECUTE precompile.
///
/// Flow:
///   1. Build L2 genesis with L2Bridge (preminted) + relayer
///   2. L2 block has 2 txs: processL1Message(l1_sender, charlie, 5 ETH, 100000, "", 0) + Alice->Bob 1 ETH
///   3. Call EXECUTE precompile with individual fields, transactions, witness, and L1 messages rolling hash
///   4. Verify returned state root, block number, and gas used
#[test]
fn test_execute_precompile_transfer_and_l1_message() {
    let (
        input,
        transactions_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
        l1_messages_rolling_hash,
    ) = build_l2_state_transition();

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
        l1_messages_rolling_hash,
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
            assert_eq!(output.len(), 192, "Expected 192-byte ABI-encoded return");
            let returned_root = H256::from_slice(&output[..32]);
            let returned_block_num = U256::from_big_endian(&output[32..64]);
            let returned_withdrawal_root = H256::from_slice(&output[64..96]);
            let returned_gas_used = U256::from_big_endian(&output[96..128]);
            let returned_burned_fees = U256::from_big_endian(&output[128..160]);
            let returned_base_fee = U256::from_big_endian(&output[160..192]);
            assert_eq!(
                returned_root, post_state_root,
                "Returned state root mismatch"
            );
            assert_eq!(
                returned_block_num,
                U256::from(1),
                "Returned block number mismatch"
            );
            // No withdrawals in this block
            assert_eq!(
                returned_withdrawal_root,
                H256::zero(),
                "Withdrawal root should be zero when no withdrawals"
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
///
/// Source: crates/vm/levm/contracts/NativeRollup.sol
/// Compile: cd crates/vm/levm/contracts && solc --bin-runtime --via-ir --optimize NativeRollup.sol -o /tmp/solc_native_out --overwrite
const NATIVE_ROLLUP_RUNTIME_HEX: &str = "60806040526004361015610160575b3615610018575f80fd5b341561012b5760405161002c602082610bf2565b5f90526005546040513360601b6bffffffffffffffffffffffff1916602082018181526034830191909152346048830152620186a060688301527fc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a4706088830181905260a88084018590528352916100a460c882610bf2565b519020600160401b83101561011757600183016005556100c383610bb9565b819291549060031b91821b915f19901b191617905560405190348252620186a06020830152604082015233907fb42098fed77baca4d18a86b35f7bbfc750de2275ad10e0837fd550faf3c9d56f60603392a4005b634e487b7160e01b5f52604160045260245ffd5b60405162461bcd60e51b815260206004820152600d60248201526c09aeae6e840e6cadcc8408aa89609b1b6044820152606490fd5b5f3560e01c8063069c7eee1461081757806307132c05146107e85780630a045444146104375780631ed873c71461041a57806336d9e47c146103fd5780633e67267f146103c557806357e871e7146103a85780635eaadcc71461026c5780637877a7971461024f5780639588eca21461023357806395c570bd146102165763a623f02e0361000e5734610212576020366003190112610212576004355f526007602052602060405f2054604051908152f35b5f80fd5b34610212575f366003190112610212576020600454604051908152f35b34610212575f3660031901126102125760205f54604051908152f35b34610212575f366003190112610212576020600254604051908152f35b606036600319011261021257610280610ba3565b60243560443567ffffffffffffffff8111610212576102a3903690600401610b75565b92906102ae84610c34565b936102bc6040519586610bf2565b8085526020850191368282011161021257815f926020928537860101526005549351902060405160208101906001600160601b03193360601b1682526001600160601b03198460601b1660348201523460488201528460688201528260888201528560a882015260a8815261033260c882610bf2565b519020600160401b851015610117576001850160055561035185610bb9565b819291549060031b91821b915f19901b1916179055604051923484526020840152604083015260018060a01b0316907fb42098fed77baca4d18a86b35f7bbfc750de2275ad10e0837fd550faf3c9d56f60603392a4005b34610212575f366003190112610212576020600154604051908152f35b3461021257602036600319011261021257600435600554811015610212576103ee602091610bb9565b90549060031b1c604051908152f35b34610212575f366003190112610212576020600354604051908152f35b34610212575f366003190112610212576020600654604051908152f35b346102125760c036600319011261021257610450610ba3565b6024356001600160a01b0381168082036102125760a43590606435906084359060443567ffffffffffffffff851161021257366023860112156102125784600401359667ffffffffffffffff8811610212573660248960051b880101116102125760095460ff81166107a35760ff1916600190811760095554841161075e5782156107265781156106e15760405160208101916001600160601b03199060601b1682526001600160601b03198860601b16603482015282604882015285606882015260688152610521608882610bf2565b519020805f52600860205260ff60405f20541661069c57835f52600760205260405f2054968715610657575f97825b8a8a10156105765761056e60019160248c60051b8c01013590610c7f565b990198610550565b0361061b575f808481949382948352600860205260408320600160ff198254161790555af16105a3610c50565b50156105e05760207f1113af8a2f367ad0f39a44a9985b12833c5e9dcb54532dd60575fc4ccbd5f98191604051908152a46009805460ff19169055005b60405162461bcd60e51b8152602060048201526013602482015272115512081d1c985b9cd9995c8819985a5b1959606a1b6044820152606490fd5b60405162461bcd60e51b815260206004820152601460248201527324b73b30b634b21026b2b935b63290383937b7b360611b6044820152606490fd5b60405162461bcd60e51b815260206004820152601d60248201527f4e6f207769746864726177616c7320666f72207468697320626c6f636b0000006044820152606490fd5b60405162461bcd60e51b815260206004820152601a60248201527f5769746864726177616c20616c726561647920636c61696d65640000000000006044820152606490fd5b60405162461bcd60e51b815260206004820152601760248201527f416d6f756e74206d75737420626520706f7369746976650000000000000000006044820152606490fd5b60405162461bcd60e51b815260206004820152601060248201526f24b73b30b634b2103932b1b2b4bb32b960811b6044820152606490fd5b60405162461bcd60e51b815260206004820152601760248201527f426c6f636b206e6f74207965742066696e616c697a65640000000000000000006044820152606490fd5b60405162461bcd60e51b815260206004820152601f60248201527f5265656e7472616e637947756172643a207265656e7472616e742063616c6c006044820152606490fd5b34610212576020366003190112610212576004355f526008602052602060ff60405f2054166040519015158152f35b34610212576101003660031901126102125760043560a03660231901126102125760c43567ffffffffffffffff811161021257610858903690600401610b75565b919060e43567ffffffffffffffff81116102125761087a903690600401610b75565b600654936108888186610be5565b60055410610b37575f945f5b828110610af15750906108a691610be5565b6006556001549260018401809411610add5760025495600354956004545f54986064359860018060a01b038a16809a0361021257604051998a9960208b019c8d5260243560408c015260443560608c015260808b01528260a08b015260c08a015260843560e08a015260a4356101008a01526101208901526101408801526101608701526101808601526101a085016101c090526101e085019061094992610c14565b90601f19848303016101c085015261096092610c14565b03601f19810182526109729082610bf2565b51905a915f9291836101018194f1610988610c50565b9080610ad2575b15610a7e5760c081805181010312610212576020810151906040810151916060820151608083015160c060a085015194015190835f5585600155600455600355835f5260076020528060405f205582610a1c575b907f90575f875cc6b35d0dca93d3812abea93f955cf5ec0a36576a5bce85e16873059260609260405192835260208301526040820152a2005b905f80808086335af1610a2d610c50565b5015610a3957906109e3565b60405162461bcd60e51b815260206004820152601b60248201527f4275726e65642066656573207472616e73666572206661696c656400000000006044820152606490fd5b60405162461bcd60e51b815260206004820152602660248201527f4558454355544520707265636f6d70696c6520766572696669636174696f6e2060448201526519985a5b195960d21b6064820152608490fd5b5060c081511461098f565b634e487b7160e01b5f52601160045260245ffd5b95600190610b07610b028985610be5565b610bb9565b90549060031b1c6040519060208201928352604082015260408152610b2d606082610bf2565b5190209601610894565b60405162461bcd60e51b81526020600482015260166024820152754e6f7420656e6f756768204c31206d6573736167657360501b6044820152606490fd5b9181601f840112156102125782359167ffffffffffffffff8311610212576020838186019501011161021257565b600435906001600160a01b038216820361021257565b600554811015610bd15760055f5260205f2001905f90565b634e487b7160e01b5f52603260045260245ffd5b91908201809211610add57565b90601f8019910116810190811067ffffffffffffffff82111761011757604052565b908060209392818452848401375f828201840152601f01601f1916010190565b67ffffffffffffffff811161011757601f01601f191660200190565b3d15610c7a573d90610c6182610c34565b91610c6f6040519384610bf2565b82523d5f602084013e565b606090565b9080821015610cad576040519060208201928352604082015260408152610ca7606082610bf2565b51902090565b906040519060208201928352604082015260408152610ca7606082610bf256fea2646970667358221220e454b917528c8db5eee9a78930ad5d3e908306185515aa430eaa0f5c3de4d39a64736f6c634300081f0033";

/// L2Bridge.sol runtime bytecode (compiled with solc 0.8.31).
///
/// Source: crates/vm/levm/contracts/L2Bridge.sol
/// Compile: cd crates/vm/levm/contracts && solc --bin-runtime L2Bridge.sol -o solc_out --overwrite
const L2_BRIDGE_RUNTIME_HEX: &str = "608060405260043610610049575f3560e01c806351cff8d91461004d5780637b259db4146100695780638406c07914610093578063b060182d146100bd578063e4a83803146100e7575b5f5ffd5b610067600480360381019061006291906104b7565b61010f565b005b348015610074575f5ffd5b5061007d610242565b60405161008a91906104fa565b60405180910390f35b34801561009e575f5ffd5b506100a7610248565b6040516100b49190610522565b60405180910390f35b3480156100c8575f5ffd5b506100d161026c565b6040516100de91906104fa565b60405180910390f35b3480156100f2575f5ffd5b5061010d600480360381019061010891906105c6565b610272565b005b5f3411610151576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610148906106f0565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168173ffffffffffffffffffffffffffffffffffffffff16036101bf576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101b690610758565b60405180910390fd5b5f60025490506001816101d291906107a3565b600281905550808273ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167f2e16c360bf25f9193c8e78b0fcdf02bacfd34fd98ec9fe4aa2549e15346dafd23460405161023691906104fa565b60405180910390a45050565b60025481565b5f5f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1681565b60015481565b5f5f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff1614610300576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016102f790610820565b60405180910390fd5b6001548114610344576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161033b90610888565b60405180910390fd5b5f600154905060018161035791906107a3565b6001819055508673ffffffffffffffffffffffffffffffffffffffff1686869086866040516103879291906108e2565b5f60405180830381858888f193505050503d805f81146103c2576040519150601f19603f3d011682016040523d82523d5f602084013e6103c7565b606091505b505050808773ffffffffffffffffffffffffffffffffffffffff168973ffffffffffffffffffffffffffffffffffffffff167f481690f24f8062803168a6eea64e8fda33ee03ae29d4fbc1e5b1e76629e13e218989898960405161042c9291906108e2565b604051809103902060405161044393929190610912565b60405180910390a45050505050505050565b5f5ffd5b5f5ffd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6104868261045d565b9050919050565b6104968161047c565b81146104a0575f5ffd5b50565b5f813590506104b18161048d565b92915050565b5f602082840312156104cc576104cb610455565b5b5f6104d9848285016104a3565b91505092915050565b5f819050919050565b6104f4816104e2565b82525050565b5f60208201905061050d5f8301846104eb565b92915050565b61051c8161047c565b82525050565b5f6020820190506105355f830184610513565b92915050565b610544816104e2565b811461054e575f5ffd5b50565b5f8135905061055f8161053b565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261058657610585610565565b5b8235905067ffffffffffffffff8111156105a3576105a2610569565b5b6020830191508360018202830111156105bf576105be61056d565b5b9250929050565b5f5f5f5f5f5f5f60c0888a0312156105e1576105e0610455565b5b5f6105ee8a828b016104a3565b97505060206105ff8a828b016104a3565b96505060406106108a828b01610551565b95505060606106218a828b01610551565b945050608088013567ffffffffffffffff81111561064257610641610459565b5b61064e8a828b01610571565b935093505060a06106618a828b01610551565b91505092959891949750929550565b5f82825260208201905092915050565b7f5769746864726177616c20616d6f756e74206d75737420626520706f736974695f8201527f7665000000000000000000000000000000000000000000000000000000000000602082015250565b5f6106da602283610670565b91506106e582610680565b604082019050919050565b5f6020820190508181035f830152610707816106ce565b9050919050565b7f496e76616c6964207265636569766572000000000000000000000000000000005f82015250565b5f610742601083610670565b915061074d8261070e565b602082019050919050565b5f6020820190508181035f83015261076f81610736565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6107ad826104e2565b91506107b8836104e2565b92508282019050808211156107d0576107cf610776565b5b92915050565b7f4c324272696467653a206e6f742072656c6179657200000000000000000000005f82015250565b5f61080a601583610670565b9150610815826107d6565b602082019050919050565b5f6020820190508181035f830152610837816107fe565b9050919050565b7f4c324272696467653a206e6f6e6365206d69736d6174636800000000000000005f82015250565b5f610872601883610670565b915061087d8261083e565b602082019050919050565b5f6020820190508181035f83015261089f81610866565b9050919050565b5f81905092915050565b828183375f83830152505050565b5f6108c983856108a6565b93506108d68385846108b0565b82840190509392505050565b5f6108ee8284866108be565b91508190509392505050565b5f819050919050565b61090c816108fa565b82525050565b5f6060820190506109255f8301866104eb565b61093260208301856104eb565b61093f6040830184610903565b94935050505056fea26469706673582212200182008a2262f45370e1ee947dd9fb58c45dd127e40e2466028f53aff2beb70c64736f6c634300081f0033";

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
///      -> NativeRollup computes rolling hash -> builds EXECUTE calldata -> CALL to 0x0101
///      -> precompile re-executes L2 block -> verifies state roots + L1 messages rolling hash
///      -> success -> contract updates stateRoot, blockNumber, l1MessageIndex
#[test]
fn test_native_rollup_contract() {
    let charlie = Address::from_low_u64_be(0xC4A);
    let l1_message_value = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

    // Deploy NativeRollup contract on "L1" (pre-loaded with runtime bytecode + initial storage)
    let contract_address = Address::from_low_u64_be(0xFFFF);
    let sender = Address::from_low_u64_be(0x1234);

    // Build L2 state with sender as the L1 message originator (must match msg.sender on L1)
    let (
        input,
        transactions_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
        _l1_messages_rolling_hash,
    ) = build_l2_state_transition_with_sender(sender);

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
