use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

use super::shared::{k256_ecrecover, k256_recover_signer};

/// LambdaVM crypto provider.
///
/// Overrides only what LambdaVM accelerates today (Keccak-f[1600]) plus the
/// ECDSA secp256k1 methods routed through pure-Rust `k256`. Every other
/// `Crypto` method inherits the trait default, which uses vetted pure-Rust
/// crates (`ark-bn254`, `bls12_381`, `malachite`, `p256`, `sha2`, `kzg-rs`)
/// that compile to the RV64IM target.
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
        keccak256_via_lambdavm(input)
    }
}

/// Keccak-256 implemented as a sponge over LambdaVM's `keccak_permute` syscall.
///
/// Keccak-f[1600], rate 1088 bits (136 bytes), capacity 512 bits.
/// Padding: `0x01 ... 0x80` (multi-rate, last bit set). The state is a
/// 25-element u64 array; bytes are absorbed into the state via little-endian
/// XOR (matching the standard Keccak byte-to-lane mapping).
fn keccak256_via_lambdavm(input: &[u8]) -> [u8; 32] {
    const RATE: usize = 136;

    let mut state = [0u64; 25];
    let mut offset = 0;

    while input.len().saturating_sub(offset) >= RATE {
        absorb_block(&mut state, &input[offset..offset + RATE]);
        lambda_vm_syscalls::syscalls::keccak_permute(&mut state);
        offset = offset.saturating_add(RATE);
    }

    // Final block with multi-rate padding.
    let mut last = [0u8; RATE];
    let remaining = input.len().saturating_sub(offset);
    if let Some(tail) = last.get_mut(..remaining)
        && let Some(src) = input.get(offset..)
    {
        tail.copy_from_slice(src);
    }
    if let Some(b) = last.get_mut(remaining) {
        *b ^= 0x01;
    }
    if let Some(b) = last.get_mut(RATE - 1) {
        *b ^= 0x80;
    }
    absorb_block(&mut state, &last);
    lambda_vm_syscalls::syscalls::keccak_permute(&mut state);

    // Squeeze the first 32 bytes (four lanes) as little-endian.
    let mut output = [0u8; 32];
    for (i, lane) in state.iter().take(4).enumerate() {
        let bytes = lane.to_le_bytes();
        let start = i.saturating_mul(8);
        if let Some(dst) = output.get_mut(start..start.saturating_add(8)) {
            dst.copy_from_slice(&bytes);
        }
    }
    output
}

/// XOR one rate-sized block of bytes into the state lanes (little-endian).
fn absorb_block(state: &mut [u64; 25], block: &[u8]) {
    for (lane, chunk) in state.iter_mut().zip(block.chunks_exact(8)) {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(chunk);
        *lane ^= u64::from_le_bytes(buf);
    }
}
