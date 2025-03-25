use crate::{
    based::{env::EnvV0, frag::FragV0, seal::SealV0},
    utils::{RpcErr, RpcRequest},
    RpcApiContext,
};
use ethrex_common::{Public, Signature};
use serde::{Deserialize, Serialize};
use strum_macros::AsRefStr;
use tree_hash::TreeHash;
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

#[derive(Serialize, Deserialize, Debug)]
pub struct SignedMessage {
    pub signature: Signature,
    pub message: VersionedMessage,
}

impl SignedMessage {
    fn parse(params: &Option<Vec<serde_json::Value>>) -> Result<Self, RpcErr> {
        tracing::info!("Parsing based message");

        let Some(params) = params else {
            return Err(RpcErr::InvalidBasedMessage(
                "Failed to parse based request into SignedMessage, params were expected but none were found".to_string(),
            ));
        };

        serde_json::from_value(
            params
                .first()
                .ok_or(RpcErr::InvalidBasedMessage(
                    "Failed to parse based request into SignedMessage, message not found"
                        .to_string(),
                ))?
                .clone(),
        )
        .map_err(|e| {
            RpcErr::InvalidBasedMessage(format!(
                "Failed to parse based request into SignedMessage: {e}"
            ))
        })
    }

    pub fn call_env(req: &RpcRequest, context: RpcApiContext) -> Result<serde_json::Value, RpcErr> {
        let request = Self::parse(&req.params)?;
        match &request.message {
            VersionedMessage::EnvV0(env) => {
                request.check_signature(&context.gateway_pubkey)?;
                env.handle(context)
            }
            other_based_message => Err(RpcErr::InvalidBasedMessage(format!(
                "Failed to handle Env message, found {other_based_message:?} instead of Env"
            ))),
        }
    }

    pub fn call_new_frag(
        req: &RpcRequest,
        context: RpcApiContext,
    ) -> Result<serde_json::Value, RpcErr> {
        let request = Self::parse(&req.params)?;
        match &request.message {
            VersionedMessage::FragV0(frag) => {
                request.check_signature(&context.gateway_pubkey)?;
                frag.handle(context)
            }
            other_based_message => Err(RpcErr::InvalidBasedMessage(format!(
                "Failed to handle Frag message, found {other_based_message:?} instead of Frag"
            ))),
        }
    }

    pub fn call_seal_frag(
        req: &RpcRequest,
        context: RpcApiContext,
    ) -> Result<serde_json::Value, RpcErr> {
        let request = Self::parse(&req.params)?;
        match &request.message {
            VersionedMessage::SealV0(seal) => {
                request.check_signature(&context.gateway_pubkey)?;
                seal.handle(context)
            }
            other_based_message => Err(RpcErr::InvalidBasedMessage(format!(
                "Failed to handle Seal message, found {other_based_message:?} instead of Seal"
            ))),
        }
    }

    fn check_signature(&self, expected: &Public) -> Result<(), RpcErr> {
        let message = libsecp256k1::Message::parse(&self.message.tree_hash_root().0);
        let signature =
            libsecp256k1::Signature::parse_standard_slice(self.signature.0.get(..64).ok_or(
                RpcErr::InvalidBasedMessage(
                    "Invalid signature: signature is too short".to_string(),
                ),
            )?)
            .map_err(|e| RpcErr::InvalidBasedMessage(format!("Invalid signature: {e}")))?;
        let recovery_id = libsecp256k1::RecoveryId::parse_rpc(*self.signature.0.get(64).ok_or(
            RpcErr::InvalidBasedMessage("Invalid signature: recovery ID is missing".to_string()),
        )?)
        .map_err(|err| {
            RpcErr::InvalidBasedMessage(format!("Invalid signature recovery ID: {err}"))
        })?;

        let signer = libsecp256k1::recover(&message, &signature, &recovery_id)
            .map_err(|err| RpcErr::InvalidBasedMessage(format!("Invalid signature: {err}")))?;

        // First byte is compression flag, which is always 0x04 for uncompressed keys
        if signer
            .serialize()
            .get(1..)
            .ok_or(RpcErr::InvalidBasedMessage(
                "Invalid signer: serialized key is too short".to_string(),
            ))?
            != expected.0
        {
            return Err(RpcErr::InvalidBasedMessage(format!(
                "Invalid signer: 0x{}",
                hex::encode(signer.serialize())
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::based::{
        env::{EnvV0, ExtraData},
        frag::{FragV0, Transaction, Transactions},
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

    #[test]
    fn test_frag_v0() {
        let tx = Transaction::from(vec![1, 2, 3]);
        let txs = Transactions::from(vec![tx]);

        let frag = FragV0 {
            block_number: 1,
            sequence: 0,
            is_last: true,
            transactions: txs,
        };

        let message = VersionedMessage::from(frag);
        let hash = H256::from_slice(message.tree_hash_root().as_ref());
        assert_eq!(
            hash,
            H256::from_str("0x2a5ebad20a81878e5f229928e5c2043580051673b89a7a286008d30f62b10963")
                .unwrap()
        );
    }

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
