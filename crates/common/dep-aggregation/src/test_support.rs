//! Material for exercising the leanVM backend from outside this crate.
//!
//! Building a leanSPHINCS witness means naming leanVM's key, signature and signing
//! types, and nothing outside this crate may do that -- leanVM restructures its
//! crate layout between revisions, so the blast radius of a rename has to stay at
//! one file. This module is how a test gets a real witness without taking that
//! dependency itself.
//!
//! Behind the `leanvm` feature, so a default build compiles none of it.

use ethereum_types::H256;
use ethrex_common::types::{DEPENDENCY_SCHEME_LEANSPHINCS, DependencyTriple};

use crate::DependencyWitness;
use crate::leanvm::leansphincs_verification_key_hash;

/// One leanSPHINCS dependency and the witness that discharges it.
///
/// Deterministic from `seed`: the key is derived from it, and signing in this
/// SPHINCS profile draws no randomness (the randomizer is a keyed hash of the
/// message). A test that fails intermittently against a proving system is not
/// worth having, and a failure that cannot be replayed is not worth debugging.
pub fn leansphincs_dependency_from_seed(
    seed: u8,
    message: [u8; 32],
) -> (DependencyTriple, DependencyWitness) {
    let (secret, public) = sphincs::key_gen_from_seed([seed; 32]);
    let signature = sphincs::sign(&secret, &message);

    let triple = DependencyTriple {
        scheme: DEPENDENCY_SCHEME_LEANSPHINCS,
        data_hash: H256(message),
        verification_key_hash: leansphincs_verification_key_hash(&public),
    };

    // The witness encoding this backend reads: the 32-byte flattened public key,
    // then the fixed-length signature.
    let mut witness = public.flatten().to_vec();
    witness.extend_from_slice(&signature.to_bytes());

    (triple, DependencyWitness { triple, witness })
}
