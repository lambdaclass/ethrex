//! Integration test for the NativeRollup contract on a real L1.
//!
//! Requires:
//!   1. Start L1: `NATIVE_ROLLUPS=1 make -C crates/l2 init-l1`
//!   2. Run: `cargo test -p ethrex-test --features native-rollups -- l2::native_rollups --nocapture`
//!
//! The test compiles and deploys NativeRollup.sol, builds an L2 state transition,
//! calls deposit() + advance(), and verifies the contract state was updated.

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
        AccountState, Block, BlockBody, BlockHeader, ChainConfig, EIP1559Transaction, Receipt,
        Transaction, TxKind, TxType,
        block_execution_witness::{ExecutionWitness, GuestProgramState},
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
    execute_precompile::{Deposit, ExecutePrecompileInput, L2_WITHDRAWAL_BRIDGE},
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

use super::utils::workspace_root;

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

/// Read the L2WithdrawalBridge compiled runtime bytecode from solc output.
fn bridge_runtime_bytecode() -> Vec<u8> {
    let path =
        workspace_root().join("crates/vm/levm/contracts/solc_out/L2WithdrawalBridge.bin-runtime");
    let hex_str = std::fs::read_to_string(&path)
        .expect("L2WithdrawalBridge.bin-runtime not found — compile with solc first");
    hex::decode(hex_str.trim()).expect("invalid hex in bridge .bin-runtime")
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

/// Build the L2 state transition (Alice→Bob transfer + Charlie deposit).
///
/// The L2 genesis state includes the L2WithdrawalBridge contract at `L2_WITHDRAWAL_BRIDGE`,
/// so that block 2 (withdrawal) can build on top of this state.
fn build_l2_state_transition() -> (ExecutePrecompileInput, Vec<u8>, Vec<u8>, H256, H256) {
    let alice_key = SigningKey::from_bytes(&[1u8; 32].into()).expect("valid key");
    let alice = address_from_key(&alice_key);
    let bob = Address::from_low_u64_be(0xB0B);
    let charlie = Address::from_low_u64_be(0xC4A);
    let coinbase = Address::from_low_u64_be(0xC01);
    let chain_id: u64 = 1;
    let base_fee: u64 = 1_000_000_000;

    let bridge_code_hash = H256(keccak_hash(&bridge_runtime_bytecode()));

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
    // address(0) is the burn address used by L2WithdrawalBridge.withdraw()
    insert_account(&mut state_trie, Address::zero(), &AccountState::default());
    insert_account(
        &mut state_trie,
        L2_WITHDRAWAL_BRIDGE,
        &AccountState {
            nonce: 1,
            code_hash: bridge_code_hash,
            ..Default::default()
        },
    );
    let pre_state_root = state_trie.hash_no_commit();

    let parent_header = BlockHeader {
        number: 0,
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_000,
        ..Default::default()
    };

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

    let gas_used: u64 = 21_000;
    let effective_gas_price: u64 = std::cmp::min(1_000_000_000 + base_fee, 2_000_000_000);
    let gas_cost = U256::from(gas_used) * U256::from(effective_gas_price);
    let priority_fee_per_gas: u64 = effective_gas_price.saturating_sub(base_fee);
    let coinbase_reward = U256::from(gas_used) * U256::from(priority_fee_per_gas);

    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);
    let receipt = Receipt::new(transaction.tx_type(), true, gas_used, vec![]);
    let receipts_root = ethrex_common::types::compute_receipts_root(&[receipt]);

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
    // Burn address unchanged from genesis (not accessed in block 1)
    insert_account(&mut post_trie, Address::zero(), &AccountState::default());
    // Bridge unchanged from genesis (not accessed in block 1)
    insert_account(
        &mut post_trie,
        L2_WITHDRAWAL_BRIDGE,
        &AccountState {
            nonce: 1,
            code_hash: bridge_code_hash,
            ..Default::default()
        },
    );
    let post_state_root = post_trie.hash_no_commit();

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

    let chain_config = test_chain_config();

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

/// Build an L2 block containing a withdrawal transaction (block 2).
///
/// Executes Alice → L2WithdrawalBridge.withdraw(receiver) with `withdrawal_amount` ETH.
/// Uses LEVM to compute exact gas_used and post-state root for the block.
///
/// Returns (block2_rlp, witness2_json, block2_post_state_root).
fn build_l2_withdrawal_block(
    block1: &Block,
    block1_post_state_root: H256,
    withdrawal_receiver: Address,
) -> (Vec<u8>, Vec<u8>, H256) {
    let alice_key = SigningKey::from_bytes(&[1u8; 32].into()).expect("valid key");
    let alice = address_from_key(&alice_key);
    let bob = Address::from_low_u64_be(0xB0B);
    let charlie = Address::from_low_u64_be(0xC4A);
    let coinbase = Address::from_low_u64_be(0xC01);
    let base_fee: u64 = 1_000_000_000;

    let bridge_runtime = bridge_runtime_bytecode();
    let bridge_code_hash = H256(keccak_hash(&bridge_runtime));

    // Reconstruct block 1 post-state balances (must match build_l2_state_transition)
    let alice_initial = U256::from(10) * U256::from(10).pow(U256::from(18));
    let transfer_value = U256::from(10).pow(U256::from(18));
    let gas_cost_b1 = U256::from(21_000u64) * U256::from(2_000_000_000u64);
    let coinbase_reward_b1 = U256::from(21_000u64) * U256::from(1_000_000_000u64);
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18));

    // Build block 2 pre-state trie (= block 1 post-state including bridge)
    let mut pre_trie = Trie::new_temp();
    insert_account(
        &mut pre_trie,
        alice,
        &AccountState {
            nonce: 1,
            balance: alice_initial - transfer_value - gas_cost_b1,
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
            balance: coinbase_reward_b1,
            ..Default::default()
        },
    );
    insert_account(
        &mut pre_trie,
        charlie,
        &AccountState {
            balance: deposit_amount,
            ..Default::default()
        },
    );
    // Burn address (matches block 1 post-state)
    insert_account(&mut pre_trie, Address::zero(), &AccountState::default());
    insert_account(
        &mut pre_trie,
        L2_WITHDRAWAL_BRIDGE,
        &AccountState {
            nonce: 1,
            code_hash: bridge_code_hash,
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
        to: TxKind::Call(L2_WITHDRAWAL_BRIDGE),
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
        codes: vec![bridge_runtime.clone()],
        block_headers_bytes: vec![block1.header.encode_to_vec(), temp_header.encode_to_vec()],
        first_block_number: block2_number,
        chain_config: chain_config.clone(),
        state_trie_root: get_trie_root_node(&pre_trie),
        storage_trie_roots: BTreeMap::new(),
        keys: vec![],
    };

    let guest_state: GuestProgramState = temp_witness
        .try_into()
        .expect("Failed to build GuestProgramState");

    let db_inner = Arc::new(GuestProgramStateDb::new(guest_state));
    let db_dyn: Arc<dyn ethrex_levm::db::Database> = db_inner.clone();
    let mut gen_db = GeneralizedDatabase::new(db_dyn);

    let config = EVMConfig::new_from_chain_config(&chain_config, &temp_header);
    let gas_price = U256::from(std::cmp::min(1_000_000_000u64 + base_fee, 2_000_000_000u64));

    let env = Environment {
        origin: alice,
        gas_limit,
        config,
        block_number: block2_number.into(),
        coinbase,
        timestamp: block2_timestamp.into(),
        prev_randao: Some(temp_header.prev_randao),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(base_fee),
        base_blob_fee_per_gas: U256::zero(),
        gas_price,
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: Some(U256::from(1_000_000_000u64)),
        tx_max_fee_per_gas: Some(U256::from(2_000_000_000u64)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 1,
        block_gas_limit: 30_000_000,
        difficulty: U256::zero(),
        is_privileged: false,
        fee_token: None,
    };

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

    // Build final block 2 with correct gas_used and state_root
    let transactions = vec![transaction.clone()];
    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);
    let receipt = Receipt::new(transaction.tx_type(), true, gas_used, report.logs);
    let receipts_root = ethrex_common::types::compute_receipts_root(&[receipt]);

    let block2 = Block {
        header: BlockHeader {
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
        },
        body: BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    // Build final witness with correct block 2 header
    let witness2 = ExecutionWitness {
        codes: vec![bridge_runtime],
        block_headers_bytes: vec![block1.header.encode_to_vec(), block2.header.encode_to_vec()],
        first_block_number: block2_number,
        chain_config,
        state_trie_root: get_trie_root_node(&pre_trie),
        storage_trie_roots: BTreeMap::new(),
        keys: vec![],
    };

    let block2_rlp = block2.encode_to_vec();
    let witness2_json = serde_json::to_vec(&witness2).expect("witness JSON serialization failed");

    (block2_rlp, witness2_json, post_state_root)
}

/// Integration test: compile and deploy NativeRollup on a real L1, then advance
/// with two L2 blocks — block 1 (transfer + deposit) and block 2 (withdrawal).
///
/// Flow:
///   1. Deploy NativeRollup contract
///   2. deposit(charlie) with 5 ETH
///   3. advance(block 1) — transfer + deposit
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
        &contracts_path.join("L2WithdrawalBridge.sol"),
        true, // runtime bytecode needed for L2 genesis state
        false,
        None,
        &[],
        None,
    )
    .expect("Failed to compile L2WithdrawalBridge.sol");
    compile_contract(
        &contracts_path,
        &contracts_path.join("NativeRollup.sol"),
        false,
        false,
        None,
        &[],
        None,
    )
    .expect("Failed to compile NativeRollup.sol");

    // 3. Build L2 state transitions
    let (input, block_rlp, witness_json, pre_state_root, post_state_root) =
        build_l2_state_transition();
    let charlie = Address::from_low_u64_be(0xC4A);
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

    // The L1 receiver of the withdrawal (fresh address for easy balance verification)
    let l1_withdrawal_receiver = Address::from_low_u64_be(0xDEAD);

    // Build block 2 (withdrawal block)
    let (block2_rlp, witness2_json, block2_post_state_root) =
        build_l2_withdrawal_block(&input.block, post_state_root, l1_withdrawal_receiver);
    let alice_l2 = address_from_key(&SigningKey::from_bytes(&[1u8; 32].into()).unwrap());
    let withdrawal_amount = U256::from(10).pow(U256::from(18)); // 1 ETH

    // 4. Deploy NativeRollup
    let deploy_hex = std::fs::read_to_string(contracts_path.join("solc_out/NativeRollup.bin"))
        .expect("Failed to read compiled contract");
    let deploy_bytecode = hex::decode(deploy_hex.trim()).expect("invalid hex in .bin file");
    let constructor_arg = pre_state_root.as_bytes().to_vec();
    let init_code: Bytes = [deploy_bytecode, constructor_arg].concat().into();

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

    // 6. Call deposit(charlie) with 5 ETH
    let deposit_calldata =
        encode_calldata("deposit(address)", &[Value::Address(charlie)]).expect("encode failed");

    let deposit_tx = build_generic_tx(
        &eth_client,
        TxType::EIP1559,
        contract_address,
        signer.address(),
        Bytes::from(deposit_calldata),
        Overrides {
            value: Some(deposit_amount),
            ..Default::default()
        },
    )
    .await
    .expect("Failed to build deposit tx");

    let deposit_tx_hash = send_generic_transaction(&eth_client, deposit_tx, &signer)
        .await
        .expect("Failed to send deposit tx");

    let deposit_receipt = wait_for_transaction_receipt(deposit_tx_hash, &eth_client, 30)
        .await
        .expect("Deposit receipt not found");
    assert!(
        deposit_receipt.receipt.status,
        "NativeRollup.deposit() reverted!"
    );
    println!("  deposit() tx: {deposit_tx_hash:?}");

    // 7. Call advance(1, block_rlp, witness_json)
    let advance_calldata = encode_calldata(
        "advance(uint256,bytes,bytes)",
        &[
            Value::Uint(U256::from(1)),
            Value::Bytes(Bytes::from(block_rlp)),
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

    let stored_deposit_index = eth_client
        .get_storage_at(
            contract_address,
            U256::from(3),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(stored_deposit_index, U256::from(1), "depositIndex mismatch");

    // 9. Verify withdrawalRoots[1] was stored (zero since no withdrawals in this block)
    // Storage slot for mapping(uint256 => bytes32) at slot 4, key 1:
    //   keccak256(abi.encode(uint256(1), uint256(4)))
    let mut slot_preimage = [0u8; 64];
    slot_preimage[31] = 1; // key = 1
    slot_preimage[63] = 4; // mapping base slot = 4
    let withdrawal_roots_slot = U256::from_big_endian(&keccak_hash(&slot_preimage));

    let stored_withdrawal_root = eth_client
        .get_storage_at(
            contract_address,
            withdrawal_roots_slot,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    // No withdrawal events in this L2 block, so root should be zero
    assert_eq!(
        stored_withdrawal_root,
        U256::zero(),
        "withdrawalRoots[1] should be zero (no withdrawals)"
    );

    println!("\n  Phase 1 passed: transfer + deposit");
    println!("  Pre-state root:  {pre_state_root:?}");
    println!("  Post-state root: {post_state_root:?}");
    println!("  Block number:    1");
    println!("  Deposit index:   1");

    // ===== Phase 2: Withdrawal =====
    // Advance with block 2 (contains Alice → bridge.withdraw(l1_receiver) with 1 ETH)

    // 10. advance(0, block2_rlp, witness2_json) — 0 deposits for block 2
    let advance2_calldata = encode_calldata(
        "advance(uint256,bytes,bytes)",
        &[
            Value::Uint(U256::from(0)), // no deposits consumed
            Value::Bytes(Bytes::from(block2_rlp)),
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

    // 12. Verify withdrawalRoots[2] is non-zero (withdrawal was included)
    let mut slot_preimage_b2 = [0u8; 64];
    slot_preimage_b2[31] = 2; // key = 2 (block number)
    slot_preimage_b2[63] = 4; // mapping base slot = 4
    let withdrawal_roots_slot_b2 = U256::from_big_endian(&keccak_hash(&slot_preimage_b2));

    let stored_withdrawal_root_b2 = eth_client
        .get_storage_at(
            contract_address,
            withdrawal_roots_slot_b2,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_ne!(
        stored_withdrawal_root_b2,
        U256::zero(),
        "withdrawalRoots[2] should be non-zero (withdrawal included)"
    );
    println!("  withdrawalRoots[2]: {stored_withdrawal_root_b2:?}");

    // 13. Check l1_withdrawal_receiver balance before claim
    let receiver_balance_before = eth_client
        .get_balance(
            l1_withdrawal_receiver,
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_balance failed");
    println!("  Receiver balance before claim: {receiver_balance_before}");

    // 14. Call claimWithdrawal(from, receiver, amount, messageId, blockNumber, proof)
    // Single withdrawal → Merkle root = leaf hash, proof is empty
    let claim_calldata = encode_calldata(
        "claimWithdrawal(address,address,uint256,uint256,uint256,bytes32[])",
        &[
            Value::Address(alice_l2),               // _from (L2 sender)
            Value::Address(l1_withdrawal_receiver), // _receiver
            Value::Uint(withdrawal_amount),         // _amount (1 ETH)
            Value::Uint(U256::zero()),              // _messageId (first withdrawal)
            Value::Uint(U256::from(2)),             // _blockNumber (L2 block 2)
            Value::Array(vec![]),                   // _merkleProof (empty for single withdrawal)
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

    println!("\nNativeRollup integration test passed (transfer + deposit + withdrawal + claim)!");
    println!("  Contract:        {contract_address:?}");
    println!("  L2 blocks:       2");
    println!("  Deposit:         5 ETH to charlie");
    println!("  Withdrawal:      1 ETH from alice to {l1_withdrawal_receiver:?}");
}
