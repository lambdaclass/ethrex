use crate::{
    clients::eth::get_address_from_secret_key,
    eth::{fee_calculator::estimate_gas_tip, gas_price::GasPrice, transaction::EstimateGasRequest},
    types::transaction::SendRawTransactionRequest,
    utils::RpcErr,
    RpcApiContext, RpcHandler,
};
use bytes::Bytes;
use ethrex_common::{
    types::{
        AuthorizationList, EIP1559Transaction, EIP7702Transaction, GenericTransaction, Signable,
        TxKind,
    },
    Address, U256,
};
use secp256k1::SecretKey;
use serde::Deserialize;
use serde_json::Value;
use std::str::FromStr;

const DELGATION_PREFIX: [u8; 3] = [0xef, 0x01, 0x00];
const EIP7702_DELEGATED_CODE_LEN: usize = 23;
// This could be an environment variable set in the config.toml is the max amount of gas we are willing to sponsor
const GAS_LIMIT_HARD_LIMIT: u64 = 100000;

#[derive(Deserialize, Debug)]
pub struct RogueSponsoredTx {
    #[serde(rename(deserialize = "authorizationList"))]
    pub authorization_list: Option<AuthorizationList>,
    #[serde(deserialize_with = "deserialize_hex_bytes")]
    pub data: Bytes,
    pub to: Address,
}

fn deserialize_hex_bytes<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(Bytes::from(
        hex::decode(s.trim_start_matches("0x")).map_err(|err| {
            serde::de::Error::custom(format!(
                "error decoding hex data when deserializing bytes: {err}"
            ))
        })?,
    ))
}
// This endpoint is inspired by the work of Ithaca in Odyssey
// https://ithaca.xyz/updates/exp-0000
// You can check the reference implementation here
// https://github.com/ithacaxyz/odyssey/blob/main/crates/wallet/src/lib.rs
impl RpcHandler for RogueSponsoredTx {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;

        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected one param and {} were provided",
                params.len()
            )));
        };
        serde_json::from_value(
            params
                .first()
                .ok_or(RpcErr::InvalidRogueMessage(
                    "Failed to parse request into rogue_SendTransaction".to_string(),
                ))?
                .clone(),
        )
        .map_err(|e| {
            RpcErr::InvalidRogueMessage(format!(
                "Failed to parse request into rogue_SendTransaction: {e}"
            ))
        })
    }

    fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // Dont allow create txs
        if self.to.is_zero() {
            return Err(RpcErr::InvalidRogueMessage(
                "Invalid Create transactions are not supported".to_string(),
            ));
        }
        // If tx is not EIP-7702 check we are calling a delegated account
        if let Some(auth_list) = &self.authorization_list {
            for tuple in auth_list {
                if !context.valid_delegation_addresses.contains(&tuple.address) {
                    return Err(RpcErr::InvalidRogueMessage(
                        "Invalid tx trying to delegate to an address that isn't sponsored"
                            .to_string(),
                    ));
                }
            }
        } else {
            let dest_account = context
                .storage
                .get_account_info(
                    context
                        .storage
                        .get_latest_block_number()
                        .map_err(RpcErr::from)?,
                    self.to,
                )
                .map_err(RpcErr::from)?
                .unwrap_or_default();
            let code = context
                .storage
                .get_account_code(dest_account.code_hash)
                .map_err(RpcErr::from)?
                .unwrap_or_default();

            let prefix: Vec<u8> = code.iter().take(3).copied().collect();
            if code.len() != EIP7702_DELEGATED_CODE_LEN || prefix != DELGATION_PREFIX {
                return Err(RpcErr::InvalidRogueMessage(
                    "Invalid tx trying to call non delegated account".to_string(),
                ));
            }
            let address = Address::from_slice(&code[3..]);
            if address.is_zero() {
                return Err(RpcErr::InvalidRogueMessage(
                    "Invalid tx trying to call non delegated account".to_string(),
                ));
            }
            if !context.valid_delegation_addresses.contains(&address) {
                return Err(RpcErr::InvalidRogueMessage(
                    "Invalid tx trying to call delegated address not in sponsored addresses"
                        .to_string(),
                ));
            }
        }
        let sponsor_pk = SecretKey::from_str(context.proposer_pk.trim_start_matches("0x"))
            .map_err(|_| RpcErr::InvalidRogueMessage("Rogue Rpc Method not enabled".to_string()))?;
        let sponsor_address = get_address_from_secret_key(&sponsor_pk)
            .map_err(|_| RpcErr::InvalidRogueMessage("Rogue Rpc method not enabled".to_string()))?;
        let latest_block_number = context
            .storage
            .get_latest_block_number()
            .map_err(RpcErr::from)?;
        let chain_config = context.storage.get_chain_config().map_err(RpcErr::from)?;
        let chain_id = chain_config.chain_id;
        let nonce = context
            .storage
            .get_nonce_by_account_address(latest_block_number, sponsor_address)
            .map_err(RpcErr::from)?
            .ok_or(RpcErr::InvalidRogueMessage("Invalid nonce".to_string()))?;
        let max_priority_fee_per_gas = estimate_gas_tip(&context.storage)
            .map_err(RpcErr::from)?
            .unwrap_or_default();
        let gas_price_request = GasPrice {}.handle(context.clone())?;
        let max_fee_per_gas = u64::from_str_radix(
            gas_price_request
                .as_str()
                .unwrap_or("0x0")
                .strip_prefix("0x")
                .unwrap(),
            16,
        )
        .map_err(|err| RpcErr::Internal(err.to_string()))?;

        let mut tx = if let Some(auth_list) = &self.authorization_list {
            SendRawTransactionRequest::EIP7702(EIP7702Transaction {
                chain_id,
                to: self.to,
                value: U256::zero(),
                data: self.data.clone(),
                access_list: Vec::new(),
                authorization_list: auth_list.clone(),
                ..Default::default()
            })
        } else {
            SendRawTransactionRequest::EIP1559(EIP1559Transaction {
                chain_id,
                to: TxKind::Call(self.to),
                value: U256::zero(),
                data: self.data.clone(),
                access_list: Vec::new(),
                ..Default::default()
            })
        };

        let mut generic = match tx.to_transaction() {
            ethrex_common::types::Transaction::EIP1559Transaction(tx) => {
                GenericTransaction::from(tx)
            }
            ethrex_common::types::Transaction::EIP7702Transaction(tx) => {
                GenericTransaction::from(tx)
            }
            _ => unreachable!("This should never happen, we are creating the tx"),
        };
        generic.gas = None;

        let estimate_gas_request = EstimateGasRequest {
            transaction: generic,
            block: None,
        }
        .handle(context.clone())?;
        let gas_limit = u64::from_str_radix(
            estimate_gas_request
                .as_str()
                .unwrap_or("0x0")
                .strip_prefix("0x")
                .unwrap(),
            16,
        )
        .unwrap();
        if gas_limit == 0 || gas_limit > GAS_LIMIT_HARD_LIMIT {
            return Err(RpcErr::InvalidRogueMessage("tx too expensive".to_string()));
        }
        match tx {
            SendRawTransactionRequest::EIP7702(ref mut tx) => {
                tx.gas_limit = gas_limit;
                tx.max_fee_per_gas = max_fee_per_gas;
                tx.max_priority_fee_per_gas = max_priority_fee_per_gas;
                tx.nonce = nonce;
                tx.sign_inplace(&sponsor_pk);
            }
            SendRawTransactionRequest::EIP1559(ref mut tx) => {
                tx.gas_limit = gas_limit;
                tx.max_fee_per_gas = max_fee_per_gas;
                tx.max_priority_fee_per_gas = max_priority_fee_per_gas;
                tx.nonce = nonce;
                tx.sign_inplace(&sponsor_pk);
            }
            _ => unreachable!("This should never happen, we are creating the tx"),
        }

        tx.handle(context)
    }
}
