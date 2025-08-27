use crate::calldata::{self};
use bytes::Bytes;
use ethrex_common::{Address, U256, types::TxType};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::{clients::send_generic_transaction, signer::LocalSigner};
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, Overrides, eth::errors::CalldataEncodeError},
};
use keccak_hash::H256;
use secp256k1::SecretKey;

#[derive(Debug)]
pub struct L1ToL2TransactionData {
    pub to: Address,
    pub gas_limit: u64,
    pub value: U256,
    pub calldata: Bytes,
}

impl L1ToL2TransactionData {
    /// Creates a new L1ToL2TransactionData instance.
    ///
    /// # Arguments
    ///
    /// * `to` - The address of the contract to call on L2.
    /// * `gas_limit` - The gas limit for the transaction on L2.
    /// * `value` - The value of the transaction on L2.
    /// * `calldata` - The calldata to send to the contract on L2.
    pub fn new(to: Address, gas_limit: u64, value: U256, calldata: Bytes) -> Self {
        Self {
            to,
            gas_limit,
            value,
            calldata,
        }
    }

    /// Encodes the `L1ToL2TransactionData` into a calldata.
    pub fn to_calldata(&self) -> Result<Vec<u8>, CalldataEncodeError> {
        let values = vec![Value::Tuple(vec![
            Value::Address(self.to),
            Value::Uint(U256::from(self.gas_limit)),
            Value::Uint(self.value),
            Value::Bytes(self.calldata.clone()),
        ])];
        calldata::encode_calldata("sendToL2((address,uint256,uint256,bytes))", &values)
    }
}

/// This function is used to send a transaction on L2 from L1 using the `CommonBridge` contract.
///
/// # Arguments
///
/// * `l1_from` - The address of the sender on L1.
/// * `l1_value` - The value to send from L1.
/// * `l1_gas_limit` - The gas limit for the transaction on L1.
/// * `l1_to_l2_tx_data` - The data for the transaction on L2.
/// * `sender_private_key` - The private key of the sender on L1.
/// * `bridge_address` - The address of the `CommonBridge` contract.
/// * `eth_client` - The Ethereum client to use.
#[allow(clippy::too_many_arguments)]
pub async fn send_l1_to_l2_tx(
    l1_from: Address,
    l1_value: Option<impl Into<U256>>,
    l1_gas_limit: Option<u64>,
    l1_to_l2_tx_data: L1ToL2TransactionData,
    sender_private_key: &SecretKey,
    bridge_address: Address,
    eth_client: &EthClient,
) -> Result<H256, EthClientError> {
    let l1_calldata = l1_to_l2_tx_data.to_calldata()?;

    let l1_tx_overrides = Overrides {
        value: l1_value.map(Into::into),
        from: Some(l1_from),
        gas_limit: l1_gas_limit,
        ..Overrides::default()
    };

    let mut l1_to_l2_tx = eth_client
        .build_generic_tx(
            TxType::EIP1559,
            bridge_address,
            l1_from,
            l1_calldata.into(),
            l1_tx_overrides,
        )
        .await?;

    l1_to_l2_tx.gas = l1_to_l2_tx.gas.map(|gas| gas * 2); // tx reverts in some cases otherwise

    let signer = LocalSigner::new(*sender_private_key).into();

    send_generic_transaction(eth_client, l1_to_l2_tx, &signer).await
}
