use ethrex_common::{
    H160,
    types::{FRAME_TX_RECENT_ROOT_LENGTH, Fork, Fork::*},
};

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

// EIP-8282 builder deposit predeploy — Nick's-method address
// (0x0000BFF46984E3725691FA540A8C7589300D8282).
pub const BUILDER_DEPOSIT_CONTRACT_ADDRESS: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0xBF, 0xF4, 0x69, 0x84, 0xE3, 0x72, 0x56, 0x91, 0xFA, 0x54, 0x0A, 0x8C, 0x75,
        0x89, 0x30, 0x0D, 0x82, 0x82,
    ]),
    name: "BUILDER_DEPOSIT_CONTRACT_ADDRESS",
    active_since_fork: Amsterdam,
};

// EIP-8282 builder exit predeploy — Nick's-method address
// (0x000064D678505AD48F8CCB093BC65613800E8282).
pub const BUILDER_EXIT_CONTRACT_ADDRESS: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0x64, 0xD6, 0x78, 0x50, 0x5A, 0xD4, 0x8F, 0x8C, 0xCB, 0x09, 0x3B, 0xC6, 0x56,
        0x13, 0x80, 0x0E, 0x82, 0x82,
    ]),
    name: "BUILDER_EXIT_CONTRACT_ADDRESS",
    active_since_fork: Amsterdam,
};

pub const SYSTEM_CONTRACTS: [SystemContract; 7] = [
    BEACON_ROOTS_ADDRESS,
    HISTORY_STORAGE_ADDRESS,
    DEPOSIT_CONTRACT_ADDRESS,
    WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
    CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
    BUILDER_DEPOSIT_CONTRACT_ADDRESS,
    BUILDER_EXIT_CONTRACT_ADDRESS,
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

// EIP-8282 request predeploys (builder deposit/exit). Active from Amsterdam.
// Empty code at these addresses on an Amsterdam+ block invalidates the block,
// mirroring the PRAGUE_SYSTEM_CONTRACTS empty-code-failure rule.
pub const AMSTERDAM_REQUEST_PREDEPLOYS: [SystemContract; 2] = [
    BUILDER_DEPOSIT_CONTRACT_ADDRESS,
    BUILDER_EXIT_CONTRACT_ADDRESS,
];

pub const EXPIRY_VERIFIER_PREDEPLOY: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x81, 0x41,
    ]),
    name: "EXPIRY_VERIFIER_PREDEPLOY",
    active_since_fork: Hegota,
};

/// Canonical runtime bytecode of the EIP-8141 expiry verifier: reverts unless
/// calldata is exactly 8 bytes and the 8-byte BE deadline is >= block.timestamp.
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

/// EIP-8272 RECENT_ROOT_ADDRESS predeploy (0x…8272). Stores recent verified
/// roots keyed by (source_id, slot), written by calling it with `salt ‖ root`.
pub const RECENT_ROOT_ADDRESS: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x82, 0x72,
    ]),
    name: "RECENT_ROOT_ADDRESS",
    active_since_fork: Hegota,
};

/// EIP-8312 UTXO vault predeploy (0x…8312). Holds every unspent UTXO's value.
/// Unlike the other Hegotá-family predeploys it carries real runtime bytecode:
/// its code implements deposits (create a UTXO), while every other write to its
/// storage or balance is performed by the protocol directly.
///
/// `active_since_fork` is `Hegota` because that is the earliest fork at which
/// frame transactions — and therefore UTXO frames — can exist, but it is NOT the
/// activation gate: EIP-8312 has its own activation timestamp (its fork
/// assignment is undecided upstream), and the install is gated on
/// `ChainConfig::is_utxo_frames_activated`. This field is descriptive metadata
/// only; no code derives activation from it.
pub const UTXO_VAULT_PREDEPLOY: SystemContract = SystemContract {
    address: H160([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x83, 0x12,
    ]),
    name: "UTXO_VAULT",
    active_since_fork: Hegota,
};

/// Canonical runtime bytecode of the EIP-8312 UTXO vault, verbatim from the
/// spec's `assets/eip-8312/utxo_vault.eas`.
///
/// Behavior: `recipient = calldata[0:20]`; revert unless `calldatasize == 20`,
/// `callvalue != 0`, and `recipient != 0`; otherwise assign
/// `index = sload(0)`, `sstore(0, index + 1)`, and emit
/// `UtxoCreated(source=caller, recipient, index, value)` as
/// `LOG3(topic, caller, recipient)` with `index ++ value` as data. A plain
/// transfer (empty calldata) reverts, so no value enters the vault without
/// creating a UTXO.
pub const UTXO_VAULT_RUNTIME_BYTECODE: [u8; 76] = [
    0x5f, 0x35, 0x60, 0x60, 0x1c, 0x80, 0x15, 0x60, 0x14, 0x36, 0x14, 0x15, 0x17, 0x34, 0x15, 0x17,
    0x60, 0x48, 0x57, 0x5f, 0x54, 0x80, 0x60, 0x01, 0x01, 0x5f, 0x55, 0x5f, 0x52, 0x34, 0x60, 0x20,
    0x52, 0x33, 0x7f, 0x3b, 0x19, 0x24, 0x14, 0x65, 0xa4, 0x7b, 0xc1, 0x87, 0xf1, 0xd9, 0xc7, 0xdb,
    0x70, 0x83, 0x48, 0x55, 0xa9, 0x07, 0x18, 0x37, 0x42, 0xa4, 0xb6, 0x3a, 0xa8, 0x24, 0xc5, 0x76,
    0x29, 0x6f, 0x5e, 0x60, 0x40, 0x5f, 0xa3, 0x00, 0x5b, 0x5f, 0x5f, 0xfd,
];

/// Runtime bytecode of the EIP-8272 RECENT_ROOT_ADDRESS predeploy: the write
/// operation of the spec's §"Recent root contract", assembled verbatim.
///
/// Reverts unless the call carries zero value and exactly 64 bytes
/// (`salt ‖ root`), then stores `entry_hash` under `storage_key` for
/// `source_id = keccak256(caller ‖ salt)` at the current EIP-7843 slot. The
/// spec's two prohibitions need no explicit check: a static context fails on
/// the `SSTORE` itself, and under `DELEGATECALL`/`CALLCODE` the write lands in
/// the calling account's storage, leaving recent-root storage untouched.
///
/// Provisional: `RECENT_ROOT_CODE` is TBD in the spec's constants table, so
/// this is the candidate proposed in ethereum/EIPs#12131.
///
/// The code derives the ring index as `SLOTNUM AND 0x1fff` (`push2 0x1fff`),
/// which equals `S mod RECENT_ROOT_LENGTH` only while `RECENT_ROOT_LENGTH` is
/// that power of two. The assertion below pins the length the mask assumes.
pub const RECENT_ROOT_RUNTIME_BYTECODE: [u8; 144] = [
    0x34, 0x15, 0x36, 0x60, 0x40, 0x14, 0x16, 0x61, 0x00, 0x10, 0x57, 0x60, 0x00, 0x60, 0x00, 0xfd,
    0x5b, 0x33, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0x60, 0x20, 0x37, 0x60, 0x34, 0x60, 0x0c,
    0x20, 0x80, 0x7f, 0x8f, 0x42, 0x48, 0x16, 0x79, 0xc8, 0xe6, 0xfe, 0xfa, 0x04, 0x09, 0x74, 0xb3,
    0xc9, 0x05, 0xe0, 0xce, 0x3f, 0x2e, 0x46, 0x4b, 0xa9, 0x3a, 0xcd, 0xb0, 0x74, 0xa4, 0x11, 0x81,
    0x61, 0x7e, 0xfc, 0x60, 0x40, 0x52, 0x4b, 0x60, 0x68, 0x52, 0x60, 0x60, 0x52, 0x60, 0x20, 0x60,
    0x20, 0x60, 0x88, 0x37, 0x60, 0x68, 0x60, 0x40, 0x20, 0x81, 0x7f, 0xbd, 0xc8, 0x97, 0xda, 0x21,
    0x77, 0xd2, 0x60, 0xff, 0x5f, 0x4b, 0xe5, 0xd4, 0xb2, 0xaa, 0xd4, 0x3f, 0x89, 0xc3, 0x34, 0x7a,
    0x30, 0x5b, 0x58, 0x4f, 0xa5, 0xa2, 0x54, 0x6d, 0x05, 0x3d, 0xaa, 0x60, 0xa8, 0x52, 0x61, 0x1f,
    0xff, 0x4b, 0x16, 0x60, 0xd0, 0x52, 0x60, 0xc8, 0x52, 0x60, 0x48, 0x60, 0xa8, 0x20, 0x55, 0x00,
];

const _: () = assert!(
    FRAME_TX_RECENT_ROOT_LENGTH == 8192,
    "RECENT_ROOT_RUNTIME_BYTECODE masks the slot with 0x1fff, which is \
     `S mod RECENT_ROOT_LENGTH` only at RECENT_ROOT_LENGTH == 8192"
);

#[cfg(test)]
mod expiry_verifier_tests {
    use super::*;
    use ethrex_common::{H256, utils::keccak};

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

    #[test]
    fn recent_root_constants_match_spec() {
        assert_eq!(RECENT_ROOT_RUNTIME_BYTECODE.len(), 144);
        assert_eq!(RECENT_ROOT_ADDRESS.address, H160::from_low_u64_be(0x8272));
        assert_eq!(RECENT_ROOT_ADDRESS.active_since_fork, Hegota);
    }

    /// The write side is a byte string with no compiler behind it, so the whole
    /// body is hashed and the two domain immediates are checked against the
    /// preimages the read side derives in
    /// `RecentRootReference::{entry_hash, storage_key}`. A transcription error
    /// in either would otherwise surface only as a storage-key mismatch at
    /// reference-validation time.
    #[test]
    fn recent_root_bytecode_matches_spec() {
        assert_eq!(
            keccak(RECENT_ROOT_RUNTIME_BYTECODE),
            H256::from_slice(
                &hex::decode("432c8b183d17d5e9939623833203b9a5b62325246cfcd9307982bfde8f18c6fb")
                    .expect("code hash literal is valid hex")
            )
        );

        // `push32 RECENT_ROOT_ENTRY_DOMAIN` at 0x22, immediate at 0x23..0x43.
        assert_eq!(RECENT_ROOT_RUNTIME_BYTECODE[0x22], 0x7f);
        assert_eq!(
            &RECENT_ROOT_RUNTIME_BYTECODE[0x23..0x43],
            keccak(b"RECENT_ROOT_ENTRY").as_bytes()
        );

        // `push32 RECENT_ROOT_STORAGE_DOMAIN` at 0x5a, immediate at 0x5b..0x7b.
        assert_eq!(RECENT_ROOT_RUNTIME_BYTECODE[0x5a], 0x7f);
        assert_eq!(
            &RECENT_ROOT_RUNTIME_BYTECODE[0x5b..0x7b],
            keccak(b"RECENT_ROOT_STORAGE").as_bytes()
        );
    }
}
