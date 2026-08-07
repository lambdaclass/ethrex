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
//! # Removal is by prefix, and what that does and does not take
//!
//! Flat state made removal trivial — an account was one map entry. A
//! trie is not so kind, and an account's state is spread over three
//! regions that a removal has to treat differently. Each is a subtree of
//! the one unified tree, and the embedding puts it there on purpose, so
//! each is one [`BinaryTrie::remove_prefix`] call:
//!
//! - **The header stem goes.** [`get_tree_prefix_for_header`] covers the
//!   account's basic data, code hash, storage slots `0..=63` and code
//!   chunks `0..=127`, and nothing of any other account.
//! - **Overflow storage goes.** Slots at `64` and above live under
//!   `0xff ‖ blake3(address)` ([`get_tree_prefix_for_overflow_storage`]),
//!   which the embedding makes contiguous for exactly this reason. It is
//!   unbounded and cannot be enumerated from outside the trie — the key
//!   of a slot is not recoverable from the slot's key — so a prefix
//!   removal is the only way to clear it, and it is why this module once
//!   had to refuse such an account instead.
//! - **Overflow code stays.** Chunks at index `128` and above are
//!   content-addressed, so two accounts with identical bytecode share
//!   the very same leaves; deleting one account's overflow chunks would
//!   corrupt the other. They are therefore left in place — the same
//!   reason ethrex never prunes its `ACCOUNT_CODES` table. This is the
//!   one region where "the account is gone" does not mean "its leaves
//!   are gone", and no prefix over it would be safe to remove.
//!
//! A storage wipe ([`AccountUpdate::removed_storage`]) takes the second
//! region and the storage part of the first
//! ([`get_tree_prefix_for_header_storage`]), leaving the account itself.
//!
//! # Reads that need more than a key
//!
//! [`has_storage`] is the read that could not be served before the trie
//! had prefixes. It answers "does this account hold any storage at all",
//! which EIP-7610 needs to refuse a `CREATE` over an account that does,
//! and it is two existence checks — one per storage region — rather than
//! the 64-lookup half-answer a header-only scan would give.

use ethrex_binary_trie::BinaryTrieError;
use ethrex_binary_trie::embedding::{
    address20_to_address32, chunkify_code, encode_basic_data, get_tree_key_for_basic_data,
    get_tree_key_for_code_chunk, get_tree_key_for_code_hash, get_tree_key_for_storage_slot,
    get_tree_prefix_for_header, get_tree_prefix_for_header_storage,
    get_tree_prefix_for_overflow_storage,
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
/// update writes and removes keys derived from its own account.
///
/// See the module docs for what removal does and does not cover.
pub fn apply_account_updates(
    trie: &mut BinaryTrie,
    updates: &[AccountUpdate],
) -> Result<(), PbtStateError> {
    for update in updates {
        apply_account_update(trie, update)?;
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
/// per-account subtrie there, it is leaves of the one unified tree. What it
/// *can* report is whether the account holds any storage at all, which is what
/// every consumer of that field actually reads out of it — see [`has_storage`]
/// and [`get_account`], and `StoreVmDatabase` in `crates/blockchain/vm.rs` for
/// what it puts in the field on the strength of them. Kept separate because
/// the account RPCs want none of it.
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

/// An account as the binary trie is able to describe it.
///
/// [`AccountInfo`] plus the one thing an MPT-shaped `AccountState` needs
/// that `AccountInfo` has no field for. The two travel together because
/// [`get_account`] answers both out of one open trie, and the storage
/// question reuses the header-stem nodes the account read already
/// loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryAccount {
    pub info: AccountInfo,
    /// Whether the account holds any storage at all — see
    /// [`has_storage`].
    pub has_storage: bool,
}

/// [`get_account_info`] together with [`has_storage`], for the caller
/// that has to fill an `AccountState`.
///
/// One open trie serves both, which is not merely tidy: the storage
/// question's first half walks to the very header stem the account read
/// just loaded, so it costs almost nothing on top. Split across two
/// opens it would be a second walk from the root.
pub fn get_account(
    trie: &mut BinaryTrie,
    address: Address,
) -> Result<Option<BinaryAccount>, PbtStateError> {
    let Some(info) = get_account_info(trie, address)? else {
        return Ok(None);
    };
    Ok(Some(BinaryAccount {
        info,
        has_storage: has_storage(trie, address)?,
    }))
}

/// Whether `trie` holds any storage for `address`.
///
/// The answer `storage_root != EMPTY_TRIE_HASH` stands for on the MPT
/// side, and the binary trie can give it directly rather than through a
/// summary hash it does not have. EIP-7610 is what makes it
/// consensus-relevant: a `CREATE` at an address that holds storage must
/// fail even when that address has no code and a zero nonce.
///
/// Two existence checks, because an account's storage lives in two
/// places — slots `0..=63` in the header stem, the rest in the storage
/// zone — and no prefix shorter than the empty one covers both. The
/// header is asked first: it is the cheaper of the two for a caller that
/// has just read the account, since the walk to the stem is already
/// loaded, and finding storage there skips the second check entirely.
///
/// Neither check scans. [`BinaryTrie::contains_prefix`] stops at the
/// first node whose subtree lies wholly under the prefix, so this is two
/// root-to-node walks whatever the account's storage looks like.
pub fn has_storage(trie: &mut BinaryTrie, address: Address) -> Result<bool, PbtStateError> {
    let address32 = address20_to_address32(address);
    Ok(
        trie.contains_prefix(&get_tree_prefix_for_header_storage(&address32))?
            || trie.contains_prefix(&get_tree_prefix_for_overflow_storage(&address32))?,
    )
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

fn slot_index(slot: &H256) -> U256 {
    U256::from_big_endian(slot.as_bytes())
}

fn apply_account_update(
    trie: &mut BinaryTrie,
    update: &AccountUpdate,
) -> Result<(), PbtStateError> {
    let address32 = address20_to_address32(update.address);

    if update.removed {
        // The whole header stem goes: basic data, code hash, storage
        // slots 0..=63 and code chunks 0..=127. Overflow code chunks
        // are content-addressed and shared, so they stay.
        trie.remove_prefix(&get_tree_prefix_for_header(&address32))?;
        trie.remove_prefix(&get_tree_prefix_for_overflow_storage(&address32))?;
        return Ok(());
    }

    if update.removed_storage {
        // Both storage regions, and neither of the two that are not
        // storage: the account itself survives, and so does its code.
        trie.remove_prefix(&get_tree_prefix_for_header_storage(&address32))?;
        trie.remove_prefix(&get_tree_prefix_for_overflow_storage(&address32))?;
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
        get_tree_key_for_header,
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

    /// Storage slots on both sides of the header/overflow boundary, and
    /// several of the overflow ones so they land in more than one group
    /// — one leaf would not need a subtree removal to clear.
    fn slots_across_the_boundary() -> Vec<(U256, U256)> {
        [0u64, 5, 63, 64, 255, 256, 1_000, 100_000]
            .into_iter()
            .map(|slot| (U256::from(slot), U256::from(slot + 1)))
            .collect()
    }

    /// The storage-only counterpart of `applied`: an account carrying
    /// [`slots_across_the_boundary`], plus a second account to check the
    /// removal does not reach past its own.
    fn trie_with_storage_on_both_sides() -> BinaryTrie {
        applied(&[
            AccountUpdate {
                added_storage: storage(&slots_across_the_boundary()),
                ..eoa_update(ADDR_A, 1, 1)
            },
            AccountUpdate {
                added_storage: storage(&slots_across_the_boundary()),
                ..eoa_update(ADDR_B, 2, 2)
            },
        ])
    }

    /// Every slot of `slots_across_the_boundary` reads back as absent
    /// for `address`, and still reads back for the other account.
    fn assert_storage_cleared_for_only(trie: &mut BinaryTrie, address: Address) {
        let other = if address == ADDR_A { ADDR_B } else { ADDR_A };
        for (slot, value) in slots_across_the_boundary() {
            let key = H256(slot.to_big_endian());
            assert_eq!(
                get_storage_slot(trie, address, &key).unwrap(),
                None,
                "slot {slot} survived the wipe"
            );
            assert_eq!(
                get_storage_slot(trie, other, &key).unwrap(),
                Some(value),
                "slot {slot} of another account must be untouched"
            );
        }
    }

    #[test]
    fn removing_an_account_clears_its_overflow_storage() {
        let mut trie = trie_with_storage_on_both_sides();
        // The premise: this account's storage reaches into the overflow
        // zone, which is what used to make it unremovable.
        assert!(has_storage(&mut trie, ADDR_A).unwrap());
        assert!(
            get_storage_slot(
                &mut trie,
                ADDR_A,
                &H256(U256::from(100_000u64).to_big_endian())
            )
            .unwrap()
            .is_some()
        );

        apply_account_updates(&mut trie, &[AccountUpdate::removed(ADDR_A)]).unwrap();

        assert_eq!(get_account_info(&mut trie, ADDR_A).unwrap(), None);
        assert!(
            !has_storage(&mut trie, ADDR_A).unwrap(),
            "no orphaned storage may be left behind"
        );
        assert_storage_cleared_for_only(&mut trie, ADDR_A);

        // And the result is the state that never held the account, not
        // merely one that answers the same to these questions.
        let without = applied(&[AccountUpdate {
            added_storage: storage(&slots_across_the_boundary()),
            ..eoa_update(ADDR_B, 2, 2)
        }]);
        assert_eq!(trie.root(), without.root());
    }

    #[test]
    fn removed_storage_clears_both_zones_and_keeps_the_account() {
        let a32 = address20_to_address32(ADDR_A);
        let mut trie = trie_with_storage_on_both_sides();

        apply_account_updates(
            &mut trie,
            &[AccountUpdate {
                removed_storage: true,
                ..AccountUpdate::new(ADDR_A)
            }],
        )
        .unwrap();

        assert_storage_cleared_for_only(&mut trie, ADDR_A);
        assert!(!has_storage(&mut trie, ADDR_A).unwrap());
        assert_eq!(
            trie.get(&get_tree_key_for_basic_data(&a32)).unwrap(),
            Some(encode_basic_data(0, 1, U256::one()).unwrap()),
            "the account itself survives a storage wipe"
        );
    }

    /// A storage wipe must not take the account's code with it: header
    /// code chunks share the stem with header storage, and only the two
    /// sub-index bits tell them apart.
    #[test]
    fn removed_storage_keeps_the_code() {
        let code = plain_code(31 * 130);
        let code_hash = code.hash;
        let a32 = address20_to_address32(ADDR_A);
        let mut trie = applied(&[AccountUpdate {
            added_storage: storage(&slots_across_the_boundary()),
            ..contract_update(ADDR_A, code.clone())
        }]);

        apply_account_updates(
            &mut trie,
            &[AccountUpdate {
                removed_storage: true,
                ..AccountUpdate::new(ADDR_A)
            }],
        )
        .unwrap();

        let chunks = chunkify_code(code.code());
        for (chunk_id, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                trie.get(&get_tree_key_for_code_chunk(
                    &a32,
                    &code_hash.0,
                    chunk_id as u64
                ))
                .unwrap(),
                Some(*chunk),
                "chunk {chunk_id} must survive a storage wipe"
            );
        }
        assert_eq!(
            trie.get(&get_tree_key_for_code_hash(&a32)).unwrap(),
            Some(code_hash.0)
        );
    }

    /// Content-addressed overflow code is shared between accounts with
    /// identical bytecode, so an account removal must leave it alone —
    /// the one region where "the account is gone" does not mean "its
    /// leaves are gone".
    #[test]
    fn removing_an_account_leaves_shared_overflow_code_alone() {
        let code = plain_code(31 * 130);
        let code_hash = code.hash;
        let a32 = address20_to_address32(ADDR_A);
        let b32 = address20_to_address32(ADDR_B);
        let mut trie = applied(&[
            contract_update(ADDR_A, code.clone()),
            contract_update(ADDR_B, code.clone()),
        ]);

        apply_account_updates(&mut trie, &[AccountUpdate::removed(ADDR_A)]).unwrap();

        let chunks = chunkify_code(code.code());
        for chunk_id in 128..chunks.len() as u64 {
            let key = get_tree_key_for_code_chunk(&b32, &code_hash.0, chunk_id);
            assert_eq!(
                trie.get(&key).unwrap(),
                Some(chunks[chunk_id as usize]),
                "chunk {chunk_id} is B's too and must survive A's removal"
            );
        }
        // A's header chunks, which are its alone, did go.
        assert_eq!(
            trie.get(&get_tree_key_for_code_chunk(&a32, &code_hash.0, 0))
                .unwrap(),
            None
        );
    }

    #[test]
    fn has_storage_sees_each_zone_on_its_own() {
        // Nothing at all.
        let mut none = applied(&[eoa_update(ADDR_A, 1, 1)]);
        assert!(!has_storage(&mut none, ADDR_A).unwrap());
        assert!(
            !has_storage(&mut none, Address::repeat_byte(0xcc)).unwrap(),
            "an absent account holds no storage"
        );

        // Header storage only, at each end of its range.
        for slot in [0u64, 63] {
            let mut trie = applied(&[AccountUpdate {
                added_storage: storage(&[(U256::from(slot), U256::from(9u64))]),
                ..eoa_update(ADDR_A, 1, 1)
            }]);
            assert!(has_storage(&mut trie, ADDR_A).unwrap(), "slot {slot}");
            assert!(
                !has_storage(&mut trie, ADDR_B).unwrap(),
                "one account's storage is not another's"
            );
        }

        // Overflow storage only, so the header check must not be the
        // one answering.
        for slot in [64u64, 100_000] {
            let mut trie = applied(&[AccountUpdate {
                added_storage: storage(&[(U256::from(slot), U256::from(9u64))]),
                ..eoa_update(ADDR_A, 1, 1)
            }]);
            assert!(has_storage(&mut trie, ADDR_A).unwrap(), "slot {slot}");
            assert!(!has_storage(&mut trie, ADDR_B).unwrap());
        }
    }

    /// Code is not storage, in either zone: a contract with no storage
    /// must not be reported as having some because its chunks share the
    /// header stem or sit in the code zone.
    #[test]
    fn has_storage_is_not_confused_by_code() {
        let mut trie = applied(&[contract_update(ADDR_A, plain_code(31 * 130))]);
        assert!(!has_storage(&mut trie, ADDR_A).unwrap());
    }

    /// Zero means absent, and absent means no storage: a slot written
    /// and then cleared leaves the account with none, exactly as if it
    /// had never been written.
    #[test]
    fn has_storage_follows_zero_writes_back_to_false() {
        for slot in [7u64, 100_000] {
            let mut trie = applied(&[
                AccountUpdate {
                    added_storage: storage(&[(U256::from(slot), U256::from(9u64))]),
                    ..eoa_update(ADDR_A, 1, 1)
                },
                AccountUpdate {
                    added_storage: storage(&[(U256::from(slot), U256::zero())]),
                    ..AccountUpdate::new(ADDR_A)
                },
            ]);
            assert!(!has_storage(&mut trie, ADDR_A).unwrap(), "slot {slot}");
        }
    }

    #[test]
    fn get_account_reports_the_info_and_the_storage_together() {
        let mut trie = trie_with_storage_on_both_sides();
        let account = get_account(&mut trie, ADDR_A).unwrap().expect("present");
        assert_eq!(
            account.info,
            get_account_info(&mut trie, ADDR_A).unwrap().unwrap()
        );
        assert!(account.has_storage);

        let mut bare = applied(&[eoa_update(ADDR_A, 1, 1)]);
        assert!(!get_account(&mut bare, ADDR_A).unwrap().unwrap().has_storage);
        assert_eq!(get_account(&mut bare, ADDR_B).unwrap(), None);
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
