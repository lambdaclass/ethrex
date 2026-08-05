//! bsc-geth's `NanoBlackList` (`core/types/blacklist.go`).
//!
//! Introduced after the June 2022 Tendermint IAVL Merkle-proof verification
//! exploit. bsc-geth rejects these addresses in two independent places:
//!
//! 1. As an **EIP-7702 authorization authority** — `validateAuthorization`
//!    (`core/state_transition.go`) rejects a recovered authority in this list
//!    immediately after signature recovery, *before* the code/nonce checks and
//!    *before* the existing-authority gas refund. The whole authorization tuple
//!    is then skipped (no nonce bump, no delegation, no refund). This check is
//!    **not** gated by the Nano fork, so it applies on any Prague+ block.
//! 2. As a **transaction sender or recipient** — gated by the Nano fork
//!    (`core/state_transition.go`, the `IsNano` branch). Not wired up here yet;
//!    it only affects the two frozen exploiter accounts.
//!
//! The third entry is a Chapel testnet "Test Account" (per bsc-geth's own
//! comment) exercising the authority-blacklist path — it is the authority in
//! Chapel block 120,342,702 tx 0, which is what surfaced this rule for ethrex.

use ethereum_types::H160;
use ethrex_common::Address;

/// The `NanoBlackList` addresses, byte-for-byte from bsc-geth
/// `core/types/blacklist.go`.
pub const BSC_NANO_BLACKLIST: [Address; 3] = [
    H160([
        0x48, 0x9a, 0x87, 0x56, 0xc1, 0x8c, 0x0b, 0x8b, 0x24, 0xec, 0x2a, 0x2b, 0x9f, 0xf3, 0xd4,
        0xd4, 0x47, 0xf7, 0x9b, 0xec,
    ]), // 0x489A8756C18C0b8B24EC2a2b9FF3D4d447F79BEc
    H160([
        0xfd, 0x60, 0x42, 0xdf, 0x3d, 0x74, 0xce, 0x99, 0x59, 0x92, 0x2f, 0xec, 0x55, 0x9d, 0x79,
        0x95, 0xf3, 0x93, 0x3c, 0x55,
    ]), // 0xFd6042Df3D74ce9959922FeC559d7995F3933c55
    H160([
        0xdb, 0x78, 0x9e, 0xb5, 0xbd, 0xb4, 0xe5, 0x59, 0xbe, 0xd1, 0x99, 0xb8, 0xb8, 0x2d, 0xed,
        0x94, 0xe1, 0xd0, 0x56, 0xc9,
    ]), // 0xdb789Eb5BDb4E559beD199B8b82dED94e1d056C9 (Chapel "Test Account")
];

/// Returns true if `address` is in bsc-geth's `NanoBlackList`.
///
/// Callers must gate this on a BSC chain id (56 or 97); the list is a
/// BSC/Parlia consensus rule with no meaning on other chains.
pub fn is_bsc_nano_blacklisted(address: &Address) -> bool {
    BSC_NANO_BLACKLIST.contains(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_all_three_nano_blacklist_entries() {
        for addr in &BSC_NANO_BLACKLIST {
            assert!(is_bsc_nano_blacklisted(addr));
        }
    }

    #[test]
    fn chapel_test_account_is_blacklisted() {
        // The authority recovered from Chapel block 120,342,702 tx 0.
        let authority = Address::from_slice(
            &hex::decode("db789Eb5BDb4E559beD199B8b82dED94e1d056C9").expect("valid hex"),
        );
        assert!(is_bsc_nano_blacklisted(&authority));
    }

    #[test]
    fn ordinary_address_is_not_blacklisted() {
        let addr = Address::from_slice(
            &hex::decode("a1bfad23a2370f208725337825d54fc78afe1970").expect("valid hex"),
        );
        assert!(!is_bsc_nano_blacklisted(&addr));
    }
}
