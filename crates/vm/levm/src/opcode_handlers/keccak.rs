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
use ethrex_common::{U256, utils::u256_from_big_endian};

/// `keccak256("")` as a `U256`. Returned directly for zero-length input so we
/// skip the permutation entirely; the result is a well-known constant
/// (matches what other clients do).
const EMPTY_KECCAK_U256: U256 = U256([
    0x7bfad8045d85a470,
    0xe500b653ca82273b,
    0x927e7db2dcc703c0,
    0xc5d2460186f7233c,
]);

/// Thread-local direct-mapped cache for 64-byte `KECCAK256` inputs (Solidity mapping
/// slot derivations `keccak256(abi.encode(key, slot))`, which dominate ERC-20 / DeFi).
/// Keyed by fast 64-bit hash with exact 64-byte verification (zero false positives,
/// lock-free, fixed 384 KiB footprint). Disabled on zkVM guest targets.
#[cfg(not(any(
    feature = "sp1",
    feature = "risc0",
    feature = "zisk",
    feature = "openvm"
)))]
mod cache64 {
    use ethrex_common::{U256, utils::u256_from_big_endian};
    use ethrex_crypto::Crypto;
    use std::cell::RefCell;
    use std::sync::Mutex;

    const SLOTS: usize = 4096;

    #[derive(Clone, Copy)]
    struct Entry {
        key: [u8; 64],
        hash: U256,
        valid: bool,
    }

    impl Default for Entry {
        fn default() -> Self {
            Self {
                key: [0u8; 64],
                hash: U256::zero(),
                valid: false,
            }
        }
    }

    /// Global 1-slot parking spot so short-lived per-block execution threads
    /// reuse an already-warm 4,096-entry cache instead of reallocating and
    /// zero-initializing 384 KiB on every block.
    static SHARED_CACHE: Mutex<Option<Box<[Entry; SLOTS]>>> = Mutex::new(None);

    struct CacheGuard(Option<Box<[Entry; SLOTS]>>);

    impl CacheGuard {
        fn checkout() -> Self {
            let table = SHARED_CACHE
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
                .unwrap_or_else(|| Box::new([Entry::default(); SLOTS]));
            Self(Some(table))
        }
    }

    impl Drop for CacheGuard {
        fn drop(&mut self) {
            if let Some(table) = self.0.take()
                && let Ok(mut slot) = SHARED_CACHE.lock()
                && slot.is_none()
            {
                *slot = Some(table);
            }
        }
    }

    thread_local! {
        static CACHE: RefCell<CacheGuard> = RefCell::new(CacheGuard::checkout());
    }

    #[inline(always)]
    #[expect(
        clippy::as_conversions,
        reason = "masked to 12-bit direct-mapped cache index"
    )]
    fn index_for(input: &[u8; 64]) -> usize {
        // Mix the low/high words of both 32-byte halves (covers address + slot in Solidity mappings).
        let w0 = u64::from_ne_bytes([
            input[0], input[1], input[2], input[3], input[4], input[5], input[6], input[7],
        ]);
        let w1 = u64::from_ne_bytes([
            input[16], input[17], input[18], input[19], input[20], input[21], input[22], input[23],
        ]);
        let w2 = u64::from_ne_bytes([
            input[24], input[25], input[26], input[27], input[28], input[29], input[30], input[31],
        ]);
        let w3 = u64::from_ne_bytes([
            input[56], input[57], input[58], input[59], input[60], input[61], input[62], input[63],
        ]);
        let h = w0.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ w1.wrapping_mul(0x517c_c1b7_2722_0a95)
            ^ w2.wrapping_mul(0xbf58_476d_1ce4_e5b9)
            ^ w3.wrapping_mul(0x94d0_49bb_1331_11eb);
        ((h ^ (h >> 32)) as usize) & (SLOTS - 1)
    }

    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "idx masked to SLOTS - 1")]
    pub fn keccak256_64(bytes: &[u8; 64], crypto: &dyn Crypto) -> U256 {
        let idx = index_for(bytes);
        CACHE.with(|cell| {
            let mut guard = cell.borrow_mut();
            let Some(table) = guard.0.as_mut() else {
                return u256_from_big_endian(&crypto.keccak256(bytes));
            };
            let entry = &mut table[idx];
            if entry.valid && entry.key == *bytes {
                return entry.hash;
            }
            let hash = u256_from_big_endian(&crypto.keccak256(bytes));
            entry.key = *bytes;
            entry.hash = hash;
            entry.valid = true;
            hash
        })
    }
}

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
                .with_range(offset, len, |bytes| {
                    #[cfg(not(any(
                        feature = "sp1",
                        feature = "risc0",
                        feature = "zisk",
                        feature = "openvm"
                    )))]
                    if let Ok(fixed) = <&[u8; 64]>::try_from(bytes) {
                        return cache64::keccak256_64(fixed, crypto);
                    }
                    u256_from_big_endian(&crypto.keccak256(bytes))
                })?
        };
        vm.current_call_frame.stack.push(hash)?;

        Ok(OpcodeResult::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::constants::EMPTY_KECCAK_HASH;
    use ethrex_crypto::{Crypto, NativeCrypto};

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
