use ethrex_common::{Address, U256};

use crate::hash::blake3_hash;

/// Sub-index for the basic_data leaf (version, nonce, balance, code_size).
pub const BASIC_DATA_LEAF_KEY: u8 = 0;

/// Sub-index for the code_hash leaf (keccak256 of the account's code).
pub const CODE_HASH_LEAF_KEY: u8 = 1;

/// Offset in the stem subtree where header storage slots begin (slots 0–63).
pub const HEADER_STORAGE_OFFSET: u64 = 64;

/// Offset in the stem subtree where code chunks begin.
pub const CODE_OFFSET: u64 = 128;

/// Number of leaf slots per stem subtree (one per sub-index byte value).
pub const STEM_SUBTREE_WIDTH: u64 = 256;

/// 2^248 — where main storage slots start.
pub fn main_storage_offset() -> U256 {
    U256::from(1) << 248
}

/// Zero-pads a 20-byte Ethereum address to 32 bytes (12 zero bytes prefix + 20 address bytes).
pub fn old_style_address_to_address32(address: &Address) -> [u8; 32] {
    let mut result = [0u8; 32];
    result[12..].copy_from_slice(address.as_bytes());
    result
}

/// BLAKE3 hash used for key derivation (EIP-7864 `tree_hash`).
///
/// This is plain BLAKE3 with NO special case for all-zero input.
/// The `hash([0x00]*64) → [0x00]*32` special case only applies to
/// merkelization (see `merkle.rs`), not key derivation.
pub fn tree_hash(data: &[u8]) -> [u8; 32] {
    blake3_hash(data)
}

/// Derives the 32-byte tree key for the given address, tree_index, and sub_index.
///
/// The first 31 bytes form the stem (from `tree_hash(address32 ++ tree_index_bytes)`),
/// and the final byte is `sub_index`.
pub fn get_tree_key(address: &Address, tree_index: U256, sub_index: u8) -> [u8; 32] {
    let address32 = old_style_address_to_address32(address);
    let tree_index_bytes = tree_index.to_big_endian();

    let mut input = [0u8; 64];
    input[..32].copy_from_slice(&address32);
    input[32..].copy_from_slice(&tree_index_bytes);

    let hash = tree_hash(&input);

    let mut key = [0u8; 32];
    key[..31].copy_from_slice(&hash[..31]);
    key[31] = sub_index;
    key
}

/// Returns the tree key for the basic_data leaf of an account.
///
/// Stores: version (1B) + reserved (4B) + code_size (3B) + nonce (8B) + balance (16B).
pub fn get_tree_key_for_basic_data(address: &Address) -> [u8; 32] {
    get_tree_key(address, U256::zero(), BASIC_DATA_LEAF_KEY)
}

/// Returns the tree key for the code_hash leaf of an account.
pub fn get_tree_key_for_code_hash(address: &Address) -> [u8; 32] {
    get_tree_key(address, U256::zero(), CODE_HASH_LEAF_KEY)
}

/// Returns the tree key for a code chunk by its chunk index.
///
/// Chunks start at CODE_OFFSET (128) within the stem subtree width (256).
/// Chunk 0 → sub_index 128, chunk 127 → sub_index 255, chunk 128 → tree_index 1, sub_index 0.
pub fn get_tree_key_for_code_chunk(address: &Address, chunk_id: u64) -> [u8; 32] {
    let pos = U256::from(CODE_OFFSET) + U256::from(chunk_id);
    let tree_index = pos / U256::from(STEM_SUBTREE_WIDTH);
    // Safe: pos % 256 is always 0–255, fits in u8.
    let sub_index = (pos % U256::from(STEM_SUBTREE_WIDTH)).as_u64() as u8;
    get_tree_key(address, tree_index, sub_index)
}

/// Returns the tree key for a storage slot.
///
/// Slots 0–63 map to the header storage area (sub_indices 64–127).
/// Slots ≥ 64 map to the main storage area starting at 2^248.
pub fn get_tree_key_for_storage_slot(address: &Address, storage_key: U256) -> [u8; 32] {
    let header_capacity = U256::from(CODE_OFFSET - HEADER_STORAGE_OFFSET); // 64
    if storage_key < header_capacity {
        let pos = U256::from(HEADER_STORAGE_OFFSET) + storage_key;
        let tree_index = pos / U256::from(STEM_SUBTREE_WIDTH);
        // Safe: pos % 256 is always 0–255, fits in u8.
        let sub_index = (pos % U256::from(STEM_SUBTREE_WIDTH)).as_u64() as u8;
        get_tree_key(address, tree_index, sub_index)
    } else {
        // pos = MAIN_STORAGE_OFFSET + storage_key = 2^248 + storage_key
        // This can overflow U256 when storage_key >= 2^256 - 2^248.
        // Compute tree_index and sub_index without materializing pos:
        //   sub_index = (2^248 + storage_key) % 256
        //             = (0 + storage_key) % 256     (since 2^248 % 256 = 0)
        //             = storage_key % 256
        //   tree_index = (2^248 + storage_key) / 256
        //              = 2^240 + storage_key / 256   (when storage_key % 256 == sub_index,
        //                                             exact since 2^248 is divisible by 256)
        // But storage_key / 256 + 2^240 can also overflow if storage_key is huge.
        // Use overflowing_add for safety — the result is truncated to 256 bits,
        // which then gets hashed in get_tree_key anyway.
        let sub_index = (storage_key % U256::from(STEM_SUBTREE_WIDTH)).as_u64() as u8;
        let (tree_index, _) = (main_storage_offset() / U256::from(STEM_SUBTREE_WIDTH))
            .overflowing_add(storage_key / U256::from(STEM_SUBTREE_WIDTH));
        get_tree_key(address, tree_index, sub_index)
    }
}

/// Packs account header fields into the 32-byte basic_data leaf layout.
///
/// Layout (big-endian):
/// - byte 0:     version
/// - bytes 1–4:  reserved (zeros)
/// - bytes 5–7:  code_size (3 bytes, upper byte of the u32 must be 0)
/// - bytes 8–15: nonce (8 bytes)
/// - bytes 16–31: balance (low 128 bits of U256, 16 bytes)
pub fn pack_basic_data(version: u8, code_size: u32, nonce: u64, balance: U256) -> [u8; 32] {
    debug_assert!(
        code_size <= 0x00FF_FFFF,
        "code_size {code_size} exceeds 3-byte field (max 16,777,215)"
    );
    debug_assert!(
        balance <= U256::from(u128::MAX),
        "balance exceeds EIP-7864 128-bit field"
    );

    let mut data = [0u8; 32];

    data[0] = version;
    // bytes 1–4: reserved, already zero

    // code_size in 3 bytes big-endian (bytes 5–7)
    let cs_bytes = code_size.to_be_bytes(); // [b0, b1, b2, b3] — b0 must be 0
    data[5] = cs_bytes[1];
    data[6] = cs_bytes[2];
    data[7] = cs_bytes[3];

    // nonce in 8 bytes big-endian (bytes 8–15)
    data[8..16].copy_from_slice(&nonce.to_be_bytes());

    // balance in 16 bytes big-endian (bytes 16–31) — low 128 bits of U256
    let balance_bytes = balance.to_big_endian(); // 32 bytes, big-endian
    data[16..32].copy_from_slice(&balance_bytes[16..32]);

    data
}

/// Unpacks a 32-byte basic_data leaf into (version, code_size, nonce, balance).
pub fn unpack_basic_data(data: &[u8; 32]) -> (u8, u32, u64, U256) {
    let version = data[0];

    // code_size from bytes 5–7 (3 bytes big-endian)
    let code_size = u32::from_be_bytes([0, data[5], data[6], data[7]]);

    // nonce from bytes 8–15
    let nonce = u64::from_be_bytes(data[8..16].try_into().expect("slice has length 8"));

    // balance from bytes 16–31 (16 bytes, low 128 bits)
    let mut balance_bytes = [0u8; 32];
    balance_bytes[16..32].copy_from_slice(&data[16..32]);
    let balance = U256::from_big_endian(&balance_bytes);

    (version, code_size, nonce, balance)
}

const PUSH_OFFSET: u8 = 95;
const PUSH1: u8 = PUSH_OFFSET + 1; // 96
const PUSH32: u8 = PUSH_OFFSET + 32; // 127

/// Splits EVM bytecode into 32-byte chunks for storage in the binary trie.
///
/// Each chunk is 32 bytes: 1 leading byte (how many of the following bytes are PUSH data,
/// capped at 31) + 31 bytes of code. The code is zero-padded to a multiple of 31 bytes first.
///
/// Based on the EIP-7864 reference implementation.
pub fn chunkify_code(code: &[u8]) -> Vec<[u8; 32]> {
    if code.is_empty() {
        return Vec::new();
    }

    // Pad code to a multiple of 31 bytes.
    let padded_len = if code.len().is_multiple_of(31) {
        code.len()
    } else {
        code.len() + (31 - code.len() % 31)
    };
    let mut padded = vec![0u8; padded_len];
    padded[..code.len()].copy_from_slice(code);

    // bytes_to_exec_data[i] = number of remaining PUSH data bytes at position i.
    let mut bytes_to_exec_data = vec![0u8; padded_len + 32];

    let mut pos = 0usize;
    while pos < padded_len {
        let byte = padded[pos];
        let pushdata_bytes = if (PUSH1..=PUSH32).contains(&byte) {
            (byte - PUSH_OFFSET) as usize
        } else {
            0
        };
        pos += 1;
        for x in 0..pushdata_bytes {
            if pos + x < bytes_to_exec_data.len() {
                bytes_to_exec_data[pos + x] = (pushdata_bytes - x) as u8;
            }
        }
        pos += pushdata_bytes;
    }

    // Build chunks: 1 leading byte + 31 code bytes each.
    let num_chunks = padded_len / 31;
    let mut chunks = Vec::with_capacity(num_chunks);
    for i in 0..num_chunks {
        let code_pos = i * 31;
        let mut chunk = [0u8; 32];
        chunk[0] = bytes_to_exec_data[code_pos].min(31);
        chunk[1..32].copy_from_slice(&padded[code_pos..code_pos + 31]);
        chunks.push(chunk);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_address() -> Address {
        Address::zero()
    }

    fn sample_address() -> Address {
        // 0x1234567890abcdef1234567890abcdef12345678
        Address::from([
            0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab,
            0xcd, 0xef, 0x12, 0x34, 0x56, 0x78,
        ])
    }

    #[test]
    fn test_address_to_address32() {
        let addr = sample_address();
        let result = old_style_address_to_address32(&addr);

        // First 12 bytes must be zero.
        assert_eq!(&result[..12], &[0u8; 12]);
        // Last 20 bytes must match the address.
        assert_eq!(&result[12..], addr.as_bytes());
    }

    #[test]
    fn test_address_to_address32_zero() {
        let addr = zero_address();
        let result = old_style_address_to_address32(&addr);
        assert_eq!(result, [0u8; 32]);
    }

    #[test]
    fn test_get_tree_key_basic_data() {
        let addr = sample_address();
        let key = get_tree_key_for_basic_data(&addr);

        // Must be 32 bytes.
        assert_eq!(key.len(), 32);
        // Last byte must be BASIC_DATA_LEAF_KEY = 0.
        assert_eq!(key[31], BASIC_DATA_LEAF_KEY);

        // Stem (first 31 bytes) must be deterministic.
        let key2 = get_tree_key_for_basic_data(&addr);
        assert_eq!(key, key2);
    }

    #[test]
    fn test_get_tree_key_code_hash() {
        let addr = sample_address();
        let key = get_tree_key_for_code_hash(&addr);

        // Last byte must be CODE_HASH_LEAF_KEY = 1.
        assert_eq!(key[31], CODE_HASH_LEAF_KEY);

        // Stem must be same as basic_data (both use tree_index=0).
        let basic_key = get_tree_key_for_basic_data(&addr);
        assert_eq!(key[..31], basic_key[..31]);
    }

    #[test]
    fn test_get_tree_key_basic_data_and_code_hash_differ() {
        let addr = sample_address();
        let basic = get_tree_key_for_basic_data(&addr);
        let code_hash = get_tree_key_for_code_hash(&addr);
        // Same stem, different sub_index.
        assert_ne!(basic, code_hash);
        assert_eq!(basic[..31], code_hash[..31]);
    }

    #[test]
    fn test_get_tree_key_code_chunk_zero() {
        let addr = sample_address();
        // chunk 0: pos = CODE_OFFSET + 0 = 128, tree_index = 128/256 = 0, sub_index = 128
        let key = get_tree_key_for_code_chunk(&addr, 0);
        assert_eq!(key[31], 128u8);

        // Should have tree_index=0, same stem as basic_data.
        let basic = get_tree_key_for_basic_data(&addr);
        assert_eq!(key[..31], basic[..31]);
    }

    #[test]
    fn test_get_tree_key_code_chunk_127() {
        let addr = sample_address();
        // chunk 127: pos = 128 + 127 = 255, tree_index = 255/256 = 0, sub_index = 255
        let key = get_tree_key_for_code_chunk(&addr, 127);
        assert_eq!(key[31], 255u8);
    }

    #[test]
    fn test_get_tree_key_code_chunk_128() {
        let addr = sample_address();
        // chunk 128: pos = 128 + 128 = 256, tree_index = 256/256 = 1, sub_index = 0
        let key = get_tree_key_for_code_chunk(&addr, 128);
        assert_eq!(key[31], 0u8);

        // tree_index=1, so stem differs from tree_index=0.
        let basic = get_tree_key_for_basic_data(&addr);
        assert_ne!(key[..31], basic[..31]);
    }

    #[test]
    fn test_get_tree_key_storage_slot_header_slot0() {
        let addr = sample_address();
        // slot 0: pos = HEADER_STORAGE_OFFSET + 0 = 64, tree_index = 0, sub_index = 64
        let key = get_tree_key_for_storage_slot(&addr, U256::from(0u64));
        assert_eq!(key[31], 64u8);

        // Same tree_index=0 → same stem as basic_data.
        let basic = get_tree_key_for_basic_data(&addr);
        assert_eq!(key[..31], basic[..31]);
    }

    #[test]
    fn test_get_tree_key_storage_slot_header_slot63() {
        let addr = sample_address();
        // slot 63: pos = 64 + 63 = 127, tree_index = 0, sub_index = 127
        let key = get_tree_key_for_storage_slot(&addr, U256::from(63u64));
        assert_eq!(key[31], 127u8);
    }

    #[test]
    fn test_get_tree_key_storage_slot_main_slot64() {
        let addr = sample_address();
        // slot 64 is >= 64 → main storage: pos = MAIN_STORAGE_OFFSET + 64
        let key = get_tree_key_for_storage_slot(&addr, U256::from(64u64));

        // tree_index = (2^248 + 64) / 256, which is enormous → stem differs from tree_index=0
        let basic = get_tree_key_for_basic_data(&addr);
        assert_ne!(key[..31], basic[..31]);

        // sub_index = (2^248 + 64) % 256 = 64 % 256 = 64
        assert_eq!(key[31], 64u8);
    }

    #[test]
    fn test_get_tree_key_storage_slot_main_vs_header_differ() {
        let addr = sample_address();
        // slot 0 (header) and slot 64 (main) must have different stems.
        let header_key = get_tree_key_for_storage_slot(&addr, U256::from(0u64));
        let main_key = get_tree_key_for_storage_slot(&addr, U256::from(64u64));
        assert_ne!(header_key[..31], main_key[..31]);
    }

    #[test]
    fn test_pack_unpack_basic_data_roundtrip() {
        let version = 0u8;
        let code_size = 1234u32;
        let nonce = 42u64;
        let balance = U256::from(999_999_999_999u64);

        let packed = pack_basic_data(version, code_size, nonce, balance);
        let (v2, cs2, n2, b2) = unpack_basic_data(&packed);

        assert_eq!(version, v2);
        assert_eq!(code_size, cs2);
        assert_eq!(nonce, n2);
        assert_eq!(balance, b2);
    }

    #[test]
    fn test_pack_basic_data_layout() {
        let packed = pack_basic_data(0, 0, 0, U256::zero());
        assert_eq!(packed, [0u8; 32]);
    }

    #[test]
    fn test_pack_basic_data_version_byte() {
        let packed = pack_basic_data(0xAB, 0, 0, U256::zero());
        assert_eq!(packed[0], 0xAB);
        assert_eq!(&packed[1..], &[0u8; 31]);
    }

    #[test]
    fn test_pack_basic_data_code_size_3_bytes() {
        // code_size = 0x010203 → bytes 5=0x01, 6=0x02, 7=0x03
        let packed = pack_basic_data(0, 0x010203, 0, U256::zero());
        assert_eq!(packed[5], 0x01);
        assert_eq!(packed[6], 0x02);
        assert_eq!(packed[7], 0x03);
    }

    #[test]
    fn test_pack_basic_data_nonce() {
        let packed = pack_basic_data(0, 0, 0x0102030405060708u64, U256::zero());
        assert_eq!(
            &packed[8..16],
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
    }

    #[test]
    fn test_pack_basic_data_balance() {
        // balance = 1 → last 16 bytes should end in 0x01
        let packed = pack_basic_data(0, 0, 0, U256::from(1u64));
        assert_eq!(&packed[16..31], &[0u8; 15]);
        assert_eq!(packed[31], 0x01);
    }

    #[test]
    fn test_chunkify_code_empty() {
        let chunks = chunkify_code(&[]);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunkify_code_simple_no_push() {
        // 31 bytes of STOP (0x00), no PUSH instructions
        let code = vec![0x00u8; 31];
        let chunks = chunkify_code(&code);
        assert_eq!(chunks.len(), 1);
        // Leading byte should be 0 (no pushdata)
        assert_eq!(chunks[0][0], 0);
        // Rest should be the code bytes
        assert_eq!(&chunks[0][1..], &code[..]);
    }

    #[test]
    fn test_chunkify_code_pads_to_31() {
        // 1 byte code → padded to 31 → 1 chunk
        let code = vec![0x00u8; 1];
        let chunks = chunkify_code(&code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0][0], 0);
        assert_eq!(chunks[0][1], 0x00);
        // bytes 2–31 should be zero padding
        assert_eq!(&chunks[0][2..], &[0u8; 30]);
    }

    #[test]
    fn test_chunkify_code_with_push1() {
        // PUSH1 (0x60) followed by one data byte (0xAA), then STOP (0x00)
        // pos 0: PUSH1 → pushdata_bytes = 1
        // pos 1: bytes_to_exec_data[1] = 1 (remaining push data)
        // pos 2: STOP, pushdata_bytes = 0
        let code = vec![0x60u8, 0xAA, 0x00];
        let chunks = chunkify_code(&code);
        assert_eq!(chunks.len(), 1);
        // Leading byte = bytes_to_exec_data[0] = 0 (position 0 is the PUSH1 opcode itself)
        assert_eq!(chunks[0][0], 0);
        assert_eq!(chunks[0][1], 0x60); // PUSH1
        assert_eq!(chunks[0][2], 0xAA); // push data
        assert_eq!(chunks[0][3], 0x00); // STOP
    }

    #[test]
    fn test_chunkify_code_with_push_spanning_chunks() {
        // Build a 62-byte code: 31 NOPs, then PUSH32 + 32 data bytes (but truncated to fit)
        // Actually use PUSH1 at position 30 so its data byte is at position 31 (in chunk 2).
        let mut code = vec![0x00u8; 30]; // 30 NOPs
        code.push(0x60u8); // PUSH1 at position 30
        code.push(0xBBu8); // data byte at position 31

        let chunks = chunkify_code(&code);
        assert_eq!(chunks.len(), 2);

        // Chunk 0 covers code[0..31]: 30 NOPs + PUSH1
        // bytes_to_exec_data[0..30] = 0, bytes_to_exec_data[30] = 0 (opcode position)
        assert_eq!(chunks[0][0], 0);

        // Chunk 1 covers code[31..62]: 0xBB (push data) + padding
        // bytes_to_exec_data[31] = 1 (remaining push data from PUSH1 at pos 30)
        assert_eq!(chunks[1][0], 1);
        assert_eq!(chunks[1][1], 0xBB);
    }

    #[test]
    fn test_chunkify_code_push32_leading_byte() {
        // PUSH32 (0x7f) at the very start: opcode at pos 0, data at positions 1–32
        // Chunk 0 (positions 0–30): opcode + 30 data bytes
        //   bytes_to_exec_data[0] = 0 (opcode), [1]=32, [2]=31, ..., [30]=3
        // Chunk 1 (positions 31–61): 2 data bytes + padding
        //   bytes_to_exec_data[31]=2, [32]=1, [33..]=0
        let mut code = vec![0x7fu8]; // PUSH32
        code.extend_from_slice(&[0xCCu8; 32]); // 32 data bytes

        let chunks = chunkify_code(&code);
        // code is 33 bytes → padded to 62 (ceil(33/31)*31) → 2 chunks
        assert_eq!(chunks.len(), 2);

        // chunk[0][0] = bytes_to_exec_data[0] = 0 (PUSH32 is the opcode, not push data)
        assert_eq!(chunks[0][0], 0);
        // chunk[1][0] = bytes_to_exec_data[31] = min(2, 31) = 2
        assert_eq!(chunks[1][0], 2);
    }
}
