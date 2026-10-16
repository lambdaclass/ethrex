use crate::utils::RpcErr;
use ethrex_common::{
    Address, H256, serde_utils,
    types::{
        BlockHash, BlockNumber, EIP1559Transaction, EIP2930Transaction, EIP7702Transaction,
        EnvelopeTxType, FeeTokenTransaction, FrameTransaction, LegacyTransaction,
        PrivilegedL2Transaction, Transaction, WrappedEIP4844Transaction, WrappedFrameTransaction,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError};
use serde::{Deserialize, Serialize};

#[allow(unused)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcTransaction {
    #[serde(flatten)]
    pub tx: Transaction,
    #[serde(with = "serde_utils::u64::hex_str_opt")]
    block_number: Option<BlockNumber>,
    block_hash: Option<BlockHash>,
    from: Address,
    pub hash: H256,
    #[serde(with = "serde_utils::u64::hex_str_opt")]
    transaction_index: Option<u64>,
}

impl RpcTransaction {
    pub fn build(
        tx: Transaction,
        block_number: Option<BlockNumber>,
        block_hash: Option<BlockHash>,
        transaction_index: Option<usize>,
    ) -> Result<Self, RpcErr> {
        let from = tx.sender(&NativeCrypto)?;
        let hash = tx.hash(&NativeCrypto);
        let transaction_index = transaction_index.map(|n| n as u64);
        Ok(RpcTransaction {
            tx,
            block_number,
            block_hash,
            from,
            hash,
            transaction_index,
        })
    }
}

#[derive(Debug)]
pub enum SendRawTransactionRequest {
    Legacy(LegacyTransaction),
    EIP2930(EIP2930Transaction),
    EIP1559(EIP1559Transaction),
    EIP4844(WrappedEIP4844Transaction),
    EIP7702(EIP7702Transaction),
    PrivilegedL2(PrivilegedL2Transaction),
    FeeToken(FeeTokenTransaction),
    Frame(FrameTransaction),
    /// A blob-carrying frame transaction in its EIP-7594 network wrapper.
    FrameWithBlobs(WrappedFrameTransaction),
}

impl SendRawTransactionRequest {
    pub fn to_transaction(&self) -> Transaction {
        match self {
            SendRawTransactionRequest::Legacy(t) => Transaction::LegacyTransaction(t.clone()),
            SendRawTransactionRequest::EIP1559(t) => Transaction::EIP1559Transaction(t.clone()),
            SendRawTransactionRequest::EIP2930(t) => Transaction::EIP2930Transaction(t.clone()),
            SendRawTransactionRequest::EIP4844(t) => Transaction::EIP4844Transaction(t.tx.clone()),
            SendRawTransactionRequest::EIP7702(t) => Transaction::EIP7702Transaction(t.clone()),
            SendRawTransactionRequest::PrivilegedL2(t) => {
                Transaction::PrivilegedL2Transaction(t.clone())
            }
            SendRawTransactionRequest::FeeToken(t) => Transaction::FeeTokenTransaction(t.clone()),
            SendRawTransactionRequest::Frame(t) => Transaction::FrameTransaction(t.clone()),
            SendRawTransactionRequest::FrameWithBlobs(t) => {
                Transaction::FrameTransaction(t.tx.clone())
            }
        }
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, RLPDecodeError> {
        // Look at the first byte to check if it corresponds to a TransactionType
        match bytes.first() {
            // First byte is a valid TransactionType https://eips.ethereum.org/EIPS/eip-2718#transactiontype-only-goes-up-to-0x7f
            Some(tx_type) if *tx_type <= 0x7f => {
                // Decode tx based on type
                let tx_bytes = &bytes[1..];

                // `from_type_byte` is the single gate for valid envelope types: it rejects 0x00
                // (legacy is the bare-list branch below) and any unassigned type byte.
                match EnvelopeTxType::from_type_byte(*tx_type)? {
                    EnvelopeTxType::EIP2930 => {
                        EIP2930Transaction::decode(tx_bytes).map(SendRawTransactionRequest::EIP2930)
                    }
                    EnvelopeTxType::EIP1559 => {
                        EIP1559Transaction::decode(tx_bytes).map(SendRawTransactionRequest::EIP1559)
                    }
                    EnvelopeTxType::EIP4844 => WrappedEIP4844Transaction::decode(tx_bytes)
                        .map(SendRawTransactionRequest::EIP4844),
                    EnvelopeTxType::EIP7702 => {
                        EIP7702Transaction::decode(tx_bytes).map(SendRawTransactionRequest::EIP7702)
                    }
                    EnvelopeTxType::Frame if WrappedFrameTransaction::is_wrapped(tx_bytes)? => {
                        WrappedFrameTransaction::decode(tx_bytes)
                            .map(SendRawTransactionRequest::FrameWithBlobs)
                    }
                    EnvelopeTxType::Frame => {
                        FrameTransaction::decode(tx_bytes).map(SendRawTransactionRequest::Frame)
                    }
                    EnvelopeTxType::FeeToken => FeeTokenTransaction::decode(tx_bytes)
                        .map(SendRawTransactionRequest::FeeToken),
                    EnvelopeTxType::Privileged => PrivilegedL2Transaction::decode(tx_bytes)
                        .map(SendRawTransactionRequest::PrivilegedL2),
                }
            }
            // LegacyTransaction
            _ => LegacyTransaction::decode(bytes).map(SendRawTransactionRequest::Legacy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::{Bytes, U256, types::Frame};

    // The execution-apis frame transaction schema names the frame fields `target`, `executionGas`
    // and `stateGas`, and carries the sender as `from`, like every other transaction type. A
    // `sender` key would repeat `from` under a name the schema does not define.
    #[test]
    fn frame_transaction_json_follows_the_execution_apis_schema() {
        let sender = Address::repeat_byte(0x11);
        let target = Address::repeat_byte(0x22);
        let tx = Transaction::FrameTransaction(FrameTransaction {
            sender,
            frames: vec![Frame {
                mode: 2,
                flags: 0,
                target: Some(target),
                gas_limit: 0x5208,
                state_gas_limit: 0x100,
                value: U256::one(),
                data: Bytes::from_static(&[0xab]),
            }],
            ..Default::default()
        });

        let json = serde_json::to_value(RpcTransaction::build(tx, None, None, None).unwrap())
            .expect("serialize");

        assert_eq!(json["from"], format!("{sender:#x}"));
        assert!(json.get("sender").is_none(), "the sender is `from` only");
        assert_eq!(
            json["frames"][0],
            serde_json::json!({
                "mode": "0x2",
                "flags": "0x0",
                "target": format!("{target:#x}"),
                "executionGas": "0x5208",
                "stateGas": "0x100",
                "value": "0x1",
                "data": "0xab",
            })
        );
    }
}
