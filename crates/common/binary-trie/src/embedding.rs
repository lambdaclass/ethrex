//! EIP-8297 Ethereum state embedding: maps accounts, storage slots and
//! contract code onto binary-tree keys and 32-byte leaf values.
//!
//! Account and storage tries are merged into the single key/value tree
//! implemented in [`crate::trie`], which also holds contract code.
//!
//! The first byte of every key is a **zone** identifier labeling the
//! category of state the key holds: account headers live in
//! [`ACCOUNT_ZONE`], content-addressed overflow code in [`CODE_ZONE`],
//! and overflow storage in [`STORAGE_ZONE`]. Keys are variable length,
//! but every key of a zone has the same length, keeping keys
//! prefix-free as the tree requires.
//!
//! A key's **stem** is every byte except its final sub-index byte.
//! Keys sharing a stem form one group of up to [`STEM_SUBTREE_WIDTH`]
//! co-located values, all reachable through the same branch of the
//! tree. This keeps data that is accessed together cheap to prove: an
//! account's header stem holds its basic data, code hash, first
//! storage slots, and first code chunks, so one proof path covers
//! them all.

use ethereum_types::{H160, H256, U256};

use crate::error::BinaryTrieError;
use crate::trie::node::blake3_hash;

/// Sub-index of the account header leaf packing version, code size,
/// nonce, and balance.
pub const BASIC_DATA_LEAF_KEY: u8 = 0;

/// Version of the basic data leaf layout, packed as the leaf's first
/// byte by [`encode_basic_data`]. A future change to the layout bumps
/// the version so readers can tell the encodings apart.
pub const BASIC_DATA_VERSION: u8 = 0;

/// Sub-index of the account header leaf holding the code hash.
pub const CODE_HASH_LEAF_KEY: u8 = 1;

/// Sub-index of storage slot `0` within the account header stem.
/// Slots `0` through `63` live in the header.
pub const HEADER_STORAGE_OFFSET: u64 = 64;

/// Sub-index of code chunk `0` within the account header stem.
/// Chunks `0` through `127` live in the header.
pub const CODE_OFFSET: u64 = 128;

/// Maximum number of values grouped under a single stem: the size of
/// the sub-index byte's space.
pub const STEM_SUBTREE_WIDTH: u64 = 256;

/// Zone byte of account header stems.
pub const ACCOUNT_ZONE: u8 = 0;

/// Zone byte of content-addressed overflow code stems.
pub const CODE_ZONE: u8 = 1;

/// Zone byte of overflow storage stems.
///
/// Storage sits at the far end of the zone byte, leaving zones `2`
/// through `254` reserved for future state categories.
pub const STORAGE_ZONE: u8 = 255;

/// Length of every account zone key: the zone byte, a full address
/// digest, and the sub-index byte.
pub const ACCOUNT_KEY_LENGTH: usize = 34;

/// Length of every code zone key: the zone byte, a full digest of the
/// code hash and group index, and the sub-index byte.
pub const CODE_KEY_LENGTH: usize = 34;

/// Length of every storage zone key: the zone byte, two full digests
/// binding the account and its group index, and the sub-index byte.
pub const STORAGE_KEY_LENGTH: usize = 66;

/// 32-byte address used to key the tree. Legacy 20-byte addresses are
/// converted by [`address20_to_address32`].
pub type Address32 = [u8; 32];

/// A binary-tree key derived by this embedding.
pub type Key = Vec<u8>;

/// Convert a legacy 20-byte address by prepending 12 zero bytes.
///
/// The embedding keys the tree by 32-byte addresses so that a future
/// address-space extension needs no re-keying.
pub fn address20_to_address32(address: H160) -> Address32 {
    let mut out = [0u8; 32];
    out[12..].copy_from_slice(address.as_bytes());
    out
}

/// Hash `data` for use in tree key derivation.
///
/// In practice this reuses the tree's own merkleization hash,
/// [`blake3_hash`].
fn key_hash(data: &[u8]) -> H256 {
    blake3_hash(data)
}

/// Build a key from its three parts: the `zone` byte, the
/// hash-derived `tree_position`, and the final `sub_index` byte.
fn get_tree_key(zone: u8, tree_position: &[u8], sub_index: u8) -> Key {
    let mut key = Vec::with_capacity(2 + tree_position.len());
    key.push(zone);
    key.extend_from_slice(tree_position);
    key.push(sub_index);
    key
}

/// Compute the key of the account header leaf at `sub_index`.
///
/// The header stem is in [`ACCOUNT_ZONE`] and is keyed by the address
/// alone, so each account has exactly one header stem. The header is
/// not one key: it is up to [`STEM_SUBTREE_WIDTH`] separate leaves
/// sharing that stem, and `sub_index` selects which one; basic data,
/// code hash, an early storage slot, or an early code chunk.
///
/// `sub_index` is a `u8` because the sub-index space is exactly the
/// byte's range: every value is in bounds by construction, so callers
/// narrow where they establish the bound rather than relying on a
/// debug-only check here.
pub fn get_tree_key_for_header(address: &Address32, sub_index: u8) -> Key {
    let key = get_tree_key(ACCOUNT_ZONE, key_hash(address).as_bytes(), sub_index);
    debug_assert_eq!(key.len(), ACCOUNT_KEY_LENGTH);
    key
}

/// Compute the key of the account's basic data leaf.
pub fn get_tree_key_for_basic_data(address: &Address32) -> Key {
    get_tree_key_for_header(address, BASIC_DATA_LEAF_KEY)
}

/// Compute the key of the account's code hash leaf.
pub fn get_tree_key_for_code_hash(address: &Address32) -> Key {
    get_tree_key_for_header(address, CODE_HASH_LEAF_KEY)
}

/// Build the hash-derived position of an account's overflow storage
/// group at `tree_index`.
///
/// The position carries two full digests:
///
/// - `key_hash(address)` gathers all of an account's overflow storage
///   under one subtree, which future expiry and sync schemes could use
///   as their unit of work: a contract's whole storage is one
///   contiguous key range rather than locations scattered across the
///   whole tree.
/// - `key_hash(address ‖ tree_index)` spreads the account's groups
///   within that subtree.
///
/// Both digests depend on the address, so storage keys that an
/// attacker grinds to sit close together under one contract cannot be
/// reused against a different contract.
fn storage_tree_position(address: &Address32, tree_index: U256) -> Vec<u8> {
    let prefix = key_hash(address);
    let mut preimage = Vec::with_capacity(64);
    preimage.extend_from_slice(address);
    preimage.extend_from_slice(&tree_index.to_big_endian());
    let suffix = key_hash(&preimage);
    let mut position = Vec::with_capacity(64);
    position.extend_from_slice(prefix.as_bytes());
    position.extend_from_slice(suffix.as_bytes());
    position
}

/// Compute the key of a storage slot.
///
/// Slots `0` through `63` live in the account header stem at
/// sub-indices [`HEADER_STORAGE_OFFSET`] onward; all other slots live
/// in [`STORAGE_ZONE`], grouped [`STEM_SUBTREE_WIDTH`] consecutive
/// slots to a stem. This leaves group `0` (`tree_index == 0`) short:
/// its storage-zone leaves are only sub-indices `64`-`255`.
pub fn get_tree_key_for_storage_slot(address: &Address32, storage_key: U256) -> Key {
    if storage_key < U256::from(CODE_OFFSET - HEADER_STORAGE_OFFSET) {
        // `low_u64` cannot truncate: the slot is below 64.
        return get_tree_key_for_header(
            address,
            (HEADER_STORAGE_OFFSET + storage_key.low_u64()) as u8,
        );
    }
    let width = U256::from(STEM_SUBTREE_WIDTH);
    let tree_index = storage_key / width;
    // `low_u64` cannot truncate: the remainder is below 256.
    let sub_index = (storage_key % width).low_u64() as u8;
    let key = get_tree_key(
        STORAGE_ZONE,
        &storage_tree_position(address, tree_index),
        sub_index,
    );
    debug_assert_eq!(key.len(), STORAGE_KEY_LENGTH);
    key
}

/// Compute the key of a code chunk.
///
/// Chunks `0` through `127` live in the account header stem: the start
/// of a contract's code (usually dispatchers and entry points) is its
/// most executed region, so the first chunks open with the same branch
/// as the account's basic data.
///
/// Chunks at index `128` and above live in [`CODE_ZONE`],
/// content-addressed by `code_hash` so contracts with identical
/// bytecode share leaves.
pub fn get_tree_key_for_code_chunk(
    address: &Address32,
    code_hash: &[u8; 32],
    chunk_id: u64,
) -> Key {
    let header_chunk_count = STEM_SUBTREE_WIDTH - CODE_OFFSET;
    if chunk_id < header_chunk_count {
        return get_tree_key_for_header(address, (CODE_OFFSET + chunk_id) as u8);
    }
    let overflow = chunk_id - header_chunk_count;
    let tree_index = overflow / STEM_SUBTREE_WIDTH;
    let sub_index = (overflow % STEM_SUBTREE_WIDTH) as u8;
    let mut preimage = Vec::with_capacity(64);
    preimage.extend_from_slice(code_hash);
    preimage.extend_from_slice(&U256::from(tree_index).to_big_endian());
    let key = get_tree_key(CODE_ZONE, key_hash(&preimage).as_bytes(), sub_index);
    debug_assert_eq!(key.len(), CODE_KEY_LENGTH);
    key
}

/// Opcode value one below `PUSH1`, so `PUSH_OFFSET + n` is the opcode
/// pushing `n` bytes.
pub const PUSH_OFFSET: u8 = 95;

/// Opcode of the smallest push instruction.
pub const PUSH1: u8 = PUSH_OFFSET + 1;

/// Opcode of the largest push instruction.
pub const PUSH32: u8 = PUSH_OFFSET + 32;

/// Split `code` into the 32-byte chunks stored in the tree.
///
/// Chunk `i` holds the `i`-th 31-byte slice of the code (zero-padded)
/// in bytes `1` through `31`, preceded by one byte counting how many
/// of the slice's leading bytes are data of a push instruction that
/// began in an earlier chunk. The count lets a chunk be interpreted
/// without its predecessors and is capped at `31`, the chunk payload
/// size.
pub fn chunkify_code(code: &[u8]) -> Vec<[u8; 32]> {
    let padded_len = code.len().div_ceil(31) * 31;
    let mut code = code.to_vec();
    code.resize(padded_len, 0);

    // Number of push-data bytes remaining at each position, counting
    // the position itself; `0` marks executable bytes. The extra 32
    // entries let the largest push record data past the end of the
    // code.
    let mut remaining_push_data = vec![0u8; padded_len + 32];
    let mut position = 0;
    while position < padded_len {
        let opcode = code[position];
        let push_data_bytes = if (PUSH1..=PUSH32).contains(&opcode) {
            (opcode - PUSH_OFFSET) as usize
        } else {
            0
        };
        position += 1;
        for offset in 0..push_data_bytes {
            remaining_push_data[position + offset] = (push_data_bytes - offset) as u8;
        }
        position += push_data_bytes;
    }

    (0..padded_len)
        .step_by(31)
        .map(|start| {
            let mut chunk = [0u8; 32];
            chunk[0] = remaining_push_data[start].min(31);
            chunk[1..].copy_from_slice(&code[start..start + 31]);
            chunk
        })
        .collect()
}

/// Pack an account's basic data into the 32-byte value stored at
/// [`BASIC_DATA_LEAF_KEY`].
///
/// The fields are packed big-endian: one version byte, three reserved
/// zero bytes, four bytes of code size, eight bytes of nonce, and
/// sixteen bytes of balance. Balances are protocol-level `U256`
/// values, so the sixteen-byte field bound is checked here and
/// [`BinaryTrieError::BalanceTooLarge`] returned past it.
///
/// Note: the 4-byte code size at offset 4 follows the EELS branch
/// this crate is ported from, which differs from EIP-7864's 3-byte
/// field at offset 5; the conformance fixture pins this choice.
pub fn encode_basic_data(
    code_size: u32,
    nonce: u64,
    balance: U256,
) -> Result<[u8; 32], BinaryTrieError> {
    if balance >= U256::from(1) << 128 {
        return Err(BinaryTrieError::BalanceTooLarge);
    }
    let mut out = [0u8; 32];
    out[0] = BASIC_DATA_VERSION;
    // Bytes 1..4 are reserved zeros: headroom for future header fields.
    out[4..8].copy_from_slice(&code_size.to_be_bytes());
    out[8..16].copy_from_slice(&nonce.to_be_bytes());
    out[16..32].copy_from_slice(&balance.to_big_endian()[16..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{H160, U256};
    use hex_literal::hex;

    const ADDR20: H160 = H160(hex!("00112233445566778899aabbccddeeff00112233"));

    #[test]
    fn address32_prepends_twelve_zero_bytes() {
        let a32 = address20_to_address32(ADDR20);
        assert_eq!(&a32[..12], &[0u8; 12]);
        assert_eq!(&a32[12..], ADDR20.as_bytes());
    }

    #[test]
    fn basic_data_key_vector() {
        // fixture: embedding.basic_data_key
        assert_eq!(
            get_tree_key_for_basic_data(&address20_to_address32(ADDR20)),
            hex!("00f4e42504054ae2ba2c9aab59b7cafad1e3df583c385d10fcb8ab0a0ab82e7a0800").to_vec()
        );
    }

    #[test]
    fn header_key_layout() {
        let a32 = address20_to_address32(ADDR20);
        let key = get_tree_key_for_header(&a32, 255);
        assert_eq!(key.len(), ACCOUNT_KEY_LENGTH);
        assert_eq!(key[0], ACCOUNT_ZONE);
        assert_eq!(key[33], 255);
        assert_eq!(get_tree_key_for_code_hash(&a32)[..33], key[..33]);
        assert_eq!(get_tree_key_for_code_hash(&a32)[33], 1);
    }

    #[test]
    fn storage_slot_63_in_header_64_in_storage_zone() {
        let a32 = address20_to_address32(ADDR20);
        let slot63 = get_tree_key_for_storage_slot(&a32, U256::from(63));
        let slot64 = get_tree_key_for_storage_slot(&a32, U256::from(64));
        assert_eq!(slot63.len(), ACCOUNT_KEY_LENGTH);
        assert_eq!(slot63[0], ACCOUNT_ZONE);
        assert_eq!(slot63[33], 64 + 63);
        assert_eq!(slot64.len(), STORAGE_KEY_LENGTH);
        assert_eq!(slot64[0], STORAGE_ZONE);
        assert_eq!(slot64[65], 64);
    }

    #[test]
    fn storage_slot_group_zero_is_short() {
        let a32 = address20_to_address32(ADDR20);
        let k255 = get_tree_key_for_storage_slot(&a32, U256::from(255));
        let k256 = get_tree_key_for_storage_slot(&a32, U256::from(256));
        assert_eq!(
            k255[..65],
            get_tree_key_for_storage_slot(&a32, U256::from(64))[..65]
        );
        assert_ne!(k255[..65], k256[..65]);
    }

    #[test]
    fn huge_storage_key_does_not_overflow() {
        let a32 = address20_to_address32(ADDR20);
        let key = get_tree_key_for_storage_slot(&a32, U256::from(2).pow(U256::from(200)));
        assert_eq!(key.len(), STORAGE_KEY_LENGTH);
        assert_eq!(key[0], STORAGE_ZONE);
    }

    #[test]
    fn code_chunk_127_in_header_128_in_code_zone() {
        let a32 = address20_to_address32(ADDR20);
        let code_hash = [0x11u8; 32];
        let c127 = get_tree_key_for_code_chunk(&a32, &code_hash, 127);
        let c128 = get_tree_key_for_code_chunk(&a32, &code_hash, 128);
        assert_eq!(c127[0], ACCOUNT_ZONE);
        assert_eq!(c127[33], 128 + 127);
        assert_eq!(c128[0], CODE_ZONE);
        assert_eq!(c128.len(), CODE_KEY_LENGTH);
        assert_eq!(c128[33], 0);
    }

    #[test]
    fn chunkify_empty_code_is_empty() {
        assert!(chunkify_code(&[]).is_empty());
    }

    #[test]
    fn chunkify_pads_to_31_and_prepends_offset_byte() {
        let chunks = chunkify_code(&[0x00]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0][0], 0);
        assert_eq!(chunks[0][1], 0x00);
        assert_eq!(&chunks[0][2..], &[0u8; 30]);
    }

    #[test]
    fn chunkify_push4_spilling_into_next_chunk() {
        // PUSH4 at position 29; data at 30..34; chunk 1 starts at 31
        // with 3 leading push-data bytes.
        let mut code = vec![0x01; 29];
        code.push(0x63);
        code.extend([0xaa, 0xbb, 0xcc, 0xdd]);
        code.extend([0x01; 10]);
        let chunks = chunkify_code(&code);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0][0], 0);
        assert_eq!(chunks[1][0], 3);
    }

    #[test]
    fn chunkify_caps_offset_byte_at_31() {
        // PUSH32 as last byte of chunk 0: all 32 data bytes follow, but
        // the count byte saturates at 31.
        let mut code = vec![0x01; 30];
        code.push(0x7f);
        code.extend(0..32u8);
        code.extend([0x01; 5]);
        let chunks = chunkify_code(&code);
        assert_eq!(chunks[1][0], 31);
    }

    #[test]
    fn encode_basic_data_layout_vector() {
        // fixture: encode_basic_data[1]
        assert_eq!(
            encode_basic_data(1234, 42, U256::from(10).pow(U256::from(18))).unwrap(),
            hex!("00000000000004d2000000000000002a00000000000000000de0b6b3a7640000")
        );
    }

    #[test]
    fn encode_basic_data_rejects_balance_at_2_pow_128() {
        assert_eq!(
            encode_basic_data(0, 0, U256::from(1) << 128),
            Err(BinaryTrieError::BalanceTooLarge)
        );
        assert!(encode_basic_data(0, 0, (U256::from(1) << 128) - 1).is_ok());
    }

    #[test]
    fn overflow_code_is_content_addressed_not_per_account() {
        let a = address20_to_address32(ADDR20);
        let b = address20_to_address32(H160([0x99; 20]));
        let code_hash = [0x11u8; 32];
        assert_eq!(
            get_tree_key_for_code_chunk(&a, &code_hash, 128),
            get_tree_key_for_code_chunk(&b, &code_hash, 128)
        );
        assert_ne!(
            get_tree_key_for_code_chunk(&a, &code_hash, 0),
            get_tree_key_for_code_chunk(&b, &code_hash, 0)
        );
    }
}
