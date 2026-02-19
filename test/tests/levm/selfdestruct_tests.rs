//! Tests exposing a FlatKeyValue bug for pre-Cancun SELFDESTRUCT + CREATE2 recreate.
//!
//! Both tests run the exact same scenario through the full Store + Blockchain
//! pipeline: a contract with storage is SELFDESTRUCTed and then recreated at
//! the same address via CREATE2.
//!
//! - `without_fkv`: queries storage via the trie path → old storage correctly gone.
//! - `with_fkv`:    triggers FKV generation first → stale genesis storage returned.

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    fork_choice::apply_fork_choice,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::constants::EMPTY_KECCACK_HASH;
use ethrex_common::types::{
    ChainConfig, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis,
    GenesisAccount, Transaction, TxKind,
};
use ethrex_common::{Address, H256, U256};
use ethrex_rlp::structs::Encoder;
use ethrex_storage::{EngineType, Store};
use std::collections::BTreeMap;

// ==================== Helpers ====================

const GAS_LIMIT: u64 = 500_000;
const BLOCK_GAS_LIMIT: u64 = 30_000_000;
const BASE_FEE: u64 = 1000;

fn shanghai_chain_config() -> ChainConfig {
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
        merge_netsplit_block: Some(0),
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        shanghai_time: Some(0),
        cancun_time: None,
        deposit_contract_address: Address::zero(),
        ..Default::default()
    }
}

fn address_from_private_key(private_key: &[u8; 32]) -> Address {
    let secp = secp256k1::Secp256k1::new();
    let secret = secp256k1::SecretKey::from_slice(private_key).expect("valid secret key");
    let public = secp256k1::PublicKey::from_secret_key(&secp, &secret);
    let hash = ethrex_crypto::keccak::keccak_hash(&public.serialize_uncompressed()[1..]);
    Address::from_slice(&hash[12..])
}

fn sign_eip1559(tx: &mut EIP1559Transaction, private_key: &[u8; 32]) {
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

    let hash = ethrex_common::utils::keccak(&buf);
    let secp = secp256k1::Secp256k1::new();
    let secret = secp256k1::SecretKey::from_slice(private_key).expect("valid secret key");
    let msg = secp256k1::Message::from_digest(hash.to_fixed_bytes());
    let sig = secp.sign_ecdsa_recoverable(&msg, &secret);
    let (recovery_id, sig_bytes) = sig.serialize_compact();

    tx.signature_r = U256::from_big_endian(&sig_bytes[..32]);
    tx.signature_s = U256::from_big_endian(&sig_bytes[32..64]);
    tx.signature_y_parity = Into::<i32>::into(recovery_id) != 0;
}

fn build_call_tx(nonce: u64, to: Address, private_key: &[u8; 32]) -> Transaction {
    let mut tx = EIP1559Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: BASE_FEE,
        gas_limit: GAS_LIMIT,
        to: TxKind::Call(to),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: vec![],
        ..Default::default()
    };
    sign_eip1559(&mut tx, private_key);
    Transaction::EIP1559Transaction(tx)
}

fn compute_create2_address(deployer: Address, init_code: &[u8], salt: U256) -> Address {
    let init_code_hash = ethrex_common::utils::keccak(init_code);
    let mut buf = Vec::with_capacity(1 + 20 + 32 + 32);
    buf.push(0xff);
    buf.extend_from_slice(deployer.as_bytes());
    buf.extend_from_slice(&salt.to_big_endian());
    buf.extend_from_slice(init_code_hash.as_bytes());
    Address::from_slice(&ethrex_common::utils::keccak(&buf)[12..])
}

fn selfdestruct_bytecode(beneficiary: Address) -> Bytes {
    let mut code = Vec::new();
    code.push(0x73); // PUSH20
    code.extend_from_slice(beneficiary.as_bytes());
    code.push(0xff); // SELFDESTRUCT
    Bytes::from(code)
}

fn create2_factory_bytecode(init_code: &[u8], salt: U256) -> Bytes {
    let mut code = Vec::new();
    for (i, byte) in init_code.iter().enumerate() {
        code.extend_from_slice(&[0x60, *byte]); // PUSH1 byte
        code.extend_from_slice(&[0x60, i as u8]); // PUSH1 offset
        code.push(0x53); // MSTORE8
    }
    code.push(0x7f); // PUSH32 salt
    code.extend_from_slice(&salt.to_big_endian());
    code.extend_from_slice(&[0x60, init_code.len() as u8]); // PUSH1 size
    code.extend_from_slice(&[0x60, 0x00]); // PUSH1 offset=0
    code.extend_from_slice(&[0x60, 0x00]); // PUSH1 value=0
    code.push(0xf5); // CREATE2
    code.push(0x50); // POP result
    code.push(0x00); // STOP
    Bytes::from(code)
}

fn simple_init_code() -> Vec<u8> {
    vec![
        0x60, 0x00, // PUSH1 0x00 (STOP opcode)
        0x60, 0x00, // PUSH1 0x00 (memory offset)
        0x53, // MSTORE8
        0x60, 0x01, // PUSH1 0x01 (return size)
        0x60, 0x00, // PUSH1 0x00 (return offset)
        0xf3, // RETURN
    ]
}

/// Common setup: genesis → mempool → build_payload → add_block → fork choice.
/// Returns (store, block_number=1).
async fn run_selfdestruct_then_recreate_scenario() -> Store {
    let private_key: [u8; 32] = [1u8; 32];
    let sender = address_from_private_key(&private_key);
    let beneficiary = Address::from_low_u64_be(0x3000);
    let factory_addr = Address::from_low_u64_be(0x2000);
    let init_code = simple_init_code();
    let salt = U256::zero();
    let target_addr = compute_create2_address(factory_addr, &init_code, salt);

    let mut alloc = BTreeMap::new();
    alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10u64.pow(18)),
            nonce: 0,
            code: Bytes::new(),
            storage: Default::default(),
        },
    );
    alloc.insert(
        target_addr,
        GenesisAccount {
            balance: U256::from(1000),
            nonce: 1,
            code: selfdestruct_bytecode(beneficiary),
            storage: BTreeMap::from([(U256::from(1), U256::from(42))]),
        },
    );
    alloc.insert(
        factory_addr,
        GenesisAccount {
            balance: U256::zero(),
            nonce: 1,
            code: create2_factory_bytecode(&init_code, salt),
            storage: Default::default(),
        },
    );
    alloc.insert(
        beneficiary,
        GenesisAccount {
            balance: U256::zero(),
            nonce: 0,
            code: Bytes::new(),
            storage: Default::default(),
        },
    );

    let genesis = Genesis {
        config: shanghai_chain_config(),
        alloc,
        gas_limit: BLOCK_GAS_LIMIT,
        base_fee_per_gas: Some(BASE_FEE),
        difficulty: U256::zero(),
        ..Default::default()
    };

    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());

    let tx0 = build_call_tx(0, target_addr, &private_key);
    let tx1 = build_call_tx(1, factory_addr, &private_key);
    blockchain
        .add_transaction_to_pool(tx0)
        .await
        .expect("tx0 pool add");
    blockchain
        .add_transaction_to_pool(tx1)
        .await
        .expect("tx1 pool add");

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: Address::zero(),
        random: H256::zero(),
        withdrawals: Some(vec![]),
        beacon_root: None,
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, &store, Bytes::new()).expect("create_payload");
    let result = blockchain.build_payload(payload).expect("build_payload");
    let block = result.payload;
    assert_eq!(block.body.transactions.len(), 2);

    blockchain
        .add_block(block.clone())
        .expect("add_block should succeed");

    let block_hash = block.hash();
    apply_fork_choice(&store, block_hash, block_hash, block_hash)
        .await
        .expect("fork choice");

    store
}

// ==================== Tests ====================

/// Without FKV generation the trie path correctly shows the old storage is gone.
#[tokio::test]
async fn pre_cancun_selfdestruct_then_recreate_clears_storage_without_fkv() {
    let store = run_selfdestruct_then_recreate_scenario().await;

    let init_code = simple_init_code();
    let salt = U256::zero();
    let factory_addr = Address::from_low_u64_be(0x2000);
    let target_addr = compute_create2_address(factory_addr, &init_code, salt);
    let storage_key = H256::from_low_u64_be(1);

    let target_info = store
        .get_account_info(1, target_addr)
        .await
        .expect("get_account_info")
        .expect("target should exist (recreated)");
    assert_ne!(target_info.code_hash, *EMPTY_KECCACK_HASH);

    let old_storage = store
        .get_storage_at(1, target_addr, storage_key)
        .expect("get_storage_at");
    assert!(
        old_storage.is_none() || old_storage == Some(U256::zero()),
        "Old storage (42) must be gone after selfdestruct + recreate, got: {old_storage:?}"
    );
}

/// After FKV generation the same query returns stale genesis storage.
#[tokio::test]
async fn pre_cancun_selfdestruct_then_recreate_clears_storage_with_fkv() {
    let store = run_selfdestruct_then_recreate_scenario().await;

    let init_code = simple_init_code();
    let salt = U256::zero();
    let factory_addr = Address::from_low_u64_be(0x2000);
    let target_addr = compute_create2_address(factory_addr, &init_code, salt);
    let storage_key = H256::from_low_u64_be(1);

    // Trigger FKV generation and wait for it to finish on this tiny state.
    store
        .generate_flatkeyvalue()
        .expect("generate_flatkeyvalue");
    std::thread::sleep(std::time::Duration::from_millis(500));

    let target_info = store
        .get_account_info(1, target_addr)
        .await
        .expect("get_account_info")
        .expect("target should exist (recreated)");
    assert_ne!(target_info.code_hash, *EMPTY_KECCACK_HASH);

    let old_storage = store
        .get_storage_at(1, target_addr, storage_key)
        .expect("get_storage_at");
    assert!(
        old_storage.is_none() || old_storage == Some(U256::zero()),
        "Old storage (42) must be gone after selfdestruct + recreate, got: {old_storage:?}"
    );
}
