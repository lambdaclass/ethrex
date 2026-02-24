//! Adapter layer bridging LEVM state ↔ revmc/revm type models.
//!
//! revmc compiles EVM bytecode using revm's type system (`Gas`, `Interpreter`,
//! `SharedMemory`, `Host`). LEVM has its own types (`CallFrame`, `Memory`,
//! `Stack`, `Substate`). This module converts between them.
//!
//! # Stack Direction
//!
//! LEVM's stack grows **downward** (offset decrements on push), while revm's
//! stack grows **upward** (pointer increments on push). The adapter copies
//! active entries and reverses the order.

use crate::error::JitError;

use ethrex_common::types::Fork;
use revm_interpreter::{Gas, SharedMemory};
use revm_primitives::{hardfork::SpecId, U256 as RevmU256};

/// Convert LEVM `Fork` to revm `SpecId`.
///
/// Maps the ethrex fork enum to the corresponding revm spec identifier.
/// Forks beyond Osaka (BPO1-5, Amsterdam) map to `SpecId::OSAKA` until
/// revm adds dedicated spec IDs for them.
pub fn fork_to_spec_id(fork: Fork) -> SpecId {
    match fork {
        Fork::Frontier => SpecId::FRONTIER,
        Fork::FrontierThawing => SpecId::FRONTIER_THAWING,
        Fork::Homestead => SpecId::HOMESTEAD,
        Fork::DaoFork => SpecId::DAO_FORK,
        Fork::Tangerine => SpecId::TANGERINE,
        Fork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        Fork::Byzantium => SpecId::BYZANTIUM,
        Fork::Constantinople => SpecId::CONSTANTINOPLE,
        Fork::Petersburg => SpecId::PETERSBURG,
        Fork::Istanbul => SpecId::ISTANBUL,
        Fork::MuirGlacier => SpecId::MUIR_GLACIER,
        Fork::Berlin => SpecId::BERLIN,
        Fork::London => SpecId::LONDON,
        Fork::ArrowGlacier => SpecId::ARROW_GLACIER,
        Fork::GrayGlacier => SpecId::GRAY_GLACIER,
        Fork::Paris => SpecId::MERGE,
        Fork::Shanghai => SpecId::SHANGHAI,
        Fork::Cancun => SpecId::CANCUN,
        Fork::Prague => SpecId::PRAGUE,
        Fork::Osaka => SpecId::OSAKA,
        Fork::BPO1 => SpecId::OSAKA,
        Fork::BPO2 => SpecId::OSAKA,
        Fork::BPO3 => SpecId::OSAKA,
        Fork::BPO4 => SpecId::OSAKA,
        Fork::BPO5 => SpecId::OSAKA,
        Fork::Amsterdam => SpecId::OSAKA,
    }
}

/// Convert LEVM `U256` to revm `U256`.
///
/// Both are 256-bit unsigned integers but from different crate ecosystems.
/// LEVM uses `ethereum_types::U256` (4×u64, little-endian limbs).
/// revm uses `ruint::Uint<256, 4>` (4×u64, little-endian limbs).
/// The underlying representation is the same, so we can convert via limbs.
pub fn levm_u256_to_revm(val: &ethrex_common::U256) -> RevmU256 {
    let limbs = val.0;
    RevmU256::from_limbs(limbs)
}

/// Convert revm `U256` to LEVM `U256`.
pub fn revm_u256_to_levm(val: &RevmU256) -> ethrex_common::U256 {
    let limbs = val.as_limbs();
    ethrex_common::U256([limbs[0], limbs[1], limbs[2], limbs[3]])
}

/// Convert LEVM `H256` to revm `B256`.
pub fn levm_h256_to_revm(val: &ethrex_common::H256) -> revm_primitives::B256 {
    revm_primitives::B256::from_slice(val.as_bytes())
}

/// Convert revm `B256` to LEVM `H256`.
pub fn revm_b256_to_levm(val: &revm_primitives::B256) -> ethrex_common::H256 {
    ethrex_common::H256::from_slice(val.as_slice())
}

/// Convert LEVM `Address` (H160) to revm `Address`.
pub fn levm_address_to_revm(val: &ethrex_common::Address) -> revm_primitives::Address {
    revm_primitives::Address::from_slice(val.as_bytes())
}

/// Convert revm `Address` to LEVM `Address`.
pub fn revm_address_to_levm(val: &revm_primitives::Address) -> ethrex_common::Address {
    ethrex_common::Address::from_slice(val.as_slice())
}

/// Convert LEVM gas_remaining (i64) to revm Gas.
///
/// LEVM uses i64 for gas (can go negative on underflow checks).
/// revm uses Gas { remaining: u64, ... }. We clamp negative values to 0.
pub fn levm_gas_to_revm(gas_remaining: i64, gas_limit: u64) -> Gas {
    #[expect(clippy::as_conversions, reason = "i64→u64 with clamping")]
    let remaining = if gas_remaining < 0 {
        0u64
    } else {
        gas_remaining as u64
    };
    let mut gas = Gas::new(gas_limit);
    // Spend the difference between limit and remaining
    let spent = gas_limit.saturating_sub(remaining);
    let _ = gas.record_cost(spent);
    gas
}

/// Convert revm Gas back to LEVM gas_remaining (i64).
///
/// Clamps to `i64::MAX` if the remaining gas exceeds `i64::MAX`,
/// which is safe because LEVM never allocates more than `i64::MAX` gas.
pub fn revm_gas_to_levm(gas: &Gas) -> i64 {
    i64::try_from(gas.remaining()).unwrap_or(i64::MAX)
}

/// Build a revm `SharedMemory` from LEVM memory contents.
///
/// LEVM's Memory uses `Rc<RefCell<Vec<u8>>>` with base offsets for nested calls.
/// We extract the active memory slice and copy it into a SharedMemory.
pub fn levm_memory_to_revm(memory: &mut ethrex_levm::memory::Memory) -> SharedMemory {
    let mut shared = SharedMemory::new();
    let mem_len = memory.len();
    if mem_len > 0
        && let Ok(data) = memory.load_range(0, mem_len)
    {
        // SharedMemory needs to be resized, then we copy data in
        shared.resize(data.len());
        shared.slice_mut(0, data.len()).copy_from_slice(&data);
    }
    shared
}

/// Copy revm SharedMemory contents back to LEVM Memory.
///
/// This is called after JIT execution to sync memory state back.
pub fn revm_memory_to_levm(
    shared: &SharedMemory,
    memory: &mut ethrex_levm::memory::Memory,
) -> Result<(), JitError> {
    let data = shared.context_memory();
    memory
        .store_data(0, &data)
        .map_err(|e| JitError::AdapterError(format!("memory write-back failed: {e:?}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u256_roundtrip() {
        let levm_val = ethrex_common::U256::from(42u64);
        let revm_val = levm_u256_to_revm(&levm_val);
        let back = revm_u256_to_levm(&revm_val);
        assert_eq!(levm_val, back);
    }

    #[test]
    fn test_u256_max_roundtrip() {
        let levm_val = ethrex_common::U256::MAX;
        let revm_val = levm_u256_to_revm(&levm_val);
        let back = revm_u256_to_levm(&revm_val);
        assert_eq!(levm_val, back);
    }

    #[test]
    fn test_h256_roundtrip() {
        let levm_val = ethrex_common::H256::from_low_u64_be(12345);
        let revm_val = levm_h256_to_revm(&levm_val);
        let back = revm_b256_to_levm(&revm_val);
        assert_eq!(levm_val, back);
    }

    #[test]
    fn test_address_roundtrip() {
        let levm_val = ethrex_common::Address::from_low_u64_be(0xDEAD);
        let revm_val = levm_address_to_revm(&levm_val);
        let back = revm_address_to_levm(&revm_val);
        assert_eq!(levm_val, back);
    }

    #[test]
    fn test_gas_conversion() {
        let gas = levm_gas_to_revm(500, 1000);
        assert_eq!(gas.remaining(), 500);
        assert_eq!(revm_gas_to_levm(&gas), 500);
    }

    #[test]
    fn test_gas_negative_clamps_to_zero() {
        let gas = levm_gas_to_revm(-100, 1000);
        assert_eq!(gas.remaining(), 0);
    }

    #[test]
    fn test_gas_overflow_clamps_to_i64_max() {
        let gas = Gas::new(u64::MAX);
        assert_eq!(revm_gas_to_levm(&gas), i64::MAX);
    }
}
