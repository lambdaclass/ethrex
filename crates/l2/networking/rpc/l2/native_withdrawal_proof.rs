use ethrex_common::{Address, H160, H256, U256, utils::keccak};
use serde::Serialize;
use serde_json::Value;
use tracing::info;

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    utils::RpcErr,
};

/// L2Bridge predeploy address (0x...fffd).
const L2_BRIDGE: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfd,
]);

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
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
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
            transaction_hash: serde_json::from_value(params[0].clone())?,
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
            .find(|l| l.address == L2_BRIDGE && l.topics.first() == Some(&topic0))
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
        let storage_key = H256::from_slice(keccak(slot_preimage).as_bytes());

        // 6. Get the block header for the state root
        let header = storage.get_block_header(block_number)?.ok_or_else(|| {
            RpcErr::Internal(format!("Block header not found for block {block_number}"))
        })?;

        // 7. Get account proof + storage proof via eth_getProof path
        let account_proof = storage
            .get_account_proof(header.state_root, L2_BRIDGE, &[storage_key])
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
