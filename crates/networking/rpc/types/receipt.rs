use ethrex_common::{
    Address, Bloom, Bytes, H256,
    constants::GAS_PER_BLOB,
    evm::calculate_create_address,
    serde_utils,
    types::{
        BlockHash, BlockHeader, BlockNumber, Log, Receipt, Transaction, TxKind, TxType,
        bloom_from_logs,
    },
};
use ethrex_crypto::NativeCrypto;

use serde::{Deserialize, Serialize};

use crate::utils::RpcErr;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcReceipt {
    #[serde(flatten)]
    pub receipt: RpcReceiptInfo,
    pub logs: Vec<RpcLog>,
    #[serde(flatten)]
    pub tx_info: RpcReceiptTxInfo,
    #[serde(flatten)]
    pub block_info: RpcReceiptBlockInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_receipts: Option<Vec<RpcFrameReceipt>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcFrameReceipt {
    /// EIP-8141 frame status code: 0 = failure, 1 = success, 2 = skipped
    /// (atomic-batch failure). Serialized as a hex-encoded byte.
    #[serde(with = "serde_utils::u8::hex_str")]
    pub status: u8,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_used: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub execution_gas_used: u64,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub state_gas_used: u64,
    pub logs: Vec<RpcLog>,
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
        let payer = receipt.payer;
        let mut frame_log_index = init_log_index;
        let frame_receipts = receipt.frame_receipts.clone().map(|frames| {
            frames
                .into_iter()
                .map(|frame| {
                    let logs = frame
                        .logs
                        .into_iter()
                        .map(|log| {
                            let log = RpcLog::new(log, frame_log_index, &tx_info, &block_info);
                            frame_log_index += 1;
                            log
                        })
                        .collect();
                    RpcFrameReceipt {
                        status: frame.status,
                        gas_used: frame.gas_used + frame.state_gas_used,
                        execution_gas_used: frame.gas_used,
                        state_gas_used: frame.state_gas_used,
                        logs,
                    }
                })
                .collect()
        });
        Self {
            receipt: receipt.into(),
            logs,
            tx_info,
            block_info,
            payer,
            frame_receipts,
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
            logs_bloom: bloom_from_logs(&receipt.logs, &ethrex_crypto::NativeCrypto),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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

#[derive(Debug, Serialize, Deserialize, Clone)]
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
            block_hash: block_header.hash(),
            block_number: block_header.number,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcReceiptTxInfo {
    pub transaction_hash: H256,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub transaction_index: u64,
    pub from: Address,
    pub to: Option<Address>,
    pub contract_address: Option<Address>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub gas_used: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    pub effective_gas_price: u64,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "serde_utils::u64::hex_str_opt",
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
        base_fee_per_gas: Option<u64>,
    ) -> Result<Self, RpcErr> {
        let nonce = transaction.nonce();
        let from = transaction.sender(&NativeCrypto)?;
        let transaction_hash = transaction.hash(&NativeCrypto);
        let effective_gas_price =
            u64::try_from(transaction.effective_gas_price(base_fee_per_gas).ok_or(
                RpcErr::Internal("Could not get effective gas price from tx".into()),
            )?)
            .map_err(|_| RpcErr::Internal("effective gas price overflows u64".into()))?;
        let transaction_index = index;
        let (blob_gas_price, blob_gas_used) = match &transaction {
            Transaction::EIP4844Transaction(tx) => (
                Some(block_blob_gas_price),
                Some(tx.blob_versioned_hashes.len() as u64 * GAS_PER_BLOB as u64),
            ),
            _ => (None, None),
        };
        let (contract_address, to) = match &transaction {
            // EIP-8141: a frame transaction carries no `to` field and creates nothing at the top
            // level. Each frame names its own target, and a creation happens inside a deploy frame.
            // `Transaction::to()` reports the sender for one so the generic call paths have an
            // address to work with; that is not a recipient and must not be presented as one.
            Transaction::FrameTransaction(_) => (None, None),
            _ => match transaction.to() {
                TxKind::Create => (Some(calculate_create_address(from, nonce)), None),
                TxKind::Call(addr) => (None, Some(addr)),
            },
        };
        Ok(Self {
            transaction_hash,
            transaction_index,
            from,
            to,
            contract_address,
            gas_used,
            effective_gas_price,
            blob_gas_price,
            blob_gas_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::{
        Bytes,
        types::{FrameReceipt, FrameTransaction, Log, TxType},
    };
    use hex_literal::hex;

    #[test]
    fn serialize_receipt() {
        let receipt = RpcReceipt::new(
            Receipt {
                tx_type: TxType::EIP4844,
                succeeded: true,
                cumulative_gas_used: 147,
                logs: vec![Log {
                    address: Address::zero(),
                    topics: vec![],
                    data: Bytes::from_static(b"strawberry"),
                }],
                payer: None,
                frame_receipts: None,
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
        let expected = r#"{"type":"0x3","status":"0x1","cumulativeGasUsed":"0x93","logsBloom":"0x00000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","logs":[{"address":"0x0000000000000000000000000000000000000000","topics":[],"data":"0x73747261776265727279","logIndex":"0x0","removed":false,"transactionHash":"0x0000000000000000000000000000000000000000000000000000000000000000","transactionIndex":"0x1","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","blockNumber":"0x3"}],"transactionHash":"0x0000000000000000000000000000000000000000000000000000000000000000","transactionIndex":"0x1","from":"0x0000000000000000000000000000000000000000","to":"0x7435ed30a8b4aeb0877cef0c6e8cffe834eb865f","contractAddress":null,"gasUsed":"0x93","effectiveGasPrice":"0x9d","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","blockNumber":"0x3"}"#;
        assert_eq!(serde_json::to_string(&receipt).unwrap(), expected);
    }

    #[test]
    fn frame_receipts_include_gas_dimensions_and_global_log_metadata() {
        let log = Log {
            address: Address::repeat_byte(1),
            topics: vec![],
            data: Bytes::new(),
        };
        let receipt = RpcReceipt::new(
            Receipt {
                tx_type: TxType::Frame,
                succeeded: false,
                cumulative_gas_used: 30_000,
                logs: vec![log.clone(), log.clone()],
                payer: Some(Address::repeat_byte(2)),
                frame_receipts: Some(vec![
                    FrameReceipt {
                        status: 1,
                        gas_used: 256,
                        state_gas_used: 512,
                        logs: vec![log.clone()],
                    },
                    FrameReceipt {
                        status: 0,
                        gas_used: 128,
                        state_gas_used: 0,
                        logs: vec![],
                    },
                    FrameReceipt {
                        status: 2,
                        gas_used: 0,
                        state_gas_used: 0,
                        logs: vec![],
                    },
                    FrameReceipt {
                        status: 1,
                        gas_used: 256,
                        state_gas_used: 0,
                        logs: vec![log],
                    },
                ]),
            },
            RpcReceiptTxInfo {
                transaction_hash: H256::repeat_byte(3),
                transaction_index: 2,
                from: Address::repeat_byte(1),
                to: None,
                contract_address: None,
                gas_used: 30_000,
                effective_gas_price: 1,
                blob_gas_price: None,
                blob_gas_used: None,
            },
            RpcReceiptBlockInfo {
                block_hash: H256::repeat_byte(4),
                block_number: 5,
            },
            7,
        );
        let value = serde_json::to_value(receipt).unwrap();
        let frames = &value["frameReceipts"];
        assert_eq!(frames[0]["gasUsed"], "0x300");
        assert_eq!(frames[0]["executionGasUsed"], "0x100");
        assert_eq!(frames[0]["stateGasUsed"], "0x200");
        assert_eq!(
            frames[0]["logs"][0],
            serde_json::json!({
                "address": "0x0101010101010101010101010101010101010101",
                "topics": [], "data": "0x", "logIndex": "0x7", "removed": false,
                "transactionHash": "0x0303030303030303030303030303030303030303030303030303030303030303",
                "transactionIndex": "0x2",
                "blockHash": "0x0404040404040404040404040404040404040404040404040404040404040404",
                "blockNumber": "0x5"
            })
        );
        assert_eq!(frames[0]["logs"][0], value["logs"][0]);
        assert_eq!(frames[3]["logs"][0], value["logs"][1]);
        assert_eq!(frames[3]["logs"][0]["logIndex"], "0x8");
        assert_eq!(
            frames[2],
            serde_json::json!({
                "status": "0x2", "gasUsed": "0x0", "executionGasUsed": "0x0",
                "stateGasUsed": "0x0", "logs": []
            })
        );
        assert_eq!(frames[1]["gasUsed"], "0x80");
        assert_eq!(frames[1]["logs"], serde_json::json!([]));
    }

    // EIP-8141: a frame transaction has no top-level recipient and creates nothing at the top level,
    // so a receipt must name neither. `Transaction::to()` reports the sender for one, which would
    // otherwise be presented as `to` and read by wallets as the account the transaction called.
    #[test]
    fn frame_transaction_receipt_names_neither_to_nor_contract_address() {
        let sender = Address::from(hex!("7435ed30a8b4aeb0877cef0c6e8cffe834eb865f"));
        let tx = Transaction::FrameTransaction(FrameTransaction {
            sender,
            max_fee_per_gas: ethrex_common::U256::one(),
            max_priority_fee_per_gas: ethrex_common::U256::one(),
            ..Default::default()
        });

        let info = RpcReceiptTxInfo::from_transaction(tx, 0, 21_000, 0, Some(0)).unwrap();

        assert_eq!(info.from, sender);
        assert_eq!(info.to, None);
        assert_eq!(info.contract_address, None);
    }
}
