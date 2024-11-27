use crate::utils::eth_client::{
    errors::{CallError, EthClientError},
    EthClient, RpcResponse,
};
use bytes::Bytes;
use ethereum_types::{Address, U256};
use ethrex_core::types::{GenericTransaction, TxKind};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::utils::{RpcRequest, RpcRequestId};
use keccak_hash::{keccak, H256};
use secp256k1::SecretKey;
use serde_json::json;

#[derive(Default, Clone)]
pub struct Overrides {
    pub from: Option<Address>,
    pub value: Option<U256>,
    pub nonce: Option<u64>,
    pub chain_id: Option<u64>,
    pub gas_limit: Option<u64>,
    pub gas_price: Option<u64>,
    pub priority_gas_price: Option<u64>,
    pub access_list: Vec<(Address, Vec<H256>)>,
    pub gas_price_per_blob: Option<U256>,
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
            gas_price: overrides
                .gas_price
                .unwrap_or(self.get_gas_price().await?.as_u64()),
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
                json!("latest"),
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
        let mut deploy_tx = self
            .build_eip1559_transaction(Address::zero(), deployer, init_code, overrides, 10)
            .await?;
        deploy_tx.to = TxKind::Create;
        let deploy_tx_hash = self
            .send_eip1559_transaction(&deploy_tx, &deployer_private_key)
            .await?;

        let encoded_from = deployer.encode_to_vec();
        // FIXME: We'll probably need to use nonce - 1 since it was updated above.
        let encoded_nonce = self.get_nonce(deployer).await?.encode_to_vec();
        let mut encoded = vec![(0xc0 + encoded_from.len() + encoded_nonce.len()) as u8];
        encoded.extend(encoded_from.clone());
        encoded.extend(encoded_nonce.clone());
        let deployed_address = Address::from(keccak(encoded));

        Ok((deploy_tx_hash, deployed_address))
    }
}
