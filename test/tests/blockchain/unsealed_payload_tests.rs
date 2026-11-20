//! Tests for the payload build loop keeping its most valuable build.
//!
//! The loop rebuilds on a blocking thread whose progress a test can't observe,
//! so tests that need a rebuild to finish drive the loop for a fixed window
//! (`DRIVE`) before cancelling. A window that is too short would make a
//! negative test pass without exercising anything, so each negative test has a
//! positive twin that uses the same window and fails if the rebuild did not run.

use std::{future::Future, pin::Pin, task::Poll, time::Duration};

use ethrex_common::{
    Address, H160, H256,
    types::{EIP1559Transaction, Transaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use secp256k1::SecretKey;
use tokio_util::sync::CancellationToken;

use super::payload_build_loop_tests::{
    address_for, blockchain_for, payload_block, read_private_keys, setup_store,
};

const L1_CHAIN_ID: u64 = 9;

/// How long a test lets the build loop run so a rebuild triggered by a new
/// mempool transaction can finish. A rebuild here is a few transfers on an
/// in-memory store, far below this.
const DRIVE: Duration = Duration::from_secs(2);

async fn transfer_with_tip(sk: &SecretKey, nonce: u64, tip: u64) -> Transaction {
    let signer = Signer::Local(LocalSigner::new(*sk));
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        value: 1_u64.into(),
        gas_limit: 250_000_u64,
        max_fee_per_gas: 10_000_000_000_u64,
        max_priority_fee_per_gas: tip,
        chain_id: L1_CHAIN_ID,
        to: TxKind::Call(H160::random()),
        ..Default::default()
    });
    tx.sign_inplace(&signer).await.unwrap();
    tx
}

fn hashes(txs: &[Transaction]) -> Vec<H256> {
    txs.iter().map(|tx| tx.hash(&NativeCrypto)).collect()
}

/// Polls `fut` once. For `build_payload_loop` this runs the first build, which
/// is synchronous up to the loop's first `await`, and parks the loop with its
/// mempool waiter registered.
async fn poll_once<F: Future + Unpin>(fut: &mut F) {
    std::future::poll_fn(|cx| {
        assert!(
            Pin::new(&mut *fut).poll(cx).is_pending(),
            "build loop finished before it was cancelled"
        );
        Poll::Ready(())
    })
    .await
}

/// Runs `fut` for `DRIVE`. The loop only starts a rebuild when polled, so
/// every transaction added before this call is in the rebuild's mempool view.
async fn drive<F: Future + Unpin>(fut: &mut F) {
    assert!(
        tokio::time::timeout(DRIVE, fut).await.is_err(),
        "build loop finished before it was cancelled"
    );
}

/// Holds a payload with one transfer per entry of `held_tips`, takes those
/// transfers out of the pool, then adds one transfer per entry of
/// `replacement_tips` to trigger one rebuild that can't contain the held ones.
/// Every transfer comes from its own sender. Returns the held transfers'
/// hashes, the replacements' hashes, and the hashes in the loop's result.
async fn rebuild_replacing(
    held_tips: &[u64],
    replacement_tips: &[u64],
) -> (Vec<H256>, Vec<H256>, Vec<H256>) {
    let keys = read_private_keys(held_tips.len() + replacement_tips.len());
    let (held_keys, replacement_keys) = keys.split_at(held_tips.len());
    let accounts: Vec<Address> = keys.iter().map(address_for).collect();
    let store = setup_store(&accounts).await;
    let blockchain = blockchain_for(&store);

    let mut held_hashes = Vec::new();
    for (key, tip) in held_keys.iter().zip(held_tips) {
        let tx = transfer_with_tip(key, 0, *tip).await;
        held_hashes.push(blockchain.add_transaction_to_pool(tx).await.unwrap());
    }

    let token = CancellationToken::new();
    let mut fut = std::pin::pin!(
        blockchain
            .clone()
            .build_payload_loop(payload_block(&store), token.clone())
    );
    poll_once(&mut fut).await;

    for hash in &held_hashes {
        blockchain.remove_transaction_from_pool(hash).unwrap();
    }
    let mut replacement_hashes = Vec::new();
    for (key, tip) in replacement_keys.iter().zip(replacement_tips) {
        let tx = transfer_with_tip(key, 0, *tip).await;
        replacement_hashes.push(blockchain.add_transaction_to_pool(tx).await.unwrap());
    }
    drive(&mut fut).await;

    token.cancel();
    let best = fut.await.unwrap();
    (held_hashes, replacement_hashes, hashes(best.transactions()))
}

/// A rebuild worth less than the held payload must not replace it.
/// Positive twin: `higher_value_rebuild_replaces_held_payload`.
#[tokio::test]
async fn lower_value_rebuild_does_not_replace_held_payload() {
    let (held, replacements, result) = rebuild_replacing(&[10], &[1]).await;
    assert!(
        result.contains(&held[0]),
        "the more valuable held payload was replaced"
    );
    assert!(
        !result.contains(&replacements[0]),
        "the less valuable rebuild was kept"
    );
}

#[tokio::test]
async fn higher_value_rebuild_replaces_held_payload() {
    let (held, replacements, result) = rebuild_replacing(&[10], &[100]).await;
    assert!(
        result.contains(&replacements[0]),
        "the more valuable rebuild was not kept"
    );
    assert!(
        !result.contains(&held[0]),
        "the less valuable held payload was kept"
    );
}

/// On equal value the build that uses more gas wins: two transfers at tip 5
/// are worth exactly one at tip 10 and use twice the gas.
/// Negative twin: `equal_value_rebuild_with_less_gas_does_not_replace_held_payload`.
#[tokio::test]
async fn equal_value_rebuild_with_more_gas_replaces_held_payload() {
    let (held, replacements, result) = rebuild_replacing(&[10], &[5, 5]).await;
    assert!(
        replacements.iter().all(|hash| result.contains(hash)),
        "the equally valuable rebuild with more gas was not kept"
    );
    assert!(
        !result.contains(&held[0]),
        "the held payload with less gas was kept"
    );
}

/// On equal value a rebuild that uses less gas must not replace the held
/// payload. This is the case that matters for zero-tip traffic: every build is
/// worth 0, so without the gas key a rebuild that lost transactions to the
/// pool would replace a fuller held block.
/// Positive twin: `equal_value_rebuild_with_more_gas_replaces_held_payload`.
#[tokio::test]
async fn equal_value_rebuild_with_less_gas_does_not_replace_held_payload() {
    let (held, replacements, result) = rebuild_replacing(&[5, 5], &[10]).await;
    assert!(
        held.iter().all(|hash| result.contains(hash)),
        "the equally valuable held payload with more gas was replaced"
    );
    assert!(
        !result.contains(&replacements[0]),
        "the equally valuable rebuild with less gas was kept"
    );
}
