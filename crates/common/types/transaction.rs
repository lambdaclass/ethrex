use std::cmp::min;

use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
pub use mempool::MempoolTransaction;
use secp256k1::{ecdsa::RecoveryId, Message, SECP256K1};
use serde::{ser::SerializeStruct, Deserialize, Serialize};
pub use serde_impl::{AccessListEntry, GenericTransaction};
use sha3::{Digest, Keccak256};

use ethereum_rust_rlp::{
    constants::RLP_NULL,
    decode::{get_rlp_bytes_item_payload, is_encoded_as_bytes, RLPDecode},
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

// The `#[serde(untagged)]` attribute allows the `Transaction` enum to be serialized without
// a tag indicating the variant type. This means that Serde will serialize the enum's variants
// directly according to the structure of the variant itself.
// For each variant, Serde will use the serialization logic implemented
// for the inner type of that variant (like `LegacyTransaction`, `EIP2930Transaction`, etc.).
// The serialization will fail if the data does not match the structure of any variant.
//
// A custom Deserialization method is implemented to match the specific transaction `type`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Transaction {
    LegacyTransaction(LegacyTransaction),
    EIP2930Transaction(EIP2930Transaction),
    EIP1559Transaction(EIP1559Transaction),
    EIP4844Transaction(EIP4844Transaction),
    PrivilegedL2Transaction(PrivilegedL2Transaction),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LegacyTransaction {
    pub nonce: u64,
    pub gas_price: u64,
    pub gas: u64,
    /// The recipient of the transaction.
    /// Create transactions contain a [`null`](RLP_NULL) value in this field.
    pub to: TxKind,
    pub value: U256,
    pub data: Bytes,
    pub v: U256,
    pub r: U256,
    pub s: U256,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EIP2930Transaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub gas_price: u64,
    pub gas_limit: u64,
    pub to: TxKind,
    pub value: U256,
    pub data: Bytes,
    pub access_list: Vec<(Address, Vec<H256>)>,
    pub signature_y_parity: bool,
    pub signature_r: U256,
    pub signature_s: U256,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EIP1559Transaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    pub to: TxKind,
    pub value: U256,
    pub data: Bytes,
    pub access_list: Vec<(Address, Vec<H256>)>,
    pub signature_y_parity: bool,
    pub signature_r: U256,
    pub signature_s: U256,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EIP4844Transaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas: u64,
    pub to: Address,
    pub value: U256,
    pub data: Bytes,
    pub access_list: Vec<(Address, Vec<H256>)>,
    pub max_fee_per_blob_gas: U256,
    pub blob_versioned_hashes: Vec<H256>,
    pub signature_y_parity: bool,
    pub signature_r: U256,
    pub signature_s: U256,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PrivilegedL2Transaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    pub to: TxKind,
    pub value: U256,
    pub data: Bytes,
    pub access_list: Vec<(Address, Vec<H256>)>,
    pub tx_type: PrivilegedTxType,
    pub signature_y_parity: bool,
    pub signature_r: U256,
    pub signature_s: U256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PrivilegedTxType {
    #[default]
    Deposit = 0x01,
    Withdrawal = 0x02,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TxType {
    #[default]
    Legacy = 0x00,
    EIP2930 = 0x01,
    EIP1559 = 0x02,
    EIP4844 = 0x03,
    // We take the same approach as Optimism to define the privileged tx prefix
    // https://github.com/ethereum-optimism/specs/blob/c6903a3b2cad575653e1f5ef472debb573d83805/specs/protocol/deposits.md#the-deposited-transaction-type
    Privileged = 0x7e,
}

impl Transaction {
    pub fn tx_type(&self) -> TxType {
        match self {
            Transaction::LegacyTransaction(_) => TxType::Legacy,
            Transaction::EIP2930Transaction(_) => TxType::EIP2930,
            Transaction::EIP1559Transaction(_) => TxType::EIP1559,
            Transaction::EIP4844Transaction(_) => TxType::EIP4844,
            Transaction::PrivilegedL2Transaction(_) => TxType::Privileged,
        }
    }
}

impl RLPEncode for Transaction {
    /// Transactions can be encoded in the following formats:
    /// A) Legacy transactions: rlp(LegacyTransaction)
    /// B) Non legacy transactions: rlp(Bytes) where Bytes represents the canonical encoding for the transaction as a bytes object.
    /// Checkout [Transaction::encode_canonical] for more information
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            Transaction::LegacyTransaction(t) => t.encode(buf),
            tx => Bytes::copy_from_slice(&tx.encode_canonical_to_vec()).encode(buf),
        };
    }
}

impl RLPDecode for Transaction {
    /// Transactions can be encoded in the following formats:
    /// A) Legacy transactions: rlp(LegacyTransaction)
    /// B) Non legacy transactions: rlp(Bytes) where Bytes represents the canonical encoding for the transaction as a bytes object.
    /// Checkout [Transaction::decode_canonical] for more information
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        if is_encoded_as_bytes(rlp) {
            // Adjust the encoding to get the payload
            let payload = get_rlp_bytes_item_payload(rlp);
            let tx_type = payload.first().unwrap();
            let tx_encoding = &payload[1..];
            // Look at the first byte to check if it corresponds to a TransactionType
            match *tx_type {
                // Legacy
                0x0 => LegacyTransaction::decode_unfinished(tx_encoding)
                    .map(|(tx, rem)| (Transaction::LegacyTransaction(tx), rem)), // TODO: check if this is a real case scenario
                // EIP2930
                0x1 => EIP2930Transaction::decode_unfinished(tx_encoding)
                    .map(|(tx, rem)| (Transaction::EIP2930Transaction(tx), rem)),
                // EIP1559
                0x2 => EIP1559Transaction::decode_unfinished(tx_encoding)
                    .map(|(tx, rem)| (Transaction::EIP1559Transaction(tx), rem)),
                // EIP4844
                0x3 => EIP4844Transaction::decode_unfinished(tx_encoding)
                    .map(|(tx, rem)| (Transaction::EIP4844Transaction(tx), rem)),
                // PriviligedL2
                0x7e => PrivilegedL2Transaction::decode_unfinished(tx_encoding)
                    .map(|(tx, rem)| (Transaction::PrivilegedL2Transaction(tx), rem)),
                ty => Err(RLPDecodeError::Custom(format!(
                    "Invalid transaction type: {ty}"
                ))),
            }
        } else {
            // LegacyTransaction
            LegacyTransaction::decode_unfinished(rlp)
                .map(|(tx, rem)| (Transaction::LegacyTransaction(tx), rem))
        }
    }
}

/// The transaction's kind: call or create.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum TxKind {
    Call(Address),
    #[default]
    Create,
}

impl RLPEncode for TxKind {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            Self::Call(address) => address.encode(buf),
            Self::Create => buf.put_u8(RLP_NULL),
        }
    }
}

impl RLPDecode for TxKind {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let first_byte = rlp.first().ok_or(RLPDecodeError::InvalidLength)?;
        if *first_byte == RLP_NULL {
            return Ok((Self::Create, &rlp[1..]));
        }
        Address::decode_unfinished(rlp).map(|(t, rest)| (Self::Call(t), rest))
    }
}

impl RLPEncode for PrivilegedTxType {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            Self::Deposit => buf.put_u8(0x01),
            Self::Withdrawal => buf.put_u8(0x02),
        }
    }
}

impl RLPDecode for PrivilegedTxType {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoded = u8::decode_unfinished(rlp)?;
        let tx_type = PrivilegedTxType::from_u8(decoded.0)
            .ok_or(RLPDecodeError::Custom("Invalid".to_string()))?;
        Ok((tx_type, decoded.1))
    }
}

impl RLPEncode for LegacyTransaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.nonce)
            .encode_field(&self.gas_price)
            .encode_field(&self.gas)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.v)
            .encode_field(&self.r)
            .encode_field(&self.s)
            .finish();
    }
}

impl RLPEncode for EIP2930Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.nonce)
            .encode_field(&self.gas_price)
            .encode_field(&self.gas_limit)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.access_list)
            .encode_field(&self.signature_y_parity)
            .encode_field(&self.signature_r)
            .encode_field(&self.signature_s)
            .finish()
    }
}

impl RLPEncode for EIP1559Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.nonce)
            .encode_field(&self.max_priority_fee_per_gas)
            .encode_field(&self.max_fee_per_gas)
            .encode_field(&self.gas_limit)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.access_list)
            .encode_field(&self.signature_y_parity)
            .encode_field(&self.signature_r)
            .encode_field(&self.signature_s)
            .finish()
    }
}

impl RLPEncode for EIP4844Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.nonce)
            .encode_field(&self.max_priority_fee_per_gas)
            .encode_field(&self.max_fee_per_gas)
            .encode_field(&self.gas)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.access_list)
            .encode_field(&self.max_fee_per_blob_gas)
            .encode_field(&self.blob_versioned_hashes)
            .encode_field(&self.signature_y_parity)
            .encode_field(&self.signature_r)
            .encode_field(&self.signature_s)
            .finish()
    }
}

impl RLPEncode for PrivilegedL2Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.nonce)
            .encode_field(&self.max_priority_fee_per_gas)
            .encode_field(&self.max_fee_per_gas)
            .encode_field(&self.gas_limit)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.access_list)
            .encode_field(&self.tx_type)
            .encode_field(&self.signature_y_parity)
            .encode_field(&self.signature_r)
            .encode_field(&self.signature_s)
            .finish()
    }
}

impl RLPDecode for LegacyTransaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(LegacyTransaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (gas_price, decoder) = decoder.decode_field("gas_price")?;
        let (gas, decoder) = decoder.decode_field("gas")?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let (v, decoder) = decoder.decode_field("v")?;
        let (r, decoder) = decoder.decode_field("r")?;
        let (s, decoder) = decoder.decode_field("s")?;

        let tx = LegacyTransaction {
            nonce,
            gas_price,
            gas,
            to,
            value,
            data,
            v,
            r,
            s,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for EIP2930Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(EIP2930Transaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (gas_price, decoder) = decoder.decode_field("gas_price")?;
        let (gas_limit, decoder) = decoder.decode_field("gas_limit")?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let (access_list, decoder) = decoder.decode_field("access_list")?;
        let (signature_y_parity, decoder) = decoder.decode_field("signature_y_parity")?;
        let (signature_r, decoder) = decoder.decode_field("signature_r")?;
        let (signature_s, decoder) = decoder.decode_field("signature_s")?;

        let tx = EIP2930Transaction {
            chain_id,
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            data,
            access_list,
            signature_y_parity,
            signature_r,
            signature_s,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for EIP1559Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(EIP1559Transaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (max_priority_fee_per_gas, decoder) =
            decoder.decode_field("max_priority_fee_per_gas")?;
        let (max_fee_per_gas, decoder) = decoder.decode_field("max_fee_per_gas")?;
        let (gas_limit, decoder) = decoder.decode_field("gas_limit")?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let (access_list, decoder) = decoder.decode_field("access_list")?;
        let (signature_y_parity, decoder) = decoder.decode_field("signature_y_parity")?;
        let (signature_r, decoder) = decoder.decode_field("signature_r")?;
        let (signature_s, decoder) = decoder.decode_field("signature_s")?;

        let tx = EIP1559Transaction {
            chain_id,
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            to,
            value,
            data,
            access_list,
            signature_y_parity,
            signature_r,
            signature_s,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for EIP4844Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(EIP4844Transaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (max_priority_fee_per_gas, decoder) =
            decoder.decode_field("max_priority_fee_per_gas")?;
        let (max_fee_per_gas, decoder) = decoder.decode_field("max_fee_per_gas")?;
        let (gas, decoder) = decoder.decode_field("gas")?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let (access_list, decoder) = decoder.decode_field("access_list")?;
        let (max_fee_per_blob_gas, decoder) = decoder.decode_field("max_fee_per_blob_gas")?;
        let (blob_versioned_hashes, decoder) = decoder.decode_field("blob_versioned_hashes")?;
        let (signature_y_parity, decoder) = decoder.decode_field("signature_y_parity")?;
        let (signature_r, decoder) = decoder.decode_field("signature_r")?;
        let (signature_s, decoder) = decoder.decode_field("signature_s")?;

        let tx = EIP4844Transaction {
            chain_id,
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas,
            to,
            value,
            data,
            access_list,
            max_fee_per_blob_gas,
            blob_versioned_hashes,
            signature_y_parity,
            signature_r,
            signature_s,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for PrivilegedL2Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(PrivilegedL2Transaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (nonce, decoder) = decoder.decode_field("nonce")?;
        let (max_priority_fee_per_gas, decoder) =
            decoder.decode_field("max_priority_fee_per_gas")?;
        let (max_fee_per_gas, decoder) = decoder.decode_field("max_fee_per_gas")?;
        let (gas_limit, decoder) = decoder.decode_field("gas_limit")?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let (access_list, decoder) = decoder.decode_field("access_list")?;
        let (tx_type, decoder) = decoder.decode_field("tx_type")?;
        let (signature_y_parity, decoder) = decoder.decode_field("signature_y_parity")?;
        let (signature_r, decoder) = decoder.decode_field("signature_r")?;
        let (signature_s, decoder) = decoder.decode_field("signature_s")?;

        let tx = PrivilegedL2Transaction {
            chain_id,
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            to,
            value,
            data,
            access_list,
            tx_type,
            signature_y_parity,
            signature_r,
            signature_s,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl Transaction {
    pub fn sender(&self) -> Address {
        match self {
            Transaction::LegacyTransaction(tx) => {
                let signature_y_parity = match self.chain_id() {
                    Some(chain_id) => tx.v.as_u64().saturating_sub(35 + chain_id * 2) != 0,
                    None => tx.v.as_u64().saturating_sub(27) != 0,
                };
                let mut buf = vec![];
                match self.chain_id() {
                    None => Encoder::new(&mut buf)
                        .encode_field(&tx.nonce)
                        .encode_field(&tx.gas_price)
                        .encode_field(&tx.gas)
                        .encode_field(&tx.to)
                        .encode_field(&tx.value)
                        .encode_field(&tx.data)
                        .finish(),
                    Some(chain_id) => Encoder::new(&mut buf)
                        .encode_field(&tx.nonce)
                        .encode_field(&tx.gas_price)
                        .encode_field(&tx.gas)
                        .encode_field(&tx.to)
                        .encode_field(&tx.value)
                        .encode_field(&tx.data)
                        .encode_field(&chain_id)
                        .encode_field(&0u8)
                        .encode_field(&0u8)
                        .finish(),
                }
                recover_address(&tx.r, &tx.s, signature_y_parity, &Bytes::from(buf))
            }
            Transaction::EIP2930Transaction(tx) => {
                let mut buf = vec![self.tx_type() as u8];
                Encoder::new(&mut buf)
                    .encode_field(&tx.chain_id)
                    .encode_field(&tx.nonce)
                    .encode_field(&tx.gas_price)
                    .encode_field(&tx.gas_limit)
                    .encode_field(&tx.to)
                    .encode_field(&tx.value)
                    .encode_field(&tx.data)
                    .encode_field(&tx.access_list)
                    .finish();
                recover_address(
                    &tx.signature_r,
                    &tx.signature_s,
                    tx.signature_y_parity,
                    &Bytes::from(buf),
                )
            }
            Transaction::EIP1559Transaction(tx) => {
                let mut buf = vec![self.tx_type() as u8];
                Encoder::new(&mut buf)
                    .encode_field(&tx.chain_id)
                    .encode_field(&tx.nonce)
                    .encode_field(&tx.max_priority_fee_per_gas)
                    .encode_field(&tx.max_fee_per_gas)
                    .encode_field(&tx.gas_limit)
                    .encode_field(&tx.to)
                    .encode_field(&tx.value)
                    .encode_field(&tx.data)
                    .encode_field(&tx.access_list)
                    .finish();
                recover_address(
                    &tx.signature_r,
                    &tx.signature_s,
                    tx.signature_y_parity,
                    &Bytes::from(buf),
                )
            }
            Transaction::EIP4844Transaction(tx) => {
                let mut buf = vec![self.tx_type() as u8];
                Encoder::new(&mut buf)
                    .encode_field(&tx.chain_id)
                    .encode_field(&tx.nonce)
                    .encode_field(&tx.max_priority_fee_per_gas)
                    .encode_field(&tx.max_fee_per_gas)
                    .encode_field(&tx.gas)
                    .encode_field(&tx.to)
                    .encode_field(&tx.value)
                    .encode_field(&tx.data)
                    .encode_field(&tx.access_list)
                    .encode_field(&tx.max_fee_per_blob_gas)
                    .encode_field(&tx.blob_versioned_hashes)
                    .finish();
                recover_address(
                    &tx.signature_r,
                    &tx.signature_s,
                    tx.signature_y_parity,
                    &Bytes::from(buf),
                )
            }
            Transaction::PrivilegedL2Transaction(tx) => {
                let mut buf = vec![self.tx_type() as u8];
                Encoder::new(&mut buf)
                    .encode_field(&tx.chain_id)
                    .encode_field(&tx.nonce)
                    .encode_field(&tx.max_priority_fee_per_gas)
                    .encode_field(&tx.max_fee_per_gas)
                    .encode_field(&tx.gas_limit)
                    .encode_field(&tx.to)
                    .encode_field(&tx.value)
                    .encode_field(&tx.data)
                    .encode_field(&tx.access_list)
                    .encode_field(&tx.tx_type)
                    .finish();
                recover_address(
                    &tx.signature_r,
                    &tx.signature_s,
                    tx.signature_y_parity,
                    &Bytes::from(buf),
                )
            }
        }
    }

    pub fn gas_limit(&self) -> u64 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.gas,
            Transaction::EIP2930Transaction(tx) => tx.gas_limit,
            Transaction::EIP1559Transaction(tx) => tx.gas_limit,
            Transaction::EIP4844Transaction(tx) => tx.gas,
            Transaction::PrivilegedL2Transaction(tx) => tx.gas_limit,
        }
    }

    pub fn gas_price(&self) -> u64 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.gas_price,
            Transaction::EIP2930Transaction(tx) => tx.gas_price,
            Transaction::EIP1559Transaction(tx) => tx.max_fee_per_gas,
            Transaction::EIP4844Transaction(tx) => tx.max_fee_per_gas,
            Transaction::PrivilegedL2Transaction(tx) => tx.max_fee_per_gas,
        }
    }

    pub fn to(&self) -> TxKind {
        match self {
            Transaction::LegacyTransaction(tx) => tx.to.clone(),
            Transaction::EIP2930Transaction(tx) => tx.to.clone(),
            Transaction::EIP1559Transaction(tx) => tx.to.clone(),
            Transaction::EIP4844Transaction(tx) => TxKind::Call(tx.to),
            Transaction::PrivilegedL2Transaction(tx) => tx.to.clone(),
        }
    }

    pub fn value(&self) -> U256 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.value,
            Transaction::EIP2930Transaction(tx) => tx.value,
            Transaction::EIP1559Transaction(tx) => tx.value,
            Transaction::EIP4844Transaction(tx) => tx.value,
            Transaction::PrivilegedL2Transaction(tx) => tx.value,
        }
    }

    pub fn max_priority_fee(&self) -> Option<u64> {
        match self {
            Transaction::LegacyTransaction(_tx) => None,
            Transaction::EIP2930Transaction(_tx) => None,
            Transaction::EIP1559Transaction(tx) => Some(tx.max_priority_fee_per_gas),
            Transaction::EIP4844Transaction(tx) => Some(tx.max_priority_fee_per_gas),
            Transaction::PrivilegedL2Transaction(tx) => Some(tx.max_priority_fee_per_gas),
        }
    }

    pub fn chain_id(&self) -> Option<u64> {
        match self {
            Transaction::LegacyTransaction(tx) => derive_legacy_chain_id(tx.v),
            Transaction::EIP2930Transaction(tx) => Some(tx.chain_id),
            Transaction::EIP1559Transaction(tx) => Some(tx.chain_id),
            Transaction::EIP4844Transaction(tx) => Some(tx.chain_id),
            Transaction::PrivilegedL2Transaction(tx) => Some(tx.chain_id),
        }
    }

    pub fn access_list(&self) -> Vec<(Address, Vec<H256>)> {
        match self {
            Transaction::LegacyTransaction(_tx) => Vec::new(),
            Transaction::EIP2930Transaction(tx) => tx.access_list.clone(),
            Transaction::EIP1559Transaction(tx) => tx.access_list.clone(),
            Transaction::EIP4844Transaction(tx) => tx.access_list.clone(),
            Transaction::PrivilegedL2Transaction(tx) => tx.access_list.clone(),
        }
    }

    pub fn nonce(&self) -> u64 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.nonce,
            Transaction::EIP2930Transaction(tx) => tx.nonce,
            Transaction::EIP1559Transaction(tx) => tx.nonce,
            Transaction::EIP4844Transaction(tx) => tx.nonce,
            Transaction::PrivilegedL2Transaction(tx) => tx.nonce,
        }
    }

    pub fn data(&self) -> &Bytes {
        match self {
            Transaction::LegacyTransaction(tx) => &tx.data,
            Transaction::EIP2930Transaction(tx) => &tx.data,
            Transaction::EIP1559Transaction(tx) => &tx.data,
            Transaction::EIP4844Transaction(tx) => &tx.data,
            Transaction::PrivilegedL2Transaction(tx) => &tx.data,
        }
    }

    pub fn blob_versioned_hashes(&self) -> Vec<H256> {
        match self {
            Transaction::LegacyTransaction(_tx) => Vec::new(),
            Transaction::EIP2930Transaction(_tx) => Vec::new(),
            Transaction::EIP1559Transaction(_tx) => Vec::new(),
            Transaction::EIP4844Transaction(tx) => tx.blob_versioned_hashes.clone(),
            Transaction::PrivilegedL2Transaction(_tx) => Vec::new(),
        }
    }

    pub fn max_fee_per_blob_gas(&self) -> Option<U256> {
        match self {
            Transaction::LegacyTransaction(_tx) => None,
            Transaction::EIP2930Transaction(_tx) => None,
            Transaction::EIP1559Transaction(_tx) => None,
            Transaction::EIP4844Transaction(tx) => Some(tx.max_fee_per_blob_gas),
            Transaction::PrivilegedL2Transaction(_tx) => None,
        }
    }

    pub fn is_contract_creation(&self) -> bool {
        match &self {
            Transaction::LegacyTransaction(t) => matches!(t.to, TxKind::Create),
            Transaction::EIP2930Transaction(t) => matches!(t.to, TxKind::Create),
            Transaction::EIP1559Transaction(t) => matches!(t.to, TxKind::Create),
            Transaction::EIP4844Transaction(_) => false,
            Transaction::PrivilegedL2Transaction(t) => matches!(t.to, TxKind::Create),
        }
    }

    pub fn max_fee_per_gas(&self) -> Option<u64> {
        match self {
            Transaction::LegacyTransaction(_tx) => None,
            Transaction::EIP2930Transaction(_tx) => None,
            Transaction::EIP1559Transaction(tx) => Some(tx.max_fee_per_gas),
            Transaction::EIP4844Transaction(tx) => Some(tx.max_fee_per_gas),
            Transaction::PrivilegedL2Transaction(tx) => Some(tx.max_fee_per_gas),
        }
    }

    pub fn compute_hash(&self) -> H256 {
        keccak_hash::keccak(self.encode_canonical_to_vec())
    }

    pub fn gas_tip_cap(&self) -> u64 {
        self.max_priority_fee().unwrap_or(self.gas_price())
    }

    pub fn gas_fee_cap(&self) -> u64 {
        self.max_fee_per_gas().unwrap_or(self.gas_price())
    }

    pub fn effective_gas_tip(&self, base_fee: Option<u64>) -> Option<u64> {
        let Some(base_fee) = base_fee else {
            return Some(self.gas_tip_cap());
        };
        self.gas_fee_cap()
            .checked_sub(base_fee)
            .map(|tip| min(tip, self.gas_tip_cap()))
    }

    /// Returns whether the transaction is replay-protected.
    /// For more information check out [EIP-155](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md)
    pub fn protected(&self) -> bool {
        match self {
            Transaction::LegacyTransaction(tx) if tx.v.bits() <= 8 => {
                let v = tx.v.as_u64();
                v != 27 && v != 28 && v != 1 && v != 0
            }
            _ => true,
        }
    }
}

fn recover_address(
    signature_r: &U256,
    signature_s: &U256,
    signature_y_parity: bool,
    message: &Bytes,
) -> Address {
    // Create signature
    let mut signature_bytes = [0; 64];
    signature_r.to_big_endian(&mut signature_bytes[0..32]);
    signature_s.to_big_endian(&mut signature_bytes[32..]);
    let signature = secp256k1::ecdsa::RecoverableSignature::from_compact(
        &signature_bytes,
        RecoveryId::from_i32(signature_y_parity as i32).unwrap(), // cannot fail
    )
    .unwrap();
    // Hash message
    let msg_digest: [u8; 32] = Keccak256::new_with_prefix(message.as_ref())
        .finalize()
        .into();
    // Recover public key
    let public = SECP256K1
        .recover_ecdsa(&Message::from_digest(msg_digest), &signature)
        .unwrap();
    // Hash public key to obtain address
    let hash = Keccak256::new_with_prefix(&public.serialize_uncompressed()[1..]).finalize();
    Address::from_slice(&hash[12..])
}

fn derive_legacy_chain_id(v: U256) -> Option<u64> {
    let v = v.as_u64(); //TODO: Could panic if v is bigger than Max u64
    if v == 27 || v == 28 {
        None
    } else {
        Some((v - 35) / 2)
    }
}

impl TxType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::Legacy),
            0x01 => Some(Self::EIP2930),
            0x02 => Some(Self::EIP1559),
            0x03 => Some(Self::EIP4844),
            0x7e => Some(Self::Privileged),
            _ => None,
        }
    }
}

impl PrivilegedTxType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::Deposit),
            0x02 => Some(Self::Withdrawal),
            _ => None,
        }
    }
}

impl PrivilegedL2Transaction {
    pub fn get_withdrawal_hash(&self) -> Option<H256> {
        match self.tx_type {
            PrivilegedTxType::Withdrawal => {
                let to = match self.to {
                    TxKind::Call(to) => to,
                    _ => return None,
                };

                let value = &mut [0u8; 32];
                self.value.to_big_endian(value);

                let mut encoded = self.encode_to_vec();
                encoded.insert(0, TxType::Privileged as u8);
                let tx_hash = keccak_hash::keccak(encoded);
                Some(keccak_hash::keccak(
                    [to.as_bytes(), value, tx_hash.as_bytes()].concat(),
                ))
            }
            _ => None,
        }
    }
}

/// Canonical Transaction Encoding
/// Based on [EIP-2718]
/// Transactions can be encoded in the following formats:
/// A) `TransactionType || Transaction` (Where Transaction type is an 8-bit number between 0 and 0x7f, and Transaction is an rlp encoded transaction of type TransactionType)
/// B) `LegacyTransaction` (An rlp encoded LegacyTransaction)
mod canonic_encoding {
    use super::*;

    impl Transaction {
        /// Decodes a single transaction in canonical format
        /// Based on [EIP-2718]
        /// Transactions can be encoded in the following formats:
        /// A) `TransactionType || Transaction` (Where Transaction type is an 8-bit number between 0 and 0x7f, and Transaction is an rlp encoded transaction of type TransactionType)
        /// B) `LegacyTransaction` (An rlp encoded LegacyTransaction)
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
                            LegacyTransaction::decode(tx_bytes).map(Transaction::LegacyTransaction)
                        } // TODO: check if this is a real case scenario
                        // EIP2930
                        0x1 => EIP2930Transaction::decode(tx_bytes)
                            .map(Transaction::EIP2930Transaction),
                        // EIP1559
                        0x2 => EIP1559Transaction::decode(tx_bytes)
                            .map(Transaction::EIP1559Transaction),
                        // EIP4844
                        0x3 => EIP4844Transaction::decode(tx_bytes)
                            .map(Transaction::EIP4844Transaction),
                        0x7e => PrivilegedL2Transaction::decode(tx_bytes)
                            .map(Transaction::PrivilegedL2Transaction),
                        ty => Err(RLPDecodeError::Custom(format!(
                            "Invalid transaction type: {ty}"
                        ))),
                    }
                }
                // LegacyTransaction
                _ => LegacyTransaction::decode(bytes).map(Transaction::LegacyTransaction),
            }
        }

        /// Encodes a transaction in canonical format
        /// Based on [EIP-2718]
        /// Transactions can be encoded in the following formats:
        /// A) `TransactionType || Transaction` (Where Transaction type is an 8-bit number between 0 and 0x7f, and Transaction is an rlp encoded transaction of type TransactionType)
        /// B) `LegacyTransaction` (An rlp encoded LegacyTransaction)
        pub fn encode_canonical(&self, buf: &mut dyn bytes::BufMut) {
            match self {
                // Legacy transactions don't have a prefix
                Transaction::LegacyTransaction(_) => {}
                _ => buf.put_u8(self.tx_type() as u8),
            }
            match self {
                Transaction::LegacyTransaction(t) => t.encode(buf),
                Transaction::EIP2930Transaction(t) => t.encode(buf),
                Transaction::EIP1559Transaction(t) => t.encode(buf),
                Transaction::EIP4844Transaction(t) => t.encode(buf),
                Transaction::PrivilegedL2Transaction(t) => t.encode(buf),
            };
        }

        /// Encodes a transaction in canonical format into a newly created buffer
        /// Based on [EIP-2718]
        /// Transactions can be encoded in the following formats:
        /// A) `TransactionType || Transaction` (Where Transaction type is an 8-bit number between 0 and 0x7f, and Transaction is an rlp encoded transaction of type TransactionType)
        /// B) `LegacyTransaction` (An rlp encoded LegacyTransaction)
        pub fn encode_canonical_to_vec(&self) -> Vec<u8> {
            let mut buf = Vec::new();
            self.encode_canonical(&mut buf);
            buf
        }
    }
}

// Serialization
// This is used for RPC messaging and passing data into a RISC-V zkVM

mod serde_impl {
    use serde::Deserialize;
    use serde_json::Value;
    use std::{collections::HashMap, str::FromStr};

    use super::*;

    impl Serialize for TxKind {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            match self {
                TxKind::Call(address) => serializer.serialize_str(&format!("{:#x}", address)),
                TxKind::Create => serializer.serialize_none(),
            }
        }
    }

    impl<'de> Deserialize<'de> for TxKind {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let str_option = Option::<String>::deserialize(deserializer)?;
            match str_option {
                Some(str) if !str.is_empty() => Ok(TxKind::Call(
                    Address::from_str(str.trim_start_matches("0x")).map_err(|_| {
                        serde::de::Error::custom(format!("Failed to deserialize hex value {str}"))
                    })?,
                )),
                _ => Ok(TxKind::Create),
            }
        }
    }

    impl Serialize for TxType {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            serializer.serialize_str(&format!("{:#x}", *self as u8))
        }
    }

    impl<'de> Deserialize<'de> for TxType {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let str = String::deserialize(deserializer)?;
            let tx_num = u8::from_str_radix(str.trim_start_matches("0x"), 16).map_err(|_| {
                serde::de::Error::custom(format!("Failed to deserialize hex value {str}"))
            })?;
            TxType::from_u8(tx_num).ok_or_else(|| {
                serde::de::Error::custom(format!("Invalid transaction type {tx_num}"))
            })
        }
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct AccessListEntry {
        pub address: Address,
        pub storage_keys: Vec<H256>,
    }

    impl From<&(Address, Vec<H256>)> for AccessListEntry {
        fn from(value: &(Address, Vec<H256>)) -> AccessListEntry {
            AccessListEntry {
                address: value.0,
                storage_keys: value.1.clone(),
            }
        }
    }

    impl Serialize for LegacyTransaction {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut struct_serializer = serializer.serialize_struct("LegacyTransaction", 11)?;
            struct_serializer.serialize_field("type", &TxType::Legacy)?;
            struct_serializer.serialize_field("nonce", &format!("{:#x}", self.nonce))?;
            struct_serializer.serialize_field("to", &self.to)?;
            struct_serializer.serialize_field("gas", &format!("{:#x}", self.gas))?;
            struct_serializer.serialize_field("value", &self.value)?;
            struct_serializer.serialize_field("input", &format!("0x{:x}", self.data))?;
            struct_serializer.serialize_field("gasPrice", &format!("{:#x}", self.gas_price))?;
            struct_serializer.serialize_field(
                "chainId",
                &format!("{:#x}", derive_legacy_chain_id(self.v).unwrap_or_default()),
            )?;
            struct_serializer.serialize_field("v", &self.v)?;
            struct_serializer.serialize_field("r", &self.r)?;
            struct_serializer.serialize_field("s", &self.s)?;
            struct_serializer.end()
        }
    }

    impl Serialize for EIP2930Transaction {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut struct_serializer = serializer.serialize_struct("Eip2930Transaction", 12)?;
            struct_serializer.serialize_field("type", &TxType::EIP2930)?;
            struct_serializer.serialize_field("nonce", &format!("{:#x}", self.nonce))?;
            struct_serializer.serialize_field("to", &self.to)?;
            struct_serializer.serialize_field("gas", &format!("{:#x}", self.gas_limit))?;
            struct_serializer.serialize_field("value", &self.value)?;
            struct_serializer.serialize_field("input", &format!("0x{:x}", self.data))?;
            struct_serializer.serialize_field("gasPrice", &format!("{:#x}", self.gas_price))?;
            struct_serializer.serialize_field(
                "accessList",
                &self
                    .access_list
                    .iter()
                    .map(AccessListEntry::from)
                    .collect::<Vec<_>>(),
            )?;
            struct_serializer.serialize_field("chainId", &format!("{:#x}", self.chain_id))?;
            struct_serializer
                .serialize_field("yParity", &format!("{:#x}", self.signature_y_parity as u8))?;
            struct_serializer
                .serialize_field("v", &format!("{:#x}", self.signature_y_parity as u8))?; // added to match Hive tests
            struct_serializer.serialize_field("r", &self.signature_r)?;
            struct_serializer.serialize_field("s", &self.signature_s)?;
            struct_serializer.end()
        }
    }

    impl Serialize for EIP1559Transaction {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut struct_serializer = serializer.serialize_struct("Eip1559Transaction", 14)?;
            struct_serializer.serialize_field("type", &TxType::EIP1559)?;
            struct_serializer.serialize_field("nonce", &format!("{:#x}", self.nonce))?;
            struct_serializer.serialize_field("to", &self.to)?;
            struct_serializer.serialize_field("gas", &format!("{:#x}", self.gas_limit))?;
            struct_serializer.serialize_field("value", &self.value)?;
            struct_serializer.serialize_field("input", &format!("0x{:x}", self.data))?;
            struct_serializer.serialize_field(
                "maxPriorityFeePerGas",
                &format!("{:#x}", self.max_priority_fee_per_gas),
            )?;
            struct_serializer
                .serialize_field("maxFeePerGas", &format!("{:#x}", self.max_fee_per_gas))?;
            struct_serializer
                .serialize_field("gasPrice", &format!("{:#x}", self.max_fee_per_gas))?;
            struct_serializer.serialize_field(
                "accessList",
                &self
                    .access_list
                    .iter()
                    .map(AccessListEntry::from)
                    .collect::<Vec<_>>(),
            )?;
            struct_serializer.serialize_field("chainId", &format!("{:#x}", self.chain_id))?;
            struct_serializer
                .serialize_field("yParity", &format!("{:#x}", self.signature_y_parity as u8))?;
            struct_serializer
                .serialize_field("v", &format!("{:#x}", self.signature_y_parity as u8))?; // added to match Hive tests
            struct_serializer.serialize_field("r", &self.signature_r)?;
            struct_serializer.serialize_field("s", &self.signature_s)?;
            struct_serializer.end()
        }
    }

    impl Serialize for EIP4844Transaction {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut struct_serializer = serializer.serialize_struct("Eip4844Transaction", 15)?;
            struct_serializer.serialize_field("type", &TxType::EIP4844)?;
            struct_serializer.serialize_field("nonce", &format!("{:#x}", self.nonce))?;
            struct_serializer.serialize_field("to", &self.to)?;
            struct_serializer.serialize_field("gas", &format!("{:#x}", self.gas))?;
            struct_serializer.serialize_field("value", &self.value)?;
            struct_serializer.serialize_field("input", &format!("0x{:x}", self.data))?;
            struct_serializer.serialize_field(
                "maxPriorityFeePerGas",
                &format!("{:#x}", self.max_priority_fee_per_gas),
            )?;
            struct_serializer
                .serialize_field("maxFeePerGas", &format!("{:#x}", self.max_fee_per_gas))?;
            struct_serializer
                .serialize_field("gasPrice", &format!("{:#x}", self.max_fee_per_gas))?;
            struct_serializer.serialize_field(
                "maxFeePerBlobGas",
                &format!("{:#x}", self.max_fee_per_blob_gas),
            )?;
            struct_serializer.serialize_field(
                "accessList",
                &self
                    .access_list
                    .iter()
                    .map(AccessListEntry::from)
                    .collect::<Vec<_>>(),
            )?;
            struct_serializer
                .serialize_field("blobVersionedHashes", &self.blob_versioned_hashes)?;
            struct_serializer.serialize_field("chainId", &format!("{:#x}", self.chain_id))?;
            struct_serializer
                .serialize_field("yParity", &format!("{:#x}", self.signature_y_parity as u8))?;
            struct_serializer
                .serialize_field("v", &format!("{:#x}", self.signature_y_parity as u8))?; // added to match Hive tests
            struct_serializer.serialize_field("r", &self.signature_r)?;
            struct_serializer.serialize_field("s", &self.signature_s)?;
            struct_serializer.end()
        }
    }

    impl<'de> Deserialize<'de> for Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;
            let tx_type =
                serde_json::from_value::<TxType>(map.remove("type").ok_or_else(|| {
                    serde::de::Error::custom("Couldn't Deserialize the 'type' field".to_string())
                })?)
                .map_err(serde::de::Error::custom)?;

            let iter = map.into_iter();
            match tx_type {
                TxType::Legacy => {
                    LegacyTransaction::deserialize(serde::de::value::MapDeserializer::new(iter))
                        .map(Transaction::LegacyTransaction)
                        .map_err(|e| {
                            serde::de::Error::custom(format!("Couldn't Deserialize Legacy {e}"))
                        })
                }
                TxType::EIP2930 => {
                    EIP2930Transaction::deserialize(serde::de::value::MapDeserializer::new(iter))
                        .map(Transaction::EIP2930Transaction)
                        .map_err(|e| {
                            serde::de::Error::custom(format!("Couldn't Deserialize EIP2930 {e}"))
                        })
                }
                TxType::EIP1559 => {
                    EIP1559Transaction::deserialize(serde::de::value::MapDeserializer::new(iter))
                        .map(Transaction::EIP1559Transaction)
                        .map_err(|e| {
                            serde::de::Error::custom(format!("Couldn't Deserialize EIP1559 {e}"))
                        })
                }
                TxType::EIP4844 => {
                    EIP4844Transaction::deserialize(serde::de::value::MapDeserializer::new(iter))
                        .map(Transaction::EIP4844Transaction)
                        .map_err(|e| {
                            serde::de::Error::custom(format!("Couldn't Deserialize EIP4844 {e}"))
                        })
                }
                TxType::Privileged => PrivilegedL2Transaction::deserialize(
                    serde::de::value::MapDeserializer::new(iter),
                )
                .map(Transaction::PrivilegedL2Transaction)
                .map_err(|e| serde::de::Error::custom(format!("Couldn't Deserialize Legacy {e}"))),
            }
        }
    }

    fn deserialize_input_field(
        map: &mut std::collections::HashMap<String, Value>,
    ) -> Result<Bytes, serde_json::Error> {
        let data_str: String = serde_json::from_value(
            map.remove("input")
                .ok_or_else(|| serde::de::Error::missing_field("input"))?,
        )
        .map_err(serde::de::Error::custom)?;
        if let Some(stripped) = data_str.strip_prefix("0x") {
            match hex::decode(stripped) {
                Ok(decoded_bytes) => Ok(Bytes::from(decoded_bytes)),
                Err(_) => Err(serde::de::Error::custom(
                    "Invalid hex format in 'input' field",
                ))?,
            }
        } else {
            Err(serde::de::Error::custom(
                "'input' field must start with '0x'",
            ))?
        }
    }

    impl<'de> Deserialize<'de> for LegacyTransaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;
            let nonce = serde_json::from_value::<U256>(
                map.remove("nonce")
                    .ok_or_else(|| serde::de::Error::missing_field("nonce"))?,
            )
            .map_err(serde::de::Error::custom)?
            .as_u64();
            let to = serde_json::from_value(
                map.remove("to")
                    .ok_or_else(|| serde::de::Error::missing_field("to"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let value = serde_json::from_value(
                map.remove("value")
                    .ok_or_else(|| serde::de::Error::missing_field("value"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let data = deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?;
            let r = serde_json::from_value(
                map.remove("r")
                    .ok_or_else(|| serde::de::Error::missing_field("r"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let s = serde_json::from_value(
                map.remove("s")
                    .ok_or_else(|| serde::de::Error::missing_field("s"))?,
            )
            .map_err(serde::de::Error::custom)?;

            Ok(LegacyTransaction {
                nonce,
                gas_price: serde_json::from_value::<U256>(
                    map.remove("gasPrice")
                        .ok_or_else(|| serde::de::Error::missing_field("gasPrice"))?,
                )
                .map_err(serde::de::Error::custom)?
                .as_u64(),
                gas: serde_json::from_value::<U256>(
                    map.remove("gas")
                        .ok_or_else(|| serde::de::Error::missing_field("gas"))?,
                )
                .map_err(serde::de::Error::custom)?
                .as_u64(),
                to,
                value,
                data,
                v: serde_json::from_value(
                    map.remove("v")
                        .ok_or_else(|| serde::de::Error::missing_field("v"))?,
                )
                .map_err(serde::de::Error::custom)?,
                r,
                s,
            })
        }
    }

    impl<'de> Deserialize<'de> for EIP2930Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;
            let nonce = serde_json::from_value::<U256>(
                map.remove("nonce")
                    .ok_or_else(|| serde::de::Error::missing_field("nonce"))?,
            )
            .map_err(serde::de::Error::custom)?
            .as_u64();
            let to = serde_json::from_value(
                map.remove("to")
                    .ok_or_else(|| serde::de::Error::missing_field("to"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let value = serde_json::from_value(
                map.remove("value")
                    .ok_or_else(|| serde::de::Error::missing_field("value"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let data = deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?;
            let r = serde_json::from_value(
                map.remove("r")
                    .ok_or_else(|| serde::de::Error::missing_field("r"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let s = serde_json::from_value(
                map.remove("s")
                    .ok_or_else(|| serde::de::Error::missing_field("s"))?,
            )
            .map_err(serde::de::Error::custom)?;

            Ok(EIP2930Transaction {
                chain_id: serde_json::from_value::<U256>(
                    map.remove("chainId")
                        .ok_or_else(|| serde::de::Error::missing_field("chainId"))?,
                )
                .map_err(serde::de::Error::custom)?
                .as_u64(),
                nonce,
                gas_price: serde_json::from_value::<U256>(
                    map.remove("gasPrice")
                        .ok_or_else(|| serde::de::Error::missing_field("gasPrice"))?,
                )
                .map_err(serde::de::Error::custom)?
                .as_u64(),
                gas_limit: serde_json::from_value::<U256>(
                    map.remove("gas")
                        .ok_or_else(|| serde::de::Error::missing_field("gas"))?,
                )
                .map_err(serde::de::Error::custom)?
                .as_u64(),
                to,
                value,
                data,
                access_list: serde_json::from_value(
                    map.remove("accessList")
                        .ok_or_else(|| serde::de::Error::missing_field("accessList"))?,
                )
                .map_err(serde::de::Error::custom)?,
                signature_y_parity: u8::from_str_radix(
                    serde_json::from_value::<String>(
                        map.remove("yParity")
                            .ok_or_else(|| serde::de::Error::missing_field("yParity"))?,
                    )
                    .map_err(serde::de::Error::custom)?
                    .trim_start_matches("0x"),
                    16,
                )
                .map_err(serde::de::Error::custom)?
                    != 0,
                signature_r: r,
                signature_s: s,
            })
        }
    }

    impl<'de> Deserialize<'de> for EIP1559Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;
            let nonce = serde_json::from_value::<U256>(
                map.remove("nonce")
                    .ok_or_else(|| serde::de::Error::missing_field("nonce"))?,
            )
            .map_err(serde::de::Error::custom)?
            .as_u64();
            let to = serde_json::from_value(
                map.remove("to")
                    .ok_or_else(|| serde::de::Error::missing_field("to"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let value = serde_json::from_value(
                map.remove("value")
                    .ok_or_else(|| serde::de::Error::missing_field("value"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let data = deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?;
            let r = serde_json::from_value(
                map.remove("r")
                    .ok_or_else(|| serde::de::Error::missing_field("r"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let s = serde_json::from_value(
                map.remove("s")
                    .ok_or_else(|| serde::de::Error::missing_field("s"))?,
            )
            .map_err(serde::de::Error::custom)?;

            Ok(EIP1559Transaction {
                chain_id: serde_json::from_value::<U256>(
                    map.remove("chainId")
                        .ok_or_else(|| serde::de::Error::missing_field("chainId"))?,
                )
                .map_err(serde::de::Error::custom)?
                .as_u64(),
                nonce,
                max_priority_fee_per_gas: serde_json::from_value::<U256>(
                    map.remove("maxPriorityFeePerGas")
                        .ok_or_else(|| serde::de::Error::missing_field("maxPriorityFeePerGas"))?,
                )
                .map_err(serde::de::Error::custom)?
                .as_u64(),
                max_fee_per_gas: serde_json::from_value::<U256>(
                    map.remove("maxFeePerGas")
                        .ok_or_else(|| serde::de::Error::missing_field("maxFeePerGas"))?,
                )
                .map_err(serde::de::Error::custom)?
                .as_u64(),
                gas_limit: serde_json::from_value::<U256>(
                    map.remove("gas")
                        .ok_or_else(|| serde::de::Error::missing_field("gas"))?,
                )
                .map_err(serde::de::Error::custom)?
                .as_u64(),
                to,
                value,
                data,
                access_list: serde_json::from_value(
                    map.remove("accessList")
                        .ok_or_else(|| serde::de::Error::missing_field("accessList"))?,
                )
                .map_err(serde::de::Error::custom)?,
                signature_y_parity: u8::from_str_radix(
                    serde_json::from_value::<String>(
                        map.remove("yParity")
                            .ok_or_else(|| serde::de::Error::missing_field("yParity"))?,
                    )
                    .map_err(serde::de::Error::custom)?
                    .trim_start_matches("0x"),
                    16,
                )
                .map_err(serde::de::Error::custom)?
                    != 0,
                signature_r: r,
                signature_s: s,
            })
        }
    }

    impl<'de> Deserialize<'de> for EIP4844Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;
            let chain_id = serde_json::from_value::<U256>(
                map.remove("chainId")
                    .ok_or_else(|| serde::de::Error::missing_field("chainId"))?,
            )
            .map_err(serde::de::Error::custom)?
            .as_u64();
            let nonce = serde_json::from_value::<U256>(
                map.remove("nonce")
                    .ok_or_else(|| serde::de::Error::missing_field("nonce"))?,
            )
            .map_err(serde::de::Error::custom)?
            .as_u64();
            let max_priority_fee_per_gas = serde_json::from_value::<U256>(
                map.remove("maxPriorityFeePerGas")
                    .ok_or_else(|| serde::de::Error::missing_field("maxPriorityFeePerGas"))?,
            )
            .map_err(serde::de::Error::custom)?
            .as_u64();
            let max_fee_per_gas = serde_json::from_value::<U256>(
                map.remove("maxFeePerGas")
                    .ok_or_else(|| serde::de::Error::missing_field("maxFeePerGas"))?,
            )
            .map_err(serde::de::Error::custom)?
            .as_u64();
            let gas = serde_json::from_value::<U256>(
                map.remove("gas")
                    .ok_or_else(|| serde::de::Error::missing_field("gas"))?,
            )
            .map_err(serde::de::Error::custom)?
            .as_u64();
            let to = serde_json::from_value(
                map.remove("to")
                    .ok_or_else(|| serde::de::Error::missing_field("to"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let value = serde_json::from_value(
                map.remove("value")
                    .ok_or_else(|| serde::de::Error::missing_field("value"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let data = deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?;
            let access_list = serde_json::from_value::<Vec<AccessListEntry>>(
                map.remove("accessList")
                    .ok_or_else(|| serde::de::Error::missing_field("accessList"))?,
            )
            .map_err(serde::de::Error::custom)?
            .into_iter()
            .map(|v| (v.address, v.storage_keys))
            .collect::<Vec<_>>();
            let max_fee_per_blob_gas = serde_json::from_value::<U256>(
                map.remove("maxFeePerBlobGas")
                    .ok_or_else(|| serde::de::Error::missing_field("maxFeePerBlobGas"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let blob_versioned_hashes = serde_json::from_value(
                map.remove("blobVersionedHashes")
                    .ok_or_else(|| serde::de::Error::missing_field("blobVersionedHashes"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let signature_y_parity = u8::from_str_radix(
                serde_json::from_value::<String>(
                    map.remove("yParity")
                        .ok_or_else(|| serde::de::Error::missing_field("yParity"))?,
                )
                .map_err(serde::de::Error::custom)?
                .trim_start_matches("0x"),
                16,
            )
            .map_err(serde::de::Error::custom)?
                != 0;
            let signature_r = serde_json::from_value(
                map.remove("r")
                    .ok_or_else(|| serde::de::Error::missing_field("r"))?,
            )
            .map_err(serde::de::Error::custom)?;
            let signature_s = serde_json::from_value(
                map.remove("s")
                    .ok_or_else(|| serde::de::Error::missing_field("s"))?,
            )
            .map_err(serde::de::Error::custom)?;

            Ok(EIP4844Transaction {
                chain_id,
                nonce,
                max_priority_fee_per_gas,
                max_fee_per_gas,
                gas,
                to,
                value,
                data,
                access_list,
                max_fee_per_blob_gas,
                blob_versioned_hashes,
                signature_y_parity,
                signature_r,
                signature_s,
            })
        }
    }

    /// Unsigned Transaction struct generic to all types which may not contain all required transaction fields
    /// Used to perform gas estimations and access list creation
    #[derive(Deserialize, Debug, PartialEq, Clone, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct GenericTransaction {
        #[serde(default)]
        pub r#type: TxType,
        #[serde(default, with = "crate::serde_utils::u64::hex_str")]
        pub nonce: u64,
        pub to: TxKind,
        #[serde(default)]
        pub from: Address,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub gas: Option<u64>,
        #[serde(default)]
        pub value: U256,
        #[serde(default, with = "crate::serde_utils::bytes", alias = "data")]
        pub input: Bytes,
        #[serde(default, with = "crate::serde_utils::u64::hex_str")]
        pub gas_price: u64,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub max_priority_fee_per_gas: Option<u64>,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub max_fee_per_gas: Option<u64>,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub max_fee_per_blob_gas: Option<u64>,
        #[serde(default)]
        pub access_list: Vec<AccessListEntry>,
        #[serde(default)]
        pub blob_versioned_hashes: Vec<H256>,
        #[serde(default, with = "crate::serde_utils::bytes::vec")]
        pub blobs: Vec<Bytes>,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub chain_id: Option<u64>,
    }

    impl From<EIP1559Transaction> for GenericTransaction {
        fn from(value: EIP1559Transaction) -> Self {
            Self {
                r#type: TxType::EIP1559,
                nonce: value.nonce,
                to: value.to,
                gas: Some(value.gas_limit),
                value: value.value,
                input: value.data,
                gas_price: value.max_fee_per_gas,
                max_priority_fee_per_gas: Some(value.max_priority_fee_per_gas),
                max_fee_per_gas: Some(value.max_fee_per_gas),
                max_fee_per_blob_gas: None,
                access_list: value
                    .access_list
                    .iter()
                    .map(AccessListEntry::from)
                    .collect(),
                blob_versioned_hashes: vec![],
                blobs: vec![],
                chain_id: Some(value.chain_id),
                ..Default::default()
            }
        }
    }
}

mod mempool {
    use super::*;
    use std::{
        cmp::Ordering,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MempoolTransaction {
        // Unix timestamp (in microseconds) created once the transaction reached the MemPool
        timestamp: u128,
        inner: Transaction,
    }

    impl MempoolTransaction {
        pub fn new(tx: Transaction) -> Self {
            Self {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Invalid system time")
                    .as_micros(),
                inner: tx,
            }
        }
        pub fn time(&self) -> u128 {
            self.timestamp
        }
    }

    impl RLPEncode for MempoolTransaction {
        fn encode(&self, buf: &mut dyn bytes::BufMut) {
            Encoder::new(buf)
                .encode_field(&self.timestamp)
                .encode_field(&self.inner)
                .finish();
        }
    }

    impl RLPDecode for MempoolTransaction {
        fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
            let decoder = Decoder::new(rlp)?;
            let (timestamp, decoder) = decoder.decode_field("timestamp")?;
            let (inner, decoder) = decoder.decode_field("inner")?;
            Ok((Self { timestamp, inner }, decoder.finish()?))
        }
    }

    impl std::ops::Deref for MempoolTransaction {
        type Target = Transaction;

        fn deref(&self) -> &Self::Target {
            &self.inner
        }
    }

    impl From<MempoolTransaction> for Transaction {
        fn from(val: MempoolTransaction) -> Self {
            val.inner
        }
    }

    // Orders transactions by lowest nonce, if the nonce is equal, orders by highest timestamp
    impl Ord for MempoolTransaction {
        fn cmp(&self, other: &Self) -> Ordering {
            match self.nonce().cmp(&other.nonce()) {
                Ordering::Equal => other.time().cmp(&self.time()),
                ordering => ordering,
            }
        }
    }

    impl PartialOrd for MempoolTransaction {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::types::{compute_receipts_root, compute_transactions_root, BlockBody, Receipt};
    use ethereum_types::H160;
    use hex_literal::hex;
    use serde_impl::{AccessListEntry, GenericTransaction};
    use std::str::FromStr;

    #[test]
    fn test_compute_transactions_root() {
        let mut body = BlockBody::empty();
        let tx = LegacyTransaction {
            nonce: 0,
            gas_price: 0x0a,
            gas: 0x05f5e100,
            to: TxKind::Call(hex!("1000000000000000000000000000000000000000").into()),
            value: 0.into(),
            data: Default::default(),
            v: U256::from(0x1b),
            r: U256::from(hex!(
                "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
            )),
            s: U256::from(hex!(
                "5f6e3f188e3e6eab7d7d3b6568f5eac7d687b08d307d3154ccd8c87b4630509b"
            )),
        };
        body.transactions.push(Transaction::LegacyTransaction(tx));
        let expected_root =
            hex!("8151d548273f6683169524b66ca9fe338b9ce42bc3540046c828fd939ae23bcb");
        let result = compute_transactions_root(&body.transactions);

        assert_eq!(result, expected_root.into());
    }
    #[test]
    fn test_compute_hash() {
        // taken from Hive
        let tx_eip2930 = EIP2930Transaction {
            chain_id: 3503995874084926u64,
            nonce: 7,
            gas_price: 0x2dbf1f9a,
            gas_limit: 0x186A0,
            to: TxKind::Call(hex!("7dcd17433742f4c0ca53122ab541d0ba67fc27df").into()),
            value: 2.into(),
            data: Bytes::from(&b"\xdbS\x06$\x8e\x03\x13\xe7emit"[..]),
            access_list: vec![(
                hex!("7dcd17433742f4c0ca53122ab541d0ba67fc27df").into(),
                vec![
                    hex!("0000000000000000000000000000000000000000000000000000000000000000").into(),
                    hex!("a3d07a7d68fbd49ec2f8e6befdd86c885f86c272819f6f345f365dec35ae6707").into(),
                ],
            )],
            signature_y_parity: false,
            signature_r: U256::from_dec_str(
                "75813812796588349127366022588733264074091236448495248199152066031778895768879",
            )
            .unwrap(),
            signature_s: U256::from_dec_str(
                "25476208226281085290728123165613764315157904411823916642262684106502155457829",
            )
            .unwrap(),
        };
        let tx = Transaction::EIP2930Transaction(tx_eip2930);

        let expected_hash =
            hex!("a0762610d794acddd2dca15fb7c437ada3611c886f3bea675d53d8da8a6c41b2");
        let hash = tx.compute_hash();
        assert_eq!(hash, expected_hash.into());
    }

    #[test]
    fn test_compute_receipts_root() {
        // example taken from
        // https://github.com/ethereum/go-ethereum/blob/f8aa62353666a6368fb3f1a378bd0a82d1542052/cmd/evm/testdata/1/exp.json#L18
        let tx_type = TxType::Legacy;
        let succeeded = true;
        let cumulative_gas_used = 0x5208;
        let logs = vec![];
        let receipt = Receipt::new(tx_type, succeeded, cumulative_gas_used, logs);

        let result = compute_receipts_root(&[receipt]);
        let expected_root =
            hex!("056b23fbba480696b65fe5a59b8f2148a1299103c4f57df839233af2cf4ca2d2");
        assert_eq!(result, expected_root.into());
    }

    #[test]
    fn legacy_tx_rlp_decode() {
        let encoded_tx = "f86d80843baa0c4082f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee538000808360306ba0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4";
        let encoded_tx_bytes = hex::decode(encoded_tx).unwrap();
        let tx = LegacyTransaction::decode(&encoded_tx_bytes).unwrap();
        let expected_tx = LegacyTransaction {
            nonce: 0,
            gas_price: 1001000000,
            gas: 63000,
            to: TxKind::Call(Address::from_slice(
                &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
            )),
            value: 3000000000000000_u64.into(),
            data: Bytes::new(),
            r: U256::from_str_radix(
                "151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65d",
                16,
            )
            .unwrap(),
            s: U256::from_str_radix(
                "64c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4",
                16,
            )
            .unwrap(),
            v: 6303851.into(),
        };
        assert_eq!(tx, expected_tx);
    }

    #[test]
    fn eip1559_tx_rlp_decode() {
        let encoded_tx = "f86c8330182480114e82f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee53800080c080a0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4";
        let encoded_tx_bytes = hex::decode(encoded_tx).unwrap();
        let tx = EIP1559Transaction::decode(&encoded_tx_bytes).unwrap();
        let expected_tx = EIP1559Transaction {
            nonce: 0,
            max_fee_per_gas: 78,
            max_priority_fee_per_gas: 17,
            to: TxKind::Call(Address::from_slice(
                &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
            )),
            value: 3000000000000000_u64.into(),
            data: Bytes::new(),
            signature_r: U256::from_str_radix(
                "151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65d",
                16,
            )
            .unwrap(),
            signature_s: U256::from_str_radix(
                "64c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4",
                16,
            )
            .unwrap(),
            signature_y_parity: false,
            chain_id: 3151908,
            gas_limit: 63000,
            access_list: vec![],
        };
        assert_eq!(tx, expected_tx);
    }

    #[test]
    fn deserialize_tx_kind() {
        let tx_kind_create = r#""""#;
        let tx_kind_call = r#""0x6177843db3138ae69679A54b95cf345ED759450d""#;
        let deserialized_tx_kind_create = TxKind::Create;
        let deserialized_tx_kind_call = TxKind::Call(Address::from_slice(
            &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
        ));
        assert_eq!(
            deserialized_tx_kind_create,
            serde_json::from_str(tx_kind_create).unwrap()
        );
        assert_eq!(
            deserialized_tx_kind_call,
            serde_json::from_str(tx_kind_call).unwrap()
        )
    }

    #[test]
    fn deserialize_tx_type() {
        let tx_type_eip2930 = r#""0x01""#;
        let tx_type_eip1559 = r#""0x02""#;
        let deserialized_tx_type_eip2930 = TxType::EIP2930;
        let deserialized_tx_type_eip1559 = TxType::EIP1559;
        assert_eq!(
            deserialized_tx_type_eip2930,
            serde_json::from_str(tx_type_eip2930).unwrap()
        );
        assert_eq!(
            deserialized_tx_type_eip1559,
            serde_json::from_str(tx_type_eip1559).unwrap()
        )
    }

    #[test]
    fn deserialize_generic_transaction() {
        let generic_transaction = r#"{
            "type":"0x01",
            "nonce":"0x02",
            "to":"",
            "from":"0x6177843db3138ae69679A54b95cf345ED759450d",
            "gas":"0x5208",
            "value":"0x01",
            "input":"0x",
            "gasPrice":"0x07",
            "accessList": [
                {
                    "address": "0x000f3df6d732807ef1319fb7b8bb8522d0beac02",
                    "storageKeys": [
                        "0x000000000000000000000000000000000000000000000000000000000000000c",
                        "0x000000000000000000000000000000000000000000000000000000000000200b"
                    ]
                }
            ]
        }"#;
        let deserialized_generic_transaction = GenericTransaction {
            r#type: TxType::EIP2930,
            nonce: 2,
            to: TxKind::Create,
            from: Address::from_slice(
                &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
            ),
            gas: Some(0x5208),
            value: U256::from(1),
            input: Bytes::new(),
            gas_price: 7,
            max_priority_fee_per_gas: Default::default(),
            max_fee_per_gas: Default::default(),
            max_fee_per_blob_gas: Default::default(),
            access_list: vec![AccessListEntry {
                address: Address::from_slice(
                    &hex::decode("000f3df6d732807ef1319fb7b8bb8522d0beac02").unwrap(),
                ),
                storage_keys: vec![H256::from_low_u64_be(12), H256::from_low_u64_be(8203)],
            }],
            blob_versioned_hashes: Default::default(),
            blobs: Default::default(),
            chain_id: Default::default(),
        };
        assert_eq!(
            deserialized_generic_transaction,
            serde_json::from_str(generic_transaction).unwrap()
        )
    }

    #[test]
    fn deserialize_eip4844_transaction() {
        let eip4844_transaction = r#"{
            "chainId":"0x01",
            "nonce":"0x02",
            "maxPriorityFeePerGas":"0x01",
            "maxFeePerGas":"0x01",
            "gas":"0x5208",
            "to":"0x6177843db3138ae69679A54b95cf345ED759450d",
            "value":"0x01",
            "input":"0x3033",
            "accessList": [
                {
                    "address": "0x000f3df6d732807ef1319fb7b8bb8522d0beac02",
                    "storageKeys": [
                        "0x000000000000000000000000000000000000000000000000000000000000000c",
                        "0x000000000000000000000000000000000000000000000000000000000000200b"
                    ]
                }
            ],
            "maxFeePerBlobGas":"0x03",
            "blobVersionedHashes": [
                    "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "0x0000000000000000000000000000000000000000000000000000000000000002"
            ],
            "yParity":"0x0",
            "r": "0x01",
            "s": "0x02"
        }"#;
        let deserialized_eip4844_transaction = EIP4844Transaction {
            chain_id: 0x01,
            nonce: 0x02,
            to: Address::from_slice(
                &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
            ),
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            max_fee_per_blob_gas: U256::from(0x03),
            gas: 0x5208,
            value: U256::from(0x01),
            // 03 in hex is 0x3033, that's why the 'input' has that number.
            data: Bytes::from_static(b"03"),
            access_list: vec![(
                Address::from_slice(
                    &hex::decode("000f3df6d732807ef1319fb7b8bb8522d0beac02").unwrap(),
                ),
                vec![H256::from_low_u64_be(12), H256::from_low_u64_be(8203)],
            )],
            blob_versioned_hashes: vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)],
            signature_y_parity: false,
            signature_r: U256::from(0x01),
            signature_s: U256::from(0x02),
        };

        assert_eq!(
            deserialized_eip4844_transaction,
            serde_json::from_str(eip4844_transaction).unwrap()
        )
    }

    #[test]
    fn serialize_deserialize_transaction() {
        let eip1559 = EIP1559Transaction {
            chain_id: 1729,
            nonce: 1,
            max_priority_fee_per_gas: 1000,
            max_fee_per_gas: 2000,
            gas_limit: 21000,
            to: TxKind::Call(H160::from_str("0x000a52D537c4150ec274dcE3962a0d179B7E71B0").unwrap()),
            value: U256::from(100000),
            data: Bytes::from_static(b"03"),
            access_list: vec![],
            signature_y_parity: true,
            signature_r: U256::one(),
            signature_s: U256::zero(),
        };
        let tx_to_serialize = Transaction::EIP1559Transaction(eip1559.clone());
        let serialized = serde_json::to_string(&tx_to_serialize).expect("Failed to serialize");

        println!("{serialized:?}");

        let deserialized_tx: Transaction =
            serde_json::from_str(&serialized).expect("Failed to deserialize");

        assert!(deserialized_tx.tx_type() == TxType::EIP1559);

        if let Transaction::EIP1559Transaction(tx) = deserialized_tx {
            assert_eq!(tx, eip1559);
        }
    }
}
