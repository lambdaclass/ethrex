use crate::{
    clients::eth::{
        EthClient, RpcResponse,
        errors::{CallError, EthClientError},
    },
    types::block_identifier::{BlockIdentifier, BlockTag},
    utils::{RpcRequest, RpcRequestId},
};
use bytes::Bytes;
use ethrex_common::H256;
use ethrex_common::types::{GenericTransaction, TxKind};
use ethrex_common::{Address, U256};
use ethrex_rlp::encode::RLPEncode;
use keccak_hash::keccak;
use secp256k1::SecretKey;
use serde_json::json;

#[derive(Default, Clone, Debug)]
pub struct Overrides {
    pub from: Option<Address>,
    pub to: Option<TxKind>,
    pub value: Option<U256>,
    pub nonce: Option<u64>,
    pub chain_id: Option<u64>,
    pub gas_limit: Option<u64>,
    pub max_fee_per_gas: Option<u64>,
    pub max_priority_fee_per_gas: Option<u64>,
    pub access_list: Vec<(Address, Vec<H256>)>,
    pub gas_price_per_blob: Option<U256>,
    pub block: Option<BlockIdentifier>,
}

impl EthClient {
    pub async fn call(
        &self,
        to: Address,
        calldata: Bytes,
        overrides: Overrides,
    ) -> Result<String, EthClientError> {
        let tx = GenericTransaction {
            to: TxKind::Call(to),
            input: calldata,
            value: overrides.value.unwrap_or_default(),
            from: overrides.from.unwrap_or_default(),
            gas: overrides.gas_limit,
            gas_price: if let Some(gas_price) = overrides.max_fee_per_gas {
                gas_price
            } else {
                self.get_gas_price().await?.as_u64()
            },
            ..Default::default()
        };

        let request = RpcRequest {
            id: RpcRequestId::Number(1),
            jsonrpc: "2.0".to_string(),
            method: "eth_call".to_string(),
            params: Some(vec![
                json!({
                    "to": match tx.to {
                        TxKind::Call(addr) => format!("{addr:#x}"),
                        TxKind::Create => format!("{:#x}", Address::zero()),
                    },
                    "input": format!("0x{:#x}", tx.input),
                    "value": format!("{:#x}", tx.value),
                    "from": format!("{:#x}", tx.from),
                }),
                overrides
                    .block
                    .map(Into::into)
                    .unwrap_or(serde_json::Value::String("latest".to_string())),
            ]),
        };

        match self.send_request(request).await {
            Ok(RpcResponse::Success(result)) => serde_json::from_value(result.result)
                .map_err(CallError::SerdeJSONError)
                .map_err(EthClientError::from),
            Ok(RpcResponse::Error(error_response)) => {
                Err(CallError::RPCError(error_response.error.message).into())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn deploy(
        &self,
        deployer: Address,
        deployer_private_key: SecretKey,
        init_code: Bytes,
        overrides: Overrides,
    ) -> Result<(H256, Address), EthClientError> {
        let mut deploy_overrides = overrides;
        deploy_overrides.to = Some(TxKind::Create);
        let deploy_tx = self
            .build_eip1559_transaction(Address::zero(), deployer, init_code, deploy_overrides)
            .await?;
        let deploy_tx_hash = self
            .send_eip1559_transaction(&deploy_tx, &deployer_private_key)
            .await?;

        let nonce = self
            .get_nonce(deployer, BlockIdentifier::Tag(BlockTag::Latest))
            .await?;
        let mut encode = vec![];
        (deployer, nonce).encode(&mut encode);

        //Taking the last 20bytes so it matches an H160 == Address length
        let deployed_address =
            Address::from_slice(keccak(encode).as_fixed_bytes().get(12..).ok_or(
                EthClientError::Custom("Failed to get deployed_address".to_owned()),
            )?);

        self.wait_for_transaction_receipt(deploy_tx_hash, 1000)
            .await?;

        Ok((deploy_tx_hash, deployed_address))
    }
}
