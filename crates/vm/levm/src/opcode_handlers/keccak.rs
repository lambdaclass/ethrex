//! # Keccak256 operations
//!
//! Includes the following opcodes:
//!   - `KECCAK256`

use crate::{
    errors::{OpcodeResult, VMError},
    gas_cost,
    memory::calculate_memory_size,
    opcode_handlers::OpcodeHandler,
    utils::size_offset_to_usize,
    vm::VM,
};
use ethrex_common::U256;
use ethrex_crypto::Crypto;

/// `keccak256("")` as a `U256`. Returned directly for zero-length input so we
/// skip the permutation entirely; the result is a well-known constant
/// (matches what other clients do).
const EMPTY_KECCAK_U256: U256 = U256([
    0x7bfad8045d85a470,
    0xe500b653ca82273b,
    0x927e7db2dcc703c0,
    0xc5d2460186f7233c,
]);

pub struct OpKeccak256Handler;
impl OpcodeHandler for OpKeccak256Handler {
    #[inline(always)]
    fn eval(vm: &mut VM<'_>) -> Result<OpcodeResult, VMError> {
        let [offset, len] = *vm.current_call_frame.stack.pop()?;
        let (len, offset) = size_offset_to_usize(len, offset)?;

        vm.current_call_frame
            .increase_consumed_gas(gas_cost::keccak256(
                calculate_memory_size(offset, len)?,
                vm.current_call_frame.memory.len(),
                len,
            )?)?;

        // Hash the memory range in place — `with_range` lends a borrow to keccak256
        // instead of allocating a throwaway `Bytes` copy (KECCAK256 fires ~15x/tx).
        // Bind `crypto` first so the closure doesn't capture `vm` while `memory` is
        // borrowed mutably.
        let hash = if len == 0 {
            EMPTY_KECCAK_U256
        } else {
            let crypto = vm.crypto;
            vm.current_call_frame
                .memory
                .with_range(offset, len, |bytes| keccak256_u256(bytes, crypto))?
        };
        vm.current_call_frame.stack.push(hash)?;

        Ok(OpcodeResult::Continue)
    }
}

/// Keccak-256 of `bytes` (non-empty) as a `U256`.
///
/// On the host this consults the [`keccak_cache`](crate::keccak_cache) — the
/// KECCAK256 opcode runs on a tiny, highly-repeated set of inputs, and a cache
/// probe is ~20× cheaper than the permutation. The cache is fully transparent
/// (`keccak256` is pure), so the result is identical to recomputing it.
#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
#[inline]
fn keccak256_u256(bytes: &[u8], crypto: &dyn Crypto) -> U256 {
    crate::keccak_cache::get_or_compute(bytes, crypto)
}

/// Keccak-256 of `bytes` (non-empty) as a `U256`.
///
/// Every zkVM guest (riscv32 for sp1/risc0/openvm, riscv64 for zisk) keeps the
/// opcode on its direct, provable path: the result cache is a host-only
/// optimization and must not affect guest behavior, witness output, or proving
/// cost.
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#[inline(always)]
fn keccak256_u256(bytes: &[u8], crypto: &dyn Crypto) -> U256 {
    ethrex_common::utils::u256_from_big_endian(&crypto.keccak256(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::constants::EMPTY_KECCAK_HASH;
    use ethrex_common::utils::u256_from_big_endian;
    use ethrex_crypto::NativeCrypto;

    #[test]
    fn empty_keccak_const_matches_hash() {
        let expected = u256_from_big_endian(&NativeCrypto.keccak256(&[]));
        assert_eq!(EMPTY_KECCAK_U256, expected);
    }

    #[test]
    fn empty_keccak_const_matches_common_constant() {
        // Guards against drift between this const and `EMPTY_KECCAK_HASH`
        // in `ethrex_common::constants`.
        assert_eq!(
            EMPTY_KECCAK_U256,
            u256_from_big_endian(EMPTY_KECCAK_HASH.as_bytes())
        );
    }
}
