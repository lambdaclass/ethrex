use ethrex_common::{
    Address, Bloom, Bytes, H256,
    constants::GAS_PER_BLOB,
    evm::calculate_create_address,
    serde_utils,
    types::{
        BlockHash, BlockHeader, BlockNumber, FrameReceipt, Log, Receipt, Transaction, TxKind,
        TxType, bloom_from_logs,
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
    /// `executionGasUsed + stateGasUsed`, as the execution-apis frame receipt
    /// schema defines it.
    #[serde(with = "serde_utils::u64::hex_str")]
    pub gas_used: u64,
    /// `gas_used.execution` of the consensus frame receipt.
    #[serde(with = "serde_utils::u64::hex_str")]
    pub execution_gas_used: u64,
    /// `gas_used.state` of the consensus frame receipt: the frame's final
    /// state-gas attribution. Omitting it made the receipt look as if state gas
    /// were charged at transaction level, when the consensus encoding attributes
    /// it per frame.
    #[serde(with = "serde_utils::u64::hex_str")]
    pub state_gas_used: u64,
    /// Full log objects, each with its block-global `logIndex`, as in the
    /// receipt's own `logs`.
    pub logs: Vec<RpcLog>,
}

impl RpcFrameReceipt {
    /// `first_log_index` is the block-global index of this frame's first log.
    fn new(
        frame_receipt: FrameReceipt,
        first_log_index: u64,
        tx_info: &RpcReceiptTxInfo,
        block_info: &RpcReceiptBlockInfo,
        block_timestamp: u64,
    ) -> Self {
        Self {
            status: frame_receipt.status,
            gas_used: frame_receipt
                .gas_used
                .saturating_add(frame_receipt.state_gas_used),
            execution_gas_used: frame_receipt.gas_used,
            state_gas_used: frame_receipt.state_gas_used,
            logs: frame_receipt
                .logs
                .into_iter()
                .zip(first_log_index..)
                .map(|(log, log_index)| {
                    RpcLog::new(log, log_index, tx_info, block_info, block_timestamp)
                })
                .collect(),
        }
    }
}

impl RpcReceipt {
    pub fn new(
        receipt: Receipt,
        tx_info: RpcReceiptTxInfo,
        block_info: RpcReceiptBlockInfo,
        init_log_index: u64,
        block_timestamp: u64,
    ) -> Self {
        let mut logs = vec![];
        let mut log_index = init_log_index;
        for log in receipt.logs.clone() {
            logs.push(RpcLog::new(
                log,
                log_index,
                &tx_info,
                &block_info,
                block_timestamp,
            ));
            log_index += 1;
        }
        let payer = receipt.payer;
        // A frame transaction's logs are its frames' logs in frame order, so each
        // frame's logs continue the index from where the previous frame's ended.
        let frame_receipts = receipt.frame_receipts.clone().map(|frame_receipts| {
            let mut first_log_index = init_log_index;
            frame_receipts
                .into_iter()
                .map(|frame_receipt| {
                    let frame_log_count = frame_receipt.logs.len() as u64;
                    let frame_receipt = RpcFrameReceipt::new(
                        frame_receipt,
                        first_log_index,
                        &tx_info,
                        &block_info,
                        block_timestamp,
                    );
                    first_log_index += frame_log_count;
                    frame_receipt
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
    /// Timestamp of the block this log was emitted in. Optional in the
    /// execution-apis `Log` schema, but every other client populates it, and
    /// indexers that read it off the log would otherwise need a separate block
    /// lookup per receipt. Passed in rather than taken from
    /// `RpcReceiptBlockInfo`, which is flattened into `RpcReceipt` and so must
    /// not gain a serialized field: `blockTimestamp` belongs on the log object
    /// only, not alongside the receipt's own keys.
    ///
    /// Defaulted on the way in: the schema marks it optional, so a peer that
    /// omits it must still decode rather than fail the whole response.
    #[serde(with = "serde_utils::u64::hex_str", default)]
    pub block_timestamp: u64,
}

impl RpcLog {
    pub fn new(
        log: Log,
        log_index: u64,
        tx_info: &RpcReceiptTxInfo,
        block_info: &RpcReceiptBlockInfo,
        block_timestamp: u64,
    ) -> RpcLog {
        Self {
            log: log.into(),
            log_index,
            removed: false,
            transaction_hash: tx_info.transaction_hash,
            transaction_index: tx_info.transaction_index,
            block_hash: block_info.block_hash,
            block_number: block_info.block_number,
            block_timestamp,
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
        // EIP-4844 transactions and EIP-8141 frame transactions that carry blobs.
        let (blob_gas_price, blob_gas_used) = if transaction.is_blob_carrying() {
            (
                Some(block_blob_gas_price),
                Some(transaction.blob_versioned_hashes().len() as u64 * GAS_PER_BLOB as u64),
            )
        } else {
            (None, None)
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
        types::{FrameTransaction, Log, TxType},
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
            1786901200,
        );
        let expected = r#"{"type":"0x3","status":"0x1","cumulativeGasUsed":"0x93","logsBloom":"0x00000000000000000080000000000000000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","logs":[{"address":"0x0000000000000000000000000000000000000000","topics":[],"data":"0x73747261776265727279","logIndex":"0x0","removed":false,"transactionHash":"0x0000000000000000000000000000000000000000000000000000000000000000","transactionIndex":"0x1","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","blockNumber":"0x3","blockTimestamp":"0x6a81f2d0"}],"transactionHash":"0x0000000000000000000000000000000000000000000000000000000000000000","transactionIndex":"0x1","from":"0x0000000000000000000000000000000000000000","to":"0x7435ed30a8b4aeb0877cef0c6e8cffe834eb865f","contractAddress":null,"gasUsed":"0x93","effectiveGasPrice":"0x9d","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","blockNumber":"0x3"}"#;
        assert_eq!(serde_json::to_string(&receipt).unwrap(), expected);
    }

    /// `blockTimestamp` belongs on the log object and nowhere else. Every other
    /// client populates it there, and indexers reading it off a receipt's logs
    /// otherwise need a separate block lookup per receipt. The placement half
    /// matters just as much: `RpcReceiptBlockInfo` is `#[serde(flatten)]`-ed into
    /// `RpcReceipt`, so sourcing the timestamp from it would also emit a
    /// receipt-level `blockTimestamp` that no other client sends.
    #[test]
    fn block_timestamp_is_on_the_log_and_not_on_the_receipt() {
        let receipt = RpcReceipt::new(
            Receipt {
                tx_type: TxType::EIP1559,
                succeeded: true,
                cumulative_gas_used: 21_000,
                logs: vec![Log {
                    address: Address::zero(),
                    topics: vec![],
                    data: Bytes::new(),
                }],
                payer: None,
                frame_receipts: None,
            },
            RpcReceiptTxInfo {
                transaction_hash: H256::zero(),
                transaction_index: 0,
                from: Address::zero(),
                to: None,
                contract_address: None,
                gas_used: 21_000,
                effective_gas_price: 1,
                blob_gas_price: None,
                blob_gas_used: None,
            },
            RpcReceiptBlockInfo {
                block_hash: BlockHash::zero(),
                block_number: 3,
            },
            0,
            1786901200,
        );

        let json = serde_json::to_value(&receipt).expect("serialize");
        let obj = json.as_object().expect("receipt is an object");
        assert!(
            !obj.contains_key("blockTimestamp"),
            "receipt-level keys must stay byte-for-byte what they were"
        );
        let log = json["logs"][0].as_object().expect("log is an object");
        assert_eq!(
            log.get("blockTimestamp").and_then(|v| v.as_str()),
            Some("0x6a81f2d0"),
            "each log must carry the block's timestamp"
        );
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

    // The execution-apis frame receipt: `gasUsed` is execution plus state gas, both parts are
    // reported, and each frame's logs are full log objects whose indexes continue the block-global
    // sequence across frames, as in the receipt's own `logs`.
    #[test]
    fn frame_receipts_follow_the_execution_apis_schema() {
        let log = |byte: u8| Log {
            address: Address::repeat_byte(byte),
            topics: vec![],
            data: Bytes::new(),
        };
        let receipt = RpcReceipt::new(
            Receipt {
                tx_type: TxType::Frame,
                succeeded: true,
                cumulative_gas_used: 100_000,
                logs: vec![log(1), log(2), log(3)],
                payer: Some(Address::repeat_byte(9)),
                frame_receipts: Some(vec![
                    FrameReceipt {
                        status: 1,
                        gas_used: 0x100,
                        state_gas_used: 0,
                        logs: vec![log(1)],
                    },
                    FrameReceipt {
                        status: 1,
                        gas_used: 0x100,
                        state_gas_used: 0x200,
                        logs: vec![],
                    },
                    FrameReceipt {
                        status: 1,
                        gas_used: 0x100,
                        state_gas_used: 0,
                        logs: vec![log(2), log(3)],
                    },
                ]),
            },
            RpcReceiptTxInfo {
                transaction_hash: H256::repeat_byte(7),
                transaction_index: 1,
                from: Address::zero(),
                to: None,
                contract_address: None,
                gas_used: 100_000,
                effective_gas_price: 1,
                blob_gas_price: None,
                blob_gas_used: None,
            },
            RpcReceiptBlockInfo {
                block_hash: BlockHash::repeat_byte(8),
                block_number: 3,
            },
            4,
            1786901200,
        );

        let json = serde_json::to_value(&receipt).expect("serialize");
        let frames = json["frameReceipts"].as_array().expect("frame receipts");
        let second = frames[1].as_object().expect("frame receipt is an object");
        let mut keys: Vec<_> = second.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "executionGasUsed",
                "gasUsed",
                "logs",
                "stateGasUsed",
                "status"
            ]
        );
        assert_eq!(second["gasUsed"], "0x300");
        assert_eq!(second["executionGasUsed"], "0x100");
        assert_eq!(second["stateGasUsed"], "0x200");

        let frame_log_indexes: Vec<_> = frames
            .iter()
            .flat_map(|frame| frame["logs"].as_array().expect("frame logs").clone())
            .map(|log| log["logIndex"].clone())
            .collect();
        let receipt_log_indexes: Vec<_> = json["logs"]
            .as_array()
            .expect("receipt logs")
            .iter()
            .map(|log| log["logIndex"].clone())
            .collect();
        assert_eq!(frame_log_indexes, ["0x4", "0x5", "0x6"]);
        assert_eq!(frame_log_indexes, receipt_log_indexes);
        assert_eq!(frames[2]["logs"][1], json["logs"][2]);
    }

    // A frame transaction that carries blobs pays for blob gas like an EIP-4844 one, so its receipt
    // reports `blobGasUsed` and `blobGasPrice`. One without blobs reports neither.
    #[test]
    fn blob_carrying_frame_transaction_receipt_reports_blob_gas() {
        let frame_tx = |blob_versioned_hashes: Vec<H256>| {
            Transaction::FrameTransaction(FrameTransaction {
                max_fee_per_gas: ethrex_common::U256::one(),
                max_priority_fee_per_gas: ethrex_common::U256::one(),
                max_fee_per_blob_gas: ethrex_common::U256::one(),
                blob_versioned_hashes,
                ..Default::default()
            })
        };

        let with_blobs = RpcReceiptTxInfo::from_transaction(
            frame_tx(vec![H256::repeat_byte(1), H256::repeat_byte(2)]),
            0,
            21_000,
            3,
            Some(0),
        )
        .unwrap();
        assert_eq!(with_blobs.blob_gas_used, Some(2 * GAS_PER_BLOB as u64));
        assert_eq!(with_blobs.blob_gas_price, Some(3));

        let without_blobs =
            RpcReceiptTxInfo::from_transaction(frame_tx(vec![]), 0, 21_000, 3, Some(0)).unwrap();
        assert_eq!(without_blobs.blob_gas_used, None);
        assert_eq!(without_blobs.blob_gas_price, None);
    }
}
