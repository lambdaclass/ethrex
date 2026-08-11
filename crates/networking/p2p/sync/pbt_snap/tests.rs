//! The driver against honest and hostile providers.
//!
//! Weighted towards the hostile ones on purpose. A client that syncs from a
//! correct server demonstrates almost nothing about a snap client, whose entire
//! job is to not be fooled by a peer that answers promptly, in the right shape,
//! and with a lie.

use super::*;
use crate::rlpx::pbtsnap::{PbtLeaf, PbtLeafRange};
use ethrex_common::types::{AccountUpdate, ChainConfig, GenesisAccount};
use ethrex_common::{Address, U256};
use ethrex_storage::{EngineType, Store};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Comfortably past the activation the fixtures schedule at genesis.
const PIVOT_TIMESTAMP: u64 = 1_000;

/// Bytecode seeded by `tag`, distinct per account so a substitution between two
/// of them is detectable, and long enough to span several 31-byte code chunks
/// so the code zone is a real range rather than a single leaf.
fn bytecode(seed: u8) -> Bytes {
    Bytes::from(
        (0..100u8)
            .map(|i| i.wrapping_mul(seed).wrapping_add(seed))
            .collect::<Vec<u8>>(),
    )
}

fn account(nonce: u64, balance: u64, code: Bytes, storage: &[(u64, u64)]) -> GenesisAccount {
    GenesisAccount {
        code,
        storage: storage
            .iter()
            .map(|(slot, value)| (U256::from(*slot), U256::from(*value)))
            .collect(),
        balance: U256::from(balance),
        nonce,
    }
}

/// A store holding binary state, with a canonical post-activation header
/// committing its root.
///
/// `tag` seeds every address, balance, slot and bytecode, so two stores built
/// with different tags share **no** leaf key and no code hash. That is
/// deliberate and load-bearing: seeding a sync target from the source's own
/// genesis has silently passed three tests on this project by handing the
/// target the answer before the sync ran.
async fn store_with_state(tag: u8) -> (Store, BlockHeader) {
    let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
    store
        .set_chain_config(&ChainConfig {
            binary_tree_time: Some(0),
            ..Default::default()
        })
        .await
        .expect("chain config");

    let mut alloc = BTreeMap::new();
    alloc.insert(
        Address::repeat_byte(tag),
        account(
            1,
            1_000 + tag as u64,
            bytecode(tag),
            // One header-storage slot and one overflow slot, so both the
            // account zone's storage sub-indices and zone 0xff appear.
            &[(1, 2), (900 + tag as u64, 3)],
        ),
    );
    alloc.insert(
        Address::repeat_byte(tag.wrapping_add(1)),
        account(2, 500, bytecode(tag.wrapping_add(1)), &[(5, 7)]),
    );
    // An account with no code at all: its code-hash leaf is the empty keccak
    // and must cost no bytecode request.
    alloc.insert(
        Address::repeat_byte(tag.wrapping_add(2)),
        account(0, 1, Bytes::new(), &[]),
    );
    let root = store
        .setup_genesis_binary_trie(alloc)
        .await
        .expect("genesis binary trie");

    let header = BlockHeader {
        number: 1,
        timestamp: PIVOT_TIMESTAMP,
        state_root: root,
        ..Default::default()
    };
    let hash = header.hash();
    store
        .add_block_header(hash, header.clone())
        .await
        .expect("header");
    store
        .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
        .await
        .expect("fcu");
    store.set_binary_trie_root(hash, root).expect("record root");
    (store, header)
}

/// The sync target: its own unrelated state, plus the pivot header a real
/// client would have downloaded in the header phase, and nothing else.
async fn target_store(pivot: &BlockHeader) -> Store {
    let (store, _own) = store_with_state(0x70).await;
    store
        .add_block_header(pivot.hash(), pivot.clone())
        .await
        .expect("pivot header");
    store
}

fn all_leaves(store: &Store, root: H256) -> Vec<(Vec<u8>, [u8; 32])> {
    store
        .binary_leaf_range_proof(root, &[], &[], usize::MAX)
        .expect("the store holds its own root")
        .leaves
}

/// Serves from a store exactly as the connection layer does, refusal shape
/// included: an unservable root comes back as an *empty* range rather than as a
/// transport error, because that is what a real peer sends.
#[derive(Clone)]
struct Honest {
    store: Store,
}

impl PbtSnapProvider for Honest {
    async fn get_leaf_range(
        &self,
        request: GetPbtLeafRange,
    ) -> Result<PbtLeafRange, PbtProviderError> {
        let id = request.id;
        Ok(
            crate::pbtsnap::process_pbt_leaf_range_request(request, self.store.clone())
                .await
                .unwrap_or(PbtLeafRange {
                    id,
                    leaves: vec![],
                    left_proof: vec![],
                    right_proof: vec![],
                }),
        )
    }

    async fn get_bytecodes(&self, hashes: &[H256]) -> Result<Vec<Bytes>, PbtProviderError> {
        Ok(hashes
            .iter()
            .map(|hash| {
                self.store
                    .get_account_code(*hash)
                    .expect("code read")
                    .map(|code| Bytes::copy_from_slice(code.code()))
                    .unwrap_or_default()
            })
            .collect())
    }
}

type RangeFault = Box<dyn Fn(u32, PbtLeafRange) -> PbtLeafRange + Send + Sync>;
type CodeFault = Box<dyn Fn(Vec<Bytes>) -> Vec<Bytes> + Send + Sync>;

/// An honest provider with one thing corrupted, and a call counter so a fault
/// can be confined to the first answer.
struct Byzantine {
    inner: Honest,
    calls: Mutex<u32>,
    range_fault: RangeFault,
    code_fault: CodeFault,
    /// Rewrites the root every request asks about, for the
    /// consistent-but-wrong-state case.
    serve_root: Option<H256>,
}

impl Byzantine {
    fn new(store: Store) -> Self {
        Self {
            inner: Honest { store },
            calls: Mutex::new(0),
            range_fault: Box::new(|_, response| response),
            code_fault: Box::new(|codes| codes),
            serve_root: None,
        }
    }

    fn ranges(mut self, fault: RangeFault) -> Self {
        self.range_fault = fault;
        self
    }

    fn codes(mut self, fault: CodeFault) -> Self {
        self.code_fault = fault;
        self
    }

    fn serving(mut self, root: H256) -> Self {
        self.serve_root = Some(root);
        self
    }

    fn call_count(&self) -> u32 {
        *self.calls.lock().expect("counter")
    }
}

impl PbtSnapProvider for Byzantine {
    async fn get_leaf_range(
        &self,
        mut request: GetPbtLeafRange,
    ) -> Result<PbtLeafRange, PbtProviderError> {
        let call = {
            let mut calls = self.calls.lock().expect("counter");
            *calls += 1;
            *calls
        };
        if let Some(root) = self.serve_root {
            request.root_hash = root;
        }
        let response = self.inner.get_leaf_range(request).await?;
        Ok((self.range_fault)(call, response))
    }

    async fn get_bytecodes(&self, hashes: &[H256]) -> Result<Vec<Bytes>, PbtProviderError> {
        Ok((self.code_fault)(self.inner.get_bytecodes(hashes).await?))
    }
}

/// Nothing landed: not the root, not the trie.
fn nothing_installed(store: &Store, pivot: &BlockHeader) {
    assert_eq!(
        store.get_binary_trie_root(pivot.hash()).expect("root read"),
        None,
        "a rejected sync must record no root for the pivot",
    );
    // Not merely "no root recorded": the trie itself must not resolve to the
    // pivot root. `binary_leaf_range_proof` refuses outright when it does not,
    // which is the strongest probe available from outside the storage crate.
    assert!(
        store
            .binary_leaf_range_proof(pivot.state_root, &[], &[], 1)
            .is_err(),
        "a rejected sync must not leave the trie resolving to the pivot root",
    );
}

// -------------------------------------------------------------------- honest

#[tokio::test]
async fn download_and_install_reproduces_the_pivot_state() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;

    // The fixture's own precondition. If it ever stops holding, every
    // assertion below is being made against a target that already had the
    // answer.
    let source_leaves = all_leaves(&source, pivot.state_root);
    let target_head = target
        .get_canonical_block_hash(1)
        .await
        .expect("canonical read")
        .expect("the target has its own head");
    let target_own_root = target
        .get_binary_trie_root(target_head)
        .expect("root read")
        .expect("the target has its own state");
    let target_leaves = all_leaves(&target, target_own_root);
    assert!(
        !source_leaves.is_empty() && !target_leaves.is_empty(),
        "both fixtures must hold state"
    );
    assert!(
        source_leaves
            .iter()
            .all(|(key, _)| !target_leaves.iter().any(|(other, _)| other == key)),
        "the target must share no leaf with the source before syncing",
    );

    let provider = Honest {
        store: source.clone(),
    };
    let installed = sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect("an honest provider must complete the sync");

    assert_eq!(installed, pivot.state_root);
    assert_eq!(
        target
            .get_binary_trie_root(pivot.hash())
            .expect("root read"),
        Some(pivot.state_root),
    );
    assert!(
        target
            .has_state_for_header(pivot.hash(), &pivot)
            .expect("state check"),
        "the pivot's state must be available after an install",
    );
    // Leaf-for-leaf agreement with the source, read back out of the target
    // rather than out of what the driver held in memory.
    assert_eq!(all_leaves(&target, pivot.state_root), source_leaves);
    // And the codes, which the trie holds only as chunks and which the EVM can
    // fetch whole only from `ACCOUNT_CODES`.
    let wanted = code_requests(&source_leaves).expect("code requests");
    assert!(!wanted.is_empty(), "the fixture has contracts");
    for (hash, size) in wanted {
        let code = target
            .get_account_code(hash)
            .expect("code read")
            .expect("every referenced code must land");
        assert_eq!(code.hash, hash);
        assert_eq!(code.code().len(), size as usize);
    }
}

/// The empty-code account must cost no bytecode request. Asserted on the
/// extraction rather than through a provider so the count is exact.
#[tokio::test]
async fn an_empty_code_hash_asks_for_no_bytecode() {
    let (source, pivot) = store_with_state(0x11).await;
    let wanted = code_requests(&all_leaves(&source, pivot.state_root)).expect("requests");
    assert_eq!(
        wanted.len(),
        2,
        "only the two accounts with real code may be requested, got {wanted:?}",
    );
    assert!(wanted.iter().all(|(hash, _)| *hash != *EMPTY_KECCAK_HASH));
}

/// A provider forced down to one leaf per answer still completes, which is the
/// progress rule and the resume cursor working together. It is also the only
/// test here that exercises `increment_key` across responses at all — the
/// fixture's whole state otherwise fits in a single range.
#[tokio::test]
async fn a_provider_that_answers_one_leaf_at_a_time_still_completes() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;
    let expected = all_leaves(&source, pivot.state_root);

    struct OneLeafAtATime {
        inner: Honest,
        calls: Mutex<u32>,
    }
    impl PbtSnapProvider for OneLeafAtATime {
        async fn get_leaf_range(
            &self,
            mut request: GetPbtLeafRange,
        ) -> Result<PbtLeafRange, PbtProviderError> {
            *self.calls.lock().expect("counter") += 1;
            request.response_bytes = 0;
            self.inner.get_leaf_range(request).await
        }
        async fn get_bytecodes(&self, hashes: &[H256]) -> Result<Vec<Bytes>, PbtProviderError> {
            self.inner.get_bytecodes(hashes).await
        }
    }

    let provider = OneLeafAtATime {
        inner: Honest {
            store: source.clone(),
        },
        calls: Mutex::new(0),
    };
    sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect("a stingy but honest provider must still complete the sync");

    assert_eq!(all_leaves(&target, pivot.state_root), expected);
    assert_eq!(
        *provider.calls.lock().expect("counter") as usize,
        expected.len(),
        "one leaf per round trip means one round trip per leaf",
    );
}

// ------------------------------------------------------------- adversarial

/// A peer that cannot serve the pivot answers with an empty range — the
/// production refusal path, since a pivot ageing out of a peer's layer window
/// is expected rather than exceptional. Accepting it would install an empty
/// state under a real root, which is the worst outcome available here.
#[tokio::test]
async fn a_peer_that_cannot_serve_the_pivot_writes_nothing() {
    let (_source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;

    // A provider holding a perfectly good state — just not this one.
    let (stranger, stranger_pivot) = store_with_state(0x55).await;
    assert_ne!(pivot.state_root, stranger_pivot.state_root);

    let provider = Honest { store: stranger };
    let error = sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect_err("an empty range must not be mistaken for an empty state");
    assert!(
        matches!(error, PbtSyncError::Range(_)),
        "expected a verification failure, got {error}",
    );
    nothing_installed(&target, &pivot);
}

/// A fully self-consistent state served for a *different* root. Every range
/// verifies against its own tree and none against the pivot's, which is the
/// pivot root doing the only job it has.
#[tokio::test]
async fn a_consistent_state_for_the_wrong_root_is_rejected() {
    let (_source, pivot) = store_with_state(0x11).await;
    let (liar, liar_pivot) = store_with_state(0x22).await;
    let target = target_store(&pivot).await;

    let provider = Byzantine::new(liar).serving(liar_pivot.state_root);
    let error = sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect_err("state for another root must not install under this one");
    assert!(
        matches!(error, PbtSyncError::Range(_)),
        "expected a verification failure, got {error}",
    );
    nothing_installed(&target, &pivot);
}

/// A tampered value costs a round trip, not the sync: the range is re-asked and
/// an honest second answer completes it.
#[tokio::test]
async fn a_tampered_leaf_is_re_asked_and_an_honest_answer_completes() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;
    let expected = all_leaves(&source, pivot.state_root);

    let provider = Byzantine::new(source).ranges(Box::new(|call, mut response| {
        if call == 1 {
            response.leaves[0].value = H256::repeat_byte(0xab);
        }
        response
    }));
    sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect("one bad answer must not end the sync");

    assert!(
        provider.call_count() >= 2,
        "the tampered range must have been re-asked",
    );
    assert_eq!(all_leaves(&target, pivot.state_root), expected);
}

/// The same tamper on *every* answer exhausts the budget and writes nothing.
/// Without this, the retry test above would pass just as well against a driver
/// that stopped verifying after the first attempt.
#[tokio::test]
async fn a_persistently_tampered_leaf_ends_the_sync_with_nothing_written() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;

    let provider = Byzantine::new(source).ranges(Box::new(|_, mut response| {
        response.leaves[0].value = H256::repeat_byte(0xab);
        response
    }));
    let error = sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect_err("a persistent liar must not be able to complete a sync");
    assert!(
        matches!(error, PbtSyncError::Range(RangeProofError::RootMismatch)),
        "expected a root mismatch, got {error}",
    );
    assert_eq!(
        provider.call_count(),
        RANGE_VERIFY_ATTEMPTS,
        "the retry budget must be spent exactly once",
    );
    nothing_installed(&target, &pivot);
}

/// Gap smuggling: a mid-range leaf dropped, both boundary walks left untouched.
/// The boundaries still bracket the range, so only the re-merkleization catches
/// it.
#[tokio::test]
async fn a_dropped_mid_range_leaf_is_rejected() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;

    let provider = Byzantine::new(source).ranges(Box::new(|_, mut response| {
        assert!(
            response.leaves.len() > 2,
            "the fixture must be big enough to smuggle a gap into"
        );
        response.leaves.remove(1);
        response
    }));
    let error = sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect_err("a gap in the middle of a range must not verify");
    assert!(
        matches!(error, PbtSyncError::Range(RangeProofError::RootMismatch)),
        "expected a root mismatch, got {error}",
    );
    nothing_installed(&target, &pivot);
}

/// A boundary walk missing its last node, on each side in turn.
#[tokio::test]
async fn a_truncated_boundary_proof_is_rejected() {
    for truncate_left in [true, false] {
        let (source, pivot) = store_with_state(0x11).await;
        let target = target_store(&pivot).await;

        let provider = Byzantine::new(source).ranges(Box::new(move |_, mut response| {
            if truncate_left {
                response.left_proof.pop();
            } else {
                response.right_proof.pop();
            }
            response
        }));
        let error = sync_pbt_state(&provider, &target, &pivot)
            .await
            .expect_err("a truncated walk must not verify");
        assert!(
            matches!(error, PbtSyncError::Range(_)),
            "expected a verification failure, got {error}",
        );
        nothing_installed(&target, &pivot);
    }
}

/// Forged emptiness: no leaves, no right walk, and a left walk that is
/// genuinely the origin's — the strongest form of the lie, since every part of
/// the response is individually authentic.
#[tokio::test]
async fn a_forged_empty_range_is_rejected() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;

    let provider = Byzantine::new(source).ranges(Box::new(|_, mut response| {
        response.leaves.clear();
        response.right_proof.clear();
        response
    }));
    let error = sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect_err("an empty range over non-empty state must not verify");
    assert!(
        matches!(error, PbtSyncError::Range(RangeProofError::MissingLeaves)),
        "expected missing leaves, got {error}",
    );
    nothing_installed(&target, &pivot);
}

/// A leaf invented past the end of the range, with the right walk left pointing
/// at the real last leaf.
#[tokio::test]
async fn an_appended_leaf_is_rejected() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;

    let provider = Byzantine::new(source).ranges(Box::new(|_, mut response| {
        response.leaves.push(PbtLeaf {
            key: Bytes::from(vec![0xffu8; STORAGE_KEY_LENGTH]),
            value: H256::repeat_byte(0x5a),
        });
        response
    }));
    let error = sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect_err("an invented trailing leaf must not verify");
    assert!(
        matches!(error, PbtSyncError::Range(_)),
        "expected a verification failure, got {error}",
    );
    nothing_installed(&target, &pivot);
}

/// Bytecode substitution. The leaves are perfect and the root check would catch
/// it in the end anyway — the chunks are leaves — but the keccak check names
/// the offending hash instead of reporting an opaque mismatch.
#[tokio::test]
async fn a_substituted_bytecode_is_rejected() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;

    let provider = Byzantine::new(source).codes(Box::new(|mut codes| {
        codes[0] = Bytes::from_static(b"not the code you asked for");
        codes
    }));
    let error = sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect_err("a bytecode that does not hash to its key must be refused");
    assert!(
        matches!(error, PbtSyncError::BytecodeHashMismatch { .. }),
        "expected a hash mismatch, got {error}",
    );
    nothing_installed(&target, &pivot);
}

/// A short bytecode answer.
#[tokio::test]
async fn a_short_bytecode_answer_is_rejected() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;

    let provider = Byzantine::new(source).codes(Box::new(|mut codes| {
        codes.pop();
        codes
    }));
    let error = sync_pbt_state(&provider, &target, &pivot)
        .await
        .expect_err("a short bytecode answer must be refused");
    assert!(
        matches!(error, PbtSyncError::BytecodeCountMismatch { .. }),
        "expected a count mismatch, got {error}",
    );
    nothing_installed(&target, &pivot);
}

/// The code-size cross-check, exercised on `download_codes` directly.
///
/// It cannot be reached through a provider, and a mutation check is what
/// established that: deleting the check left the whole suite green. Keccak
/// already pins a bytecode's content and therefore its length, so a provider
/// serving genuine leaves and genuine code can never disagree with the size its
/// own basic-data leaf states — the two would have to come from different
/// states, which the pivot root rejects first. What the check buys is the
/// error *message* when a chain is internally inconsistent: a named hash and
/// two lengths, instead of an opaque root mismatch several steps later.
#[tokio::test]
async fn a_bytecode_disagreeing_with_its_accounts_code_size_is_refused() {
    let (source, pivot) = store_with_state(0x11).await;
    let provider = Honest { store: source };
    let (hash, real_size) =
        code_requests(&all_leaves(&provider.store, pivot.state_root)).expect("requests")[0];

    download_codes(&provider, &[(hash, real_size)])
        .await
        .expect("the truthful size must be accepted");

    let error = download_codes(&provider, &[(hash, real_size + 1)])
        .await
        .expect_err("a size the code cannot have must be refused");
    assert!(
        matches!(
            error,
            PbtSyncError::BytecodeSizeMismatch { hash: got, expected, actual }
                if got == hash && expected == real_size + 1 && actual == real_size as usize
        ),
        "expected a size mismatch naming the code, got {error}",
    );
}

/// The plan's open question 2, answered: a node that has installed a snapshot
/// **cannot** silently rebuild from its old chain.
///
/// The hazard was that `install_binary_snapshot` parks the single-version trie
/// at the pivot's state while `BINARY_TRIE_ROOTS` still records roots for every
/// earlier block, so a later `advance_binary_trie_for_block` from one of those
/// blocks would find its row, open a path-keyed trie at a root the disk no
/// longer holds, and commit a root computed over the wrong base with nothing
/// downstream able to tell.
///
/// It does not: `advance_binary_trie_for_block` re-reads the root group through
/// its own layered db and refuses with `BinaryTrieRootNotHeld` when it does not
/// hash to the recorded root. The guard is already in the store and its comment
/// already names the snapshot-install case; what was missing was a test that a
/// snapshot install actually trips it. The node stays *sound* — it just cannot
/// recover in place, and the operator has to wipe the datadir.
#[tokio::test]
async fn replaying_the_old_chain_after_an_install_fails_loudly() {
    let (source, pivot) = store_with_state(0x11).await;
    let target = target_store(&pivot).await;
    let old_head = target
        .get_canonical_block_hash(1)
        .await
        .expect("canonical read")
        .expect("the target has its own head");
    let old_root = target
        .get_binary_trie_root(old_head)
        .expect("root read")
        .expect("the target has its own state");

    // Before the install, extending the old chain is ordinary business.
    target
        .advance_binary_trie_for_block(
            H256::repeat_byte(0xa1),
            old_head,
            &[AccountUpdate::removed(Address::repeat_byte(0x99))],
        )
        .expect("the old chain extends while the trie still holds its root");

    sync_pbt_state(&Honest { store: source }, &target, &pivot)
        .await
        .expect("sync");

    // The row survives the install — it is durable and the nodes are not — so
    // the lookup still succeeds and only the re-check stands between here and a
    // root built on the wrong base.
    assert_eq!(
        target.get_binary_trie_root(old_head).expect("root read"),
        Some(old_root),
        "the pre-install root row is still recorded, which is what makes the guard necessary",
    );
    let error = target
        .advance_binary_trie_for_block(
            H256::repeat_byte(0xa2),
            old_head,
            &[AccountUpdate::removed(Address::repeat_byte(0x99))],
        )
        .expect_err("a snapshot install must make the old chain unextendable");
    assert!(
        matches!(
            error,
            StoreError::BinaryTrieRootNotHeld { parent_hash, parent_root }
                if parent_hash == old_head && parent_root == old_root
        ),
        "expected an explicit refusal naming the unheld root, got {error}",
    );
}

// --------------------------------------------------- the client's own rules

/// The one check the root cannot make for us.
///
/// Tested on the function rather than through a provider, and that is not a
/// shortcut: `verify_range` runs first, so a sub-index-3 leaf *injected* into a
/// response is caught as a root mismatch and never reaches this rule. The case
/// the rule exists for is a server that legitimately holds such a leaf because
/// the embedding grew — which no fixture here can build, since this client's
/// embedding is the one under test.
#[test]
fn the_key_rules_accept_only_what_this_embedding_defines() {
    let account_key = |sub_index: u8| {
        let mut key = vec![ACCOUNT_ZONE];
        key.extend_from_slice(&[0u8; ACCOUNT_KEY_LENGTH - 2]);
        key.push(sub_index);
        (key, [0u8; 32])
    };

    for sub_index in [
        BASIC_DATA_LEAF_KEY,
        CODE_HASH_LEAF_KEY,
        DELEGATION_LEAF_KEY,
        HEADER_STORAGE_OFFSET as u8,
        (HEADER_STORAGE_OFFSET + HEADER_STORAGE_SLOTS - 1) as u8,
    ] {
        validate_keys(&[account_key(sub_index)])
            .unwrap_or_else(|e| panic!("sub-index {sub_index} must be accepted: {e}"));
    }
    for sub_index in [
        3,
        (HEADER_STORAGE_OFFSET - 1) as u8,
        (HEADER_STORAGE_OFFSET + HEADER_STORAGE_SLOTS) as u8,
        255,
    ] {
        assert!(
            matches!(
                validate_keys(&[account_key(sub_index)]),
                Err(PbtSyncError::UnknownSubIndex(got)) if got == sub_index
            ),
            "sub-index {sub_index} must be refused",
        );
    }

    // Reserved zones, which no embedding this client implements can produce.
    for zone in [1u8, 2, 3, 128, 254] {
        let mut key = vec![zone];
        key.extend_from_slice(&[0u8; CODE_KEY_LENGTH - 1]);
        let result = validate_keys(&[(key, [0u8; 32])]);
        if zone == CODE_ZONE {
            assert!(result.is_ok(), "the code zone is defined");
        } else {
            assert!(
                matches!(result, Err(PbtSyncError::UnknownZone(got)) if got == zone),
                "zone {zone} must be refused, got {result:?}",
            );
        }
    }

    // Wrong widths. The resume cursor's successor function is only a successor
    // because the key set is prefix-free, so a short key is not cosmetic.
    assert!(matches!(
        validate_keys(&[(vec![STORAGE_ZONE; ACCOUNT_KEY_LENGTH], [0u8; 32])]),
        Err(PbtSyncError::BadKeyLength {
            zone: STORAGE_ZONE,
            ..
        })
    ));
    assert!(matches!(
        validate_keys(&[(vec![ACCOUNT_ZONE], [0u8; 32])]),
        Err(PbtSyncError::BadKeyLength {
            zone: ACCOUNT_ZONE,
            ..
        })
    ));
    assert!(matches!(
        validate_keys(&[(vec![], [0u8; 32])]),
        Err(PbtSyncError::UnknownZone(_))
    ));
}

/// Every leaf a real store serves passes the client's own rules. A rule that
/// rejected genuine state would turn every sync into a failure, and the
/// negative cases above cannot catch that.
#[tokio::test]
async fn a_genuine_leaf_set_passes_the_key_rules() {
    let (source, pivot) = store_with_state(0x11).await;
    let leaves = all_leaves(&source, pivot.state_root);
    assert!(leaves.iter().any(|(key, _)| key[0] == ACCOUNT_ZONE));
    assert!(leaves.iter().any(|(key, _)| key[0] == CODE_ZONE));
    assert!(leaves.iter().any(|(key, _)| key[0] == STORAGE_ZONE));
    validate_keys(&leaves).expect("genuine state must pass the client's own rules");
}
