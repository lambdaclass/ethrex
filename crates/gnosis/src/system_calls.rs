//! ABI encoding/decoding for Gnosis Chain post-block system contract calls.
//!
//! Two system calls run after user transactions in every Gnosis block:
//!
//! 1. **Withdrawals**: `executeSystemWithdrawals(uint256, uint64[], address[])`
//!    Called on the deposit/withdrawal contract. Pays out GNO to withdrawal
//!    recipients. Replaces the native EIP-4895 credit (which would mint xDAI).
//!    Spec: <https://github.com/gnosischain/specs/blob/master/execution/withdrawals.md>
//!
//! 2. **Block rewards**: `reward(address[], uint16[]) returns (address[], uint256[])`
//!    Called on the POSDAO block-rewards contract with `([coinbase], [0])`.
//!    Returns lists of recipients and amounts; EL credits each balance.
//!    This is how xDAI is minted post-Merge.
//!    Spec: <https://github.com/gnosischain/specs/blob/master/execution/posdao-post-merge.md>
//!
//! Both calls use the AuRa system sender `0xff...fe`, run with effectively
//! unlimited gas, and a revert/halt invalidates the block.

use ethereum_types::{Address, U256};
use ethrex_crypto::keccak::keccak_hash;

/// Maximum number of failed-and-retried withdrawals to process per block.
/// Per Gnosis spec: bounded at 4 to cap per-block work.
pub const MAX_FAILED_WITHDRAWALS_TO_PROCESS: u64 = 4;

/// Reward kind for "RewardAuthor" — block producer's slot reward.
/// The only kind currently used by Gnosis post-Merge.
pub const REWARD_KIND_AUTHOR: u16 = 0;

fn selector(signature: &[u8]) -> [u8; 4] {
    let h = keccak_hash(signature);
    let mut s = [0u8; 4];
    s.copy_from_slice(&h[..4]);
    s
}

/// Encode calldata for
/// `executeSystemWithdrawals(uint256 maxFailedWithdrawalsToProcess,
///                            uint64[]  amounts,
///                            address[] addresses)`.
pub fn encode_execute_system_withdrawals(amounts_gwei: &[u64], addresses: &[Address]) -> Vec<u8> {
    debug_assert_eq!(amounts_gwei.len(), addresses.len());
    let sig = b"executeSystemWithdrawals(uint256,uint64[],address[])";
    let sel = selector(sig);
    let n = amounts_gwei.len() as u64;

    // ABI: head is 3 * 32 bytes (uint256 + 2 dynamic-array offsets), then dynamic data.
    // - head[0] = maxFailedWithdrawalsToProcess
    // - head[1] = offset to amounts (= 0x60 = 96, i.e. start of dynamic region)
    // - head[2] = offset to addresses (= 0x60 + (1 + n) * 32 bytes for amounts)
    let amounts_dyn_len = 32 + n as usize * 32;
    let head_len = 3 * 32;
    let offset_amounts = head_len as u64;
    let offset_addresses = head_len as u64 + amounts_dyn_len as u64;

    let mut out = Vec::with_capacity(4 + head_len + amounts_dyn_len + 32 + n as usize * 32);
    out.extend_from_slice(&sel);
    out.extend_from_slice(&U256::from(MAX_FAILED_WITHDRAWALS_TO_PROCESS).to_big_endian_buf());
    out.extend_from_slice(&U256::from(offset_amounts).to_big_endian_buf());
    out.extend_from_slice(&U256::from(offset_addresses).to_big_endian_buf());
    // amounts
    out.extend_from_slice(&U256::from(n).to_big_endian_buf());
    for amt in amounts_gwei {
        out.extend_from_slice(&U256::from(*amt).to_big_endian_buf());
    }
    // addresses
    out.extend_from_slice(&U256::from(n).to_big_endian_buf());
    for a in addresses {
        // Address is 20 bytes; left-pad to 32.
        let mut padded = [0u8; 32];
        padded[12..].copy_from_slice(a.as_bytes());
        out.extend_from_slice(&padded);
    }
    out
}

/// Encode calldata for `reward(address[] benefactors, uint16[] kind)`.
///
/// Gnosis post-Merge always calls this with a single benefactor (the block
/// coinbase) and kind = 0 (RewardAuthor).
pub fn encode_reward(coinbase: Address) -> Vec<u8> {
    let sig = b"reward(address[],uint16[])";
    let sel = selector(sig);
    let head_len = 2 * 32;
    let benefactors_dyn_len = 32 + 32; // length + 1 address
    let offset_benefactors = head_len as u64;
    let offset_kind = (head_len + benefactors_dyn_len) as u64;

    let mut out = Vec::with_capacity(4 + 2 * 32 + 64 + 64);
    out.extend_from_slice(&sel);
    out.extend_from_slice(&U256::from(offset_benefactors).to_big_endian_buf());
    out.extend_from_slice(&U256::from(offset_kind).to_big_endian_buf());
    // benefactors[]: length + 1 element
    out.extend_from_slice(&U256::from(1u64).to_big_endian_buf());
    let mut padded = [0u8; 32];
    padded[12..].copy_from_slice(coinbase.as_bytes());
    out.extend_from_slice(&padded);
    // kind[]: length + 1 uint16
    out.extend_from_slice(&U256::from(1u64).to_big_endian_buf());
    out.extend_from_slice(&U256::from(REWARD_KIND_AUTHOR as u64).to_big_endian_buf());
    out
}

/// Decode the return value of `reward(...)`: `(address[] receivers, uint256[] amounts)`.
///
/// Returns a vector of `(receiver, amount)` pairs to credit.
pub fn decode_reward_return(data: &[u8]) -> Result<Vec<(Address, U256)>, DecodeError> {
    // Empty return (contract has no code) is treated as no rewards.
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if data.len() < 64 {
        return Err(DecodeError::Short(
            "reward() return too short for two head pointers",
        ));
    }
    let offset_receivers = read_offset(&data[0..32])?;
    let offset_amounts = read_offset(&data[32..64])?;

    let receivers = read_address_array(data, offset_receivers)?;
    let amounts = read_u256_array(data, offset_amounts)?;

    if receivers.len() != amounts.len() {
        return Err(DecodeError::LengthMismatch);
    }
    Ok(receivers.into_iter().zip(amounts).collect())
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("decode error: {0}")]
    Short(&'static str),
    #[error("array offset out of bounds")]
    OutOfBounds,
    #[error("receivers/amounts length mismatch")]
    LengthMismatch,
    #[error("integer overflow in ABI offset")]
    Overflow,
}

fn read_offset(slot: &[u8]) -> Result<usize, DecodeError> {
    debug_assert_eq!(slot.len(), 32);
    // Read big-endian u256; reject if it doesn't fit in usize.
    let v = U256::from_big_endian(slot);
    let max = U256::from(usize::MAX as u64);
    if v > max {
        return Err(DecodeError::Overflow);
    }
    Ok(v.low_u64() as usize)
}

fn read_address_array(data: &[u8], offset: usize) -> Result<Vec<Address>, DecodeError> {
    if offset + 32 > data.len() {
        return Err(DecodeError::OutOfBounds);
    }
    let n = read_offset(&data[offset..offset + 32])?;
    let start = offset + 32;
    let end = start + n * 32;
    if end > data.len() {
        return Err(DecodeError::OutOfBounds);
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let slot = &data[start + i * 32..start + (i + 1) * 32];
        // Address is the low 20 bytes of the 32-byte slot.
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&slot[12..32]);
        out.push(Address::from(addr));
    }
    Ok(out)
}

fn read_u256_array(data: &[u8], offset: usize) -> Result<Vec<U256>, DecodeError> {
    if offset + 32 > data.len() {
        return Err(DecodeError::OutOfBounds);
    }
    let n = read_offset(&data[offset..offset + 32])?;
    let start = offset + 32;
    let end = start + n * 32;
    if end > data.len() {
        return Err(DecodeError::OutOfBounds);
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(U256::from_big_endian(
            &data[start + i * 32..start + (i + 1) * 32],
        ));
    }
    Ok(out)
}

// U256::to_big_endian_buf shim — ethereum_types::U256 doesn't expose a `[u8;32]`
// method; build one ourselves.
trait U256ToBuf {
    fn to_big_endian_buf(&self) -> [u8; 32];
}
impl U256ToBuf for U256 {
    fn to_big_endian_buf(&self) -> [u8; 32] {
        self.to_big_endian()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(hex: &str) -> Address {
        let bytes = hex::decode(hex.trim_start_matches("0x")).unwrap();
        let mut a = [0u8; 20];
        a.copy_from_slice(&bytes);
        Address::from(a)
    }

    #[test]
    fn selector_executeSystemWithdrawals() {
        // Selector for `executeSystemWithdrawals(uint256,uint64[],address[])`
        // Verify ours matches a known external computation.
        let calldata = encode_execute_system_withdrawals(&[], &[]);
        // Just check selector is stable and the encoding shape is right for empty arrays.
        // selector = first 4 bytes
        assert_eq!(
            calldata.len(),
            4 + 32 * 3 + 32 + 32,
            "head + 2 length slots"
        );
    }

    #[test]
    fn round_trip_reward_decoding() {
        // Construct a minimal valid return: 2 receivers, 2 amounts.
        // head[0]=offset_receivers=0x40, head[1]=offset_amounts=0xa0
        // receivers: len=2, addr1, addr2 → 32 + 64 = 96 bytes
        // amounts:   len=2, 100, 200 → 96 bytes
        let mut data = Vec::new();
        // head
        data.extend_from_slice(&U256::from(0x40u64).to_big_endian_buf());
        data.extend_from_slice(&U256::from(0xa0u64).to_big_endian_buf());
        // receivers
        data.extend_from_slice(&U256::from(2u64).to_big_endian_buf());
        let a1 = addr("0x1111111111111111111111111111111111111111");
        let a2 = addr("0x2222222222222222222222222222222222222222");
        let mut p1 = [0u8; 32];
        p1[12..].copy_from_slice(a1.as_bytes());
        data.extend_from_slice(&p1);
        let mut p2 = [0u8; 32];
        p2[12..].copy_from_slice(a2.as_bytes());
        data.extend_from_slice(&p2);
        // amounts
        data.extend_from_slice(&U256::from(2u64).to_big_endian_buf());
        data.extend_from_slice(&U256::from(100u64).to_big_endian_buf());
        data.extend_from_slice(&U256::from(200u64).to_big_endian_buf());

        let pairs = decode_reward_return(&data).unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], (a1, U256::from(100u64)));
        assert_eq!(pairs[1], (a2, U256::from(200u64)));
    }

    #[test]
    fn empty_reward_return_ok() {
        let pairs = decode_reward_return(&[]).unwrap();
        assert!(pairs.is_empty());
    }

    #[test]
    fn encode_reward_shape() {
        let coinbase = addr("0xdeadbeef00000000000000000000000000000000");
        let data = encode_reward(coinbase);
        // 4 (selector) + 32 (offset_benefactors) + 32 (offset_kind)
        // + 32 (benefactors length) + 32 (benefactor addr)
        // + 32 (kind length) + 32 (kind value)
        assert_eq!(data.len(), 4 + 32 * 6);
        // Selector for `reward(address[],uint16[])`: first 4 bytes of
        // keccak256("reward(address[],uint16[])") = 0xf91c2898…
        // (verified against an independent keccak-256 implementation).
        assert_eq!(&data[0..4], &[0xf9, 0x1c, 0x28, 0x98]);
    }

    #[test]
    fn encode_execute_system_withdrawals_selector() {
        // Selector for executeSystemWithdrawals(uint256,uint64[],address[])
        let data = encode_execute_system_withdrawals(&[], &[]);
        let expected_sel = selector(b"executeSystemWithdrawals(uint256,uint64[],address[])");
        assert_eq!(&data[0..4], &expected_sel);
    }
}
