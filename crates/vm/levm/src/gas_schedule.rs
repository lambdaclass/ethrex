//! Fork-aware gas schedules for the EVM.
//!
//! This module defines gas costs that vary across Ethereum hard forks.
//! Each fork has its own `GasSchedule` with the appropriate costs.
//!
//! # Fork History (Gas-relevant changes)
//!
//! - **Frontier/Homestead**: Original costs
//! - **Tangerine Whistle (EIP-150)**: Increased IO costs (SLOAD, BALANCE, CALL, etc.)
//! - **Spurious Dragon (EIP-158/160)**: EXP cost change
//! - **Byzantium**: New precompiles
//! - **Istanbul (EIP-1884/2200)**: SLOAD increase, net gas metering
//! - **Berlin (EIP-2929)**: Cold/warm access lists
//! - **London+**: Current costs

use ethrex_common::types::Fork;

/// Gas costs that vary by fork.
///
/// This struct contains only the costs that change across forks.
/// Costs that remain constant (like arithmetic operations) are still
/// defined as constants in `gas_cost.rs`.
#[derive(Debug, Clone, Copy)]
pub struct GasSchedule {
    // Storage operations
    pub sload: u64,
    pub sstore_set: u64,
    pub sstore_reset: u64,
    pub sstore_clears_refund: i64,

    // Account access
    pub balance: u64,
    pub extcodesize: u64,
    pub extcodecopy_base: u64,
    pub extcodehash: u64,

    // Call operations (base cost, not including memory/value/etc.)
    pub call_base: u64,
    pub callcode_base: u64,
    pub delegatecall_base: u64,
    pub staticcall_base: u64,

    // Self-destruct
    pub selfdestruct: u64,
    pub selfdestruct_new_account: u64,

    // CALL to empty account with value (EIP-150)
    pub call_new_account: u64,

    // EXP operation
    pub exp_byte: u64,

    // Calldata costs
    pub calldata_zero: u64,
    pub calldata_nonzero: u64,

    // Whether this fork uses cold/warm access tracking (EIP-2929)
    pub has_access_lists: bool,

    // Cold/warm costs (only relevant if has_access_lists is true)
    pub cold_sload: u64,
    pub warm_sload: u64,
    pub cold_account_access: u64,
    pub warm_account_access: u64,

    // Whether this fork uses the 63/64 gas rule (EIP-150)
    // Before EIP-150, callers could pass all remaining gas to subcalls.
    pub has_63_64_rule: bool,
}

impl GasSchedule {
    /// Get the gas schedule for a specific fork.
    pub const fn for_fork(fork: Fork) -> &'static GasSchedule {
        match fork {
            Fork::Frontier | Fork::FrontierThawing | Fork::Homestead | Fork::DaoFork => {
                &FRONTIER_SCHEDULE
            }
            Fork::Tangerine => &TANGERINE_WHISTLE_SCHEDULE,
            Fork::SpuriousDragon => &SPURIOUS_DRAGON_SCHEDULE,
            Fork::Byzantium | Fork::Constantinople | Fork::Petersburg => &BYZANTIUM_SCHEDULE,
            Fork::Istanbul | Fork::MuirGlacier => &ISTANBUL_SCHEDULE,
            // Berlin and later use access lists
            _ => &BERLIN_SCHEDULE,
        }
    }

    /// Get SLOAD cost, considering cold/warm access for Berlin+.
    #[inline]
    pub const fn sload_cost(&self, is_cold: bool) -> u64 {
        if self.has_access_lists {
            if is_cold {
                self.cold_sload
            } else {
                self.warm_sload
            }
        } else {
            self.sload
        }
    }

    /// Get account access cost (BALANCE, EXTCODESIZE, etc.), considering cold/warm.
    #[inline]
    pub const fn account_access_cost(&self, is_cold: bool, base_cost: u64) -> u64 {
        if self.has_access_lists {
            if is_cold {
                self.cold_account_access
            } else {
                self.warm_account_access
            }
        } else {
            base_cost
        }
    }

    /// Get CALL-family base cost, considering cold/warm access.
    #[inline]
    pub const fn call_cost(&self, is_cold: bool) -> u64 {
        if self.has_access_lists {
            if is_cold {
                self.cold_account_access
            } else {
                self.warm_account_access
            }
        } else {
            self.call_base
        }
    }
}

/// Frontier/Homestead gas schedule (blocks 0 - 2,463,000)
///
/// Original Ethereum gas costs before any IO repricing.
pub static FRONTIER_SCHEDULE: GasSchedule = GasSchedule {
    sload: 50,
    sstore_set: 20000,
    sstore_reset: 5000,
    sstore_clears_refund: 15000,

    balance: 20,
    extcodesize: 20,
    extcodecopy_base: 20,
    extcodehash: 20, // Didn't exist, but set to extcodesize equivalent

    call_base: 40,
    callcode_base: 40,
    delegatecall_base: 40, // Didn't exist until Homestead
    staticcall_base: 40,   // Didn't exist until Byzantium

    selfdestruct: 0,
    selfdestruct_new_account: 0,

    // G_newaccount existed since Frontier (Yellow Paper). It's the cost for
    // CALLing an empty account with non-zero value (creating a new account).
    call_new_account: 25000,

    exp_byte: 10, // EIP-160 changed this to 50 in Spurious Dragon

    calldata_zero: 4,
    calldata_nonzero: 68,

    has_access_lists: false,
    cold_sload: 0,
    warm_sload: 0,
    cold_account_access: 0,
    warm_account_access: 0,

    has_63_64_rule: false, // EIP-150 not yet introduced
};

/// Tangerine Whistle gas schedule (EIP-150, block 2,463,000)
///
/// Major IO cost increases to prevent DoS attacks.
pub static TANGERINE_WHISTLE_SCHEDULE: GasSchedule = GasSchedule {
    sload: 200,
    sstore_set: 20000,
    sstore_reset: 5000,
    sstore_clears_refund: 15000,

    balance: 400,
    extcodesize: 700,
    extcodecopy_base: 700,
    extcodehash: 400, // Didn't exist yet

    call_base: 700,
    callcode_base: 700,
    delegatecall_base: 700,
    staticcall_base: 700, // Didn't exist yet

    selfdestruct: 5000,
    selfdestruct_new_account: 25000,

    call_new_account: 25000, // EIP-150 introduced this cost

    exp_byte: 10,

    calldata_zero: 4,
    calldata_nonzero: 68,

    has_access_lists: false,
    cold_sload: 0,
    warm_sload: 0,
    cold_account_access: 0,
    warm_account_access: 0,

    has_63_64_rule: true, // EIP-150 introduced the 63/64 rule
};

/// Spurious Dragon gas schedule (EIP-158/160, block 2,675,000)
///
/// EXP repricing (EIP-160).
pub static SPURIOUS_DRAGON_SCHEDULE: GasSchedule = GasSchedule {
    exp_byte: 50, // Changed from 10
    ..TANGERINE_WHISTLE_SCHEDULE
};

/// Byzantium gas schedule (block 4,370,000)
///
/// Same costs as Spurious Dragon, added STATICCALL and precompiles.
pub static BYZANTIUM_SCHEDULE: GasSchedule = SPURIOUS_DRAGON_SCHEDULE;

/// Istanbul gas schedule (EIP-1884/2200, block 9,069,000)
///
/// SLOAD increase, CHAINID/SELFBALANCE opcodes, net gas metering.
pub static ISTANBUL_SCHEDULE: GasSchedule = GasSchedule {
    sload: 800, // Increased from 200

    balance: 700,     // Increased from 400
    extcodehash: 700, // Increased from 400

    calldata_nonzero: 16, // Decreased from 68 (EIP-2028)

    ..SPURIOUS_DRAGON_SCHEDULE
};

/// Berlin gas schedule (EIP-2929/2930, block 12,244,000)
///
/// Introduces cold/warm access lists.
pub static BERLIN_SCHEDULE: GasSchedule = GasSchedule {
    // Base costs are 0 when using access lists; actual cost comes from cold/warm
    sload: 0,
    balance: 0,
    extcodesize: 0,
    extcodecopy_base: 0,
    extcodehash: 0,
    call_base: 0,
    callcode_base: 0,
    delegatecall_base: 0,
    staticcall_base: 0,

    sstore_set: 20000,
    sstore_reset: 2900, // Changed in EIP-2929
    sstore_clears_refund: 15000,

    selfdestruct: 5000,
    selfdestruct_new_account: 25000,

    call_new_account: 25000,

    exp_byte: 50,

    calldata_zero: 4,
    calldata_nonzero: 16,

    has_access_lists: true,
    cold_sload: 2100,
    warm_sload: 100,
    cold_account_access: 2600,
    warm_account_access: 100,

    has_63_64_rule: true,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frontier_schedule() {
        let schedule = GasSchedule::for_fork(Fork::Frontier);
        assert_eq!(schedule.sload, 50);
        assert_eq!(schedule.balance, 20);
        assert_eq!(schedule.call_base, 40);
        assert!(!schedule.has_access_lists);
    }

    #[test]
    fn test_tangerine_whistle_schedule() {
        let schedule = GasSchedule::for_fork(Fork::Tangerine);
        assert_eq!(schedule.sload, 200);
        assert_eq!(schedule.balance, 400);
        assert_eq!(schedule.call_base, 700);
    }

    #[test]
    fn test_istanbul_schedule() {
        let schedule = GasSchedule::for_fork(Fork::Istanbul);
        assert_eq!(schedule.sload, 800);
        assert_eq!(schedule.balance, 700);
        assert_eq!(schedule.calldata_nonzero, 16);
    }

    #[test]
    fn test_berlin_schedule() {
        let schedule = GasSchedule::for_fork(Fork::Berlin);
        assert!(schedule.has_access_lists);
        assert_eq!(schedule.cold_sload, 2100);
        assert_eq!(schedule.warm_sload, 100);
    }

    #[test]
    fn test_sload_cost_pre_berlin() {
        let schedule = GasSchedule::for_fork(Fork::Istanbul);
        // Pre-Berlin ignores cold/warm
        assert_eq!(schedule.sload_cost(true), 800);
        assert_eq!(schedule.sload_cost(false), 800);
    }

    #[test]
    fn test_sload_cost_berlin() {
        let schedule = GasSchedule::for_fork(Fork::Berlin);
        assert_eq!(schedule.sload_cost(true), 2100);
        assert_eq!(schedule.sload_cost(false), 100);
    }
}
