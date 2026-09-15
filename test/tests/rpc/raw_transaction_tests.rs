//! `debug_getRawTransaction` must return the transaction's CANONICAL bytes.
//!
//! The canonical encoding is `type || rlp(payload)` for a typed transaction — the bytes as
//! they appear in a block, and the bytes every consumer expects: `eth_sendRawTransaction`,
//! an engine payload's `transactions` list, and any tool that moves a transaction between
//! nodes. The *network* encoding wraps those bytes in an RLP string, and its length prefix
//! makes all three reject the result with an RLP `UnexpectedString`.
//!
//! Nothing caught the difference before: the value is a hex string either way, and it only
//! fails where it is fed back in.
use ethrex_common::types::{
    Block, BlockBody, BlockHeader, EIP1559Transaction, Transaction, TxKind,
};
use ethrex_common::{Address, U256};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::test_utils::{call_http, default_context_with_storage};
use ethrex_storage::{EngineType, Store};
use serde_json::json;

fn pooled_tx() -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        gas_limit: 21_000,
        to: TxKind::Call(Address::repeat_byte(0x22)),
        value: U256::zero(),
        ..Default::default()
    })
}

/// Put `tx` in a block in the store and read it back through the JSON-RPC surface.
///
/// A block rather than the mempool: pool admission recovers the sender, which would make
/// this test carry a signed fixture for no benefit — the subject is how the handler encodes
/// a transaction it found, and the store is where it looks first.
async fn raw_transaction_over_rpc(tx: &Transaction) -> String {
    let store = Store::new("raw-tx-store.db", EngineType::InMemory).expect("build store");
    let block = Block::new(
        BlockHeader {
            number: 1,
            ..Default::default()
        },
        BlockBody {
            transactions: vec![tx.clone()],
            ..Default::default()
        },
    );
    let block_hash = block.hash();
    let hash = tx.hash(&ethrex_crypto::NativeCrypto);
    store.add_block(block).await.expect("store the block");
    store
        .add_transaction_location(hash, 1, block_hash, 0)
        .await
        .expect("index the transaction");
    // A transaction location resolves only when its block is canonical, so the block has
    // to be made head as well as stored.
    store
        .forkchoice_update(vec![(1, block_hash)], 1, block_hash, None, None)
        .await
        .expect("make the block canonical");

    let context = default_context_with_storage(store).await;
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "debug_getRawTransaction",
        "params": [format!("{hash:#x}")],
    })
    .to_string();
    let response = call_http(context, body).await;
    response["result"]
        .as_str()
        .unwrap_or_else(|| panic!("debug_getRawTransaction returned {response}"))
        .to_string()
}

#[tokio::test]
async fn debug_get_raw_transaction_returns_canonical_bytes() {
    let tx = pooled_tx();
    let raw = raw_transaction_over_rpc(&tx).await;
    let bytes = hex::decode(raw.trim_start_matches("0x")).expect("hex");

    assert_eq!(
        bytes,
        tx.encode_canonical_to_vec(),
        "debug_getRawTransaction must return the canonical `type || payload` bytes"
    );
    assert_ne!(
        bytes,
        tx.encode_to_vec(),
        "the network encoding wraps the canonical bytes in an RLP string, whose length \
         prefix makes the result unusable in eth_sendRawTransaction and engine payloads"
    );
    assert_eq!(
        bytes.first().copied(),
        Some(0x02),
        "an EIP-1559 transaction's canonical bytes start with its type byte, not an RLP \
         string header"
    );
}
