use crate::based::{env::EnvV0, frag::FragV0, seal::SealV0};
use serde::{Deserialize, Serialize};
use strum_macros::AsRefStr;
use tree_hash_derive::TreeHash;

#[derive(Debug, Clone, PartialEq, Eq, TreeHash, Serialize, Deserialize, AsRefStr)]
#[tree_hash(enum_behaviour = "union")]
#[serde(untagged)]
#[non_exhaustive]
pub enum VersionedMessage {
    FragV0(FragV0),
    SealV0(SealV0),
    EnvV0(EnvV0),
}

impl From<FragV0> for VersionedMessage {
    fn from(value: FragV0) -> Self {
        Self::FragV0(value)
    }
}

impl From<SealV0> for VersionedMessage {
    fn from(value: SealV0) -> Self {
        Self::SealV0(value)
    }
}

impl From<EnvV0> for VersionedMessage {
    fn from(value: EnvV0) -> Self {
        Self::EnvV0(value)
    }
}

#[cfg(test)]
mod tests {
    use crate::based::{
        env::{EnvV0, ExtraData},
        seal::SealV0,
        versioned_message::VersionedMessage,
    };
    use ethrex_common::{H160, H256, U256};
    use std::str::FromStr;
    use tree_hash::TreeHash;

    //0xf648cd70e6e22c6f5898fa57d74b87ec1f4b82661f5c82ccc39a6325f5f0038d
    #[test]
    fn test_env_v0() {
        let env = EnvV0 {
            number: 1,
            beneficiary: H160::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            timestamp: 2,
            gas_limit: 3,
            basefee: 4,
            difficulty: U256::from(5),
            prevrandao: H256::from_str(
                "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            )
            .unwrap(),
            parent_hash: H256::from_str(
                "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            )
            .unwrap(),
            extra_data: ExtraData::from(vec![1, 2, 3]),
            parent_beacon_block_root: H256::from_str(
                "0xe75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            )
            .unwrap(),
        };

        let message = VersionedMessage::from(env);
        let hash = H256::from_slice(message.tree_hash_root().as_ref());
        assert_eq!(
            hash,
            H256::from_str("0xfa09df7670737568ba783dfd934e19b06e6681e367a866a5647449bd4e5ca324")
                .unwrap()
        );
    }

    // #[test]
    // fn test_frag_v0() {
    //     let tx = Transaction::from(vec![1, 2, 3]);
    //     let txs = Transactions::from(vec![tx]);

    //     let frag = FragV0 {
    //         block_number: 1,
    //         seq: 0,
    //         is_last: true,
    //         txs,
    //     };

    //     let message = VersionedMessage::from(frag);
    //     let hash = message.tree_hash_root();
    //     assert_eq!(
    //         hash,
    //         b256!("2a5ebad20a81878e5f229928e5c2043580051673b89a7a286008d30f62b10963")
    //     );
    // }

    #[test]
    fn test_seal_v0() {
        let sealed = SealV0 {
            total_frags: 8,
            block_number: 123,
            gas_used: 25_000,
            gas_limit: 1_000_000,
            parent_hash: H256::from_str(
                "e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            )
            .unwrap(),
            transactions_root: H256::from_str(
                "e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            )
            .unwrap(),
            receipts_root: H256::from_str(
                "e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            )
            .unwrap(),
            state_root: H256::from_str(
                "e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            )
            .unwrap(),
            block_hash: H256::from_str(
                "e75fae0065403d4091f3d6549c4219db69c96d9de761cfc75fe9792b6166c758",
            )
            .unwrap(),
        };

        let message = VersionedMessage::from(sealed);
        let hash = H256::from_slice(message.tree_hash_root().as_ref());
        assert_eq!(
            hash,
            H256::from_str("e86afda21ddc7338c7e84561681fde45e2ab55cce8cde3163e0ae5f1c378439e")
                .unwrap()
        );
    }
}
