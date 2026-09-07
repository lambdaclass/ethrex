//! Cross-implementation parity for EIP-7916 progressive merkleization.
//!
//! execution-specs at `3c3b6f4af` (#3248, "change StatelessInput SSZ
//! serialization to be EIP-7688 aligned") moves the stateless payload
//! containers to `ProgressiveList` and `ProgressiveContainer`. ethrex computes
//! those roots with `libssz-merkle`, the spec with `remerkleable` — two
//! independent implementations of the same EIP. Nothing in the type system
//! forces them to agree, and a divergence would silently change every
//! `new_payload_request_root` the guest commits to.
//!
//! The expected roots below were produced by remerkleable at that pinned
//! commit. Sizes 0, 1, 2, 5 and 21 straddle the progressive subtree boundaries
//! (leaf counts grow 1, 4, 16, 64, …), so they exercise the recursion rather
//! than just the base case.
//!
//! Regenerate with, from an execution-specs checkout at the pin:
//!
//! ```text
//! PYTHONPATH=src python -c "
//! from remerkleable.progressive import ProgressiveList
//! from remerkleable.basic import uint64
//! for n in (0,1,2,5,21):
//!     v = ProgressiveList[uint64](*[uint64(i+1) for i in range(n)])
//!     print(n, v.hash_tree_root().hex())"
//! ```

// `Crypto` is imported for its `sha256` method, which the bridge below calls.
use ethrex_crypto::{Crypto, NativeCrypto};
use libssz_merkle::{HashTreeRoot, Sha256Hasher};
use libssz_types::ProgressiveList;

/// Bridges `ethrex-crypto` to the SSZ hasher, mirroring what the guest does so
/// this test exercises the same hashing path production code uses.
struct CryptoHasher(NativeCrypto);

impl Sha256Hasher for CryptoHasher {
    fn hash(&self, data: &[u8]) -> [u8; 32] {
        self.0.sha256(data)
    }
}

/// remerkleable `ProgressiveList[uint64]` roots at execution-specs `3c3b6f4af`.
const REMERKLEABLE_UINT64_ROOTS: &[(usize, &str)] = &[
    (
        0,
        "f5a5fd42d16a20302798ef6ed309979b43003d2320d9f0e8ea9831a92759fb4b",
    ),
    (
        1,
        "905efb51c2764c2c7a4efb0548e372569df06db82115c3b1896c186632f3fe5b",
    ),
    (
        2,
        "4250789d7838bee417a2b0d7639d928b05e8b75f1fc59588a4301b6e8f70ba58",
    ),
    (
        5,
        "29918e0447260511bc5be0f7dbb9817201e16e30c56af228b9cb931a16e8799d",
    ),
    (
        21,
        "ed360c03ecbdfbb6f4b1cf5d9cbf6887038423e31121700797de968a9969aaed",
    ),
];

/// Guards the EIP-7916 progressive child order against `remerkleable`.
///
/// This failed against `libssz-merkle 0.2.2`, which had the two children of each
/// progressive subtree swapped, making every progressive root ethrex computed
/// disagree with the spec. Fixed upstream in libssz 0.3.0; the root workspace
/// pins the branch carrying it. Keep this test passing rather than deleting it:
/// it is what pins the pin.
///
/// `merkleize_progressive_inner` (libssz-merkle-0.2.2/src/lib.rs:151) ends with
/// `hash_nodes(hasher, &rest, &subtree)` — remainder left, subtree right — and a
/// comment claiming parity with ethereum/remerkleable `667eab00`. Current
/// remerkleable does the opposite (`remerkleable/progressive.py:29`):
///
/// ```text
/// PairNode(
///     subtree_fill_to_contents(nodes[:base_size], depth),   # LEFT  = subtree
///     subtree_fill_progressive(nodes[base_size:], depth+2), # RIGHT = remainder
/// )
/// ```
///
/// Proven by hand for a single `uint64(1)`, where the packed chunk is
/// `01 00.. || 24 zero bytes` and `Z` is a zero node:
///
/// ```text
/// mix_in_length(hash(Z || chunk), 1) = 573a032d…  <- libssz's output
/// mix_in_length(hash(chunk || Z), 1) = 905efb51…  <- the spec's root
/// ```
///
/// The leaf-count progression agrees (libssz's `num_leaves *= 4` matches
/// remerkleable's `depth + 2`); only the child order differs. The fix is to swap
/// that one `hash_nodes` argument pair in `libssz-merkle`.
///
/// `SszExecutionPayload` and `SszExecutionRequests` are progressive containers,
/// so every `new_payload_request_root` the guest commits flows through this.
#[test]
fn progressive_list_roots_match_remerkleable() {
    let hasher = CryptoHasher(NativeCrypto);

    for &(n, expected_hex) in REMERKLEABLE_UINT64_ROOTS {
        let list: ProgressiveList<u64> = (1..=n as u64).collect::<Vec<u64>>().into();
        let got = list.hash_tree_root(&hasher);
        let expected: [u8; 32] = hex::decode(expected_hex)
            .expect("static hex is valid")
            .try_into()
            .expect("32 bytes");

        assert_eq!(
            got,
            expected,
            "progressive-list root diverged from remerkleable at n={n}:\n  \
             libssz       {}\n  remerkleable {expected_hex}",
            hex::encode(got),
        );
    }
}

/// The shape #3248 actually uses: `SszExecutionPayload` is
/// `ProgressiveContainer(active_fields=[1; 19])` and `SszExecutionRequests` is
/// `[1; 5]`. There is no libssz type for this, so production code composes
/// `mix_in_active_fields(merkleize_progressive(field_roots), &[true; N])` by hand
/// — and so does this test, which is why it is the one that matters most.
///
/// This is the case that pins the composition: `mix_in_active_fields`,
/// `merkleize`, `pack` and `mix_in_length` were each verified against
/// remerkleable independently, so a failure here isolates to the progressive
/// child order that `libssz-merkle 0.2.2` had reversed.
///
/// Reference roots from remerkleable at the pin:
///
/// ```text
/// PC(active_fields=[1,1,1], x=1, y=2, z=3)             -> e9ff4f89…
/// PC(active_fields=[1;5],   a=1, b=2, c=3, d=4, e=5)   -> 5a167eaf…
/// ```
#[test]
fn progressive_container_roots_match_remerkleable() {
    use libssz_merkle::{Node, merkleize_progressive, mix_in_active_fields};

    let hasher = CryptoHasher(NativeCrypto);

    for (n, expected_hex) in [
        (
            3usize,
            "e9ff4f8918d6640489dbb084574dbaf57cc7e8a5b4cd1fcd904a7af79a0dc89d",
        ),
        (
            5,
            "5a167eafdb77037933df6b87009c5d116ef0b6e6800d37b2e70693875b64318d",
        ),
    ] {
        // Each field is a uint64 i, whose root is the value LE in a zero-padded chunk.
        let field_roots: Vec<Node> = (1..=n as u64)
            .map(|i| {
                let mut chunk = [0u8; 32];
                chunk[..8].copy_from_slice(&i.to_le_bytes());
                chunk
            })
            .collect();

        let root = merkleize_progressive(&hasher, &field_roots);
        let got = mix_in_active_fields(&hasher, &root, &vec![true; n]);
        let expected: [u8; 32] = hex::decode(expected_hex)
            .expect("static hex is valid")
            .try_into()
            .expect("32 bytes");

        assert_eq!(
            got,
            expected,
            "progressive-container root diverged at {n} fields:\n  \
             libssz       {}\n  remerkleable {expected_hex}",
            hex::encode(got),
        );
    }
}

/// The empty list is the one case a broken implementation is most likely to get
/// right by accident (zero hash), so assert the non-empty cases move the root.
#[test]
fn progressive_list_roots_are_distinct_per_length() {
    let hasher = CryptoHasher(NativeCrypto);
    let mut seen = Vec::new();

    for n in 0..=21u64 {
        let list: ProgressiveList<u64> = (1..=n).collect::<Vec<u64>>().into();
        let root = list.hash_tree_root(&hasher);
        assert!(
            !seen.contains(&root),
            "length {n} collided with an earlier length; \
             mix_in_length is not being applied"
        );
        seen.push(root);
    }
}
