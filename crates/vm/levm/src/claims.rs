//! # EIP-8141 claim verification
//!
//! A frame transaction's claims are satisfied by witnesses that travel beside
//! it. This module checks one witness against one resolved claim, a
//! [`Dependency`]:
//!
//!   - the three protocol verifiers (`SECP256K1_VERIFIER`, `P256_VERIFIER`,
//!     `LEANSPHINCS_VERIFIER`) are native functions evaluated here, outside the
//!     EVM. They are not precompiles: no EVM code can call them;
//!   - any other verifier is an active precompile or a deployed contract, run in
//!     the pure context (EIP-8141 §Pure Context) against the state it is given.

use crate::{
    db::gen_db::GeneralizedDatabase,
    environment::{EVMConfig, Environment},
    errors::VMError,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use bytes::Bytes;
use ethrex_common::types::{Dependency, LegacyTransaction, ProtocolVerifier, Transaction, TxKind};
use ethrex_crypto::Crypto;

/// Whether this build links the native leanSPHINCS verifier (the `leansphincs`
/// feature). A build without it, such as a zkVM guest, cannot check a
/// `LEANSPHINCS_VERIFIER` claim and treats every one as unsatisfied.
pub const LEANSPHINCS_VERIFIER_LINKED: bool = cfg!(feature = "leansphincs");

/// secp256k1 group order `n`.
const SECP256K1_N: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe,
    0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
];
/// secp256k1 `n // 2`.
const SECP256K1_N_HALF: [u8; 32] = [
    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b, 0x20, 0xa0,
];
/// P-256 (secp256r1) group order `n`.
const SECP256R1_N: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xbc, 0xe6, 0xfa, 0xad, 0xa7, 0x17, 0x9e, 0x84, 0xf3, 0xb9, 0xca, 0xc2, 0xfc, 0x63, 0x25, 0x51,
];
/// P-256 (secp256r1) `n // 2`.
const SECP256R1_N_HALF: [u8; 32] = [
    0x7f, 0xff, 0xff, 0xff, 0x80, 0x00, 0x00, 0x00, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xde, 0x73, 0x7d, 0x56, 0xd3, 0x8b, 0xcf, 0x42, 0x79, 0xdc, 0xe5, 0x61, 0x7e, 0x31, 0x92, 0xa8,
];

/// True when the big-endian 32-byte scalar is in `(0, upper)`.
fn scalar_below(scalar: &[u8], upper: &[u8; 32]) -> bool {
    scalar.iter().any(|&b| b != 0) && scalar < &upper[..]
}

/// True when the big-endian 32-byte scalar is in `(0, upper]`.
fn scalar_at_most(scalar: &[u8], upper: &[u8; 32]) -> bool {
    scalar.iter().any(|&b| b != 0) && scalar <= &upper[..]
}

/// Whether `key` is `address` left-padded to 32 bytes.
fn key_is_address(key: &[u8], address: &[u8]) -> bool {
    key.len() == 32
        && address.len() == 20
        && key.iter().take(12).all(|&b| b == 0)
        && key.get(12..) == Some(address)
}

/// EIP-8141 `secp256k1_verifier`.
///
/// Input: `key (32 bytes) || msg (32 bytes) || v (1 byte) || r (32 bytes) || s (32
/// bytes)`, where `key` is the signer's address left-padded to 32 bytes. Returns
/// whether the signature is a canonical one of `msg` by `key`: `v` is the bare
/// recovery id (0 or 1), `0 < r < n` and `0 < s <= n // 2`, so each signature has
/// one encoding.
pub fn secp256k1_verifier(input: &[u8], crypto: &dyn Crypto) -> bool {
    if input.len() != 129 {
        return false;
    }
    #[expect(clippy::indexing_slicing, reason = "length checked")]
    let (key, msg, v, r, s) = (
        &input[0..32],
        &input[32..64],
        input[64],
        &input[65..97],
        &input[97..129],
    );
    if v > 1 || !scalar_below(r, &SECP256K1_N) || !scalar_at_most(s, &SECP256K1_N_HALF) {
        return false;
    }
    let Ok(msg) = <[u8; 32]>::try_from(msg) else {
        return false;
    };
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(r);
    signature[32..].copy_from_slice(s);
    let Ok(public_key_hash) = crypto.secp256k1_ecrecover(&signature, v, &msg) else {
        return false;
    };
    // The signer's address is the last 20 bytes of the hash of its public key.
    public_key_hash
        .get(12..)
        .is_some_and(|address| key_is_address(key, address))
}

/// EIP-8141 `p256_verifier`.
///
/// Input: `key (32 bytes) || msg (32 bytes) || r || s || qx || qy` (32 bytes
/// each), where `key` is the public key's address, `keccak256(qx || qy)[12:]`,
/// left-padded to 32 bytes. Returns whether the signature is a canonical one of
/// `msg` under that public key (`0 < r < n`, `0 < s <= n // 2`). `P256VERIFY`
/// itself accepts a high `s`, so a signer that produces one normalizes it first.
pub fn p256_verifier(input: &[u8], crypto: &dyn Crypto) -> bool {
    if input.len() != 192 {
        return false;
    }
    #[expect(clippy::indexing_slicing, reason = "length checked")]
    let (key, msg, r, s, public_key) = (
        &input[0..32],
        &input[32..64],
        &input[64..96],
        &input[96..128],
        &input[128..192],
    );
    if !scalar_below(r, &SECP256R1_N) || !scalar_at_most(s, &SECP256R1_N_HALF) {
        return false;
    }
    let public_key_hash = crypto.keccak256(public_key);
    if !public_key_hash
        .get(12..)
        .is_some_and(|address| key_is_address(key, address))
    {
        return false;
    }
    let (Ok(msg), Ok(public_key)) = (<[u8; 32]>::try_from(msg), <[u8; 64]>::try_from(public_key))
    else {
        return false;
    };
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(r);
    signature[32..].copy_from_slice(s);
    crypto.secp256r1_verify(&msg, &signature, &public_key)
}

/// Length of the input of [`leansphincs_verifier`]: the 64-byte claim followed by
/// a 7740-byte signature.
pub const LEANSPHINCS_VERIFIER_INPUT_LENGTH: usize = 7804;

/// EIP-8141 `leansphincs_verifier`.
///
/// Input: `key (32 bytes) || msg (32 bytes) || signature (7740 bytes)`, where
/// `key` is the leanSPHINCS public key itself, its 16-byte `PK.seed` followed by
/// its 16-byte `PK.root`. Returns whether `signature` is a valid signature of
/// `msg` under `key`.
///
/// The verifier is linked only into native builds (the `leansphincs` feature). A
/// build without it returns `false` on every input, so it never vouches for a
/// signature it did not check (see [`LEANSPHINCS_VERIFIER_LINKED`]).
pub fn leansphincs_verifier(input: &[u8]) -> bool {
    if input.len() != LEANSPHINCS_VERIFIER_INPUT_LENGTH {
        return false;
    }
    #[cfg(feature = "leansphincs")]
    {
        #[expect(clippy::indexing_slicing, reason = "length checked")]
        let (key, msg, signature) = (&input[0..32], &input[32..64], &input[64..]);
        let (Ok(key), Ok(msg)) = (<&[u8; 32]>::try_from(key), <&[u8; 32]>::try_from(msg)) else {
            return false;
        };
        ethrex_crypto::leansphincs::leansphincs_verify(key, msg, signature)
    }
    #[cfg(not(feature = "leansphincs"))]
    {
        false
    }
}

/// Verifies `witness` against a dependency that names a protocol verifier, with
/// the verifier's native function and without reading any state. `None` when the
/// dependency names any other verifier.
pub fn verify_protocol_dependency(
    dependency: &Dependency,
    witness: &[u8],
    crypto: &dyn Crypto,
) -> Option<bool> {
    let verifier = dependency.protocol_verifier()?;
    if u64::try_from(witness.len()) != Ok(dependency.witness_len) {
        return Some(false);
    }
    let input = dependency.verifier_input(witness);
    Some(match verifier {
        ProtocolVerifier::Secp256k1 => secp256k1_verifier(&input, crypto),
        ProtocolVerifier::P256 => p256_verifier(&input, crypto),
        ProtocolVerifier::LeanSphincs => leansphincs_verifier(&input),
    })
}

/// EIP-8141 `verify_dependency`: whether `witness` satisfies `dependency`.
///
/// The witness must have exactly the dependency's `witness_len`. A protocol
/// verifier's dependency is then checked natively. Any other verifier is called
/// in the pure context with `key || msg || witness` and the dependency's
/// `gas_limit`, against the state `db` exposes: the pre-state of the block that
/// includes the transaction, or the current state when a node admits it to its
/// pool. `config` is the EVM configuration of that block.
///
/// The pure context reads only the verifier's own code, so it leaves `db`'s
/// state unchanged; the verifier's account may remain loaded in its cache.
pub fn verify_dependency(
    dependency: &Dependency,
    witness: &[u8],
    db: &mut GeneralizedDatabase,
    config: EVMConfig,
    vm_type: VMType,
    crypto: &dyn Crypto,
) -> Result<bool, VMError> {
    if let Some(satisfied) = verify_protocol_dependency(dependency, witness, crypto) {
        return Ok(satisfied);
    }
    if u64::try_from(witness.len()) != Ok(dependency.witness_len) {
        return Ok(false);
    }

    // The pure context halts on every instruction that reads the transaction or
    // the block, so the transaction and the environment the VM is built from are
    // placeholders: only the fork and the verifier's call matter.
    let tx = Transaction::LegacyTransaction(LegacyTransaction {
        to: TxKind::Call(dependency.verifier),
        ..Default::default()
    });
    let env = Environment {
        config,
        ..Default::default()
    };
    let mut vm = VM::new(
        env,
        db,
        &tx,
        LevmCallTracer::disabled(),
        vm_type,
        crypto,
        None,
    )?;
    vm.run_pure_verifier(
        dependency.verifier,
        Bytes::from(dependency.verifier_input(witness)),
        dependency.gas_limit,
    )
}
