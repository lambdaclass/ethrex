use ethrex_blockchain::constants::GAS_PER_BLOB;
use ethrex_core::{
    serde_utils,
    types::{BlockHash, BlockHeader, BlockNumber, Log, Receipt, Transaction, TxKind, TxType},
    Address, Bloom, Bytes, H256,
};
use ethrex_vm::RevmAddress;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcReceipt {
    #[serde(flatten)]
    pub receipt: RpcReceiptInfo,
    pub logs: Vec<RpcLog>,
    #[serde(flatten)]
    pub tx_info: RpcReceiptTxInfo,
    #[serde(flatten)]
    pub block_info: RpcReceiptBlockInfo,
}

impl RpcReceipt {
    pub fn new(
        receipt: Receipt,
        tx_info: RpcReceiptTxInfo,
        block_info: RpcReceiptBlockInfo,
        init_log_index: u64,
    ) -> Self {
        let mut logs = vec![];
        let mut log_index = init_log_index;
        for log in receipt.logs.clone() {
            logs.push(RpcLog::new(log, log_index, &tx_info, &block_info));
            log_index += 1;
        }
        Self {
            receipt: receipt.into(),
            logs,
            tx_info,
            block_info,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcReceiptInfo {
    #[serde(rename = "type")]
    pub tx_type: TxType,
    #[serde(with = "serde_utils::bool")]
    pub status: bool,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub cumulative_gas_used: u64,
    pub logs_bloom: Bloom,
}

impl From<Receipt> for RpcReceiptInfo {
    fn from(receipt: Receipt) -> Self {
        Self {
            tx_type: receipt.tx_type,
            status: receipt.succeeded,
            cumulative_gas_used: receipt.cumulative_gas_used,
            logs_bloom: receipt.bloom,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcLog {
    #[serde(flatten)]
    pub log: RpcLogInfo,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub log_index: u64,
    pub removed: bool,
    pub transaction_hash: H256,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub transaction_index: u64,
    pub block_hash: BlockHash,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub block_number: BlockNumber,
}

impl RpcLog {
    pub fn new(
        log: Log,
        log_index: u64,
        tx_info: &RpcReceiptTxInfo,
        block_info: &RpcReceiptBlockInfo,
    ) -> RpcLog {
        Self {
            log: log.into(),
            log_index,
            removed: false,
            transaction_hash: tx_info.transaction_hash,
            transaction_index: tx_info.transaction_index,
            block_hash: block_info.block_hash,
            block_number: block_info.block_number,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcLogInfo {
    pub address: Address,
    pub topics: Vec<H256>,
    #[serde(with = "serde_utils::bytes")]
    pub data: Bytes,
}

impl From<Log> for RpcLogInfo {
    fn from(log: Log) -> Self {
        Self {
            address: log.address,
            topics: log.topics,
            data: log.data,
        }
    }
}

#[derive(Debug, Serialize, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcReceiptBlockInfo {
    pub block_hash: BlockHash,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub block_number: BlockNumber,
}

impl RpcReceiptBlockInfo {
    pub fn from_block_header(block_header: BlockHeader) -> Self {
        RpcReceiptBlockInfo {
            block_hash: block_header.compute_block_hash(),
            block_number: block_header.number,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcReceiptTxInfo {
    pub transaction_hash: H256,
    #[serde(with = "ethrex_core::serde_utils::u64::hex_str")]
    pub transaction_index: u64,
    pub from: Address,
    pub to: Option<Address>,
    pub contract_address: Option<Address>,
    #[serde(with = "ethrex_core::serde_utils::u64::hex_str")]
    pub gas_used: u64,
    #[serde(with = "ethrex_core::serde_utils::u64::hex_str")]
    pub effective_gas_price: u64,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "ethrex_core::serde_utils::u64::hex_str_opt",
        default = "Option::default"
    )]
    pub blob_gas_price: Option<u64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "serde_utils::u64::hex_str_opt",
        default = "Option::default"
    )]
    pub blob_gas_used: Option<u64>,
}

impl RpcReceiptTxInfo {
    pub fn from_transaction(
        transaction: Transaction,
        index: u64,
        gas_used: u64,
        block_blob_gas_price: u64,
    ) -> Self {
        let nonce = transaction.nonce();
        let from = transaction.sender();
        let transaction_hash = transaction.compute_hash();
        let effective_gas_price = transaction.gas_price();
        let transaction_index = index;
        let (blob_gas_price, blob_gas_used) = match &transaction {
            Transaction::EIP4844Transaction(tx) => (
                Some(block_blob_gas_price),
                Some(tx.blob_versioned_hashes.len() as u64 * GAS_PER_BLOB),
            ),
            _ => (None, None),
        };
        let (contract_address, to) = match transaction.to() {
            TxKind::Create => (
                // Calculate contract_address from `sender` and `nonce` fields.
                Some(Address::from_slice(
                    RevmAddress(from.0.into()).create(nonce).0.as_ref(),
                )),
                None,
            ),
            TxKind::Call(addr) => (None, Some(addr)),
        };
        Self {
            transaction_hash,
            transaction_index,
            from,
            to,
            contract_address,
            gas_used,
            effective_gas_price,
            blob_gas_price,
            blob_gas_used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_core::{
        types::{Log, TxType},
        Bloom, Bytes,
    };
    use hex_literal::hex;

    #[test]
    fn serialize_receipt() {
        let receipt = RpcReceipt::new(
            Receipt {
                tx_type: TxType::EIP4844,
                succeeded: true,
                cumulative_gas_used: 147,
                bloom: Bloom::zero(),
                logs: vec![Log {
                    address: Address::zero(),
                    topics: vec![],
                    data: Bytes::from_static(b"strawberry"),
                }],
            },
            RpcReceiptTxInfo {
                transaction_hash: H256::zero(),
                transaction_index: 1,
                from: Address::zero(),
                to: Some(Address::from(hex!(
                    "7435ed30a8b4aeb0877cef0c6e8cffe834eb865f"
                ))),
                contract_address: None,
                gas_used: 147,
                effective_gas_price: 157,
                blob_gas_price: None,
                blob_gas_used: None,
            },
            RpcReceiptBlockInfo {
                block_hash: BlockHash::zero(),
                block_number: 3,
            },
            0,
        );
        let expected = r#"{"type":"0x3","status":"0x1","cumulativeGasUsed":"0x93","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","logs":[{"address":"0x0000000000000000000000000000000000000000","topics":[],"data":"0x73747261776265727279","logIndex":"0x0","removed":false,"transactionHash":"0x0000000000000000000000000000000000000000000000000000000000000000","transactionIndex":"0x1","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","blockNumber":"0x3"}],"transactionHash":"0x0000000000000000000000000000000000000000000000000000000000000000","transactionIndex":"0x1","from":"0x0000000000000000000000000000000000000000","to":"0x7435ed30a8b4aeb0877cef0c6e8cffe834eb865f","contractAddress":null,"gasUsed":"0x93","effectiveGasPrice":"0x9d","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","blockNumber":"0x3"}"#;
        assert_eq!(serde_json::to_string(&receipt).unwrap(), expected);
    }
}
