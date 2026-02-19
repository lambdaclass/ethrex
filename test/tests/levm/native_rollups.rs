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
        Withdrawal,
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

/// Build ABI-encoded calldata for the EXECUTE precompile.
///
/// Format: abi.encode(bytes32 preStateRoot, bytes blockRlp, bytes witnessJson, bytes32 l1MessagesRollingHash)
///
/// ABI layout:
///   slot 0: preStateRoot            (bytes32, static)
///   slot 1: offset_to_blockRlp      (uint256, dynamic pointer -> 0x80)
///   slot 2: offset_to_witness       (uint256, dynamic pointer)
///   slot 3: l1MessagesRollingHash   (bytes32, static -- NOT a pointer)
///   tail:   [block data] [witness data]
fn build_precompile_calldata(
    pre_state_root: H256,
    l1_messages_rolling_hash: H256,
    block_rlp: &[u8],
    witness_json: &[u8],
) -> Vec<u8> {
    // Helper: pad to 32-byte boundary
    fn pad32(len: usize) -> usize {
        len + ((32 - (len % 32)) % 32)
    }

    // Head is 4 * 32 = 128 bytes. Dynamic data starts after the head.
    let block_offset: usize = 128;
    let block_padded = pad32(block_rlp.len());
    let witness_offset: usize = block_offset + 32 + block_padded;

    let mut data = Vec::new();

    // slot 0: preStateRoot (bytes32)
    data.extend_from_slice(pre_state_root.as_bytes());

    // slot 1: offset to blockRlp
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(block_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // slot 2: offset to witnessJson
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(witness_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // slot 3: l1MessagesRollingHash (bytes32, static)
    data.extend_from_slice(l1_messages_rolling_hash.as_bytes());

    // tail: blockRlp (length + data + padding)
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(block_rlp.len() as u64).to_be_bytes());
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(block_rlp);
    data.resize(data.len() + (block_padded - block_rlp.len()), 0);

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
/// and wraps the given block in an ExecutePrecompileInput.
fn build_rejection_test_input(block: Block) -> ExecutePrecompileInput {
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
        number: block.header.number.saturating_sub(1),
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        timestamp: block.header.timestamp.saturating_sub(12),
        ..Default::default()
    };

    let chain_config = test_chain_config();

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
        l1_messages_rolling_hash: H256::zero(),
        execution_witness: witness,
        block,
    }
}

/// Convenience wrapper using the default L1 sender address (0x1A1).
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
///   - block RLP bytes (for binary calldata / contract call)
///   - witness JSON bytes (for binary calldata / contract call)
///   - pre_state_root
///   - post_state_root
///   - l1_messages_rolling_hash
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
    // Burn address used by L2Bridge.withdraw()
    insert_account(&mut state_trie, Address::zero(), &AccountState::default());
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

    // ===== Build final block =====
    let block = Block {
        header: BlockHeader {
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
        },
        body: BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    // ===== Build final witness (with correct block header) =====
    let witness = ExecutionWitness {
        codes: vec![bridge_runtime],
        block_headers_bytes: vec![parent_header.encode_to_vec(), block.header.encode_to_vec()],
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

    let block_rlp = block.encode_to_vec();
    let witness_json = serde_json::to_vec(&witness).expect("witness JSON serialization failed");

    let input = ExecutePrecompileInput {
        pre_state_root,
        l1_messages_rolling_hash,
        execution_witness: witness,
        block,
    };

    (
        input,
        block_rlp,
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
///   3. Call EXECUTE precompile with the block, witness, and L1 messages rolling hash
///   4. Verify returned state root, block number, and gas used
#[test]
fn test_execute_precompile_transfer_and_l1_message() {
    let (
        _input,
        block_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
        l1_messages_rolling_hash,
    ) = build_l2_state_transition();

    // Build ABI-encoded calldata and execute via execute_precompile()
    let calldata = build_precompile_calldata(
        pre_state_root,
        l1_messages_rolling_hash,
        &block_rlp,
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
            let returned_withdrawal_root = H256::from_slice(&output[64..96]);
            let returned_gas_used = U256::from_big_endian(&output[96..128]);
            let returned_burned_fees = U256::from_big_endian(&output[128..160]);
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
            println!("EXECUTE precompile succeeded!");
            println!("  Pre-state root:  {pre_state_root:?}");
            println!("  Post-state root: {post_state_root:?}");
            println!("  Relayer processed L1 message: 5 ETH to charlie");
            println!("  Alice sent 1 ETH to Bob");
            println!("  Gas used: {returned_gas_used}");
            println!("  Burned fees: {returned_burned_fees}");
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
const NATIVE_ROLLUP_RUNTIME_HEX: &str = "608060405260043610610089575f3560e01c806357e871e71161005857806357e871e7146101be5780635eaadcc7146101e85780639588eca214610204578063a623f02e1461022e578063ed3133f21461026a576100f0565b806307132c05146100f45780630a045444146101305780631ed873c7146101585780633e67267f14610182576100f0565b366100f0575f34116100d0576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016100c790610c2a565b60405180910390fd5b6100ee333334620186a060405180602001604052805f815250610292565b005b5f5ffd5b3480156100ff575f5ffd5b5061011a60048036038101906101159190610c83565b610374565b6040516101279190610cc8565b60405180910390f35b34801561013b575f5ffd5b5061015660048036038101906101519190610dcf565b610391565b005b348015610163575f5ffd5b5061016c61076c565b6040516101799190610e88565b60405180910390f35b34801561018d575f5ffd5b506101a860048036038101906101a39190610ea1565b610772565b6040516101b59190610edb565b60405180910390f35b3480156101c9575f5ffd5b506101d2610792565b6040516101df9190610e88565b60405180910390f35b61020260048036038101906101fd9190610f49565b610798565b005b34801561020f575f5ffd5b506102186107ee565b6040516102259190610edb565b60405180910390f35b348015610239575f5ffd5b50610254600480360381019061024f9190610ea1565b6107f3565b6040516102619190610edb565b60405180910390f35b348015610275575f5ffd5b50610290600480360381019061028b9190610fba565b610808565b005b5f60028054905090505f828051906020012090505f8787878785876040516020016102c2969594939291906110d0565b604051602081830303815290604052805190602001209050600281908060018154018082558091505060019003905f5260205f20015f9091909190915055828773ffffffffffffffffffffffffffffffffffffffff168973ffffffffffffffffffffffffffffffffffffffff167fb42098fed77baca4d18a86b35f7bbfc750de2275ad10e0837fd550faf3c9d56f8989876040516103629392919061113f565b60405180910390a45050505050505050565b6005602052805f5260405f205f915054906101000a900460ff1681565b60065f9054906101000a900460ff16156103e0576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016103d7906111be565b60405180910390fd5b600160065f6101000a81548160ff02191690831515021790555060015483111561043f576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161043690611226565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168673ffffffffffffffffffffffffffffffffffffffff16036104ad576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104a49061128e565b60405180910390fd5b5f85116104ef576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104e6906112f6565b60405180910390fd5b5f878787876040516020016105079493929190611314565b60405160208183030381529060405280519060200120905060055f8281526020019081526020015f205f9054906101000a900460ff161561057d576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610574906113ab565b60405180910390fd5b5f60045f8681526020019081526020015f205490505f5f1b81036105d6576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016105cd90611413565b60405180910390fd5b5f6105e385858486610b0f565b905080610625576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161061c9061147b565b60405180910390fd5b600160055f8581526020019081526020015f205f6101000a81548160ff0219169083151502179055505f8973ffffffffffffffffffffffffffffffffffffffff1689604051610673906114c6565b5f6040518083038185875af1925050503d805f81146106ad576040519150601f19603f3d011682016040523d82523d5f602084013e6106b2565b606091505b50509050806106f6576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016106ed90611524565b60405180910390fd5b87878b73ffffffffffffffffffffffffffffffffffffffff167f1113af8a2f367ad0f39a44a9985b12833c5e9dcb54532dd60575fc4ccbd5f9818c60405161073e9190610e88565b60405180910390a4505050505f60065f6101000a81548160ff02191690831515021790555050505050505050565b60035481565b60028181548110610781575f80fd5b905f5260205f20015f915090505481565b60015481565b6107e83385348686868080601f0160208091040260200160405190810160405280939291908181526020018383808284375f81840152601f19601f82011690508083019250505050505050610292565b50505050565b5f5481565b6004602052805f5260405f205f915090505481565b5f60035490506002805490508682610820919061156f565b1115610861576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610858906115ec565b60405180910390fd5b5f5f5f1b90505f5f90505b878110156108d3578160028285610883919061156f565b815481106108945761089361160a565b5b905f5260205f2001546040516020016108ae929190611637565b604051602081830303815290604052805190602001209150808060010191505061086c565b5086826108e0919061156f565b6003819055505f5f548787878786604051602001610903969594939291906116bc565b60405160208183030381529060405290505f5f61010173ffffffffffffffffffffffffffffffffffffffff168360405161093d9190611759565b5f604051808303815f865af19150503d805f8114610976576040519150601f19603f3d011682016040523d82523d5f602084013e61097b565b606091505b509150915081801561098e575060a08151145b6109cd576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016109c4906117df565b60405180910390fd5b5f5f5f5f848060200190518101906109e59190611825565b945050935093509350835f81905550826001819055508160045f8581526020019081526020015f20819055505f811115610ac3575f3373ffffffffffffffffffffffffffffffffffffffff1682604051610a3e906114c6565b5f6040518083038185875af1925050503d805f8114610a78576040519150601f19603f3d011682016040523d82523d5f602084013e610a7d565b606091505b5050905080610ac1576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610ab8906118e6565b60405180910390fd5b505b827f90575f875cc6b35d0dca93d3812abea93f955cf5ec0a36576a5bce85e1687305858484604051610af793929190611904565b60405180910390a25050505050505050505050505050565b5f5f8290505f5f90505b86869050811015610b5657610b4782888884818110610b3b57610b3a61160a565b5b90506020020135610b65565b91508080600101915050610b19565b50838114915050949350505050565b5f81831015610b9e578282604051602001610b81929190611637565b604051602081830303815290604052805190602001209050610bca565b8183604051602001610bb1929190611637565b6040516020818303038152906040528051906020012090505b92915050565b5f82825260208201905092915050565b7f4d7573742073656e6420455448000000000000000000000000000000000000005f82015250565b5f610c14600d83610bd0565b9150610c1f82610be0565b602082019050919050565b5f6020820190508181035f830152610c4181610c08565b9050919050565b5f5ffd5b5f5ffd5b5f819050919050565b610c6281610c50565b8114610c6c575f5ffd5b50565b5f81359050610c7d81610c59565b92915050565b5f60208284031215610c9857610c97610c48565b5b5f610ca584828501610c6f565b91505092915050565b5f8115159050919050565b610cc281610cae565b82525050565b5f602082019050610cdb5f830184610cb9565b92915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610d0a82610ce1565b9050919050565b610d1a81610d00565b8114610d24575f5ffd5b50565b5f81359050610d3581610d11565b92915050565b5f819050919050565b610d4d81610d3b565b8114610d57575f5ffd5b50565b5f81359050610d6881610d44565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f840112610d8f57610d8e610d6e565b5b8235905067ffffffffffffffff811115610dac57610dab610d72565b5b602083019150836020820283011115610dc857610dc7610d76565b5b9250929050565b5f5f5f5f5f5f5f60c0888a031215610dea57610de9610c48565b5b5f610df78a828b01610d27565b9750506020610e088a828b01610d27565b9650506040610e198a828b01610d5a565b9550506060610e2a8a828b01610d5a565b9450506080610e3b8a828b01610d5a565b93505060a088013567ffffffffffffffff811115610e5c57610e5b610c4c565b5b610e688a828b01610d7a565b925092505092959891949750929550565b610e8281610d3b565b82525050565b5f602082019050610e9b5f830184610e79565b92915050565b5f60208284031215610eb657610eb5610c48565b5b5f610ec384828501610d5a565b91505092915050565b610ed581610c50565b82525050565b5f602082019050610eee5f830184610ecc565b92915050565b5f5f83601f840112610f0957610f08610d6e565b5b8235905067ffffffffffffffff811115610f2657610f25610d72565b5b602083019150836001820283011115610f4257610f41610d76565b5b9250929050565b5f5f5f5f60608587031215610f6157610f60610c48565b5b5f610f6e87828801610d27565b9450506020610f7f87828801610d5a565b935050604085013567ffffffffffffffff811115610fa057610f9f610c4c565b5b610fac87828801610ef4565b925092505092959194509250565b5f5f5f5f5f60608688031215610fd357610fd2610c48565b5b5f610fe088828901610d5a565b955050602086013567ffffffffffffffff81111561100157611000610c4c565b5b61100d88828901610ef4565b9450945050604086013567ffffffffffffffff8111156110305761102f610c4c565b5b61103c88828901610ef4565b92509250509295509295909350565b5f8160601b9050919050565b5f6110618261104b565b9050919050565b5f61107282611057565b9050919050565b61108a61108582610d00565b611068565b82525050565b5f819050919050565b6110aa6110a582610d3b565b611090565b82525050565b5f819050919050565b6110ca6110c582610c50565b6110b0565b82525050565b5f6110db8289611079565b6014820191506110eb8288611079565b6014820191506110fb8287611099565b60208201915061110b8286611099565b60208201915061111b82856110b9565b60208201915061112b8284611099565b602082019150819050979650505050505050565b5f6060820190506111525f830186610e79565b61115f6020830185610e79565b61116c6040830184610ecc565b949350505050565b7f5265656e7472616e637947756172643a207265656e7472616e742063616c6c005f82015250565b5f6111a8601f83610bd0565b91506111b382611174565b602082019050919050565b5f6020820190508181035f8301526111d58161119c565b9050919050565b7f426c6f636b206e6f74207965742066696e616c697a65640000000000000000005f82015250565b5f611210601783610bd0565b915061121b826111dc565b602082019050919050565b5f6020820190508181035f83015261123d81611204565b9050919050565b7f496e76616c6964207265636569766572000000000000000000000000000000005f82015250565b5f611278601083610bd0565b915061128382611244565b602082019050919050565b5f6020820190508181035f8301526112a58161126c565b9050919050565b7f416d6f756e74206d75737420626520706f7369746976650000000000000000005f82015250565b5f6112e0601783610bd0565b91506112eb826112ac565b602082019050919050565b5f6020820190508181035f83015261130d816112d4565b9050919050565b5f61131f8287611079565b60148201915061132f8286611079565b60148201915061133f8285611099565b60208201915061134f8284611099565b60208201915081905095945050505050565b7f5769746864726177616c20616c726561647920636c61696d65640000000000005f82015250565b5f611395601a83610bd0565b91506113a082611361565b602082019050919050565b5f6020820190508181035f8301526113c281611389565b9050919050565b7f4e6f207769746864726177616c7320666f72207468697320626c6f636b0000005f82015250565b5f6113fd601d83610bd0565b9150611408826113c9565b602082019050919050565b5f6020820190508181035f83015261142a816113f1565b9050919050565b7f496e76616c6964204d65726b6c652070726f6f660000000000000000000000005f82015250565b5f611465601483610bd0565b915061147082611431565b602082019050919050565b5f6020820190508181035f83015261149281611459565b9050919050565b5f81905092915050565b50565b5f6114b15f83611499565b91506114bc826114a3565b5f82019050919050565b5f6114d0826114a6565b9150819050919050565b7f455448207472616e73666572206661696c6564000000000000000000000000005f82015250565b5f61150e601383610bd0565b9150611519826114da565b602082019050919050565b5f6020820190508181035f83015261153b81611502565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61157982610d3b565b915061158483610d3b565b925082820190508082111561159c5761159b611542565b5b92915050565b7f4e6f7420656e6f756768204c31206d65737361676573000000000000000000005f82015250565b5f6115d6601683610bd0565b91506115e1826115a2565b602082019050919050565b5f6020820190508181035f830152611603816115ca565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f61164282856110b9565b60208201915061165282846110b9565b6020820191508190509392505050565b5f82825260208201905092915050565b828183375f83830152505050565b5f601f19601f8301169050919050565b5f61169b8385611662565b93506116a8838584611672565b6116b183611680565b840190509392505050565b5f6080820190506116cf5f830189610ecc565b81810360208301526116e2818789611690565b905081810360408301526116f7818587611690565b90506117066060830184610ecc565b979650505050505050565b5f81519050919050565b8281835e5f83830152505050565b5f61173382611711565b61173d8185611499565b935061174d81856020860161171b565b80840191505092915050565b5f6117648284611729565b915081905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f6117c9602683610bd0565b91506117d48261176f565b604082019050919050565b5f6020820190508181035f8301526117f6816117bd565b9050919050565b5f8151905061180b81610c59565b92915050565b5f8151905061181f81610d44565b92915050565b5f5f5f5f5f60a0868803121561183e5761183d610c48565b5b5f61184b888289016117fd565b955050602061185c88828901611811565b945050604061186d888289016117fd565b935050606061187e88828901611811565b925050608061188f88828901611811565b9150509295509295909350565b7f4275726e65642066656573207472616e73666572206661696c656400000000005f82015250565b5f6118d0601b83610bd0565b91506118db8261189c565b602082019050919050565b5f6020820190508181035f8301526118fd816118c4565b9050919050565b5f6060820190506119175f830186610ecc565b6119246020830185610ecc565b6119316040830184610e79565b94935050505056fea2646970667358221220c8933a81e67b543ef55f6572b5bce087fa608fb7ec9a9ec8101292dc1b4843be64736f6c634300081f0033";

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

/// Encode a call to NativeRollup.advance(uint256, bytes, bytes).
fn encode_advance_call(l1_messages_count: u64, block_rlp: &[u8], witness_json: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::new();

    // Function selector: advance(uint256,bytes,bytes) = 0xed3133f2
    encoded.extend_from_slice(&[0xed, 0x31, 0x33, 0xf2]);

    // _l1MessagesCount (uint256)
    let mut count_bytes = [0u8; 32];
    count_bytes[24..].copy_from_slice(&l1_messages_count.to_be_bytes());
    encoded.extend_from_slice(&count_bytes);

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
        _input,
        block_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
        _l1_messages_rolling_hash,
    ) = build_l2_state_transition_with_sender(sender);

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
    let stored_l1_message_index = contract_account
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
