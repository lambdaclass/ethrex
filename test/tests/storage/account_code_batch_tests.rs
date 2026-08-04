//! Correctness parity between the batched bytecode lookup
//! (`Store::get_account_codes_batch`) used by the BAL code prefetch and the per-hash
//! single-get path (`Store::get_account_code`) the executor reads through.
//!
//! The prefetch warms a cache that execution trusts, so the batched path must return the
//! same code (and the same jump-destination bitmap) for every hash, including "absent
//! code" -> None, and must keep results aligned with the caller's order rather than the
//! order the keys happen to be read in.

use bytes::Bytes;
use ethrex_common::{H256, types::Code};
use ethrex_storage::{EngineType, Store};

/// Number of distinct codes written.
///
/// The batched read picks between a parallel fan-out of point gets and sharded blocking
/// reads, switching once the batch forms more 256-key shards than there are cores. This
/// is sized past that switch so the parity checks cover the sharded path; the fan-out
/// path is covered by [`batch_matches_the_single_get_below_the_shard_threshold`].
fn code_count() -> u64 {
    let parallelism = std::thread::available_parallelism().map_or(8, |p| p.get()) as u64;
    256 * parallelism + 256
}

const JUMPDEST: u8 = 0x5b;
const PUSH1: u8 = 0x60;

/// A distinct code per `id`. The length varies with `id` and a `PUSH1` is planted so
/// that the byte after it is *not* a valid jump destination, which makes the bitmap
/// depend on the bytecode rather than being a constant.
fn code_of(id: u64) -> Code {
    let len = 32 + (id as usize % 97);
    let mut bytecode = vec![JUMPDEST; len];
    bytecode[0] = PUSH1;
    let bytecode: Bytes = bytecode.into();
    Code::from_bytecode_unchecked(bytecode, H256::from_low_u64_be(id))
}

/// A hash never written, to cover the absent case.
fn absent_hash(id: u64) -> H256 {
    H256::from_low_u64_be(1_000_000 + id)
}

async fn store_with_codes(count: u64) -> Store {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::new(dir.path(), EngineType::InMemory).expect("store");
    for id in 0..count {
        store
            .add_account_code(code_of(id))
            .await
            .expect("add_account_code");
    }
    store
}

/// The batched read SHALL agree with the per-hash read on the bytecode, the jumpdest
/// bitmap, and absence, for a batch spanning several shards.
#[tokio::test]
async fn batch_matches_the_single_get_including_absent_hashes() {
    let count = code_count();
    let store = store_with_codes(count).await;

    // Present and absent hashes interleaved, so a shard cannot be all-present.
    let mut requested = Vec::new();
    for id in 0..count {
        requested.push(H256::from_low_u64_be(id));
        requested.push(absent_hash(id));
    }

    let batched = store
        .get_account_codes_batch(&requested)
        .expect("get_account_codes_batch");
    assert_eq!(batched.len(), requested.len());

    for (hash, batched) in requested.iter().zip(batched.iter()) {
        let single = store.get_account_code(*hash).expect("get_account_code");
        assert_eq!(
            batched.as_ref().map(|c| c.code()),
            single.as_ref().map(|c| c.code()),
            "bytecode mismatch for {hash:?}"
        );
        assert_eq!(
            batched.as_ref().map(|c| c.jumpdests()),
            single.as_ref().map(|c| c.jumpdests()),
            "jumpdest bitmap mismatch for {hash:?}"
        );
    }
}

/// Results SHALL follow the caller's order. The batched read sorts internally, so a
/// result vector keyed by read order would silently mismatch the request.
#[tokio::test]
async fn batch_results_follow_the_requested_order() {
    let count = code_count();
    let store = store_with_codes(count).await;

    let requested: Vec<H256> = (0..count).rev().map(H256::from_low_u64_be).collect();
    let batched = store
        .get_account_codes_batch(&requested)
        .expect("get_account_codes_batch");

    for (id, batched) in (0..count).rev().zip(batched.iter()) {
        assert_eq!(
            batched.as_ref().map(|c| c.code()),
            Some(code_of(id).code()),
            "wrong code returned for id {id}"
        );
    }
}

/// A hash repeated within one batch SHALL be answered at every position it occupies,
/// since the read deduplicates before going to the database.
#[tokio::test]
async fn batch_answers_every_position_of_a_repeated_hash() {
    let store = store_with_codes(code_count()).await;

    let repeated = H256::from_low_u64_be(3);
    let other = H256::from_low_u64_be(4);
    let requested = vec![
        repeated,
        other,
        repeated,
        repeated,
        absent_hash(0),
        repeated,
    ];

    let batched = store
        .get_account_codes_batch(&requested)
        .expect("get_account_codes_batch");

    for i in [0, 2, 3, 5] {
        assert_eq!(
            batched[i].as_ref().map(|c| c.code()),
            Some(code_of(3).code()),
            "position {i} lost the repeated hash"
        );
    }
    assert_eq!(
        batched[1].as_ref().map(|c| c.code()),
        Some(code_of(4).code())
    );
    assert!(batched[4].is_none());
}

/// The same parity, for a batch small enough to take the parallel fan-out instead of the
/// sharded reads. Ordinary blocks land here, so it is the path that must not regress.
#[tokio::test]
async fn batch_matches_the_single_get_below_the_shard_threshold() {
    let store = store_with_codes(64).await;

    let mut requested: Vec<H256> = (0..64).map(H256::from_low_u64_be).collect();
    requested.push(absent_hash(0));

    let batched = store
        .get_account_codes_batch(&requested)
        .expect("get_account_codes_batch");

    for (hash, batched) in requested.iter().zip(batched.iter()) {
        let single = store.get_account_code(*hash).expect("get_account_code");
        assert_eq!(
            batched.as_ref().map(|c| c.code()),
            single.as_ref().map(|c| c.code()),
            "bytecode mismatch for {hash:?}"
        );
        assert_eq!(
            batched.as_ref().map(|c| c.jumpdests()),
            single.as_ref().map(|c| c.jumpdests()),
            "jumpdest bitmap mismatch for {hash:?}"
        );
    }
}
