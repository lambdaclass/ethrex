//! BSC system call ABI encoding and block finalization helpers.
//!
//! BSC executes a set of privileged "system transactions" at the end of every
//! block, after all user transactions have been included.  These transactions
//! call well-known system contracts (validator set, slash, …) and are
//! constructed deterministically from the block header.
//!
//! This module provides:
//!   - ABI encoding helpers for each system call.
//!   - The [`SystemMessage`] type that describes a constructed system
//!     transaction before it is applied to the EVM state.
//!   - [`is_breathe_block`] — predicate for UTC-day-boundary blocks.
//!   - [`finalize_bsc_block`] — stub that will be wired into block execution.
//!
//! Reference: `consensus/parlia/parlia.go` (`Finalize`, `distributeIncoming`,
//! `slash`, `distributeFinalityReward`) and `feynmanfork.go`
//! (`updateValidatorSetV2`, `isBreatheBlock`).

use ethereum_types::{Address, U256};
use ethrex_crypto::keccak::keccak_hash;

use crate::parlia_config::{BREATHE_BLOCK_INTERVAL, SLASH_CONTRACT, VALIDATOR_CONTRACT};

// ── Gas constants ─────────────────────────────────────────────────────────────

/// Gas limit used for all BSC system calls.
///
/// Matches the reference client: `math.MaxUint64 / 2`.
/// Reference: `consensus/parlia/parlia.go` `getSystemMessage`.
pub const SYSTEM_CALL_GAS_LIMIT: u64 = u64::MAX / 2;

// ── ABI encoding ──────────────────────────────────────────────────────────────

/// Compute the 4-byte ABI function selector for `signature`.
///
/// The selector is the first 4 bytes of `keccak256(signature)`.
fn selector(signature: &str) -> [u8; 4] {
    let hash = keccak_hash(signature.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

/// ABI-encode a single `address` argument (padded to 32 bytes).
///
/// The Ethereum ABI pads a 20-byte address to a 32-byte word by left-padding
/// with 12 zero bytes.
fn encode_address(addr: Address) -> [u8; 32] {
    let mut word = [0u8; 32];
    // bytes 0..11 are already zero (left-padding)
    word[12..32].copy_from_slice(addr.as_bytes());
    word
}

/// ABI-encode a `uint256` value (big-endian 32-byte word).
fn encode_uint256(value: U256) -> [u8; 32] {
    value.to_big_endian()
}

/// ABI-encode a `uint64` value as a `uint256` word (big-endian, 32 bytes).
fn encode_uint64_as_uint256(value: u64) -> [u8; 32] {
    encode_uint256(U256::from(value))
}

/// Encode `deposit(address _validator)` calldata for `ValidatorContract`
/// (0x1000).
///
/// The `deposit` call is made every block to credit the accumulated gas fees
/// (held at `SYSTEM_ADDRESS`) to the current block's coinbase validator.
/// The fee amount is passed as `msg.value`, not as an ABI argument.
///
/// Solidity signature: `deposit(address)`
/// Reference: `consensus/parlia/parlia.go` `distributeToValidator`.
pub fn encode_deposit(validator: Address) -> Vec<u8> {
    // keccak256("deposit(address)") = 0xf340fa01
    let sel = selector("deposit(address)");
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&sel);
    data.extend_from_slice(&encode_address(validator));
    data
}

/// Encode `slash(address _spoiledValidator)` calldata for `SlashContract`
/// (0x1001).
///
/// `slash` is called when an out-of-turn block is produced and the expected
/// in-turn validator has not signed recently (i.e. missed their slot).
///
/// Solidity signature: `slash(address)`
/// Reference: `consensus/parlia/parlia.go` `slash`.
pub fn encode_slash(spoiled_validator: Address) -> Vec<u8> {
    // keccak256("slash(address)") = 0xc96be4cb  (first 4 bytes)
    let sel = selector("slash(address)");
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&sel);
    data.extend_from_slice(&encode_address(spoiled_validator));
    data
}

/// Encode `distributeFinalityReward(address[],uint256[])` calldata for
/// `ValidatorContract` (0x1000).
///
/// Called every [`FINALITY_REWARD_INTERVAL`] blocks (200 blocks) to
/// distribute rewards to validators that participated in BLS finality voting
/// over the preceding window.
///
/// The ABI encoding for dynamic arrays follows the Ethereum ABI spec:
///
/// ```text
/// selector                    (4 bytes)
/// offset of validators[]      (32 bytes) = 0x40 (points past the two head words)
/// offset of weights[]         (32 bytes) = 0x40 + 32*(1 + validators.len())
/// validators.len()            (32 bytes)
/// validators[0]               (32 bytes, left-padded address)
/// …
/// validators[n-1]             (32 bytes)
/// weights.len()               (32 bytes)
/// weights[0]                  (32 bytes)
/// …
/// weights[n-1]                (32 bytes)
/// ```
///
/// Solidity signature: `distributeFinalityReward(address[],uint256[])`
/// Reference: `consensus/parlia/parlia.go` `distributeFinalityReward`.
///
/// # Panics
///
/// Panics in debug mode if `validators.len() != weights.len()`.
pub fn encode_distribute_finality_reward(validators: &[Address], weights: &[u64]) -> Vec<u8> {
    debug_assert_eq!(
        validators.len(),
        weights.len(),
        "validators and weights must have equal length"
    );

    let sel = selector("distributeFinalityReward(address[],uint256[])");
    let n = validators.len();

    // Two dynamic arrays: each head slot is a 32-byte offset.
    // Head: [offset_validators, offset_weights]
    // validators array starts right after the two head words → offset = 0x40
    let offset_validators: usize = 64; // 2 * 32
    // weights array starts after: offset_validators header + length word + n elements
    let offset_weights: usize = offset_validators + 32 + n * 32;

    let mut data = Vec::with_capacity(4 + 32 * (2 + 1 + n + 1 + n));
    data.extend_from_slice(&sel);
    // head: offsets
    data.extend_from_slice(&encode_uint64_as_uint256(offset_validators as u64));
    data.extend_from_slice(&encode_uint64_as_uint256(offset_weights as u64));
    // validators[] body
    data.extend_from_slice(&encode_uint64_as_uint256(n as u64));
    for addr in validators {
        data.extend_from_slice(&encode_address(*addr));
    }
    // weights[] body
    data.extend_from_slice(&encode_uint64_as_uint256(n as u64));
    for &w in weights {
        data.extend_from_slice(&encode_uint64_as_uint256(w));
    }
    data
}

/// Encode `updateValidatorSetV2(address[],uint64[],bytes[])` calldata for
/// `ValidatorContract` (0x1000).
///
/// Called on "breathe blocks" (the first block of a new UTC day, determined
/// by [`is_breathe_block`]) to elect the next validator set from the stake hub.
///
/// The ABI encoding for `bytes[]` (a dynamic array of dynamic elements)
/// follows the two-level offset scheme from the Ethereum ABI spec:
///
/// ```text
/// selector                       (4 bytes)
/// offset of _consensusAddrs[]    (32 bytes) head
/// offset of _votingPowers[]      (32 bytes) head
/// offset of _voteAddrs[]         (32 bytes) head
/// ──── _consensusAddrs[] ────
/// length                         (32 bytes)
/// addr[0] … addr[n-1]            (32 bytes each)
/// ──── _votingPowers[] ────
/// length                         (32 bytes)
/// power[0] … power[n-1]          (32 bytes each)
/// ──── _voteAddrs[] ────
/// length                         (32 bytes)
/// inner_offset[0]                (32 bytes) → relative to start of _voteAddrs[] data
/// …
/// inner_offset[n-1]              (32 bytes)
/// ──── bytes[i] tails ────
/// length_i                       (32 bytes)
/// data_i (right-padded to 32-byte boundary)
/// ```
///
/// Solidity signature: `updateValidatorSetV2(address[],uint64[],bytes[])`
/// Reference: `consensus/parlia/feynmanfork.go` `updateValidatorSetV2`.
///
/// # Panics
///
/// Panics in debug mode if the three slices have unequal lengths.
pub fn encode_update_validator_set_v2(
    consensus_addrs: &[Address],
    voting_powers: &[u64],
    vote_addrs: &[Vec<u8>],
) -> Vec<u8> {
    debug_assert_eq!(consensus_addrs.len(), voting_powers.len());
    debug_assert_eq!(consensus_addrs.len(), vote_addrs.len());

    let sel = selector("updateValidatorSetV2(address[],uint64[],bytes[])");
    let n = consensus_addrs.len();

    // ── Compute layout offsets (all relative to the start of the ABI data,
    //    i.e. the byte immediately after the 4-byte selector) ────────────────

    // Three head words (one offset per dynamic argument).
    let head_size: usize = 3 * 32;

    // _consensusAddrs[] encoding: 32 (length) + n * 32
    let addrs_size: usize = 32 + n * 32;
    // _votingPowers[] encoding: 32 (length) + n * 32
    let powers_size: usize = 32 + n * 32;

    // _voteAddrs[] encoding:
    //   32 (length) + n * 32 (inner offsets) + sum of padded byte blobs
    let vote_addrs_inner_offsets_size: usize = 32 + n * 32;
    // Each bytes element: 32-byte length + ceil(len / 32) * 32 data bytes.
    let vote_addrs_blob_sizes: Vec<usize> = vote_addrs
        .iter()
        .map(|b| 32 + b.len().div_ceil(32) * 32)
        .collect();
    let vote_addrs_blobs_total: usize = vote_addrs_blob_sizes.iter().sum();

    // Absolute offsets from the start of the ABI data (after selector):
    let offset_addrs: usize = head_size;
    let offset_powers: usize = offset_addrs + addrs_size;
    let offset_vote_addrs: usize = offset_powers + powers_size;

    // Total size estimate for pre-allocation.
    let total = 4
        + head_size
        + addrs_size
        + powers_size
        + vote_addrs_inner_offsets_size
        + vote_addrs_blobs_total;

    let mut data = Vec::with_capacity(total);

    // ── Selector ─────────────────────────────────────────────────────────────
    data.extend_from_slice(&sel);

    // ── Head: three absolute offsets ─────────────────────────────────────────
    data.extend_from_slice(&encode_uint64_as_uint256(offset_addrs as u64));
    data.extend_from_slice(&encode_uint64_as_uint256(offset_powers as u64));
    data.extend_from_slice(&encode_uint64_as_uint256(offset_vote_addrs as u64));

    // ── _consensusAddrs[] ────────────────────────────────────────────────────
    data.extend_from_slice(&encode_uint64_as_uint256(n as u64));
    for addr in consensus_addrs {
        data.extend_from_slice(&encode_address(*addr));
    }

    // ── _votingPowers[] ──────────────────────────────────────────────────────
    data.extend_from_slice(&encode_uint64_as_uint256(n as u64));
    for &power in voting_powers {
        data.extend_from_slice(&encode_uint64_as_uint256(power));
    }

    // ── _voteAddrs[] ─────────────────────────────────────────────────────────
    // Length of the outer array.
    data.extend_from_slice(&encode_uint64_as_uint256(n as u64));

    // Inner offsets: each is relative to the start of the _voteAddrs[] array
    // data (i.e. the byte immediately after the outer length word).
    // The first blob starts after all n inner offset words.
    let inner_offsets_region: usize = n * 32;
    let mut inner_offset_acc: usize = inner_offsets_region;
    for blob_size in &vote_addrs_blob_sizes {
        data.extend_from_slice(&encode_uint64_as_uint256(inner_offset_acc as u64));
        inner_offset_acc += blob_size;
    }

    // Blobs: length word + right-padded data.
    for blob in vote_addrs {
        data.extend_from_slice(&encode_uint64_as_uint256(blob.len() as u64));
        data.extend_from_slice(blob);
        // Right-pad to 32-byte boundary.
        let pad = (32 - blob.len() % 32) % 32;
        data.extend(std::iter::repeat_n(0u8, pad));
    }

    data
}

// ── Breathe-block predicate ───────────────────────────────────────────────────

/// Returns `true` if `parent_timestamp` and `block_timestamp` straddle a UTC
/// day boundary, meaning `block` is the first block of a new UTC day.
///
/// This is the "breathe block" condition used to trigger `updateValidatorSetV2`.
///
/// Reference: `consensus/parlia/feynmanfork.go` `isBreatheBlock` /
/// `sameDayInUTC`.
///
/// # Notes
///
/// * Returns `false` when `parent_timestamp == 0` (genesis / no parent),
///   matching the BSC reference: `lastBlockTime != 0 && !sameDayInUTC(...)`.
/// * The `BREATHE_BLOCK_INTERVAL` is 86 400 seconds (one UTC day).
pub fn is_breathe_block(parent_timestamp: u64, block_timestamp: u64) -> bool {
    if parent_timestamp == 0 {
        return false;
    }
    parent_timestamp / BREATHE_BLOCK_INTERVAL != block_timestamp / BREATHE_BLOCK_INTERVAL
}

// ── SystemMessage ─────────────────────────────────────────────────────────────

/// A fully-constructed BSC system transaction, ready to be applied to the EVM
/// state.
///
/// System messages are not signed by a user key; they are injected by the
/// consensus engine during block finalization.  The `from` address is always
/// the block's `coinbase` (the active validator), and `gas_price` is always 0
/// so that fee deduction does not apply.
///
/// Reference: `consensus/parlia/parlia.go` `getSystemMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemMessage {
    /// `msg.sender` — always `header.coinbase` (the active validator).
    pub from: Address,
    /// Target system contract address.
    pub to: Address,
    /// ABI-encoded call data (selector + arguments).
    pub data: Vec<u8>,
    /// `msg.value` in wei — non-zero only for `deposit()`, where it carries
    /// the accumulated gas fees drained from `SYSTEM_ADDRESS`.
    pub value: U256,
    /// Gas limit for the call (`u64::MAX / 2`).
    pub gas_limit: u64,
    /// Gas price — always 0 for system calls (no fee deduction).
    pub gas_price: u64,
}

impl SystemMessage {
    /// Construct a zero-value system message (for slash, distributeFinalityReward,
    /// updateValidatorSetV2).
    pub fn new(from: Address, to: Address, data: Vec<u8>) -> Self {
        Self {
            from,
            to,
            data,
            value: U256::zero(),
            gas_limit: SYSTEM_CALL_GAS_LIMIT,
            gas_price: 0,
        }
    }

    /// Construct a system message that carries a `msg.value` (for `deposit()`).
    pub fn with_value(from: Address, to: Address, data: Vec<u8>, value: U256) -> Self {
        Self {
            from,
            to,
            data,
            value,
            gas_limit: SYSTEM_CALL_GAS_LIMIT,
            gas_price: 0,
        }
    }
}

// ── Block finalization ────────────────────────────────────────────────────────

/// Errors that can arise during BSC block finalization.
#[derive(Debug, thiserror::Error)]
pub enum SystemCallError {
    /// An EVM-level error occurred while executing a system transaction.
    #[error("BSC system call EVM error: {0}")]
    Evm(String),
    /// The system call executed but reverted (non-fatal; logged as a warning).
    #[error("BSC system call reverted at contract {contract}: {reason}")]
    Reverted { contract: Address, reason: String },
}

/// Apply system contract bytecode upgrades at fork activation blocks.
///
/// At certain BSC fork boundaries the on-chain system contract bytecodes are
/// patched by the consensus engine before any user or system transactions run.
/// For latest-fork-only checkpoint sync this is a no-op because the contracts
/// are already at their current version in the snapshot.
///
/// Reference: `consensus/parlia/feynmanfork.go` `applyUpgrade` /
/// `consensus/parlia/parlia.go` `Finalize` (upgrade call before system txs).
pub fn apply_system_contract_upgrades(
    _block_number: u64,
    _timestamp: u64,
) -> Result<(), SystemCallError> {
    // TODO: implement for full historical sync from genesis.
    // For checkpoint sync from a recent snapshot the contracts are already
    // at their current version, so this is intentionally left as a no-op.
    Ok(())
}

/// Build the list of BSC system messages to execute after user transactions.
///
/// Order matches the BSC reference client exactly:
///
/// 1. `slash()` — if block difficulty == 1 (out-of-turn) and a spoiled
///    validator is provided (i.e. the expected in-turn validator missed).
/// 2. `deposit()` — every block; drains `SYSTEM_ADDRESS` balance to
///    `ValidatorContract` as the block's fee reward.
/// 3. `distributeFinalityReward()` — every [`FINALITY_REWARD_INTERVAL`]
///    blocks; distributes BLS finality voting rewards.
/// 4. `updateValidatorSetV2()` — on breathe blocks (first block of a new UTC
///    day); elects the next validator set from the stake hub.
///
/// **The caller is responsible for draining the `SYSTEM_ADDRESS` balance to
/// zero before applying the `deposit()` message** — the `deposit_value` passed
/// here should equal whatever was held at `SYSTEM_ADDRESS`.
///
/// Reference: `consensus/parlia/parlia.go` `Finalize` (line ~1394).
pub fn build_bsc_system_messages(
    coinbase: Address,
    parent_timestamp: u64,
    block_timestamp: u64,
    block_number: u64,
    deposit_value: U256,
    spoiled_validator: Option<Address>,
) -> Vec<SystemMessage> {
    let mut messages = Vec::new();

    // 1. slash() — only when the block is out-of-turn (difficulty == 1) and we
    //    know which validator missed their slot.
    if let Some(spoiled) = spoiled_validator {
        messages.push(build_slash_message(coinbase, spoiled));
    }

    // 2. deposit() — every block, credit accumulated fees to the validator.
    //    BSC reference calls deposit() unconditionally every block, even when
    //    the fee balance is zero.
    messages.push(build_deposit_message(coinbase, deposit_value));

    // 3. distributeFinalityReward() — every FINALITY_REWARD_INTERVAL blocks.
    //    TODO: pass actual finality voters once snapshot data is available.
    //    For now the call is skipped (empty validator/weight lists would be
    //    a no-op on-chain anyway, but we omit it to avoid spurious calls).
    if block_number.is_multiple_of(crate::parlia_config::FINALITY_REWARD_INTERVAL) {
        // TODO: collect finality voters from snapshot data.
        // messages.push(build_distribute_finality_reward_message(coinbase, &[], &[]));
    }

    // 4. updateValidatorSetV2() — breathe blocks only.
    //    TODO: pass actual validator set once snapshot data is available.
    if is_breathe_block(parent_timestamp, block_timestamp) {
        // TODO: collect next validator set from snapshot/stake hub query.
        // messages.push(build_update_validator_set_v2_message(coinbase, &[], &[], &[]));
    }

    messages
}

// ── Convenience builders ──────────────────────────────────────────────────────

/// Build the `deposit()` [`SystemMessage`] for the given coinbase and fee
/// amount.
pub fn build_deposit_message(coinbase: Address, fees: U256) -> SystemMessage {
    let data = encode_deposit(coinbase);
    SystemMessage::with_value(coinbase, VALIDATOR_CONTRACT, data, fees)
}

/// Build the `slash()` [`SystemMessage`] for a missed in-turn validator.
pub fn build_slash_message(coinbase: Address, spoiled_validator: Address) -> SystemMessage {
    let data = encode_slash(spoiled_validator);
    SystemMessage::new(coinbase, SLASH_CONTRACT, data)
}

/// Build the `distributeFinalityReward()` [`SystemMessage`].
pub fn build_distribute_finality_reward_message(
    coinbase: Address,
    validators: &[Address],
    weights: &[u64],
) -> SystemMessage {
    let data = encode_distribute_finality_reward(validators, weights);
    SystemMessage::new(coinbase, VALIDATOR_CONTRACT, data)
}

/// Build the `updateValidatorSetV2()` [`SystemMessage`].
pub fn build_update_validator_set_v2_message(
    coinbase: Address,
    consensus_addrs: &[Address],
    voting_powers: &[u64],
    vote_addrs: &[Vec<u8>],
) -> SystemMessage {
    let data = encode_update_validator_set_v2(consensus_addrs, voting_powers, vote_addrs);
    SystemMessage::new(coinbase, VALIDATOR_CONTRACT, data)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parlia_config::{
        SLASH_CONTRACT, STAKE_HUB_CONTRACT, SYSTEM_REWARD_CONTRACT, VALIDATOR_CONTRACT,
    };
    use ethereum_types::H160;
    use ethrex_common::constants::SYSTEM_ADDRESS;

    // ── Contract address sanity ───────────────────────────────────────────────

    #[test]
    fn test_contract_addresses() {
        assert_eq!(
            format!("{VALIDATOR_CONTRACT:?}"),
            "0x0000000000000000000000000000000000001000"
        );
        assert_eq!(
            format!("{SLASH_CONTRACT:?}"),
            "0x0000000000000000000000000000000000001001"
        );
        assert_eq!(
            format!("{SYSTEM_REWARD_CONTRACT:?}"),
            "0x0000000000000000000000000000000000001002"
        );
        assert_eq!(
            format!("{STAKE_HUB_CONTRACT:?}"),
            "0x0000000000000000000000000000000000002002"
        );
        assert_eq!(
            format!("{SYSTEM_ADDRESS:?}"),
            "0xfffffffffffffffffffffffffffffffffffffffe"
        );
    }

    // ── ABI selector tests ────────────────────────────────────────────────────

    /// Verify the 4-byte selector for `deposit(address)`.
    ///
    /// keccak256("deposit(address)") = 0xf340fa01…
    /// Verified with pycryptodome and the Rust `keccak_hash` implementation.
    #[test]
    fn test_deposit_selector() {
        let data = encode_deposit(Address::zero());
        // First 4 bytes are the selector.
        assert_eq!(&data[..4], &[0xf3, 0x40, 0xfa, 0x01]);
        // Total length: 4 (selector) + 32 (address word)
        assert_eq!(data.len(), 36);
    }

    /// Verify the 4-byte selector for `slash(address)`.
    ///
    /// keccak256("slash(address)") = 0xc96be4cb…
    #[test]
    fn test_slash_selector() {
        let data = encode_slash(Address::zero());
        assert_eq!(&data[..4], &[0xc9, 0x6b, 0xe4, 0xcb]);
        assert_eq!(data.len(), 36);
    }

    /// Verify the 4-byte selector for `distributeFinalityReward(address[],uint256[])`.
    ///
    /// keccak256("distributeFinalityReward(address[],uint256[])") = 0x300c3567…
    #[test]
    fn test_distribute_finality_reward_selector() {
        let data = encode_distribute_finality_reward(&[], &[]);
        assert_eq!(&data[..4], &[0x30, 0x0c, 0x35, 0x67]);
        // Empty arrays: 4 + 32 (offset addrs) + 32 (offset weights)
        //               + 32 (len addrs=0) + 32 (len weights=0) = 132 bytes
        assert_eq!(data.len(), 132);
    }

    /// Verify the 4-byte selector for
    /// `updateValidatorSetV2(address[],uint64[],bytes[])`.
    ///
    /// keccak256("updateValidatorSetV2(address[],uint64[],bytes[])") = 0x1e4c1524…
    #[test]
    fn test_update_validator_set_v2_selector() {
        let data = encode_update_validator_set_v2(&[], &[], &[]);
        assert_eq!(&data[..4], &[0x1e, 0x4c, 0x15, 0x24]);
    }

    // ── ABI encoding correctness ──────────────────────────────────────────────

    /// Verify that `encode_deposit` left-pads the address correctly.
    #[test]
    fn test_deposit_address_encoding() {
        let addr = H160([
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
            0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
        ]);
        let data = encode_deposit(addr);
        // Bytes 4..16 must be zero (left padding of address).
        assert_eq!(&data[4..16], &[0u8; 12]);
        // Bytes 16..36 must be the address bytes.
        assert_eq!(&data[16..36], addr.as_bytes());
    }

    /// Verify ABI encoding of `distributeFinalityReward` with one entry.
    ///
    /// Expected layout (after selector):
    /// - word 0: offset to validators[] = 0x40 (64)
    /// - word 1: offset to weights[]    = 0x40 + 32 + 1*32 = 0xa0 (160)
    /// - word 2: validators.len() = 1
    /// - word 3: addr (left-padded)
    /// - word 4: weights.len() = 1
    /// - word 5: weight = 7
    #[test]
    fn test_distribute_finality_reward_encoding() {
        let addr = H160([0xAA; 20]);
        let validators = vec![addr];
        let weights = vec![7u64];
        let data = encode_distribute_finality_reward(&validators, &weights);

        // Total: 4 + 6*32 = 196
        assert_eq!(data.len(), 4 + 6 * 32);

        // offset_validators = 64 = 0x40
        let mut w = [0u8; 32];
        w[31] = 64;
        assert_eq!(&data[4..36], &w);

        // offset_weights = 64 + 32 + 32 = 128 = 0x80
        let mut w = [0u8; 32];
        w[31] = 128;
        assert_eq!(&data[36..68], &w);

        // validators.len() = 1
        let mut w = [0u8; 32];
        w[31] = 1;
        assert_eq!(&data[68..100], &w);

        // addr word (left-padded)
        let expected_addr = encode_address(addr);
        assert_eq!(&data[100..132], &expected_addr);

        // weights.len() = 1
        let mut w = [0u8; 32];
        w[31] = 1;
        assert_eq!(&data[132..164], &w);

        // weight = 7
        let mut w = [0u8; 32];
        w[31] = 7;
        assert_eq!(&data[164..196], &w);
    }

    // ── is_breathe_block tests ────────────────────────────────────────────────

    #[test]
    fn test_breathe_block_genesis_parent() {
        // parent_timestamp == 0 → never a breathe block (no parent).
        assert!(!is_breathe_block(0, 86_400));
    }

    #[test]
    fn test_breathe_block_same_day() {
        // Both timestamps within the same day.
        assert!(!is_breathe_block(3_600, 7_200));
    }

    #[test]
    fn test_breathe_block_crosses_day() {
        // parent is in day 0 (0..86399), block is in day 1 (86400..172799).
        assert!(is_breathe_block(86_399, 86_400));
    }

    #[test]
    fn test_breathe_block_exactly_on_boundary() {
        // parent exactly at day boundary → day 1, block in day 2.
        assert!(is_breathe_block(86_400, 2 * 86_400));
    }

    #[test]
    fn test_breathe_block_same_day_large_timestamps() {
        let day = 86_400u64;
        let base = 1_700_000_000u64; // some real-world mainnet timestamp
        let parent = base - (base % day); // first second of a day
        let block = parent + day - 1; // last second of the same day
        assert!(!is_breathe_block(parent, block));
    }

    #[test]
    fn test_breathe_block_next_day_large_timestamps() {
        let day = 86_400u64;
        let base = 1_700_000_000u64;
        let parent = base - (base % day) + day - 1; // last second of a day
        let block = parent + 1; // first second of the next day
        assert!(is_breathe_block(parent, block));
    }

    // ── SystemMessage builders ────────────────────────────────────────────────

    #[test]
    fn test_build_deposit_message() {
        let coinbase = H160([0x01; 20]);
        let fees = U256::from(1_000u64);
        let msg = build_deposit_message(coinbase, fees);

        assert_eq!(msg.from, coinbase);
        assert_eq!(msg.to, VALIDATOR_CONTRACT);
        assert_eq!(msg.value, fees);
        assert_eq!(msg.gas_limit, SYSTEM_CALL_GAS_LIMIT);
        assert_eq!(msg.gas_price, 0);
        assert_eq!(&msg.data[..4], &[0xf3, 0x40, 0xfa, 0x01]);
    }

    #[test]
    fn test_build_slash_message() {
        let coinbase = H160([0x01; 20]);
        let spoiled = H160([0x02; 20]);
        let msg = build_slash_message(coinbase, spoiled);

        assert_eq!(msg.from, coinbase);
        assert_eq!(msg.to, SLASH_CONTRACT);
        assert_eq!(msg.value, U256::zero());
        assert_eq!(&msg.data[..4], &[0xc9, 0x6b, 0xe4, 0xcb]);
    }

    // ── updateValidatorSetV2 with non-empty bytes[] ───────────────────────────

    /// Smoke test: verify `updateValidatorSetV2` with one validator whose
    /// vote address is a 48-byte BLS public key blob.
    #[test]
    fn test_update_validator_set_v2_one_entry() {
        let addr = H160([0xAB; 20]);
        let power = 100u64;
        let bls_key = vec![0xCC; 48]; // 48-byte BLS pubkey

        let data = encode_update_validator_set_v2(&[addr], &[power], &[bls_key.clone()]);

        // Selector check
        assert_eq!(&data[..4], &[0x1e, 0x4c, 0x15, 0x24]);

        // The encoding must be at least: 4 + 3*32 (heads) + 3 arrays bodies.
        // Just assert it's longer than the selector + heads.
        assert!(data.len() > 4 + 3 * 32);

        // Verify the first head word (offset of _consensusAddrs[]) = 3*32 = 96.
        let mut expected = [0u8; 32];
        expected[31] = 96;
        assert_eq!(&data[4..36], &expected);
    }
}
