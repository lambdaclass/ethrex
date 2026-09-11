//! The witness store: what makes a declared dependency admissible, and what lets a
//! builder produce the proof its own block needs.
//!
//! A transaction declares `(scheme, data_hash, verification_key_hash)` and nothing
//! in it proves the named signature exists. The proof travels separately, so a node
//! that tracked nothing would admit claims it never checked and then build blocks it
//! cannot prove. These pin both halves of the answer.

use bytes::Bytes;
use ethrex_blockchain::dependency_witnesses::DependencyWitnessStore;
use ethrex_common::H256;
use ethrex_common::types::{DEPENDENCY_SCHEME_LEANSPHINCS, DependencyTriple};
use ethrex_dep_aggregation::{
    AggregateError, DependencyAggregator, DependencyWitness, MempoolWrapper, UnavailableAggregator,
    WrapperContent, WrapperEntry,
};

fn triple(n: u64) -> DependencyTriple {
    DependencyTriple {
        scheme: DEPENDENCY_SCHEME_LEANSPHINCS,
        data_hash: H256::from_low_u64_be(n),
        verification_key_hash: H256::from_low_u64_be(n + 1000),
    }
}

fn witness(n: u64) -> DependencyWitness {
    DependencyWitness {
        triple: triple(n),
        witness: vec![n as u8; 8],
    }
}

/// A backend that accepts every witness, so these tests exercise the store's own
/// behaviour rather than a signature check.
#[derive(Debug)]
struct CredulousAggregator;

impl DependencyAggregator for CredulousAggregator {
    fn verify(&self, _proof: &[u8], _expected: &[DependencyTriple]) -> Result<(), AggregateError> {
        Ok(())
    }

    fn aggregate(
        &self,
        _raw: &[DependencyWitness],
        _children: &[&[u8]],
        _declare: Option<&[DependencyTriple]>,
    ) -> Result<Vec<u8>, AggregateError> {
        Ok(vec![0xAB; 16])
    }

    fn verify_witness(&self, _witness: &DependencyWitness) -> Result<(), AggregateError> {
        Ok(())
    }

    fn supports_scheme(&self, scheme: u8) -> bool {
        scheme == DEPENDENCY_SCHEME_LEANSPHINCS
    }

    fn aggregated_vk(&self) -> H256 {
        H256::zero()
    }

    fn name(&self) -> &'static str {
        "credulous"
    }
}

/// Membership has to mean "this node established this claim", so verification
/// happens inside the store rather than at its call sites. A backend that refuses
/// leaves the store empty.
#[test]
fn a_witness_the_backend_refuses_is_not_kept() {
    let store = DependencyWitnessStore::new(8);
    assert!(matches!(
        store.insert_verified(&UnavailableAggregator, &witness(1)),
        Err(AggregateError::NoBackend)
    ));
    assert!(store.is_empty());
    assert!(!store.holds(&triple(1)));
}

#[test]
fn a_verified_witness_becomes_admissible() {
    let store = DependencyWitnessStore::new(8);
    store
        .insert_verified(&CredulousAggregator, &witness(1))
        .expect("a backend that accepts the witness keeps it");
    assert!(store.holds(&triple(1)));
    assert!(!store.holds(&triple(2)), "and only that one");
}

/// A builder aggregating over a partial set would publish a proof covering less than
/// the block declares, which fails rule 2 exactly as surely as publishing nothing.
/// So a miss is a miss for the whole request.
#[test]
fn a_partial_witness_set_yields_nothing() {
    let store = DependencyWitnessStore::new(8);
    store
        .insert_verified(&CredulousAggregator, &witness(1))
        .unwrap();

    assert!(store.witnesses_for(&[triple(1)]).is_some());
    assert!(
        store.witnesses_for(&[triple(1), triple(2)]).is_none(),
        "one missing witness makes the whole set unusable"
    );
}

/// Entries arrive from the network, so the store is bounded: a peer that could add
/// without limit could exhaust memory for the price of signing.
///
/// `Ok` means stored, under eviction pressure too. A success that silently kept
/// nothing would leave the caller believing a dependency had become admissible when
/// it had not.
#[test]
fn the_store_is_bounded_and_success_means_stored() {
    let store = DependencyWitnessStore::new(4);
    for n in 0..16 {
        store
            .insert_verified(&CredulousAggregator, &witness(n))
            .expect("the backend accepts every witness here");
        assert!(
            store.holds(&triple(n)),
            "witness {n} reported stored but is not there"
        );
        assert!(store.len() <= 4, "the bound holds after every insert");
    }
}

/// Re-inserting a dependency already held must not evict anything to make room for
/// something already present.
#[test]
fn reinserting_a_held_dependency_evicts_nothing() {
    let store = DependencyWitnessStore::new(2);
    store
        .insert_verified(&CredulousAggregator, &witness(1))
        .unwrap();
    store
        .insert_verified(&CredulousAggregator, &witness(2))
        .unwrap();
    assert_eq!(store.len(), 2);

    store
        .insert_verified(&CredulousAggregator, &witness(1))
        .unwrap();
    assert!(store.holds(&triple(1)) && store.holds(&triple(2)));
    assert_eq!(store.len(), 2);
}

/// A store that can hold nothing would make every dependency permanently
/// inadmissible with nothing reporting a problem, so neither the default nor a zero
/// capacity may produce one.
#[test]
fn a_store_always_has_room_for_at_least_one_witness() {
    for store in [
        DependencyWitnessStore::default(),
        DependencyWitnessStore::new(0),
    ] {
        store
            .insert_verified(&CredulousAggregator, &witness(1))
            .expect("verification succeeded, so the witness must be kept");
        assert!(store.holds(&triple(1)));
    }
}

/// The wrapper object's purpose: mode 0 carries individually verifiable proofs, and
/// without somewhere to keep them they would be checked and then thrown away.
#[test]
fn a_mode_zero_wrapper_yields_reusable_witnesses() {
    let deps = vec![triple(1), triple(2)];
    let wrapper = MempoolWrapper {
        transactions: vec![WrapperEntry::Hash(H256::from_low_u64_be(9))],
        content: WrapperContent::Direct {
            deps: deps.clone(),
            proofs: vec![vec![1u8; 4], vec![2u8; 4]],
        },
    };

    // Whatever the wrapper's own receive checks decide, the shape a node keeps from
    // it is per-dependency material -- one witness per declared dependency.
    let WrapperContent::Direct { deps, proofs } = &wrapper.content else {
        unreachable!("built as mode 0")
    };
    assert_eq!(deps.len(), proofs.len());

    let store = DependencyWitnessStore::new(8);
    for (triple, proof) in deps.iter().zip(proofs) {
        store
            .insert_verified(
                &CredulousAggregator,
                &DependencyWitness {
                    triple: *triple,
                    witness: proof.clone(),
                },
            )
            .unwrap();
    }
    assert!(deps.iter().all(|d| store.holds(d)));
}

/// A mode-1 wrapper carries one recursive proof over the whole set. It discharges
/// those dependencies, but cannot be split back into per-claim material for a later,
/// different set -- so nothing reusable comes out of it.
#[test]
fn a_mode_one_wrapper_yields_no_reusable_witnesses() {
    let wrapper = MempoolWrapper {
        transactions: vec![WrapperEntry::Hash(H256::from_low_u64_be(9))],
        content: WrapperContent::Recursive {
            deps: vec![triple(1)],
            recursive_stark: Bytes::from_static(b"one proof over the set").to_vec(),
        },
    };
    assert!(
        matches!(wrapper.content, WrapperContent::Recursive { .. }),
        "there is no per-dependency material to keep"
    );
}
