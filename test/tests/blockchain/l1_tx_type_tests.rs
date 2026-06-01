//! Regression test for the `l1-privileged-transaction` finding: an L1 chain must
//! reject a block containing an L2-only transaction type (0x7e
//! `PrivilegedL2Transaction` / 0x7d `FeeTokenTransaction`). Both are L2-only types
//! unknown to other L1 clients, so accepting one on L1 diverges consensus; the
//! privileged type additionally takes its sender from an unsigned, caller-chosen
//! `from`, so it would also let a block forge a sender.
use std::collections::BTreeMap;

use ethrex_blockchain::Blockchain;
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, ChainConfig, ELASTICITY_MULTIPLIER, Genesis, GenesisAccount,
    PrivilegedL2Transaction, Receipt, Transaction, TxKind, TxType, calculate_base_fee_per_gas,
    compute_receipts_root, compute_transactions_root,
};
use ethrex_common::{Address, Bloom, Bytes, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

#[tokio::test]
async fn l1_rejects_privileged_l2_transaction_block() {
    let sender = Address::from_low_u64_be(0x100);
    let recipient = Address::from_low_u64_be(0x200);

    let mut alloc = BTreeMap::new();
    alloc.insert(
        sender,
        GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance: U256::one(),
            nonce: 0,
        },
    );

    let genesis = Genesis {
        config: ChainConfig {
            chain_id: 9,
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
            ..Default::default()
        },
        alloc,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(0),
        ..Default::default()
    };

    let parent = genesis.get_block().header.clone();
    let parent_hash = parent.hash();
    let mut store = Store::new("", EngineType::InMemory).expect("open in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("initialize genesis");

    let tx = Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction {
        chain_id: 9,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 21_000,
        to: TxKind::Call(recipient),
        value: U256::one(),
        data: Bytes::new(),
        access_list: Vec::new(),
        from: sender,
        ..Default::default()
    });
    // Round-trip through the canonical decoder, exactly as L1 block import does.
    let decoded_tx = Transaction::decode_canonical(&tx.encode_canonical_to_vec())
        .expect("decode canonical type-0x7e transaction");
    assert!(matches!(decoded_tx, Transaction::PrivilegedL2Transaction(_)));

    let body = BlockBody {
        transactions: vec![decoded_tx],
        ommers: Vec::new(),
        withdrawals: None,
    };

    // Post-state consistent with ethrex executing the privileged tx, so the block
    // passes the root checks and reaches the transaction-type acceptance path.
    let mut final_alloc = BTreeMap::new();
    final_alloc.insert(
        sender,
        GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );
    final_alloc.insert(
        recipient,
        GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance: U256::one(),
            nonce: 0,
        },
    );
    let final_state = Genesis {
        alloc: final_alloc,
        ..Default::default()
    };

    let base_fee_per_gas = calculate_base_fee_per_gas(
        parent.gas_limit,
        parent.gas_limit,
        parent.gas_used,
        parent.base_fee_per_gas.unwrap_or_default(),
        ELASTICITY_MULTIPLIER,
    )
    .expect("calculate child base fee");
    let receipts = [Receipt::new(TxType::Privileged, true, 21_000, Vec::new())];

    let header = BlockHeader {
        parent_hash,
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: Address::zero(),
        state_root: final_state.compute_state_root(),
        transactions_root: compute_transactions_root(&body.transactions, &NativeCrypto),
        receipts_root: compute_receipts_root(&receipts, &NativeCrypto),
        logs_bloom: Bloom::zero(),
        difficulty: U256::zero(),
        number: parent.number + 1,
        gas_limit: parent.gas_limit,
        gas_used: 21_000,
        timestamp: parent.timestamp + 1,
        extra_data: Bytes::new(),
        prev_randao: H256::zero(),
        nonce: 0,
        base_fee_per_gas: Some(base_fee_per_gas),
        withdrawals_root: None,
        blob_gas_used: None,
        excess_blob_gas: None,
        parent_beacon_block_root: None,
        requests_hash: None,
        block_access_list_hash: None,
        slot_number: None,
        ..Default::default()
    };

    let block = Block::new(header, body);
    let blockchain = Blockchain::default_with_store(store);

    let result = blockchain.add_block(block);
    assert!(
        result.is_err(),
        "L1 must reject a block containing a type-0x7e PrivilegedL2Transaction, \
         but add_block returned: {result:?}"
    );
}
