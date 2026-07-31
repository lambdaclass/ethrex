//! Regression test for the `wrong-chain-id-block-tx` finding: an imported block must
//! be rejected if it carries a typed transaction whose embedded chain id does not match
//! the chain.
//!
//! ethrex recovers a typed tx's sender from the tx's *own* chain-id signing domain and
//! never re-checks that chain id against the node's chain during block import, so a block
//! containing a wrong-chain typed tx executes and is accepted. geth's signer rejects the
//! same tx (`ErrInvalidChainId`), so ethrex would import a block the rest of the network
//! rejects — a consensus divergence. The mempool already rejects it
//! (`MempoolError::InvalidChainId`), but the engine/p2p block-import path does not.
use std::collections::BTreeMap;

use ethrex_blockchain::Blockchain;
use ethrex_common::constants::DEFAULT_OMMERS_HASH;
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, ChainConfig, EIP1559Transaction, ELASTICITY_MULTIPLIER, Genesis,
    GenesisAccount, Receipt, Transaction, TxKind, TxType, calculate_base_fee_per_gas,
    compute_receipts_root, compute_transactions_root,
};
use ethrex_common::{Address, Bloom, Bytes, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::encode::PayloadRLPEncode;
use ethrex_storage::{EngineType, Store};
use secp256k1::{Message, PublicKey, SECP256K1, SecretKey};

const LOCAL_CHAIN_ID: u64 = 9;
const FOREIGN_CHAIN_ID: u64 = 10;

fn sender_address(sk: &SecretKey) -> Address {
    let pk = PublicKey::from_secret_key(SECP256K1, sk);
    let hash = keccak_hash(&pk.serialize_uncompressed()[1..]);
    Address::from_slice(&hash[12..])
}

/// A validly-signed EIP-1559 transaction carrying `FOREIGN_CHAIN_ID`, returned with the
/// sender address it recovers to.
fn foreign_chain_tx(recipient: Address) -> (Transaction, Address) {
    let sk = SecretKey::from_slice(&hex_literal::hex!(
        "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318"
    ))
    .expect("valid secret key");
    let sender = sender_address(&sk);

    let mut tx = EIP1559Transaction {
        chain_id: FOREIGN_CHAIN_ID,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 21_000,
        to: TxKind::Call(recipient),
        value: U256::one(),
        data: Bytes::new(),
        access_list: Vec::new(),
        ..Default::default()
    };

    // EIP-1559 signing hash: keccak(0x02 ‖ rlp(chain_id, nonce, …, access_list)).
    let mut signing_payload = vec![0x02u8];
    tx.encode_payload(&mut signing_payload);
    let msg = Message::from_digest(keccak_hash(&signing_payload));
    let (recovery_id, signature) = SECP256K1
        .sign_ecdsa_recoverable(&msg, &sk)
        .serialize_compact();
    tx.signature_r = U256::from_big_endian(&signature[..32]);
    tx.signature_s = U256::from_big_endian(&signature[32..]);
    tx.signature_y_parity = Into::<i32>::into(recovery_id) != 0;

    (Transaction::EIP1559Transaction(tx), sender)
}

#[tokio::test]
async fn rejects_block_with_wrong_chain_id_transaction() {
    let recipient = Address::from_low_u64_be(0x200);
    let (tx, sender) = foreign_chain_tx(recipient);
    // Sanity: ethrex recovers the sender despite the foreign chain id (the bug's premise).
    assert_eq!(
        tx.sender(&NativeCrypto).expect("ethrex recovers sender"),
        sender
    );
    assert_eq!(tx.chain_id(), Some(FOREIGN_CHAIN_ID));

    // Genesis funds the sender with exactly the 1 wei it transfers (fees are zero).
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
            chain_id: LOCAL_CHAIN_ID,
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

    let body = BlockBody {
        transactions: vec![tx],
        ommers: Vec::new(),
        withdrawals: None,
    };

    // Post-state consistent with ethrex executing the transfer (sender drained, nonce
    // bumped; recipient credited) so the block passes the post-execution root checks and
    // would be accepted — isolating the missing chain-id check as the only defect.
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
    let receipts = [Receipt::new(TxType::EIP1559, true, 21_000, Vec::new())];

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

    let err = blockchain.add_block(block).expect_err(
        "a block carrying a typed tx with a foreign chain id must be rejected (geth: ErrInvalidChainId)",
    );
    assert!(
        err.to_string().to_lowercase().contains("chain id"),
        "expected a chain-id rejection, got: {err:?}"
    );
}
