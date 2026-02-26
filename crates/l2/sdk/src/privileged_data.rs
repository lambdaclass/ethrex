use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{PrivilegedL2Transaction, TxType},
};
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, Overrides},
    types::receipt::RpcLogInfo,
};

use crate::build_generic_tx;

pub struct PrivilegedTransactionData {
    pub value: U256,
    pub to_address: H160,
    pub transaction_id: U256,
    pub from: H160,
    pub gas_limit: U256,
    pub calldata: Vec<u8>,
}

impl PrivilegedTransactionData {
    pub fn from_log(log: RpcLogInfo) -> Result<PrivilegedTransactionData, String> {
        /*
            event PrivilegedTxSent (
                address indexed L1from, => part of topics, not data
                address from, => 0..32
                address to, => 32..64
                uint256 transactionId, => 64..96
                uint256 value, => 96..128
                uint256 gasLimit, => 128..160
                bytes data
                    => offset_data => 160..192
                    => length_data => 192..224
                    => data => 224..
            );
            Any value that is not 32 bytes is padded with zeros.
        */

        let from = H256::from_slice(log.data.get(0..32).ok_or(
            "Failed to parse gas_limit from log: log.data[0..32] out of bounds".to_owned(),
        )?);
        let from_address = hash_to_address(from);

        let to = H256::from_slice(log.data.get(32..64).ok_or(
            "Failed to parse gas_limit from log: log.data[32..64] out of bounds".to_owned(),
        )?);
        let to_address = hash_to_address(to);

        let transaction_id = U256::from_big_endian(log.data.get(64..96).ok_or(
            "Failed to parse gas_limit from log: log.data[64..96] out of bounds".to_owned(),
        )?);

        let value = U256::from_big_endian(log.data.get(96..128).ok_or(
            "Failed to parse gas_limit from log: log.data[96..128] out of bounds".to_owned(),
        )?);

        let gas_limit = U256::from_big_endian(log.data.get(128..160).ok_or(
            "Failed to parse gas_limit from log: log.data[128..160] out of bounds".to_owned(),
        )?);

        // 160..192 is taken by offset_data, which we do not need

        let calldata_len = U256::from_big_endian(log.data.get(192..224).ok_or(
            "Failed to parse calldata_len from log: log.data[192..224] out of bounds".to_owned(),
        )?);

        let calldata = log.data.get(224..224 + calldata_len.as_usize()).ok_or(
            "Failed to parse calldata from log: log.data[224..224 + calldata_len] out of bounds"
                .to_owned(),
        )?;

        Ok(Self {
            value,
            to_address,
            transaction_id,
            from: from_address,
            gas_limit,
            calldata: calldata.to_vec(),
        })
    }
    pub async fn into_tx(
        &self,
        eth_client: &EthClient,
        chain_id: u64,
        gas_price: u64,
    ) -> Result<PrivilegedL2Transaction, EthClientError> {
        let generic_tx = build_generic_tx(
            eth_client,
            TxType::Privileged,
            self.to_address,
            self.from,
            Bytes::copy_from_slice(&self.calldata),
            Overrides {
                chain_id: Some(chain_id),
                // Using the transaction_id as nonce.
                // If we make a transaction on the L2 with this address, we may break the
                // privileged transaction workflow.
                nonce: Some(self.transaction_id.as_u64()),
                value: Some(self.value),
                gas_limit: Some(self.gas_limit.as_u64()),
                // TODO(CHECK): Seems that when we start the L2, we need to set the gas.
                // Otherwise, the transaction is not included in the mempool.
                // We should override the blockchain to always include the transaction.
                max_fee_per_gas: Some(gas_price),
                max_priority_fee_per_gas: Some(gas_price),
                ..Default::default()
            },
        )
        .await?;
        Ok(generic_tx.try_into()?)
    }
}

pub fn hash_to_address(hash: H256) -> Address {
    Address::from_slice(&hash.as_fixed_bytes()[12..])
}
