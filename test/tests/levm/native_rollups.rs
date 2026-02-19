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

/// Helper: encode L2Bridge.processDeposit(address,uint256,uint256) calldata.
fn encode_process_deposit_call(recipient: Address, amount: U256, nonce: u64) -> Vec<u8> {
    // selector = keccak256("processDeposit(address,uint256,uint256)")[0:4] = 0xa95ecfec
    let selector = &keccak_hash(b"processDeposit(address,uint256,uint256)")[..4];
    let mut data = Vec::with_capacity(4 + 96);
    data.extend_from_slice(selector);
    // recipient (address, left-padded to 32 bytes)
    let mut addr_bytes = [0u8; 32];
    addr_bytes[12..].copy_from_slice(recipient.as_bytes());
    data.extend_from_slice(&addr_bytes);
    // amount (uint256)
    data.extend_from_slice(&amount.to_big_endian());
    // nonce (uint256)
    let mut nonce_bytes = [0u8; 32];
    nonce_bytes[24..].copy_from_slice(&nonce.to_be_bytes());
    data.extend_from_slice(&nonce_bytes);
    data
}

/// Build ABI-encoded calldata for the EXECUTE precompile.
///
/// Format: abi.encode(bytes32 preStateRoot, bytes blockRlp, bytes witnessJson, bytes32 depositsRollingHash)
///
/// ABI layout:
///   slot 0: preStateRoot        (bytes32, static)
///   slot 1: offset_to_blockRlp  (uint256, dynamic pointer → 0x80)
///   slot 2: offset_to_witness   (uint256, dynamic pointer)
///   slot 3: depositsRollingHash (bytes32, static — NOT a pointer)
///   tail:   [block data] [witness data]
fn build_precompile_calldata(
    pre_state_root: H256,
    deposits_rolling_hash: H256,
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

    // slot 3: depositsRollingHash (bytes32, static)
    data.extend_from_slice(deposits_rolling_hash.as_bytes());

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
        deposits_rolling_hash: H256::zero(),
        execution_witness: witness,
        block,
    }
}

/// Build the L2 state transition: processDeposit (relayer→L2Bridge) + transfer (Alice→Bob).
///
/// The L2 genesis includes the L2Bridge predeploy at `L2_BRIDGE` with preminted ETH and
/// a relayer account with gas budget. The block contains two transactions:
///   1. Relayer calls L2Bridge.processDeposit(charlie, 5 ETH, 0)
///   2. Alice sends 1 ETH to Bob
///
/// Returns:
///   - ExecutePrecompileInput (for direct execute_inner calls)
///   - block RLP bytes (for binary calldata / contract call)
///   - witness JSON bytes (for binary calldata / contract call)
///   - pre_state_root
///   - post_state_root
///   - deposits_rolling_hash
fn build_l2_state_transition() -> (ExecutePrecompileInput, Vec<u8>, Vec<u8>, H256, H256, H256) {
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
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

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
    // TX0: relayer → L2Bridge.processDeposit(charlie, 5 ETH, 0)
    let deposit_calldata = encode_process_deposit_call(charlie, deposit_amount, 0);
    let mut tx0 = EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 2_000_000_000,
        gas_limit: 100_000,
        to: TxKind::Call(L2_BRIDGE),
        value: U256::zero(),
        data: Bytes::from(deposit_calldata),
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

    // Execute TX0: processDeposit
    let env0 = Environment {
        origin: relayer,
        gas_limit: 100_000,
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
        "processDeposit transaction failed: {:?}",
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

    // ===== Compute deposits rolling hash =====
    // deposit_hash = keccak256(abi.encodePacked(charlie[20], amount[32], nonce[32]))
    let mut deposit_preimage = Vec::with_capacity(84);
    deposit_preimage.extend_from_slice(charlie.as_bytes()); // 20 bytes
    deposit_preimage.extend_from_slice(&deposit_amount.to_big_endian()); // 32 bytes
    deposit_preimage.extend_from_slice(&U256::zero().to_big_endian()); // nonce=0, 32 bytes
    let deposit_hash = H256::from(keccak_hash(&deposit_preimage));

    // rolling = keccak256(abi.encodePacked(H256::zero(), deposit_hash))
    let mut rolling_preimage = [0u8; 64];
    rolling_preimage[..32].copy_from_slice(H256::zero().as_bytes());
    rolling_preimage[32..].copy_from_slice(deposit_hash.as_bytes());
    let deposits_rolling_hash = H256::from(keccak_hash(rolling_preimage));

    let block_rlp = block.encode_to_vec();
    let witness_json = serde_json::to_vec(&witness).expect("witness JSON serialization failed");

    let input = ExecutePrecompileInput {
        pre_state_root,
        deposits_rolling_hash,
        execution_witness: witness,
        block,
    };

    (
        input,
        block_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
        deposits_rolling_hash,
    )
}

// ===== Unit Tests =====

/// The main test: execute a processDeposit + transfer via the EXECUTE precompile.
///
/// Flow:
///   1. Build L2 genesis with L2Bridge (preminted) + relayer
///   2. L2 block has 2 txs: processDeposit(charlie, 5 ETH) + Alice→Bob 1 ETH
///   3. Call EXECUTE precompile with the block, witness, and deposits rolling hash
///   4. Verify returned state root, block number, and gas used
#[test]
fn test_execute_precompile_transfer_and_deposit() {
    let (_input, block_rlp, witness_json, pre_state_root, post_state_root, deposits_rolling_hash) =
        build_l2_state_transition();

    // Build ABI-encoded calldata and execute via execute_precompile()
    let calldata = build_precompile_calldata(
        pre_state_root,
        deposits_rolling_hash,
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
            assert_eq!(output.len(), 128, "Expected 128-byte ABI-encoded return");
            let returned_root = H256::from_slice(&output[..32]);
            let returned_block_num = U256::from_big_endian(&output[32..64]);
            let returned_withdrawal_root = H256::from_slice(&output[64..96]);
            let returned_gas_used = U256::from_big_endian(&output[96..128]);
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
            println!("EXECUTE precompile succeeded!");
            println!("  Pre-state root:  {pre_state_root:?}");
            println!("  Post-state root: {post_state_root:?}");
            println!("  Relayer processed deposit of 5 ETH for charlie");
            println!("  Alice sent 1 ETH to Bob");
            println!("  Gas used: {returned_gas_used}");
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
const NATIVE_ROLLUP_RUNTIME_HEX: &str = "608060405260043610610089575f3560e01c80639588eca2116100585780639588eca21461023a578063a623f02e14610264578063a7932794146102a0578063ed3133f2146102dc578063f340fa01146103045761017e565b806307132c05146101825780630a045444146101be57806357e871e7146101e65780637b898939146102105761017e565b3661017e575f3411610095576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161008c90610bba565b60405180910390fd5b5f60028054905090505f3334836040516020016100b493929190610c76565b604051602081830303815290604052805190602001209050600281908060018154018082558091505060019003905f5260205f20015f9091909190915055813373ffffffffffffffffffffffffffffffffffffffff167f63746a529035c061c0f4d3d89777a673fe489a291caf751b3a0c40b7c89448a4346040516101399190610cc1565b60405180910390a3005b5f5ffd5b34801561014d575f5ffd5b5061016860048036038101906101639190610d15565b610320565b6040516101759190610d5a565b60405180910390f35b3480156101c9575f5ffd5b506101e460048036038101906101df9190610e28565b61033d565b005b3480156101f1575f5ffd5b506101fa610718565b6040516102079190610cc1565b60405180910390f35b34801561021b575f5ffd5b5061022461071e565b6040516102319190610cc1565b60405180910390f35b348015610245575f5ffd5b5061024e610724565b60405161025b9190610ee1565b60405180910390f35b34801561026f575f5ffd5b5061028a60048036038101906102859190610efa565b610729565b6040516102979190610ee1565b60405180910390f35b3480156102ab575f5ffd5b506102c660048036038101906102c19190610efa565b61073e565b6040516102d39190610ee1565b60405180910390f35b3480156102e7575f5ffd5b5061030260048036038101906102fd9190610f7a565b61075e565b005b61031e6004803603810190610319919061100b565b6109ac565b005b6005602052805f5260405f205f915054906101000a900460ff1681565b60065f9054906101000a900460ff161561038c576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161038390611080565b60405180910390fd5b600160065f6101000a81548160ff0219169083151502179055506001548311156103eb576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016103e2906110e8565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168673ffffffffffffffffffffffffffffffffffffffff1603610459576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161045090611150565b60405180910390fd5b5f851161049b576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610492906111b8565b60405180910390fd5b5f878787876040516020016104b394939291906111d6565b60405160208183030381529060405280519060200120905060055f8281526020019081526020015f205f9054906101000a900460ff1615610529576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016105209061126d565b60405180910390fd5b5f60045f8681526020019081526020015f205490505f5f1b8103610582576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610579906112d5565b60405180910390fd5b5f61058f85858486610a9f565b9050806105d1576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016105c89061133d565b60405180910390fd5b600160055f8581526020019081526020015f205f6101000a81548160ff0219169083151502179055505f8973ffffffffffffffffffffffffffffffffffffffff168960405161061f90611388565b5f6040518083038185875af1925050503d805f8114610659576040519150601f19603f3d011682016040523d82523d5f602084013e61065e565b606091505b50509050806106a2576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610699906113e6565b60405180910390fd5b87878b73ffffffffffffffffffffffffffffffffffffffff167f1113af8a2f367ad0f39a44a9985b12833c5e9dcb54532dd60575fc4ccbd5f9818c6040516106ea9190610cc1565b60405180910390a4505050505f60065f6101000a81548160ff02191690831515021790555050505050505050565b60015481565b60035481565b5f5481565b6004602052805f5260405f205f915090505481565b6002818154811061074d575f80fd5b905f5260205f20015f915090505481565b5f600354905060028054905086826107769190611431565b11156107b7576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016107ae906114ae565b60405180910390fd5b5f5f5f1b90505f5f90505b878110156108295781600282856107d99190611431565b815481106107ea576107e96114cc565b5b905f5260205f200154604051602001610804929190611519565b60405160208183030381529060405280519060200120915080806001019150506107c2565b5086826108369190611431565b6003819055505f5f5487878787866040516020016108599695949392919061159e565b60405160208183030381529060405290505f5f61010173ffffffffffffffffffffffffffffffffffffffff1683604051610893919061163b565b5f604051808303815f865af19150503d805f81146108cc576040519150601f19603f3d011682016040523d82523d5f602084013e6108d1565b606091505b50915091508180156108e4575060808151145b610923576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161091a906116c1565b60405180910390fd5b5f5f5f8380602001905181019061093a9190611707565b925092509250825f81905550816001819055508060045f8481526020019081526020015f2081905550817ff043fac73e6b482de32fb49fc68e40396eba14ca2d2c494f5a795a2dd317c5e88483604051610995929190611757565b60405180910390a250505050505050505050505050565b5f34116109ee576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016109e590610bba565b60405180910390fd5b5f60028054905090505f823483604051602001610a0d93929190610c76565b604051602081830303815290604052805190602001209050600281908060018154018082558091505060019003905f5260205f20015f9091909190915055818373ffffffffffffffffffffffffffffffffffffffff167f63746a529035c061c0f4d3d89777a673fe489a291caf751b3a0c40b7c89448a434604051610a929190610cc1565b60405180910390a3505050565b5f5f8290505f5f90505b86869050811015610ae657610ad782888884818110610acb57610aca6114cc565b5b90506020020135610af5565b91508080600101915050610aa9565b50838114915050949350505050565b5f81831015610b2e578282604051602001610b11929190611519565b604051602081830303815290604052805190602001209050610b5a565b8183604051602001610b41929190611519565b6040516020818303038152906040528051906020012090505b92915050565b5f82825260208201905092915050565b7f4d7573742073656e6420455448000000000000000000000000000000000000005f82015250565b5f610ba4600d83610b60565b9150610baf82610b70565b602082019050919050565b5f6020820190508181035f830152610bd181610b98565b9050919050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610c0182610bd8565b9050919050565b5f8160601b9050919050565b5f610c1e82610c08565b9050919050565b5f610c2f82610c14565b9050919050565b610c47610c4282610bf7565b610c25565b82525050565b5f819050919050565b5f819050919050565b610c70610c6b82610c4d565b610c56565b82525050565b5f610c818286610c36565b601482019150610c918285610c5f565b602082019150610ca18284610c5f565b602082019150819050949350505050565b610cbb81610c4d565b82525050565b5f602082019050610cd45f830184610cb2565b92915050565b5f5ffd5b5f5ffd5b5f819050919050565b610cf481610ce2565b8114610cfe575f5ffd5b50565b5f81359050610d0f81610ceb565b92915050565b5f60208284031215610d2a57610d29610cda565b5b5f610d3784828501610d01565b91505092915050565b5f8115159050919050565b610d5481610d40565b82525050565b5f602082019050610d6d5f830184610d4b565b92915050565b610d7c81610bf7565b8114610d86575f5ffd5b50565b5f81359050610d9781610d73565b92915050565b610da681610c4d565b8114610db0575f5ffd5b50565b5f81359050610dc181610d9d565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f840112610de857610de7610dc7565b5b8235905067ffffffffffffffff811115610e0557610e04610dcb565b5b602083019150836020820283011115610e2157610e20610dcf565b5b9250929050565b5f5f5f5f5f5f5f60c0888a031215610e4357610e42610cda565b5b5f610e508a828b01610d89565b9750506020610e618a828b01610d89565b9650506040610e728a828b01610db3565b9550506060610e838a828b01610db3565b9450506080610e948a828b01610db3565b93505060a088013567ffffffffffffffff811115610eb557610eb4610cde565b5b610ec18a828b01610dd3565b925092505092959891949750929550565b610edb81610ce2565b82525050565b5f602082019050610ef45f830184610ed2565b92915050565b5f60208284031215610f0f57610f0e610cda565b5b5f610f1c84828501610db3565b91505092915050565b5f5f83601f840112610f3a57610f39610dc7565b5b8235905067ffffffffffffffff811115610f5757610f56610dcb565b5b602083019150836001820283011115610f7357610f72610dcf565b5b9250929050565b5f5f5f5f5f60608688031215610f9357610f92610cda565b5b5f610fa088828901610db3565b955050602086013567ffffffffffffffff811115610fc157610fc0610cde565b5b610fcd88828901610f25565b9450945050604086013567ffffffffffffffff811115610ff057610fef610cde565b5b610ffc88828901610f25565b92509250509295509295909350565b5f602082840312156110205761101f610cda565b5b5f61102d84828501610d89565b91505092915050565b7f5265656e7472616e637947756172643a207265656e7472616e742063616c6c005f82015250565b5f61106a601f83610b60565b915061107582611036565b602082019050919050565b5f6020820190508181035f8301526110978161105e565b9050919050565b7f426c6f636b206e6f74207965742066696e616c697a65640000000000000000005f82015250565b5f6110d2601783610b60565b91506110dd8261109e565b602082019050919050565b5f6020820190508181035f8301526110ff816110c6565b9050919050565b7f496e76616c6964207265636569766572000000000000000000000000000000005f82015250565b5f61113a601083610b60565b915061114582611106565b602082019050919050565b5f6020820190508181035f8301526111678161112e565b9050919050565b7f416d6f756e74206d75737420626520706f7369746976650000000000000000005f82015250565b5f6111a2601783610b60565b91506111ad8261116e565b602082019050919050565b5f6020820190508181035f8301526111cf81611196565b9050919050565b5f6111e18287610c36565b6014820191506111f18286610c36565b6014820191506112018285610c5f565b6020820191506112118284610c5f565b60208201915081905095945050505050565b7f5769746864726177616c20616c726561647920636c61696d65640000000000005f82015250565b5f611257601a83610b60565b915061126282611223565b602082019050919050565b5f6020820190508181035f8301526112848161124b565b9050919050565b7f4e6f207769746864726177616c7320666f72207468697320626c6f636b0000005f82015250565b5f6112bf601d83610b60565b91506112ca8261128b565b602082019050919050565b5f6020820190508181035f8301526112ec816112b3565b9050919050565b7f496e76616c6964204d65726b6c652070726f6f660000000000000000000000005f82015250565b5f611327601483610b60565b9150611332826112f3565b602082019050919050565b5f6020820190508181035f8301526113548161131b565b9050919050565b5f81905092915050565b50565b5f6113735f8361135b565b915061137e82611365565b5f82019050919050565b5f61139282611368565b9150819050919050565b7f455448207472616e73666572206661696c6564000000000000000000000000005f82015250565b5f6113d0601383610b60565b91506113db8261139c565b602082019050919050565b5f6020820190508181035f8301526113fd816113c4565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61143b82610c4d565b915061144683610c4d565b925082820190508082111561145e5761145d611404565b5b92915050565b7f4e6f7420656e6f756768206465706f73697473000000000000000000000000005f82015250565b5f611498601383610b60565b91506114a382611464565b602082019050919050565b5f6020820190508181035f8301526114c58161148c565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f819050919050565b61151361150e82610ce2565b6114f9565b82525050565b5f6115248285611502565b6020820191506115348284611502565b6020820191508190509392505050565b5f82825260208201905092915050565b828183375f83830152505050565b5f601f19601f8301169050919050565b5f61157d8385611544565b935061158a838584611554565b61159383611562565b840190509392505050565b5f6080820190506115b15f830189610ed2565b81810360208301526115c4818789611572565b905081810360408301526115d9818587611572565b90506115e86060830184610ed2565b979650505050505050565b5f81519050919050565b8281835e5f83830152505050565b5f611615826115f3565b61161f818561135b565b935061162f8185602086016115fd565b80840191505092915050565b5f611646828461160b565b915081905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f6116ab602683610b60565b91506116b682611651565b604082019050919050565b5f6020820190508181035f8301526116d88161169f565b9050919050565b5f815190506116ed81610ceb565b92915050565b5f8151905061170181610d9d565b92915050565b5f5f5f6060848603121561171e5761171d610cda565b5b5f61172b868287016116df565b935050602061173c868287016116f3565b925050604061174d868287016116df565b9150509250925092565b5f60408201905061176a5f830185610ed2565b6117776020830184610ed2565b939250505056fea26469706673582212207de8510038ade9cf3da10d5f2994778bcf509b304aae11bc8ed0afd3efb9b7b264736f6c634300081f0033";

/// L2Bridge.sol runtime bytecode (compiled with solc 0.8.31).
///
/// Source: crates/vm/levm/contracts/L2Bridge.sol
/// Compile: cd crates/vm/levm/contracts && solc --bin-runtime L2Bridge.sol -o solc_out --overwrite
const L2_BRIDGE_RUNTIME_HEX: &str = "608060405260043610610049575f3560e01c806351cff8d91461004d5780637b259db4146100695780638406c07914610093578063a95ecfec146100bd578063de35f5cb146100e5575b5f5ffd5b6100676004803603810190610062919061051f565b61010f565b005b348015610074575f5ffd5b5061007d6102eb565b60405161008a9190610562565b60405180910390f35b34801561009e575f5ffd5b506100a76102f1565b6040516100b4919061058a565b60405180910390f35b3480156100c8575f5ffd5b506100e360048036038101906100de91906105cd565b610315565b005b3480156100f0575f5ffd5b506100f96104bb565b6040516101069190610562565b60405180910390f35b5f3411610151576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101489061069d565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168173ffffffffffffffffffffffffffffffffffffffff16036101bf576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101b690610705565b60405180910390fd5b5f60025490506001816101d29190610750565b6002819055505f5f73ffffffffffffffffffffffffffffffffffffffff16346040516101fd906107b0565b5f6040518083038185875af1925050503d805f8114610237576040519150601f19603f3d011682016040523d82523d5f602084013e61023c565b606091505b5050905080610280576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016102779061080e565b60405180910390fd5b818373ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff167f2e16c360bf25f9193c8e78b0fcdf02bacfd34fd98ec9fe4aa2549e15346dafd2346040516102de9190610562565b60405180910390a4505050565b60025481565b5f5f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1681565b5f5f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff163373ffffffffffffffffffffffffffffffffffffffff16146103a3576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161039a90610876565b60405180910390fd5b60015481146103e7576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016103de906108de565b60405180910390fd5b5f60015490506001816103fa9190610750565b6001819055508373ffffffffffffffffffffffffffffffffffffffff1683604051610424906107b0565b5f6040518083038185875af1925050503d805f811461045e576040519150601f19603f3d011682016040523d82523d5f602084013e610463565b606091505b505050808473ffffffffffffffffffffffffffffffffffffffff167f782ea2005b7f873a0190a1ccb553c732171348d6e16d2b7d3d493c0677dc296e856040516104ad9190610562565b60405180910390a350505050565b60015481565b5f5ffd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6104ee826104c5565b9050919050565b6104fe816104e4565b8114610508575f5ffd5b50565b5f81359050610519816104f5565b92915050565b5f60208284031215610534576105336104c1565b5b5f6105418482850161050b565b91505092915050565b5f819050919050565b61055c8161054a565b82525050565b5f6020820190506105755f830184610553565b92915050565b610584816104e4565b82525050565b5f60208201905061059d5f83018461057b565b92915050565b6105ac8161054a565b81146105b6575f5ffd5b50565b5f813590506105c7816105a3565b92915050565b5f5f5f606084860312156105e4576105e36104c1565b5b5f6105f18682870161050b565b9350506020610602868287016105b9565b9250506040610613868287016105b9565b9150509250925092565b5f82825260208201905092915050565b7f5769746864726177616c20616d6f756e74206d75737420626520706f736974695f8201527f7665000000000000000000000000000000000000000000000000000000000000602082015250565b5f61068760228361061d565b91506106928261062d565b604082019050919050565b5f6020820190508181035f8301526106b48161067b565b9050919050565b7f496e76616c6964207265636569766572000000000000000000000000000000005f82015250565b5f6106ef60108361061d565b91506106fa826106bb565b602082019050919050565b5f6020820190508181035f83015261071c816106e3565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f61075a8261054a565b91506107658361054a565b925082820190508082111561077d5761077c610723565b5b92915050565b5f81905092915050565b50565b5f61079b5f83610783565b91506107a68261078d565b5f82019050919050565b5f6107ba82610790565b9150819050919050565b7f4661696c656420746f206275726e2045746865720000000000000000000000005f82015250565b5f6107f860148361061d565b9150610803826107c4565b602082019050919050565b5f6020820190508181035f830152610825816107ec565b9050919050565b7f4c324272696467653a206e6f742072656c6179657200000000000000000000005f82015250565b5f61086060158361061d565b915061086b8261082c565b602082019050919050565b5f6020820190508181035f83015261088d81610854565b9050919050565b7f4c324272696467653a206e6f6e6365206d69736d6174636800000000000000005f82015250565b5f6108c860188361061d565b91506108d382610894565b602082019050919050565b5f6020820190508181035f8301526108f5816108bc565b905091905056fea264697066735822122027ed27d5e9cfda6ce920005ed4fa610e53c4f07e03cfc68e8e0b901d82bed8be64736f6c634300081f0033";

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
///   1. deposit(charlie) with 5 ETH → records pending deposit hash
///   2. advance(1, blockRlp, witnessJson)
///      → NativeRollup computes rolling hash → builds EXECUTE calldata → CALL to 0x0101
///      → precompile re-executes L2 block → verifies state roots + deposits rolling hash
///      → success → contract updates stateRoot, blockNumber, depositIndex
#[test]
fn test_native_rollup_contract() {
    let (_input, block_rlp, witness_json, pre_state_root, post_state_root, _deposits_rolling_hash) =
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

    let l1_chain_config = test_chain_config();

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
