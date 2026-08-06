//! Applying ethrex [`AccountUpdate`]s to an EIP-8297 binary trie.
//!
//! This is where the embedding and the trie meet: the embedding
//! ([`ethrex_binary_trie::embedding`]) says which key an account field,
//! storage slot or code chunk lands on, and this module drives the
//! resulting inserts and removals into a [`BinaryTrie`]. The embedding
//! itself stays free of trie concerns, and the trie crate stays free of
//! ethrex's own types.
//!
//! # Zero means absent
//!
//! Every leaf written here goes through [`state_write`], which
//! resolves a value of 32 zero bytes to a removal rather than an
//! insertion. That collapse is the state model's to make — the trie
//! stores any 32-byte value quite happily — and the spec makes it for
//! every state leaf, so a state written through this module can never
//! commit to a zero-valued leaf.
//!
//! # Removal is only partly supported
//!
//! Flat state made removal trivial — an account was one map entry. A
//! trie is not so kind, and the three parts of an account's state
//! behave differently:
//!
//! - **Header leaves are enumerable.** An account's header stem is a
//!   known 33 bytes and its leaves are sub-indices `0..=255`, so
//!   clearing them is a bounded loop of at most 256 removals. That
//!   covers basic data, the code hash, storage slots `0..=63` and code
//!   chunks `0..=127`.
//! - **Overflow code must not be removed.** Chunks at index `128` and
//!   above are content-addressed, so two accounts with identical
//!   bytecode share the very same leaves; deleting one account's
//!   overflow chunks would corrupt the other. They are therefore left
//!   in place — the same reason ethrex never prunes its `ACCOUNT_CODES`
//!   table.
//! - **Overflow storage is unbounded and cannot be enumerated.** Slots
//!   at `64` and above live under `0xff ‖ blake3(address) ‖ …`. The
//!   embedding deliberately makes them contiguous, but [`BinaryTrie`]
//!   offers no iteration and no prefix removal, so their extent is
//!   unknowable from the trie alone.
//!
//! Rather than silently leave orphaned storage behind, a removal this
//! module can see would strand overflow storage is refused with
//! [`PbtStateError::OverflowStorageNotRemovable`]. What it can see is
//! the batch it is given: any overflow-zone slot written for an account
//! anywhere in the same `updates` slice. That is exactly the modern
//! case — post-EIP-6780 `SELFDESTRUCT` only takes effect in the
//! transaction that created the account, so a removable account's
//! storage was necessarily written in the same block. Overflow storage
//! written by an *earlier* block is invisible here and would be
//! stranded silently: a correctness gap for old-chain replay, not for
//! live operation on a modern chain.
//!
//! TODO: fix this properly by giving [`BinaryTrie`] a prefix-removal
//! (or prefix-iteration) operation and calling it on
//! `0xff ‖ blake3(address32)`, the prefix under which the embedding
//! already gathers an account's entire overflow storage as one
//! contiguous subtree. Until then the gap above stands.

use std::collections::HashSet;

use ethrex_binary_trie::BinaryTrieError;
use ethrex_binary_trie::embedding::{
    CODE_OFFSET, HEADER_STORAGE_OFFSET, address20_to_address32, chunkify_code, encode_basic_data,
    get_tree_key_for_basic_data, get_tree_key_for_code_chunk, get_tree_key_for_code_hash,
    get_tree_key_for_header, get_tree_key_for_storage_slot,
};
use ethrex_binary_trie::trie::BinaryTrie;

use crate::constants::EMPTY_KECCAK_HASH;
use crate::types::{AccountInfo, AccountUpdate};
use crate::{Address, H256, U256};

/// Byte range of the code size field inside an encoded basic-data leaf,
/// as laid out by [`encode_basic_data`].
const BASIC_DATA_CODE_SIZE_RANGE: std::ops::Range<usize> = 4..8;

/// Byte range of the nonce field inside an encoded basic-data leaf.
const BASIC_DATA_NONCE_RANGE: std::ops::Range<usize> = 8..16;

/// Byte range of the balance field inside an encoded basic-data leaf.
const BASIC_DATA_BALANCE_RANGE: std::ops::Range<usize> = 16..32;

#[derive(Debug, thiserror::Error)]
pub enum PbtStateError {
    #[error(transparent)]
    Trie(#[from] BinaryTrieError),
    /// The account's storage cannot be cleared because part of it lives
    /// in the overflow storage zone, which the trie cannot enumerate.
    /// See the module docs.
    #[error(
        "account {0:?}: cannot clear storage, it reaches past slot 63 into the overflow storage \
         zone, which the binary trie can neither enumerate nor remove by prefix"
    )]
    OverflowStorageNotRemovable(Address),
    /// The update changed an account's basic data without carrying its
    /// bytecode, and the trie holds no previous basic-data leaf to take
    /// the code size from.
    #[error(
        "account {0:?}: code size unknown, the update carries no bytecode and the account has no \
         basic-data leaf to read the previous size from"
    )]
    UnknownCodeSize(Address),
}

/// Apply `updates` to `trie`.
///
/// Order within the slice does not affect the resulting trie: every
/// update writes and removes keys derived from its own account, and the
/// overflow-storage check is computed over the whole batch up front.
///
/// See the module docs for what removal does and does not cover.
pub fn apply_account_updates(
    trie: &mut BinaryTrie,
    updates: &[AccountUpdate],
) -> Result<(), PbtStateError> {
    let overflow_storage = accounts_with_overflow_storage(updates);
    for update in updates {
        apply_account_update(trie, update, &overflow_storage)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reads
//
// The counterpart of [`apply_account_updates`], and in this module for one
// reason: a read has to look under exactly the key the write put the value on.
// Both sides therefore derive their keys through the same `embedding` calls, a
// few lines apart, so the two cannot drift; a second copy of the key layout
// somewhere else could disagree with this one, and the disagreement would show
// up as silently missing state rather than as a failure.
// ---------------------------------------------------------------------------

/// The account `trie` holds at `address`, or `None` if it holds none.
///
/// # What counts as existing
///
/// The code-hash leaf is the account's tombstone-proof marker: every write
/// that creates or updates an account emits it, and it can never collapse to
/// absence, because no code hash is 32 zero bytes — an account with no code
/// carries `EMPTY_KECCAK_HASH`. The basic-data leaf cannot play that role: an
/// account with zero nonce, zero balance and no code encodes to 32 zero bytes,
/// which [`state_write`] resolves to absence. So an account is present when
/// *either* leaf is, and the basic data missing means zeros rather than
/// nonexistence.
///
/// # No storage root
///
/// [`AccountInfo`] has no `storage_root`, and that is why it is what this
/// returns. The binary trie has no such value to report: storage is not a
/// per-account subtrie there, it is leaves of the one unified tree. Callers
/// that need an `AccountState` have to decide what to put in that field, and
/// the decision belongs to them rather than here — see `StoreVmDatabase` in
/// `crates/blockchain/vm.rs`, which makes it and documents what it costs.
pub fn get_account_info(
    trie: &mut BinaryTrie,
    address: Address,
) -> Result<Option<AccountInfo>, PbtStateError> {
    let address32 = address20_to_address32(address);
    let code_hash = trie.get(&get_tree_key_for_code_hash(&address32))?;
    let basic_data = trie.get(&get_tree_key_for_basic_data(&address32))?;
    if code_hash.is_none() && basic_data.is_none() {
        return Ok(None);
    }
    let (nonce, balance) = basic_data.map_or((0, U256::zero()), |data| decode_basic_data(&data));
    Ok(Some(AccountInfo {
        code_hash: code_hash.map_or(*EMPTY_KECCAK_HASH, H256),
        balance,
        nonce,
    }))
}

/// The value `trie` holds for `address`'s storage `slot`, or `None` if it
/// holds none.
///
/// `None` is the same answer the MPT gives for an unwritten slot, and for the
/// same reason: a slot written to zero is stored as absent (see
/// [`state_write`]), so absence and zero are one state on both sides. Callers
/// read either as zero, which is what the EVM does.
pub fn get_storage_slot(
    trie: &mut BinaryTrie,
    address: Address,
    slot: &H256,
) -> Result<Option<U256>, PbtStateError> {
    let address32 = address20_to_address32(address);
    let key = get_tree_key_for_storage_slot(&address32, slot_index(slot));
    Ok(trie.get(&key)?.map(|value| U256::from_big_endian(&value)))
}

/// The nonce and balance packed into an encoded basic-data leaf.
///
/// The inverse of [`encode_basic_data`] for the two fields a read needs; the
/// code size is only ever consumed by [`resolve_code_size`], which reads it
/// straight out of the leaf.
fn decode_basic_data(basic_data: &[u8; 32]) -> (u64, U256) {
    let mut nonce = [0u8; 8];
    nonce.copy_from_slice(&basic_data[BASIC_DATA_NONCE_RANGE]);
    (
        u64::from_be_bytes(nonce),
        U256::from_big_endian(&basic_data[BASIC_DATA_BALANCE_RANGE]),
    )
}

/// Addresses this batch writes an overflow-zone storage slot for.
///
/// The whole batch is scanned before anything is applied, so the answer
/// — and therefore which removals are refused — does not depend on the
/// order of `updates`.
fn accounts_with_overflow_storage(updates: &[AccountUpdate]) -> HashSet<Address> {
    updates
        .iter()
        .filter(|update| {
            update
                .added_storage
                .keys()
                .any(|slot| !slot_lives_in_header(slot_index(slot)))
        })
        .map(|update| update.address)
        .collect()
}

fn slot_index(slot: &H256) -> U256 {
    U256::from_big_endian(slot.as_bytes())
}

/// Whether a storage slot lives in the account header stem, which holds
/// slots `0` through `63`.
fn slot_lives_in_header(slot: U256) -> bool {
    slot < U256::from(CODE_OFFSET - HEADER_STORAGE_OFFSET)
}

fn apply_account_update(
    trie: &mut BinaryTrie,
    update: &AccountUpdate,
    overflow_storage: &HashSet<Address>,
) -> Result<(), PbtStateError> {
    let address32 = address20_to_address32(update.address);

    if update.removed {
        refuse_if_overflow_storage(update.address, overflow_storage)?;
        // The whole header stem goes: basic data, code hash, storage
        // slots 0..=63 and code chunks 0..=127. Overflow code chunks
        // are content-addressed and shared, so they stay.
        remove_header_leaves(trie, &address32, 0..=u8::MAX)?;
        return Ok(());
    }

    if update.removed_storage {
        refuse_if_overflow_storage(update.address, overflow_storage)?;
        // Only the storage region of the header: the account survives.
        remove_header_leaves(
            trie,
            &address32,
            header_storage_sub_index(0)..=header_storage_sub_index(63),
        )?;
    }

    if let Some(info) = &update.info {
        let code_size = resolve_code_size(trie, update, info)?;
        state_write(
            trie,
            get_tree_key_for_basic_data(&address32),
            encode_basic_data(code_size, info.nonce, info.balance)?,
        )?;
        state_write(
            trie,
            get_tree_key_for_code_hash(&address32),
            info.code_hash.0,
        )?;
    }

    if let Some(code) = &update.code {
        // `Code::hash` is a placeholder for initcode, which never
        // reaches an account update; where the update states the hash,
        // that statement wins.
        let code_hash = update
            .info
            .as_ref()
            .map_or(code.hash, |info| info.code_hash);
        for (chunk_id, chunk) in chunkify_code(code.code()).into_iter().enumerate() {
            state_write(
                trie,
                get_tree_key_for_code_chunk(&address32, &code_hash.0, chunk_id as u64),
                chunk,
            )?;
        }
    }

    for (slot, value) in &update.added_storage {
        state_write(
            trie,
            get_tree_key_for_storage_slot(&address32, slot_index(slot)),
            value.to_big_endian(),
        )?;
    }

    Ok(())
}

/// Write `value` at `key`, resolving 32 zero bytes to a removal rather
/// than an insertion.
///
/// The trie itself has no value that means absence: every 32-byte value
/// is storable, and only a key's presence distinguishes it from an
/// absent one. Collapsing zero onto absence is therefore the state
/// model's choice, and it is made here — once, for every state leaf —
/// so that an absent key and a zero-valued one are the same state with
/// the same root. Reads recover what was collapsed: an absent key reads
/// back as the zero it stood for.
///
/// Three leaves actually reach the zero case:
///
/// - A storage slot written to zero, which is how the EVM clears one.
/// - A code chunk of 31 zero bytes, as in a run of `STOP` or a
///   zero-filled data region. Chunk presence therefore does not
///   delimit the code — its length is the basic-data `code_size`.
/// - The basic data of an account with no code, zero nonce and zero
///   balance, since the version and reserved bytes are zero too. Such
///   an account is still distinguished from an absent one by its
///   code-hash leaf.
fn state_write(
    trie: &mut BinaryTrie,
    key: Vec<u8>,
    value: [u8; 32],
) -> Result<(), BinaryTrieError> {
    if value == [0u8; 32] {
        trie.remove(&key)?;
        return Ok(());
    }
    trie.insert(key, value)
}

fn refuse_if_overflow_storage(
    address: Address,
    overflow_storage: &HashSet<Address>,
) -> Result<(), PbtStateError> {
    if overflow_storage.contains(&address) {
        return Err(PbtStateError::OverflowStorageNotRemovable(address));
    }
    Ok(())
}

/// Header sub-index of storage slot `slot`, for `slot` below 64.
fn header_storage_sub_index(slot: u8) -> u8 {
    debug_assert!(u64::from(slot) < CODE_OFFSET - HEADER_STORAGE_OFFSET);
    HEADER_STORAGE_OFFSET as u8 + slot
}

/// Remove the account header leaves at `sub_indices`.
///
/// Most are absent, which [`BinaryTrie::remove`] reports as `None`
/// without erroring, so the loop is bounded and cheap in the common
/// case of an account with few leaves.
fn remove_header_leaves(
    trie: &mut BinaryTrie,
    address32: &[u8; 32],
    sub_indices: std::ops::RangeInclusive<u8>,
) -> Result<(), BinaryTrieError> {
    for sub_index in sub_indices {
        trie.remove(&get_tree_key_for_header(address32, sub_index))?;
    }
    Ok(())
}

/// The code size that goes into the basic-data leaf, which must be the
/// account's actual bytecode length.
///
/// The update carries the bytecode whenever it changed; otherwise the
/// size is unchanged and is read back from the account's existing
/// basic-data leaf.
fn resolve_code_size(
    trie: &mut BinaryTrie,
    update: &AccountUpdate,
    info: &AccountInfo,
) -> Result<u32, PbtStateError> {
    if let Some(code) = &update.code {
        return Ok(u32::try_from(code.len()).unwrap_or(u32::MAX));
    }
    if info.code_hash == *EMPTY_KECCAK_HASH {
        return Ok(0);
    }
    let address32 = address20_to_address32(update.address);
    let Some(basic_data) = trie.get(&get_tree_key_for_basic_data(&address32))? else {
        return Err(PbtStateError::UnknownCodeSize(update.address));
    };
    let mut size = [0u8; 4];
    size.copy_from_slice(&basic_data[BASIC_DATA_CODE_SIZE_RANGE]);
    Ok(u32::from_be_bytes(size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Bytes;
    use crate::types::Code;
    use ethrex_binary_trie::embedding::{
        ACCOUNT_ZONE, BASIC_DATA_LEAF_KEY, CODE_HASH_LEAF_KEY, CODE_ZONE, STORAGE_ZONE,
    };
    use ethrex_binary_trie::trie::{BinaryTrie, InMemoryBinaryTrieDB};
    use ethrex_crypto::NativeCrypto;
    use rustc_hash::FxHashMap;

    const ADDR_A: Address = Address::repeat_byte(0xaa);
    const ADDR_B: Address = Address::repeat_byte(0xbb);

    fn code_of(bytes: Vec<u8>) -> Code {
        Code::from_bytecode(Bytes::from(bytes), &NativeCrypto)
    }

    /// Bytecode with no push opcodes, so chunking is a plain split.
    fn plain_code(len: usize) -> Code {
        code_of(vec![0x01; len])
    }

    fn storage(slots: &[(U256, U256)]) -> FxHashMap<H256, U256> {
        slots
            .iter()
            .map(|(slot, value)| (H256(slot.to_big_endian()), *value))
            .collect()
    }

    fn eoa_update(address: Address, nonce: u64, balance: u64) -> AccountUpdate {
        AccountUpdate {
            address,
            info: Some(AccountInfo {
                code_hash: *EMPTY_KECCAK_HASH,
                balance: U256::from(balance),
                nonce,
            }),
            ..AccountUpdate::new(address)
        }
    }

    fn contract_update(address: Address, code: Code) -> AccountUpdate {
        AccountUpdate {
            address,
            info: Some(AccountInfo {
                code_hash: code.hash,
                balance: U256::from(7u64),
                nonce: 1,
            }),
            code: Some(code),
            ..AccountUpdate::new(address)
        }
    }

    fn applied(updates: &[AccountUpdate]) -> BinaryTrie {
        let mut trie = BinaryTrie::new_temp();
        apply_account_updates(&mut trie, updates).expect("updates apply");
        trie
    }

    #[test]
    fn eoa_update_writes_basic_data_and_code_hash() {
        let update = eoa_update(ADDR_A, 3, 1_000);
        let mut trie = applied(&[update]);

        let a32 = address20_to_address32(ADDR_A);
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(),
            Some(encode_basic_data(0, 3, U256::from(1_000u64)).unwrap())
        );
        assert_eq!(
            trie.get(&get_tree_key_for_code_hash(&a32)).unwrap(),
            Some(EMPTY_KECCAK_HASH.0)
        );

        // Exactly those two header leaves, nothing else.
        for sub_index in 0..=u8::MAX {
            let present = trie
                .get(&get_tree_key_for_header(&a32, sub_index))
                .unwrap()
                .is_some();
            assert_eq!(
                present,
                sub_index == BASIC_DATA_LEAF_KEY || sub_index == CODE_HASH_LEAF_KEY,
                "sub-index {sub_index}"
            );
        }
    }

    /// Zero is absent, for every leaf and not just storage: a leaf whose
    /// value encodes to 32 zero bytes is never committed. Pinned against
    /// the spec by `pbt_state`'s `eoa_zero_nonce_and_balance` and
    /// `code_chunks_of_zero_bytes` cases.
    #[test]
    fn leaves_encoding_to_zero_are_left_absent() {
        let a32 = address20_to_address32(ADDR_A);

        // Zero nonce, zero balance, no code: the basic data is 32 zero
        // bytes, so only the code-hash leaf distinguishes this account
        // from an absent one.
        let mut trie = applied(&[eoa_update(ADDR_A, 0, 0)]);
        assert_eq!(trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(), None);
        assert_eq!(
            trie.get(&get_tree_key_for_code_hash(&a32)).unwrap(),
            Some(EMPTY_KECCAK_HASH.0)
        );

        // 62 zero bytes of code: both chunks are 32 zero bytes and are
        // absent, so chunk presence does not delimit the code — its
        // length lives in the basic data's code size.
        let code = code_of(vec![0u8; 62]);
        let code_hash = code.hash;
        let mut trie = applied(&[contract_update(ADDR_B, code)]);
        let b32 = address20_to_address32(ADDR_B);
        for chunk_id in 0..2 {
            assert_eq!(
                trie.get(&get_tree_key_for_code_chunk(&b32, &code_hash.0, chunk_id))
                    .unwrap(),
                None,
                "chunk {chunk_id} of zero bytes must not be committed"
            );
        }
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&b32)).unwrap(),
            Some(encode_basic_data(62, 1, U256::from(7u64)).unwrap())
        );
    }

    #[test]
    fn contract_code_is_chunked_into_header_and_overflow() {
        // 130 chunks: 128 in the header stem, 2 in the code zone.
        let code = plain_code(31 * 130);
        let chunks = chunkify_code(code.code());
        assert_eq!(chunks.len(), 130);
        let code_hash = code.hash;
        let mut trie = applied(&[contract_update(ADDR_A, code)]);

        let a32 = address20_to_address32(ADDR_A);
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(),
            Some(encode_basic_data(31 * 130, 1, U256::from(7u64)).unwrap())
        );

        for (chunk_id, chunk) in chunks.iter().enumerate() {
            let key = get_tree_key_for_code_chunk(&a32, &code_hash.0, chunk_id as u64);
            if chunk_id < 128 {
                assert_eq!(key[0], ACCOUNT_ZONE, "chunk {chunk_id} zone");
                assert_eq!(key[..33], get_tree_key_for_basic_data(&a32)[..33]);
            } else {
                assert_eq!(key[0], CODE_ZONE, "chunk {chunk_id} zone");
            }
            assert_eq!(trie.get(&key).unwrap(), Some(*chunk), "chunk {chunk_id}");
        }
    }

    #[test]
    fn identical_bytecode_shares_overflow_leaves() {
        let code = plain_code(31 * 130);
        let code_hash = code.hash;
        let a32 = address20_to_address32(ADDR_A);
        let b32 = address20_to_address32(ADDR_B);

        let overflow_keys: Vec<_> = (128..130)
            .map(|chunk_id| get_tree_key_for_code_chunk(&a32, &code_hash.0, chunk_id))
            .collect();
        for (offset, key) in overflow_keys.iter().enumerate() {
            assert_eq!(
                *key,
                get_tree_key_for_code_chunk(&b32, &code_hash.0, 128 + offset as u64),
                "overflow chunk keys are content-addressed, not per-account"
            );
        }

        let mut trie = applied(&[contract_update(ADDR_A, code.clone())]);
        let before: Vec<_> = overflow_keys
            .iter()
            .map(|key| trie.get(key).unwrap())
            .collect();
        assert!(before.iter().all(Option::is_some));
        let root_after_a = trie.root();

        apply_account_updates(&mut trie, &[contract_update(ADDR_B, code)]).unwrap();
        let after: Vec<_> = overflow_keys
            .iter()
            .map(|key| trie.get(key).unwrap())
            .collect();
        assert_eq!(
            before, after,
            "shared overflow leaves must not be rewritten"
        );
        assert_ne!(root_after_a, trie.root(), "B's own leaves did land");
    }

    #[test]
    fn storage_below_and_above_the_header_boundary() {
        let update = AccountUpdate {
            added_storage: storage(&[
                (U256::from(63), U256::from(0xaaaau64)),
                (U256::from(64), U256::from(0xbbbbu64)),
            ]),
            ..eoa_update(ADDR_A, 1, 1)
        };
        let mut trie = applied(&[update]);

        let a32 = address20_to_address32(ADDR_A);
        let slot63 = get_tree_key_for_storage_slot(&a32, U256::from(63));
        let slot64 = get_tree_key_for_storage_slot(&a32, U256::from(64));

        assert_eq!(slot63[0], ACCOUNT_ZONE);
        assert_eq!(slot63, get_tree_key_for_header(&a32, 64 + 63));
        assert_eq!(slot64[0], STORAGE_ZONE);

        assert_eq!(
            trie.get(&slot63).unwrap(),
            Some(U256::from(0xaaaau64).to_big_endian())
        );
        assert_eq!(
            trie.get(&slot64).unwrap(),
            Some(U256::from(0xbbbbu64).to_big_endian())
        );
    }

    #[test]
    fn zero_storage_write_removes_the_leaf() {
        let a32 = address20_to_address32(ADDR_A);
        let written = get_tree_key_for_storage_slot(&a32, U256::from(9));
        let kept = get_tree_key_for_storage_slot(&a32, U256::from(10));

        // The same account, but slot 9 never written at all.
        let mut never_written = applied(&[AccountUpdate {
            added_storage: storage(&[(U256::from(10), U256::from(5u64))]),
            ..eoa_update(ADDR_A, 1, 1)
        }]);

        let mut trie = applied(&[
            AccountUpdate {
                added_storage: storage(&[
                    (U256::from(9), U256::from(42u64)),
                    (U256::from(10), U256::from(5u64)),
                ]),
                ..eoa_update(ADDR_A, 1, 1)
            },
            AccountUpdate {
                added_storage: storage(&[(U256::from(9), U256::zero())]),
                ..AccountUpdate::new(ADDR_A)
            },
        ]);

        assert_eq!(
            trie.get(&written).unwrap(),
            None,
            "zero means absent, not a zero-valued leaf"
        );
        assert_eq!(trie.get(&kept).unwrap(), never_written.get(&kept).unwrap());
        assert_eq!(trie.root(), never_written.root());
    }

    #[test]
    fn applying_updates_is_order_independent() {
        let updates = vec![
            contract_update(ADDR_A, plain_code(31 * 130)),
            AccountUpdate {
                added_storage: storage(&[
                    (U256::from(1), U256::from(11u64)),
                    (U256::from(1_000), U256::from(12u64)),
                ]),
                ..eoa_update(ADDR_B, 4, 400)
            },
            eoa_update(Address::repeat_byte(0xcc), 9, 900),
        ];
        let forwards = applied(&updates).root();

        let mut reversed = updates.clone();
        reversed.reverse();
        assert_eq!(forwards, applied(&reversed).root());

        let mut rotated = updates;
        rotated.rotate_left(1);
        assert_eq!(forwards, applied(&rotated).root());
    }

    #[test]
    fn removing_an_account_clears_its_header_leaves() {
        let a32 = address20_to_address32(ADDR_A);
        // A contract with storage in the header, code in both zones.
        let code = plain_code(31 * 130);
        let code_hash = code.hash;
        let populated = AccountUpdate {
            added_storage: storage(&[
                (U256::from(0), U256::from(1u64)),
                (U256::from(63), U256::from(2u64)),
            ]),
            ..contract_update(ADDR_A, code)
        };

        let mut trie = applied(&[populated, eoa_update(ADDR_B, 1, 1)]);
        apply_account_updates(&mut trie, &[AccountUpdate::removed(ADDR_A)]).unwrap();

        for sub_index in 0..=u8::MAX {
            assert_eq!(
                trie.get(&get_tree_key_for_header(&a32, sub_index)).unwrap(),
                None,
                "header sub-index {sub_index} survived removal"
            );
        }
        // Content-addressed overflow code is shared and must stay.
        assert!(
            trie.get(&get_tree_key_for_code_chunk(&a32, &code_hash.0, 128))
                .unwrap()
                .is_some()
        );
        // The other account is untouched.
        let b32 = address20_to_address32(ADDR_B);
        assert!(
            trie.get(&get_tree_key_for_basic_data(&b32))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn removing_an_account_with_overflow_storage_errors() {
        let updates = vec![
            AccountUpdate {
                added_storage: storage(&[(U256::from(1_000), U256::from(7u64))]),
                ..eoa_update(ADDR_A, 1, 1)
            },
            AccountUpdate::removed(ADDR_A),
        ];
        let mut trie = BinaryTrie::new_temp();
        let error = apply_account_updates(&mut trie, &updates).expect_err("must refuse");
        assert!(matches!(
            error,
            PbtStateError::OverflowStorageNotRemovable(address) if address == ADDR_A
        ));
        assert!(
            format!("{error}").contains(&format!("{ADDR_A:?}")),
            "error names the account: {error}"
        );

        // `removed_storage` alone is refused for the same reason.
        let wipe = vec![AccountUpdate {
            added_storage: storage(&[(U256::from(1_000), U256::from(7u64))]),
            removed_storage: true,
            ..eoa_update(ADDR_A, 1, 1)
        }];
        assert!(matches!(
            apply_account_updates(&mut BinaryTrie::new_temp(), &wipe),
            Err(PbtStateError::OverflowStorageNotRemovable(_))
        ));
    }

    #[test]
    fn removed_storage_clears_header_slots_and_keeps_the_account() {
        let a32 = address20_to_address32(ADDR_A);
        let mut trie = applied(&[AccountUpdate {
            added_storage: storage(&[
                (U256::from(0), U256::from(1u64)),
                (U256::from(63), U256::from(2u64)),
            ]),
            ..eoa_update(ADDR_A, 2, 200)
        }]);

        apply_account_updates(
            &mut trie,
            &[AccountUpdate {
                removed_storage: true,
                ..AccountUpdate::new(ADDR_A)
            }],
        )
        .unwrap();

        for slot in [0u64, 63] {
            assert_eq!(
                trie.get(&get_tree_key_for_storage_slot(&a32, U256::from(slot)))
                    .unwrap(),
                None
            );
        }
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(),
            Some(encode_basic_data(0, 2, U256::from(200u64)).unwrap()),
            "the account itself survives a storage wipe"
        );
    }

    #[test]
    fn code_size_without_bytecode_comes_from_the_existing_leaf() {
        let code = plain_code(100);
        let code_hash = code.hash;
        let a32 = address20_to_address32(ADDR_A);
        let mut trie = applied(&[contract_update(ADDR_A, code)]);

        // A later update touching only the balance carries no bytecode.
        apply_account_updates(
            &mut trie,
            &[AccountUpdate {
                info: Some(AccountInfo {
                    code_hash,
                    balance: U256::from(99u64),
                    nonce: 2,
                }),
                ..AccountUpdate::new(ADDR_A)
            }],
        )
        .unwrap();

        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(),
            Some(encode_basic_data(100, 2, U256::from(99u64)).unwrap()),
            "code size is preserved, not reset to zero"
        );

        // With no previous leaf to read, the size is not inventable.
        assert!(matches!(
            apply_account_updates(
                &mut BinaryTrie::new_temp(),
                &[AccountUpdate {
                    info: Some(AccountInfo {
                        code_hash,
                        balance: U256::zero(),
                        nonce: 0,
                    }),
                    ..AccountUpdate::new(ADDR_B)
                }],
            ),
            Err(PbtStateError::UnknownCodeSize(_))
        ));
    }

    #[test]
    fn reads_return_what_the_writes_put_there() {
        let code = plain_code(31 * 3);
        let code_hash = code.hash;
        let mut trie = applied(&[
            AccountUpdate {
                added_storage: storage(&[
                    // One slot on each side of the header/overflow boundary,
                    // so both key derivations are exercised.
                    (U256::from(5), U256::from(0x55u64)),
                    (U256::from(5_000), U256::from(0x5000u64)),
                ]),
                ..contract_update(ADDR_A, code)
            },
            eoa_update(ADDR_B, 9, 900),
        ]);

        assert_eq!(
            get_account_info(&mut trie, ADDR_A).unwrap(),
            Some(AccountInfo {
                code_hash,
                balance: U256::from(7u64),
                nonce: 1,
            })
        );
        assert_eq!(
            get_account_info(&mut trie, ADDR_B).unwrap(),
            Some(AccountInfo {
                code_hash: *EMPTY_KECCAK_HASH,
                balance: U256::from(900u64),
                nonce: 9,
            })
        );
        for (slot, value) in [(5u64, 0x55u64), (5_000, 0x5000)] {
            assert_eq!(
                get_storage_slot(&mut trie, ADDR_A, &H256(U256::from(slot).to_big_endian()))
                    .unwrap(),
                Some(U256::from(value)),
                "slot {slot}"
            );
        }

        // Absence, on both axes.
        assert_eq!(
            get_account_info(&mut trie, Address::repeat_byte(0xcc)).unwrap(),
            None
        );
        assert_eq!(
            get_storage_slot(&mut trie, ADDR_A, &H256(U256::from(6u64).to_big_endian())).unwrap(),
            None
        );
        // A slot written to zero is stored as absent and reads back as absent,
        // which is the same state the MPT reports for it.
        apply_account_updates(
            &mut trie,
            &[AccountUpdate {
                added_storage: storage(&[(U256::from(5), U256::zero())]),
                ..AccountUpdate::new(ADDR_A)
            }],
        )
        .unwrap();
        assert_eq!(
            get_storage_slot(&mut trie, ADDR_A, &H256(U256::from(5u64).to_big_endian())).unwrap(),
            None
        );
    }

    /// The counterpart of `leaves_encoding_to_zero_are_left_absent`: an account
    /// whose basic data collapsed to absence is still an account, and the read
    /// path has to find it by its code-hash leaf.
    #[test]
    fn an_account_whose_basic_data_is_absent_is_still_found() {
        let mut trie = applied(&[AccountUpdate {
            added_storage: storage(&[(U256::from(1), U256::from(1u64))]),
            ..eoa_update(ADDR_A, 0, 0)
        }]);
        let a32 = address20_to_address32(ADDR_A);
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(),
            None,
            "the premise: this account has no basic-data leaf"
        );

        assert_eq!(
            get_account_info(&mut trie, ADDR_A).unwrap(),
            Some(AccountInfo {
                code_hash: *EMPTY_KECCAK_HASH,
                balance: U256::zero(),
                nonce: 0,
            }),
            "an absent basic-data leaf means zeros, not a missing account"
        );
    }

    #[test]
    fn basic_data_decodes_to_what_it_was_encoded_from() {
        for (code_size, nonce, balance) in [
            (0u32, 0u64, U256::zero()),
            (1, 1, U256::one()),
            (u32::MAX, u64::MAX, (U256::from(1) << 128) - 1),
        ] {
            let encoded = encode_basic_data(code_size, nonce, balance).unwrap();
            assert_eq!(decode_basic_data(&encoded), (nonce, balance));
        }
    }

    #[test]
    fn a_removed_account_reads_back_as_absent() {
        let mut trie = applied(&[eoa_update(ADDR_A, 3, 300), eoa_update(ADDR_B, 4, 400)]);
        apply_account_updates(&mut trie, &[AccountUpdate::removed(ADDR_A)]).unwrap();

        assert_eq!(get_account_info(&mut trie, ADDR_A).unwrap(), None);
        assert!(get_account_info(&mut trie, ADDR_B).unwrap().is_some());
    }

    #[test]
    fn state_survives_commit_and_reopen() {
        let db = InMemoryBinaryTrieDB::new_empty();
        let mut trie = BinaryTrie::new(Box::new(db.clone()));
        let code = plain_code(31 * 130);
        let code_hash = code.hash;
        apply_account_updates(
            &mut trie,
            &[
                AccountUpdate {
                    added_storage: storage(&[
                        (U256::from(3), U256::from(30u64)),
                        (U256::from(3_000), U256::from(31u64)),
                    ]),
                    ..contract_update(ADDR_A, code)
                },
                eoa_update(ADDR_B, 5, 500),
            ],
        )
        .unwrap();
        let root = trie.commit().unwrap();
        assert_eq!(root, trie.root());

        let mut reopened = BinaryTrie::open(Box::new(InMemoryBinaryTrieDB::new(db.inner())), root);
        assert_eq!(reopened.root(), root);

        let a32 = address20_to_address32(ADDR_A);
        assert_eq!(
            reopened
                .get(&get_tree_key_for_storage_slot(&a32, U256::from(3_000)))
                .unwrap(),
            Some(U256::from(31u64).to_big_endian())
        );
        assert_eq!(
            reopened.get(&get_tree_key_for_code_hash(&a32)).unwrap(),
            Some(code_hash.0)
        );
        assert_eq!(
            reopened
                .get(&get_tree_key_for_code_chunk(&a32, &code_hash.0, 129))
                .unwrap(),
            Some(chunkify_code(plain_code(31 * 130).code())[129])
        );
    }
}
