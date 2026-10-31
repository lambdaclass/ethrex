use ethrex_common::{Address, H256, U256, utils::keccak};
use ethrex_l2_common::messages::NATIVE_ROLLUP_L2_BRIDGE;
use serde::Serialize;
use serde_json::{Value, value::RawValue};
use tracing::info;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

/// Storage slot of `sentMessages` mapping in L2Bridge (slot 3).
const SENT_MESSAGES_SLOT: u64 = 3;

/// `keccak256("WithdrawalInitiated(address,address,uint256,uint256)")`
fn withdrawal_initiated_topic() -> H256 {
    keccak(b"WithdrawalInitiated(address,address,uint256,uint256)")
}

pub struct GetNativeWithdrawalProof {
    pub transaction_hash: H256,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeWithdrawalProof {
    pub from: Address,
    pub receiver: Address,
    pub amount: String,
    pub message_id: String,
    pub block_number: String,
    pub account_proof: Vec<String>,
    pub storage_proof: Vec<String>,
}

impl RpcHandler for GetNativeWithdrawalProof {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
        let params = params.as_ref().ok_or(ethrex_rpc::RpcErr::BadParams(
            "No params provided".to_owned(),
        ))?;
        if params.len() != 1 {
            return Err(ethrex_rpc::RpcErr::BadParams(format!(
                "Expected one param and {} were provided",
                params.len()
            ))
            .into());
        }
        Ok(GetNativeWithdrawalProof {
            transaction_hash: serde_json::from_str(params[0].get())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.l1_ctx.storage;
        info!(
            "Requested native withdrawal proof for transaction {:#x}",
            self.transaction_hash
        );

        // 1. Look up transaction location and receipt
        let (block_number, _, tx_index) = match storage
            .get_transaction_location(self.transaction_hash)
            .await?
        {
            Some(loc) => loc,
            None => return Ok(Value::Null),
        };
        let receipt = match storage.get_receipt(block_number, tx_index).await? {
            Some(r) => r,
            None => return Ok(Value::Null),
        };

        // 2. Find the WithdrawalInitiated event in the receipt logs
        let topic0 = withdrawal_initiated_topic();
        let log = match receipt
            .logs
            .iter()
            .find(|l| l.address == NATIVE_ROLLUP_L2_BRIDGE && l.topics.first() == Some(&topic0))
        {
            Some(l) => l,
            None => {
                return Err(RpcErr::Internal(
                    "No WithdrawalInitiated event found in transaction receipt".to_string(),
                ));
            }
        };

        // 3. Decode event fields
        //    topic[1] = from (indexed address, padded to 32 bytes)
        //    topic[2] = receiver (indexed address, padded to 32 bytes)
        //    topic[3] = messageId (indexed uint256)
        //    data      = abi.encode(amount) — 32 bytes
        if log.topics.len() < 4 || log.data.len() < 32 {
            return Err(RpcErr::Internal(
                "Malformed WithdrawalInitiated event".to_string(),
            ));
        }
        let from = Address::from_slice(&log.topics[1].as_bytes()[12..]);
        let receiver = Address::from_slice(&log.topics[2].as_bytes()[12..]);
        let message_id = U256::from_big_endian(log.topics[3].as_bytes());
        let amount = U256::from_big_endian(&log.data[..32]);

        // 4. Compute withdrawal hash: keccak256(abi.encodePacked(from, receiver, amount, messageId))
        //    abi.encodePacked: 20 + 20 + 32 + 32 = 104 bytes
        let mut preimage = Vec::with_capacity(104);
        preimage.extend_from_slice(from.as_bytes());
        preimage.extend_from_slice(receiver.as_bytes());
        preimage.extend_from_slice(&amount.to_big_endian());
        preimage.extend_from_slice(&message_id.to_big_endian());
        let withdrawal_hash = keccak(&preimage);

        // 5. Compute storage slot: keccak256(abi.encode(withdrawalHash, uint256(SENT_MESSAGES_SLOT)))
        //    abi.encode: 32 + 32 = 64 bytes
        let mut slot_preimage = [0u8; 64];
        slot_preimage[..32].copy_from_slice(withdrawal_hash.as_bytes());
        slot_preimage[32..].copy_from_slice(&U256::from(SENT_MESSAGES_SLOT).to_big_endian());
        let storage_key = keccak(slot_preimage);

        // 6. Get the block header for the state root
        let header = storage.get_block_header(block_number)?.ok_or_else(|| {
            RpcErr::Internal(format!("Block header not found for block {block_number}"))
        })?;

        // 7. Get account proof + storage proof via eth_getProof path
        let account_proof = storage
            .get_account_proof(header.state_root, NATIVE_ROLLUP_L2_BRIDGE, &[storage_key])
            .await?
            .ok_or_else(|| {
                RpcErr::Internal("Could not generate account proof for L2Bridge".to_string())
            })?;

        // 8. Format proofs as hex strings
        let account_proof_hex: Vec<String> = account_proof
            .proof
            .iter()
            .map(|node| format!("0x{}", hex::encode(node)))
            .collect();

        let storage_proof_hex: Vec<String> = account_proof
            .storage_proof
            .first()
            .map(|sp| {
                sp.proof
                    .iter()
                    .map(|node| format!("0x{}", hex::encode(node)))
                    .collect()
            })
            .unwrap_or_default();

        let result = NativeWithdrawalProof {
            from,
            receiver,
            amount: format!("{amount:#x}"),
            message_id: format!("{message_id:#x}"),
            block_number: format!("{block_number:#x}"),
            account_proof: account_proof_hex,
            storage_proof: storage_proof_hex,
        };

        serde_json::to_value(result)
            .map_err(|error| ethrex_rpc::RpcErr::Internal(error.to_string()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::{
        AccountState, BlockHeader, Genesis, GenesisAccount, Log, Receipt, TxType,
    };
    use ethrex_crypto::NativeCrypto;
    use ethrex_rlp::decode::RLPDecode;
    use ethrex_rpc::test_utils::{TEST_GENESIS, default_context_with_storage};
    use ethrex_storage::{EngineType, Store};
    use ethrex_storage_rollup::{EngineTypeRollup, StoreRollup};
    use ethrex_trie::{Node, Trie};
    use secp256k1::SecretKey;
    use std::path::Path;

    async fn withdrawal_proof(available: bool) -> (Result<Value, RpcErr>, H256, H256, H256) {
        let from = Address::from_low_u64_be(1);
        let receiver = Address::from_low_u64_be(2);
        let amount = U256::from(4);
        let message_id = U256::from(3);
        let withdrawal_hash = keccak(
            [
                from.as_bytes(),
                receiver.as_bytes(),
                &amount.to_big_endian(),
                &message_id.to_big_endian(),
            ]
            .concat(),
        );
        let slot = keccak([withdrawal_hash.as_bytes(), &U256::from(3).to_big_endian()].concat());
        let mut genesis: Genesis = serde_json::from_str(TEST_GENESIS).unwrap();
        genesis.alloc.clear();
        genesis.alloc.insert(
            NATIVE_ROLLUP_L2_BRIDGE,
            GenesisAccount {
                code: Default::default(),
                storage: [(U256::from_big_endian(slot.as_bytes()), U256::one())].into(),
                balance: U256::zero(),
                nonce: 1,
            },
        );
        let mut storage = Store::new("", EngineType::InMemory).unwrap();
        storage.add_initial_state(genesis).await.unwrap();
        let genesis = storage.get_block_header(0).unwrap().unwrap();
        let storage_root = storage
            .state_view(genesis.state_root)
            .unwrap()
            .unwrap()
            .account_state(NATIVE_ROLLUP_L2_BRIDGE)
            .unwrap()
            .unwrap()
            .storage_root;
        let header = BlockHeader {
            number: 1,
            parent_hash: genesis.hash(),
            state_root: if available {
                genesis.state_root
            } else {
                H256::repeat_byte(0xad)
            },
            ..Default::default()
        };
        let block_hash = header.hash();
        storage
            .add_block_headers(vec![header.clone()])
            .await
            .unwrap();
        storage
            .forkchoice_update(vec![(1, block_hash)], 1, block_hash, None, None)
            .await
            .unwrap();
        let transaction_hash = H256::from_low_u64_be(0x1234);
        storage
            .add_transaction_location(transaction_hash, 1, block_hash, 0)
            .await
            .unwrap();
        storage
            .add_receipt(
                block_hash,
                0,
                Receipt::new(
                    TxType::Legacy,
                    true,
                    21_000,
                    vec![Log {
                        address: NATIVE_ROLLUP_L2_BRIDGE,
                        topics: vec![
                            withdrawal_initiated_topic(),
                            H256::from_low_u64_be(1),
                            H256::from_low_u64_be(2),
                            H256::from_low_u64_be(3),
                        ],
                        data: amount.to_big_endian().to_vec().into(),
                    }],
                ),
            )
            .await
            .unwrap();
        let l1_context = default_context_with_storage(storage).await;
        let context = RpcApiContext {
            l1_ctx: l1_context.clone(),
            valid_delegation_addresses: vec![],
            sponsor_pk: SecretKey::from_byte_array(&[0xab; 32]).unwrap(),
            rollup_store: StoreRollup::new(Path::new(""), EngineTypeRollup::InMemory).unwrap(),
            sponsored_gas_limit: 0,
            ethrex_namespace_allowed: true,
        };
        let response = GetNativeWithdrawalProof { transaction_hash }
            .handle(context)
            .await;
        (response, header.state_root, storage_root, slot)
    }

    fn proof_value(root: H256, proof: &Value, path: &[u8]) -> Option<Vec<u8>> {
        let nodes = proof
            .as_array()
            .unwrap()
            .iter()
            .map(|node| {
                let rlp = hex::decode(node.as_str().unwrap().strip_prefix("0x").unwrap()).unwrap();
                (keccak(&rlp), Node::decode(&rlp).unwrap())
            })
            .collect();
        let trie = Trie::from_nodes(root, &nodes).unwrap();
        assert_eq!(trie.hash_no_commit(&NativeCrypto), root);
        trie.get(path).unwrap()
    }

    #[tokio::test]
    async fn native_withdrawal_proof_uses_the_requested_roots() {
        let (response, state_root, storage_root, slot) = withdrawal_proof(true).await;
        let response = response.unwrap();
        assert_eq!(response["amount"], "0x4");
        assert_eq!(response["messageId"], "0x3");
        assert_eq!(response["blockNumber"], "0x1");
        let account = AccountState::decode(
            &proof_value(
                state_root,
                &response["accountProof"],
                keccak(NATIVE_ROLLUP_L2_BRIDGE.as_bytes()).as_bytes(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(account.storage_root, storage_root);
        let value = U256::decode(
            &proof_value(
                account.storage_root,
                &response["storageProof"],
                keccak(slot.as_bytes()).as_bytes(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(value, U256::one());
    }

    #[tokio::test]
    async fn native_withdrawal_proof_errors_when_state_is_unavailable() {
        let (response, _, _, _) = withdrawal_proof(false).await;
        assert!(matches!(
            response,
            Err(RpcErr::Internal(message))
                if message == "Could not generate account proof for L2Bridge"
        ));
    }
}
