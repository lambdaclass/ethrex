use ethrex_core::{
    serde_utils,
    types::{
        BlockHash, BlockNumber, EIP1559Transaction, EIP2930Transaction, LegacyTransaction,
        PrivilegedL2Transaction, Transaction, WrappedEIP4844Transaction,
    },
    Address, H256,
};
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError};
use serde::{Deserialize, Serialize};

#[allow(unused)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcTransaction {
    #[serde(flatten)]
    pub tx: Transaction,
    #[serde(with = "serde_utils::u64::hex_str")]
    block_number: BlockNumber,
    block_hash: BlockHash,
    from: Address,
    pub hash: H256,
    #[serde(with = "serde_utils::u64::hex_str")]
    transaction_index: u64,
}

impl RpcTransaction {
    pub fn build(
        tx: Transaction,
        block_number: BlockNumber,
        block_hash: BlockHash,
        transaction_index: usize,
    ) -> Self {
        let from = tx.sender();
        let hash = tx.compute_hash();
        let transaction_index = transaction_index as u64;
        RpcTransaction {
            tx,
            block_number,
            block_hash,
            from,
            hash,
            transaction_index,
        }
    }
}

pub enum SendRawTransactionRequest {
    Legacy(LegacyTransaction),
    EIP2930(EIP2930Transaction),
    EIP1559(EIP1559Transaction),
    EIP4844(WrappedEIP4844Transaction),
    PriviligedL2(PrivilegedL2Transaction),
}

impl SendRawTransactionRequest {
    pub fn to_transaction(&self) -> Transaction {
        match self {
            SendRawTransactionRequest::Legacy(t) => Transaction::LegacyTransaction(t.clone()),
            SendRawTransactionRequest::EIP1559(t) => Transaction::EIP1559Transaction(t.clone()),
            SendRawTransactionRequest::EIP2930(t) => Transaction::EIP2930Transaction(t.clone()),
            SendRawTransactionRequest::EIP4844(t) => Transaction::EIP4844Transaction(t.tx.clone()),
            SendRawTransactionRequest::PriviligedL2(t) => {
                Transaction::PrivilegedL2Transaction(t.clone())
            }
        }
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, RLPDecodeError> {
        // Look at the first byte to check if it corresponds to a TransactionType
        match bytes.first() {
            // First byte is a valid TransactionType
            Some(tx_type) if *tx_type < 0x7f => {
                // Decode tx based on type
                let tx_bytes = &bytes[1..];
                match *tx_type {
                    // Legacy
                    0x0 => {
                        LegacyTransaction::decode(tx_bytes).map(SendRawTransactionRequest::Legacy)
                    }
                    // EIP2930
                    0x1 => {
                        EIP2930Transaction::decode(tx_bytes).map(SendRawTransactionRequest::EIP2930)
                    }
                    // EIP1559
                    0x2 => {
                        EIP1559Transaction::decode(tx_bytes).map(SendRawTransactionRequest::EIP1559)
                    }
                    // EIP4844
                    0x3 => WrappedEIP4844Transaction::decode(tx_bytes)
                        .map(SendRawTransactionRequest::EIP4844),
                    0x7e => PrivilegedL2Transaction::decode(tx_bytes)
                        .map(SendRawTransactionRequest::PriviligedL2),
                    ty => Err(RLPDecodeError::Custom(format!(
                        "Invalid transaction type: {ty}"
                    ))),
                }
            }
            // LegacyTransaction
            _ => LegacyTransaction::decode(bytes).map(SendRawTransactionRequest::Legacy),
        }
    }
}
