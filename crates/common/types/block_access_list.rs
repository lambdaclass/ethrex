#![allow(dead_code)]

use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::structs;
use serde::{Deserialize, Serialize};

use crate::constants::EMPTY_BLOCK_ACCESS_LIST_HASH;
use crate::utils::keccak;

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct StorageChange {
    block_access_index: usize,
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

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct SlotChange {
    slot: U256,
    slot_changes: Vec<StorageChange>,
}

impl RLPEncode for SlotChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.slot)
            .encode_field(&self.slot_changes)
            .finish();
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct BalanceChange {
    block_access_index: usize,
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

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct NonceChange {
    block_access_index: usize,
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

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct CodeChange {
    block_access_index: usize,
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

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct AccountChanges {
    address: Address,
    storage_changes: Vec<SlotChange>,
    storage_reads: Vec<U256>,
    balance_changes: Vec<BalanceChange>,
    nonce_changes: Vec<NonceChange>,
    code_changes: Vec<CodeChange>,
}

impl RLPEncode for AccountChanges {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let mut sorted = self.clone();
        sorted.storage_changes.sort_by(|a, b| a.slot.cmp(&b.slot));
        sorted.storage_reads.sort();
        sorted
            .balance_changes
            .sort_by(|a, b| a.block_access_index.cmp(&b.block_access_index));
        sorted
            .nonce_changes
            .sort_by(|a, b| a.block_access_index.cmp(&b.block_access_index));
        sorted
            .code_changes
            .sort_by(|a, b| a.block_access_index.cmp(&b.block_access_index));

        structs::Encoder::new(buf)
            .encode_field(&sorted.address)
            .encode_field(&sorted.storage_changes)
            .encode_field(&sorted.storage_reads)
            .encode_field(&sorted.balance_changes)
            .encode_field(&sorted.nonce_changes)
            .encode_field(&sorted.code_changes)
            .finish();
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
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
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let mut sorted = self.inner.clone();
        sorted.sort_by(|a, b| a.address.cmp(&b.address));
        sorted.encode(buf);
    }
}

#[cfg(test)]
mod tests {
    use ethereum_types::{H160, U256};
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
    fn test_empty_list_validation() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "dbda94000000000000000000000000000000000000000ac0c0c0c0c0"
        );
    }

    #[test]
    fn test_partial_validation() {
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

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "e3e294000000000000000000000000000000000000000ac0c20102c3c20164c3c20101c0"
        );
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

        let encoded_rlp = hex::encode(buf);
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
}
