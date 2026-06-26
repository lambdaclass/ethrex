use ethrex_common::{H160, types::Fork, types::Fork::*};

pub use ethrex_common::constants::SYSTEM_ADDRESS;

pub struct SystemContract {
    pub address: H160,
    pub name: &'static str,
    pub active_since_fork: Fork,
}

pub const DEPOSIT_CONTRACT_ADDRESS: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x21, 0x9A, 0xB5, 0x40, 0x35, 0x6C, 0xBB, 0x83, 0x9C, 0xBE, 0x05,
        0x30, 0x3D, 0x77, 0x05, 0xFA,
    ]),
    name: "DEPOSIT_CONTRACT_ADDRESS",
    active_since_fork: Prague,
};

pub const BEACON_ROOTS_ADDRESS: SystemContract = SystemContract {
    address: H160([
        0x00, 0x0F, 0x3D, 0xF6, 0xD7, 0x32, 0x80, 0x7E, 0xF1, 0x31, 0x9F, 0xB7, 0xB8, 0xBB, 0x85,
        0x22, 0xD0, 0xBE, 0xAC, 0x02,
    ]),
    name: "BEACON_ROOTS_ADDRESS",
    active_since_fork: Paris,
};

pub const HISTORY_STORAGE_ADDRESS: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0xF9, 0x08, 0x27, 0xF1, 0xC5, 0x3A, 0x10, 0xCB, 0x7A, 0x02, 0x33, 0x5B, 0x17,
        0x53, 0x20, 0x00, 0x29, 0x35,
    ]),
    name: "HISTORY_STORAGE_ADDRESS",
    active_since_fork: Prague,
};

pub const WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0x09, 0x61, 0xEF, 0x48, 0x0E, 0xB5, 0x5E, 0x80, 0xD1, 0x9A, 0xD8, 0x35, 0x79,
        0xA6, 0x4C, 0x00, 0x70, 0x02,
    ]),
    name: "WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS",
    active_since_fork: Prague,
};

pub const CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0xBB, 0xDD, 0xC7, 0xCE, 0x48, 0x86, 0x42, 0xFB, 0x57, 0x9F, 0x8B, 0x00, 0xF3,
        0xA5, 0x90, 0x00, 0x72, 0x51,
    ]),
    name: "CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS",
    active_since_fork: Prague,
};

pub const SYSTEM_CONTRACTS: [SystemContract; 5] = [
    BEACON_ROOTS_ADDRESS,
    HISTORY_STORAGE_ADDRESS,
    DEPOSIT_CONTRACT_ADDRESS,
    WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
    CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
];

pub fn system_contracts_for_fork(fork: Fork) -> impl Iterator<Item = SystemContract> {
    SYSTEM_CONTRACTS
        .into_iter()
        .filter(move |system_contract| system_contract.active_since_fork <= fork)
}

pub const PRAGUE_SYSTEM_CONTRACTS: [SystemContract; 2] = [
    WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
    CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
];

pub const EXPIRY_VERIFIER_PREDEPLOY: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x81, 0x41,
    ]),
    name: "EXPIRY_VERIFIER_PREDEPLOY",
    active_since_fork: Hegota,
};

/// Canonical runtime bytecode of the EIP-8141 expiry verifier (spec commit
/// 0b197156): reverts unless calldata is exactly 8 bytes and the 8-byte BE
/// deadline is >= block.timestamp.
pub const EXPIRY_VERIFIER_RUNTIME_BYTECODE: [u8; 26] = [
    0x60, 0x08, 0x36, 0x14, 0x60, 0x0a, 0x57, 0x5f, 0x5f, 0xfd, 0x5b, 0x5f, 0x35, 0x60, 0xc0, 0x1c,
    0x42, 0x11, 0x60, 0x16, 0x57, 0x00, 0x5b, 0x5f, 0x5f, 0xfd,
];

/// EIP-8250 NONCE_MANAGER predeploy (address 0x…8250). Stores keyed-nonce
/// sequence values for non-zero nonce keys, keyed by
/// `keccak256(left_pad_32(sender) || uint256_to_bytes32(nonce_key))`. The
/// protocol writes it during APPROVE; direct user calls revert.
pub const NONCE_MANAGER_PREDEPLOY: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x82, 0x50,
    ]),
    name: "NONCE_MANAGER",
    active_since_fork: Hegota,
};

/// Runtime bytecode of the EIP-8250 NONCE_MANAGER: `PUSH1 0 PUSH1 0 REVERT` —
/// non-callable by users; the contract exists only as a protocol-managed
/// storage namespace.
pub const NONCE_MANAGER_RUNTIME_BYTECODE: [u8; 5] = [0x60, 0x00, 0x60, 0x00, 0xfd];

/// EIP-8272 RECENT_ROOT_ADDRESS predeploy (0x…8272). Intended to store recent
/// verified roots keyed by (source_id, slot); the spec leaves RECENT_ROOT_CODE
/// TBD and ethrex plans to handle the 64-byte write natively (see docs/eip-8272.md).
/// NOTE: this address is declared here but is NOT yet installed at the Hegota
/// boundary (unlike EXPIRY_VERIFIER / NONCE_MANAGER), so the account does not yet
/// exist on-chain. Installation lands together with the reference-validity check.
pub const RECENT_ROOT_ADDRESS: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x82, 0x72,
    ]),
    name: "RECENT_ROOT_ADDRESS",
    active_since_fork: Hegota,
};
#[cfg(test)]
mod expiry_verifier_tests {
    use super::*;

    #[test]
    fn expiry_verifier_constants_match_spec() {
        let expected: [u8; 26] = [
            0x60, 0x08, 0x36, 0x14, 0x60, 0x0a, 0x57, 0x5f, 0x5f, 0xfd, 0x5b, 0x5f, 0x35, 0x60,
            0xc0, 0x1c, 0x42, 0x11, 0x60, 0x16, 0x57, 0x00, 0x5b, 0x5f, 0x5f, 0xfd,
        ];
        assert_eq!(
            EXPIRY_VERIFIER_RUNTIME_BYTECODE.as_slice(),
            expected.as_slice()
        );
        assert_eq!(EXPIRY_VERIFIER_RUNTIME_BYTECODE.len(), 26);
        assert_eq!(
            EXPIRY_VERIFIER_PREDEPLOY.address,
            H160::from_low_u64_be(0x8141)
        );
    }

    #[test]
    fn nonce_manager_constants_match_spec() {
        assert_eq!(
            NONCE_MANAGER_RUNTIME_BYTECODE.as_slice(),
            [0x60, 0x00, 0x60, 0x00, 0xfd].as_slice()
        );
        assert_eq!(NONCE_MANAGER_RUNTIME_BYTECODE.len(), 5);
        assert_eq!(
            NONCE_MANAGER_PREDEPLOY.address,
            H160::from_low_u64_be(0x8250)
        );
        assert_eq!(NONCE_MANAGER_PREDEPLOY.active_since_fork, Hegota);
    }
}
