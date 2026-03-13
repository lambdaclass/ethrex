use ethereum_types::{Address, H160, U256};
use ethrex_common::utils::keccak;

/// BorValidatorSet contract address (0x0000...1000).
pub const VALIDATOR_CONTRACT: Address = H160([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10, 0x00,
]);

/// StateReceiver contract address (0x0000...1001).
pub const STATE_RECEIVER_CONTRACT: Address = H160([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10, 0x01,
]);

/// System address used as msg.sender for system calls (0xffffFFFf...fFFfE).
pub const SYSTEM_ADDRESS: Address = H160([
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xfe,
]);

/// Maximum gas for Bor system calls (50 million, matching Bor's MaxTxGas).
pub const MAX_SYSTEM_CALL_GAS: u64 = 50_000_000;

/// Context for executing a system call against a Bor contract.
pub struct SystemCallContext {
    pub from: Address,
    pub to: Address,
    pub data: Vec<u8>,
    pub gas_limit: u64,
    pub gas_price: U256,
    pub value: U256,
    /// If true, EVM reverts should be logged but not fail the block.
    /// This is true for commitState calls (state sync) where individual
    /// event failures are non-fatal.
    pub revert_ok: bool,
}

/// Compute the 4-byte Solidity function selector from a signature string.
fn selector(sig: &str) -> [u8; 4] {
    let hash = keccak(sig.as_bytes());
    let mut sel = [0u8; 4];
    sel.copy_from_slice(&hash.0[..4]);
    sel
}

/// Pad a u64 value as a 32-byte big-endian uint256.
fn encode_uint256(val: u64) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[24..].copy_from_slice(&val.to_be_bytes());
    buf
}

/// Encode the offset portion of a dynamic `bytes` argument (pointer to where data lives).
fn encode_offset(offset: usize) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[24..].copy_from_slice(&(offset as u64).to_be_bytes());
    buf
}

/// Encode the data section for a `bytes` argument: length (32 bytes) + data (padded to 32).
fn encode_bytes_data(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // length
    out.extend_from_slice(&encode_uint256(data.len() as u64));
    // data padded to 32-byte boundary
    out.extend_from_slice(data);
    let padding = (32 - data.len() % 32) % 32;
    out.extend(std::iter::repeat_n(0u8, padding));
    out
}

/// Encode `lastStateId()` — selector only, no arguments.
pub fn encode_last_state_id() -> Vec<u8> {
    selector("lastStateId()").to_vec()
}

/// Encode `commitState(uint256,bytes)`.
///
/// ABI layout: selector | uint256 sync_time | offset for record_bytes | bytes data section
pub fn encode_commit_state(sync_time: u64, record_bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&selector("commitState(uint256,bytes)"));

    // Head section: sync_time (uint256) + offset to bytes data
    out.extend_from_slice(&encode_uint256(sync_time));
    // offset = 2 * 32 = 64 (past the two head slots)
    out.extend_from_slice(&encode_offset(64));

    // Data section for record_bytes
    out.extend(encode_bytes_data(record_bytes));

    out
}

/// Encode `getCurrentSpan()` — selector only, no arguments.
pub fn encode_get_current_span() -> Vec<u8> {
    selector("getCurrentSpan()").to_vec()
}

/// Encode `commitSpan(uint256,uint256,uint256,bytes,bytes)`.
///
/// ABI layout:
///   selector
///   | new_span (uint256)
///   | start_block (uint256)
///   | end_block (uint256)
///   | offset for validator_bytes
///   | offset for producer_bytes
///   | bytes data section for validator_bytes
///   | bytes data section for producer_bytes
pub fn encode_commit_span(
    new_span: u64,
    start_block: u64,
    end_block: u64,
    validator_bytes: &[u8],
    producer_bytes: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&selector("commitSpan(uint256,uint256,uint256,bytes,bytes)"));

    // Head section: 3 uint256 + 2 offsets = 5 * 32 = 160 bytes
    out.extend_from_slice(&encode_uint256(new_span));
    out.extend_from_slice(&encode_uint256(start_block));
    out.extend_from_slice(&encode_uint256(end_block));

    // Offset for validator_bytes: starts at byte 160 (5 * 32)
    let validator_offset = 5 * 32;
    out.extend_from_slice(&encode_offset(validator_offset));

    // Offset for producer_bytes: starts after validator data section
    let validator_data_len = 32 + validator_bytes.len() + (32 - validator_bytes.len() % 32) % 32;
    let producer_offset = validator_offset + validator_data_len;
    out.extend_from_slice(&encode_offset(producer_offset));

    // Data sections
    out.extend(encode_bytes_data(validator_bytes));
    out.extend(encode_bytes_data(producer_bytes));

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_addresses() {
        assert_eq!(
            format!("{VALIDATOR_CONTRACT:?}"),
            "0x0000000000000000000000000000000000001000"
        );
        assert_eq!(
            format!("{STATE_RECEIVER_CONTRACT:?}"),
            "0x0000000000000000000000000000000000001001"
        );
        assert_eq!(
            format!("{SYSTEM_ADDRESS:?}"),
            "0xfffffffffffffffffffffffffffffffffffffffe"
        );
    }

    #[test]
    fn test_last_state_id_selector() {
        let data = encode_last_state_id();
        assert_eq!(data.len(), 4);
        // keccak256("lastStateId()") first 4 bytes
        let expected = selector("lastStateId()");
        assert_eq!(&data[..], &expected[..]);
        // Verify against known value: keccak256("lastStateId()")[:4]
        assert_eq!(data, hex::decode("5407ca67").unwrap());
    }

    #[test]
    fn test_get_current_span_selector() {
        let data = encode_get_current_span();
        assert_eq!(data.len(), 4);
        // keccak256("getCurrentSpan()") first 4 bytes: 0xaf26aa96
        assert_eq!(data, hex::decode("af26aa96").unwrap());
    }

    #[test]
    fn test_commit_state_selector() {
        let data = encode_commit_state(0, &[]);
        // First 4 bytes are the selector for commitState(uint256,bytes)
        // keccak256("commitState(uint256,bytes)") first 4 bytes
        let sel = &data[..4];
        assert_eq!(sel, hex::decode("19494a17").unwrap());
    }

    #[test]
    fn test_commit_state_encoding() {
        let data = encode_commit_state(42, &[0xab, 0xcd]);
        // selector (4) + uint256 (32) + offset (32) + length (32) + padded data (32) = 132
        assert_eq!(data.len(), 4 + 32 + 32 + 32 + 32);

        // sync_time = 42 at offset 4..36
        let mut expected_time = [0u8; 32];
        expected_time[31] = 42;
        assert_eq!(&data[4..36], &expected_time);

        // offset = 64 at offset 36..68
        let mut expected_offset = [0u8; 32];
        expected_offset[31] = 64;
        assert_eq!(&data[36..68], &expected_offset);

        // bytes length = 2 at offset 68..100
        let mut expected_len = [0u8; 32];
        expected_len[31] = 2;
        assert_eq!(&data[68..100], &expected_len);

        // data starts at offset 100: 0xab, 0xcd, then 30 zero-padding bytes
        assert_eq!(data[100], 0xab);
        assert_eq!(data[101], 0xcd);
        assert!(data[102..132].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_commit_span_selector() {
        let data = encode_commit_span(0, 0, 0, &[], &[]);
        // keccak256("commitSpan(uint256,uint256,uint256,bytes,bytes)") first 4 bytes
        let sel = &data[..4];
        assert_eq!(sel, hex::decode("23c2a2b4").unwrap());
    }

    #[test]
    fn test_commit_span_encoding() {
        let validators = vec![0x01; 33];
        let producers = vec![0x02; 5];
        let data = encode_commit_span(10, 100, 200, &validators, &producers);

        // Head: selector(4) + 3*uint256(96) + 2*offsets(64) = 164
        // validator data: 32 (len) + 64 (33 bytes padded to 64) = 96
        // producer data: 32 (len) + 32 (5 bytes padded to 32) = 64
        // Total: 164 + 96 + 64 = 324
        assert_eq!(data.len(), 324);

        // new_span = 10
        assert_eq!(data[4 + 31], 10);
        // start_block = 100
        assert_eq!(data[4 + 32 + 31], 100);
        // end_block = 200
        assert_eq!(data[4 + 64 + 31], 200);

        // validator offset = 160 (5 * 32)
        let mut expected_off = [0u8; 32];
        expected_off[31] = 160;
        assert_eq!(&data[4 + 96..4 + 128], &expected_off);

        // producer offset = 160 + 96 = 256
        let mut expected_off2 = [0u8; 32];
        expected_off2[30] = 1; // 256 = 0x0100
        assert_eq!(&data[4 + 128..4 + 160], &expected_off2);
    }
}
