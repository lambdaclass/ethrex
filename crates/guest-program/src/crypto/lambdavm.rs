use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::shared::{
    bls12_381_fp_to_g1, bls12_381_fp2_to_g2, bls12_381_g1_add, bls12_381_g1_msm, bls12_381_g2_add,
    bls12_381_g2_msm, bls12_381_pairing_check, k256_ecrecover, k256_recover_signer,
};

/// LambdaVM crypto provider.
///
/// Overrides only what LambdaVM accelerates today (Keccak-f[1600], via the
/// upstream `lambda-vm-syscalls` sponge over the `keccak_permute` syscall),
/// the ECDSA secp256k1 methods routed through pure-Rust `k256`, and the BLS12-381
/// (EIP-2537) methods routed through the portable pure-Rust `bls12_381`
/// backend — the trait defaults for those return `Unsupported` when `blst` is
/// compiled out of guest builds. Every other `Crypto` method inherits the
/// trait default, which uses vetted pure-Rust crates (`ark-bn254`,
/// `malachite`, `p256`, `sha2`) that compile to the RV64IM target.
///
/// KZG point-evaluation is unsupported in the LambdaVM guest: `kzg-rs` pulls
/// in SP1-specific symbols that do not link for this target, so the trait
/// default returns an error — same stance as the LambdaVM team's own PoC.
///
/// Routing ECDSA through the `super::shared` helpers (rather than the trait
/// default) matches the OpenVM adapter and is forward-compatible: when
/// LambdaVM later patches `k256` for circuit acceleration via
/// `[patch.crates-io]` in `bin/lambdavm/Cargo.toml`, the override will pick
/// up the patched implementation transparently.
#[derive(Debug)]
pub struct LambdaVmCrypto;

impl Crypto for LambdaVmCrypto {
    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        k256_ecrecover(sig, recid, msg)
    }

    fn recover_signer(&self, sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
        k256_recover_signer(sig, msg)
    }

    fn keccak256(&self, input: &[u8]) -> [u8; 32] {
        // Delegated to the upstream, host-tested sponge in `lambda-vm-syscalls`
        // (in-place absorption over the `keccak_permute` syscall) rather than a
        // hand-rolled copy here.
        lambda_vm_syscalls::keccak::keccak256(input)
    }

    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        bls12_381_g1_add(a, b)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        bls12_381_g1_msm(pairs)
    }

    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        bls12_381_g2_add(a, b)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        bls12_381_g2_msm(pairs)
    }

    #[allow(clippy::type_complexity)]
    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, CryptoError> {
        bls12_381_pairing_check(pairs)
    }

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        bls12_381_fp_to_g1(fp)
    }

    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        bls12_381_fp2_to_g2(fp2)
    }
}
