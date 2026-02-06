#![allow(dead_code)]

use bytes::{BufMut, Bytes};
use ethereum_types::{Address, H256, U256};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::{RLPEncode, encode_length, list_length},
    structs,
};
use serde::{Deserialize, Serialize};

use crate::constants::EMPTY_BLOCK_ACCESS_LIST_HASH;
use crate::utils::keccak;

/// Encode a slice of items in sorted order without cloning.
fn encode_sorted_by<T, K, F>(items: &[T], buf: &mut dyn BufMut, key_fn: F)
where
    T: RLPEncode,
    K: Ord,
    F: Fn(&T) -> K,
{
    if items.is_empty() {
        buf.put_u8(0xc0);
        return;
    }
    let mut indices: Vec<usize> = (0..items.len()).collect();
    indices.sort_unstable_by(|&i, &j| key_fn(&items[i]).cmp(&key_fn(&items[j])));

    let payload_len: usize = items.iter().map(|item| item.length()).sum();
    encode_length(payload_len, buf);
    for &i in &indices {
        items[i].encode(buf);
    }
}

/// Calculate the encoded length of a sorted list.
fn sorted_list_length<T: RLPEncode>(items: &[T]) -> usize {
    if items.is_empty() {
        return 1;
    }
    let payload_len: usize = items.iter().map(|item| item.length()).sum();
    list_length(payload_len)
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StorageChange {
    block_access_index: u32,
    post_value: U256,
}

impl RLPEncode for StorageChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.block_access_index)
            .encode_field(&self.post_value)
            .finish();
    }
}

impl RLPDecode for StorageChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (block_access_index, decoder) = decoder.decode_field("block_access_index")?;
        let (post_value, decoder) = decoder.decode_field("post_value")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                block_access_index,
                post_value,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SlotChange {
    slot: U256,
    slot_changes: Vec<StorageChange>,
}

impl RLPEncode for SlotChange {
    fn encode(&self, buf: &mut dyn BufMut) {
        let payload_len = self.slot.length() + sorted_list_length(&self.slot_changes);
        encode_length(payload_len, buf);
        self.slot.encode(buf);
        encode_sorted_by(&self.slot_changes, buf, |s| s.block_access_index);
    }
}

impl RLPDecode for SlotChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (slot, decoder) = decoder.decode_field("slot")?;
        let (slot_changes, decoder) = decoder.decode_field("slot_changes")?;
        let remaining = decoder.finish()?;
        Ok((Self { slot, slot_changes }, remaining))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BalanceChange {
    block_access_index: u32,
    post_balance: U256,
}

impl RLPEncode for BalanceChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.block_access_index)
            .encode_field(&self.post_balance)
            .finish();
    }
}

impl RLPDecode for BalanceChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (block_access_index, decoder) = decoder.decode_field("block_access_index")?;
        let (post_balance, decoder) = decoder.decode_field("post_balance")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                block_access_index,
                post_balance,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct NonceChange {
    block_access_index: u32,
    post_nonce: u64,
}

impl RLPEncode for NonceChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.block_access_index)
            .encode_field(&self.post_nonce)
            .finish();
    }
}

impl RLPDecode for NonceChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (block_access_index, decoder) = decoder.decode_field("block_access_index")?;
        let (post_nonce, decoder) = decoder.decode_field("post_nonce")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                block_access_index,
                post_nonce,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CodeChange {
    block_access_index: u32,
    new_code: Bytes,
}

impl RLPEncode for CodeChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.block_access_index)
            .encode_field(&self.new_code)
            .finish();
    }
}

impl RLPDecode for CodeChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (block_access_index, decoder) = decoder.decode_field("block_access_index")?;
        let (new_code, decoder) = decoder.decode_field("new_code")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                block_access_index,
                new_code,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AccountChanges {
    address: Address,
    storage_changes: Vec<SlotChange>,
    storage_reads: Vec<U256>,
    balance_changes: Vec<BalanceChange>,
    nonce_changes: Vec<NonceChange>,
    code_changes: Vec<CodeChange>,
}

impl RLPEncode for AccountChanges {
    fn encode(&self, buf: &mut dyn BufMut) {
        let payload_len = self.address.length()
            + sorted_list_length(&self.storage_changes)
            + sorted_list_length(&self.storage_reads)
            + sorted_list_length(&self.balance_changes)
            + sorted_list_length(&self.nonce_changes)
            + sorted_list_length(&self.code_changes);

        encode_length(payload_len, buf);
        self.address.encode(buf);
        encode_sorted_by(&self.storage_changes, buf, |s| s.slot);
        encode_sorted_by(&self.storage_reads, buf, |s| *s);
        encode_sorted_by(&self.balance_changes, buf, |b| b.block_access_index);
        encode_sorted_by(&self.nonce_changes, buf, |n| n.block_access_index);
        encode_sorted_by(&self.code_changes, buf, |c| c.block_access_index);
    }
}

impl RLPDecode for AccountChanges {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (address, decoder) = decoder.decode_field("address")?;
        let (storage_changes, decoder) = decoder.decode_field("storage_changes")?;
        let (storage_reads, decoder) = decoder.decode_field("storage_reads")?;
        let (balance_changes, decoder) = decoder.decode_field("balance_changes")?;
        let (nonce_changes, decoder) = decoder.decode_field("nonce_changes")?;
        let (code_changes, decoder) = decoder.decode_field("code_changes")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                address,
                storage_changes,
                storage_reads,
                balance_changes,
                nonce_changes,
                code_changes,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BlockAccessList {
    inner: Vec<AccountChanges>,
}

impl BlockAccessList {
    pub fn compute_hash(&self) -> H256 {
        if self.inner.is_empty() {
            return *EMPTY_BLOCK_ACCESS_LIST_HASH;
        }

        let buf = self.encode_to_vec();
        keccak(buf)
    }
}

impl RLPEncode for BlockAccessList {
    fn encode(&self, buf: &mut dyn BufMut) {
        encode_sorted_by(&self.inner, buf, |a| a.address);
    }
}

impl RLPDecode for BlockAccessList {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let (inner, remaining) = RLPDecode::decode_unfinished(rlp)?;
        Ok((Self { inner }, remaining))
    }
}

#[cfg(test)]
mod tests {
    use ethereum_types::{H160, U256};
    use ethrex_rlp::decode::RLPDecode;
    use ethrex_rlp::encode::RLPEncode;

    use crate::types::block_access_list::{
        AccountChanges, BalanceChange, NonceChange, SlotChange, StorageChange,
    };

    use super::BlockAccessList;

    const ALICE_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10]); //0xA
    const BOB_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 11]); //0xB
    const CHARLIE_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 12]); //0xC
    const CONTRACT_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 12]); //0xC

    #[test]
    fn test_encode_decode_empty_list_validation() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(&buf);
        assert_eq!(
            &encoded_rlp,
            "dbda94000000000000000000000000000000000000000ac0c0c0c0c0"
        );

        let decoded_bal = BlockAccessList::decode(&buf).unwrap();
        assert_eq!(decoded_bal, actual_bal);
    }

    #[test]
    fn test_encode_decode_partial_validation() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                storage_reads: vec![U256::from(1), U256::from(2)],
                balance_changes: vec![BalanceChange {
                    block_access_index: 1,
                    post_balance: U256::from(100),
                }],
                nonce_changes: vec![NonceChange {
                    block_access_index: 1,
                    post_nonce: 1,
                }],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(&buf);
        assert_eq!(
            &encoded_rlp,
            "e3e294000000000000000000000000000000000000000ac0c20102c3c20164c3c20101c0"
        );

        let decoded_bal = BlockAccessList::decode(&buf).unwrap();
        assert_eq!(decoded_bal, actual_bal);
    }

    #[test]
    fn test_storage_changes_validation() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: CONTRACT_ADDR,
                storage_changes: vec![SlotChange {
                    slot: U256::from(0x1),
                    slot_changes: vec![StorageChange {
                        block_access_index: 1,
                        post_value: U256::from(0x42),
                    }],
                }],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "e1e094000000000000000000000000000000000000000cc6c501c3c20142c0c0c0c0"
        );
    }

    #[test]
    fn test_expected_addresses_auto_sorted() {
        let actual_bal = BlockAccessList {
            inner: vec![
                AccountChanges {
                    address: CHARLIE_ADDR,
                    ..Default::default()
                },
                AccountChanges {
                    address: ALICE_ADDR,
                    ..Default::default()
                },
                AccountChanges {
                    address: BOB_ADDR,
                    ..Default::default()
                },
            ],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "f851da94000000000000000000000000000000000000000ac0c0c0c0c0da94000000000000000000000000000000000000000bc0c0c0c0c0da94000000000000000000000000000000000000000cc0c0c0c0c0"
        );
    }

    #[test]
    fn test_expected_storage_slots_ordering_correct_order_should_pass() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                storage_changes: vec![
                    SlotChange {
                        slot: U256::from(0x02),
                        slot_changes: vec![],
                    },
                    SlotChange {
                        slot: U256::from(0x01),
                        slot_changes: vec![],
                    },
                    SlotChange {
                        slot: U256::from(0x03),
                        slot_changes: vec![],
                    },
                ],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(&buf);
        assert_eq!(
            &encoded_rlp,
            "e4e394000000000000000000000000000000000000000ac9c201c0c202c0c203c0c0c0c0c0"
        );
    }

    #[test]
    fn test_expected_storage_reads_ordering_correct_order_should_pass() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                storage_reads: vec![U256::from(0x02), U256::from(0x01), U256::from(0x03)],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "dedd94000000000000000000000000000000000000000ac0c3010203c0c0c0"
        );
    }

    #[test]
    fn test_expected_tx_indices_ordering_correct_order_should_pass() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                nonce_changes: vec![
                    NonceChange {
                        block_access_index: 2,
                        post_nonce: 2,
                    },
                    NonceChange {
                        block_access_index: 3,
                        post_nonce: 3,
                    },
                    NonceChange {
                        block_access_index: 1,
                        post_nonce: 1,
                    },
                ],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "e4e394000000000000000000000000000000000000000ac0c0c0c9c20101c20202c20303c0"
        );
    }

    #[test]
    fn test_decode_storage_slots_ordering_correct_order_should_pass() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                storage_changes: vec![
                    SlotChange {
                        slot: U256::from(0x01),
                        slot_changes: vec![],
                    },
                    SlotChange {
                        slot: U256::from(0x02),
                        slot_changes: vec![],
                    },
                    SlotChange {
                        slot: U256::from(0x03),
                        slot_changes: vec![],
                    },
                ],
                ..Default::default()
            }],
        };

        let encoded_rlp: Vec<u8> = hex::decode(
            "e4e394000000000000000000000000000000000000000ac9c201c0c202c0c203c0c0c0c0c0",
        )
        .unwrap();

        let decoded_bal = BlockAccessList::decode(&encoded_rlp).unwrap();
        assert_eq!(decoded_bal, actual_bal);
    }
}
