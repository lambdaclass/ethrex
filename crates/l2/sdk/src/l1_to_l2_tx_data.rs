use crate::calldata::{self};
use bytes::Bytes;
use ethrex_common::{Address, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_rpc::{
    clients::{eth::errors::CalldataEncodeError, EthClientError, Overrides},
    EthClient,
};
use keccak_hash::H256;
use secp256k1::SecretKey;

#[derive(Debug)]
pub struct L1ToL2TransactionData {
    pub to: Address,
    pub recipient: Address,
    pub gas_limit: u64,
    pub calldata: Bytes,
}

impl L1ToL2TransactionData {
    /// Creates a new L1ToL2TransactionData instance.
    ///
    /// # Arguments
    ///
    /// * `to` - The address of the contract to call on L2.
    /// * `recipient` - The address of the recipient on L2 that will receive the
    ///   L1 transaction value.
    /// * `gas_limit` - The gas limit for the transaction on L2.
    /// * `calldata` - The calldata to send to the contract on L2.
    pub fn new(to: Address, recipient: Address, gas_limit: u64, calldata: Bytes) -> Self {
        Self {
            to,
            recipient,
            gas_limit,
            calldata,
        }
    }

    /// Creates a new `L1ToL2TransactionData` instance for a deposit transaction.
    ///
    /// In deposit transactions, the `to` and the `recipient` are the same, and
    /// the `calldata` is empty.
    ///
    /// # Arguments
    ///
    /// * `recipient` - The address of the recipient on L2 that will receive the
    ///   L1 transaction value (the deposit).
    /// * `gas_limit` - The gas limit for the transaction on L2.
    pub fn new_deposit_data(recipient: Address, gas_limit: u64) -> Self {
        Self {
            to: recipient,
            recipient,
            gas_limit,
            calldata: Bytes::from_static(b""),
        }
    }

    /// Encodes the `L1ToL2TransactionData` into a calldata.
    pub fn to_calldata(&self) -> Result<Vec<u8>, CalldataEncodeError> {
        let values = vec![Value::Tuple(vec![
            Value::Address(self.to),
            Value::Address(self.recipient),
            Value::Uint(U256::from(self.gas_limit)),
            Value::Bytes(self.calldata.clone()),
        ])];
        calldata::encode_calldata("deposit((address,address,uint256,bytes))", &values)
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

    let l1_to_l2_tx = eth_client
        .build_eip1559_transaction(bridge_address, l1_from, l1_calldata.into(), l1_tx_overrides)
        .await?;

    eth_client
        .send_eip1559_transaction(&l1_to_l2_tx, sender_private_key)
        .await
}
