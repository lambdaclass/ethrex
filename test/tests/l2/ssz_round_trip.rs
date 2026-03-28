//! Test that Block → SSZ StatelessInput → Block round-trip preserves the block hash.
//!
//! This catches encoding mismatches between `build_ssz_stateless_input` (advancer)
//! and `new_payload_request_to_block` (verify_stateless_new_payload).

#![allow(clippy::unwrap_used)]

use bytes::Bytes;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::types::stateless_ssz::SszStatelessInput;
use ethrex_common::types::{Block, BlockBody, BlockHeader};
use ethrex_common::{Address, H256};
use ethrex_crypto::NativeCrypto;
use ethrex_guest_program::l1::new_payload_request_to_block;
use ethrex_l2::sequencer::native_rollup::l1_advancer::build_ssz_stateless_input;
use ssz::SszDecode;

/// Build a minimal L2-style block (Shanghai chain, empty txs).
fn make_test_block() -> (BlockHeader, BlockBody) {
    let header = BlockHeader {
        parent_hash: H256::zero(),
        ommers_hash: *ethrex_common::constants::DEFAULT_OMMERS_HASH,
        coinbase: Address::zero(),
        state_root: H256::from_low_u64_be(0xabcd),
        transactions_root: ethrex_common::types::compute_transactions_root(&[], &NativeCrypto),
        receipts_root: H256::from_low_u64_be(0x1234),
        number: 1,
        gas_limit: 30_000_000,
        gas_used: 0,
        timestamp: 1000,
        base_fee_per_gas: Some(7),
        prev_randao: H256::zero(),
        extra_data: Bytes::new(),
        // Shanghai fields
        withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(
            &[],
            &NativeCrypto,
        )),
        // Cancun fields (present in L2 blocks even on Shanghai chain)
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(H256::from_low_u64_be(0xbeef)),
        // Prague fields
        requests_hash: Some(ethrex_common::types::requests::compute_requests_hash(&[])),
        ..Default::default()
    };

    let body = BlockBody {
        transactions: vec![],
        ommers: vec![],
        withdrawals: Some(vec![]),
    };

    (header, body)
}

#[test]
fn block_to_ssz_to_block_preserves_hash() {
    let (header, body) = make_test_block();
    let original_hash = header.compute_block_hash(&NativeCrypto);

    // Create a minimal witness (empty — we only care about header round-trip)
    let witness = ExecutionWitness {
        codes: vec![],
        block_headers_bytes: vec![],
        first_block_number: 0,
        chain_config: ethrex_common::types::ChainConfig {
            chain_id: 1,
            ..Default::default()
        },
        state_trie_root: None,
        storage_trie_roots: Default::default(),
    };

    // Block → SSZ
    let ssz_bytes =
        build_ssz_stateless_input(&header, &body, &witness).expect("SSZ encoding failed");

    // SSZ → deserialize
    let input = SszStatelessInput::from_ssz_bytes(&ssz_bytes).expect("SSZ decoding failed");

    // SSZ → Block
    let reconstructed_block =
        new_payload_request_to_block(&input.new_payload_request, &NativeCrypto)
            .expect("block reconstruction failed");

    let reconstructed_hash = reconstructed_block.hash();

    // Compare all header fields for debugging
    assert_eq!(
        header.parent_hash, reconstructed_block.header.parent_hash,
        "parent_hash mismatch"
    );
    assert_eq!(
        header.coinbase, reconstructed_block.header.coinbase,
        "coinbase mismatch"
    );
    assert_eq!(
        header.state_root, reconstructed_block.header.state_root,
        "state_root mismatch"
    );
    assert_eq!(
        header.transactions_root, reconstructed_block.header.transactions_root,
        "transactions_root mismatch"
    );
    assert_eq!(
        header.receipts_root, reconstructed_block.header.receipts_root,
        "receipts_root mismatch"
    );
    assert_eq!(
        header.number, reconstructed_block.header.number,
        "number mismatch"
    );
    assert_eq!(
        header.gas_limit, reconstructed_block.header.gas_limit,
        "gas_limit mismatch"
    );
    assert_eq!(
        header.gas_used, reconstructed_block.header.gas_used,
        "gas_used mismatch"
    );
    assert_eq!(
        header.timestamp, reconstructed_block.header.timestamp,
        "timestamp mismatch"
    );
    assert_eq!(
        header.base_fee_per_gas, reconstructed_block.header.base_fee_per_gas,
        "base_fee_per_gas mismatch"
    );
    assert_eq!(
        header.prev_randao, reconstructed_block.header.prev_randao,
        "prev_randao mismatch"
    );
    assert_eq!(
        header.extra_data, reconstructed_block.header.extra_data,
        "extra_data mismatch"
    );
    assert_eq!(
        header.logs_bloom, reconstructed_block.header.logs_bloom,
        "logs_bloom mismatch"
    );
    assert_eq!(
        header.difficulty, reconstructed_block.header.difficulty,
        "difficulty mismatch"
    );
    assert_eq!(
        header.nonce, reconstructed_block.header.nonce,
        "nonce mismatch"
    );
    assert_eq!(
        header.ommers_hash, reconstructed_block.header.ommers_hash,
        "ommers_hash mismatch"
    );
    assert_eq!(
        header.withdrawals_root, reconstructed_block.header.withdrawals_root,
        "withdrawals_root mismatch"
    );
    assert_eq!(
        header.blob_gas_used, reconstructed_block.header.blob_gas_used,
        "blob_gas_used mismatch"
    );
    assert_eq!(
        header.excess_blob_gas, reconstructed_block.header.excess_blob_gas,
        "excess_blob_gas mismatch"
    );
    assert_eq!(
        header.parent_beacon_block_root, reconstructed_block.header.parent_beacon_block_root,
        "parent_beacon_block_root mismatch"
    );
    assert_eq!(
        header.requests_hash, reconstructed_block.header.requests_hash,
        "requests_hash mismatch"
    );

    // Final hash check
    assert_eq!(
        original_hash, reconstructed_hash,
        "Block hash mismatch after SSZ round-trip"
    );
}
