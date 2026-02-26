use std::{cmp::min, fmt::Display};

use crate::{errors::EcdsaError, utils::keccak};
use bytes::Bytes;
use ethereum_types::{Address, H256, Signature, U256};
use hex_literal::hex;
pub use mempool::MempoolTransaction;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Serialize, ser::SerializeStruct};
pub use serde_impl::{
    AccessListEntry, AuthorizationTupleEntry, GenericTransaction, GenericTransactionError,
};

/// The serialized length of a default eip1559 transaction
pub const EIP1559_DEFAULT_SERIALIZED_LENGTH: usize = 15;

use ethrex_rlp::{
    constants::RLP_NULL,
    decode::{RLPDecode, decode_rlp_item},
    encode::{PayloadRLPEncode, RLPEncode},
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

use crate::types::{AccessList, AuthorizationList, BlobsBundle};
use once_cell::sync::OnceCell;

// The `#[serde(untagged)]` attribute allows the `Transaction` enum to be serialized without
// a tag indicating the variant type. This means that Serde will serialize the enum's variants
// directly according to the structure of the variant itself.
// For each variant, Serde will use the serialization logic implemented
// for the inner type of that variant (like `LegacyTransaction`, `EIP2930Transaction`, etc.).
// The serialization will fail if the data does not match the structure of any variant.
//
// A custom Deserialization method is implemented to match the specific transaction `type`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, RSerialize, RDeserialize, Archive)]
#[serde(untagged)]
pub enum Transaction {
    LegacyTransaction(LegacyTransaction),
    EIP2930Transaction(EIP2930Transaction),
    EIP1559Transaction(EIP1559Transaction),
    EIP4844Transaction(EIP4844Transaction),
    EIP7702Transaction(EIP7702Transaction),
    PrivilegedL2Transaction(PrivilegedL2Transaction),
    FeeTokenTransaction(FeeTokenTransaction),
}

/// The same as a Transaction enum, only that blob transactions are in wrapped format, including
/// the blobs bundle.
/// PrivilegedL2Transaction is not included as it is not expected to be sent over P2P.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum P2PTransaction {
    LegacyTransaction(LegacyTransaction),
    EIP2930Transaction(EIP2930Transaction),
    EIP1559Transaction(EIP1559Transaction),
    EIP4844TransactionWithBlobs(WrappedEIP4844Transaction),
    EIP7702Transaction(EIP7702Transaction),
    FeeTokenTransaction(FeeTokenTransaction),
}

impl TryInto<Transaction> for P2PTransaction {
    type Error = String;

    fn try_into(self) -> Result<Transaction, Self::Error> {
        match self {
            P2PTransaction::LegacyTransaction(itx) => Ok(Transaction::LegacyTransaction(itx)),
            P2PTransaction::EIP2930Transaction(itx) => Ok(Transaction::EIP2930Transaction(itx)),
            P2PTransaction::EIP1559Transaction(itx) => Ok(Transaction::EIP1559Transaction(itx)),
            P2PTransaction::EIP7702Transaction(itx) => Ok(Transaction::EIP7702Transaction(itx)),
            _ => Err("Can't convert blob p2p transaction into regular transaction. Blob bundle would be lost.".to_string()),
        }
    }
}

impl RLPEncode for P2PTransaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            P2PTransaction::LegacyTransaction(t) => t.encode(buf),
            tx => <[u8] as RLPEncode>::encode(&tx.encode_canonical_to_vec(), buf),
        };
    }
}

impl RLPDecode for P2PTransaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (is_list, payload, remainder) = decode_rlp_item(rlp)?;
        if !is_list {
            let tx_type = payload.first().ok_or(RLPDecodeError::InvalidLength)?;
            let tx_encoding = &payload.get(1..).ok_or(RLPDecodeError::InvalidLength)?;
            // Look at the first byte to check if it corresponds to a TransactionType
            match *tx_type {
                // Legacy
                0x0 => LegacyTransaction::decode(tx_encoding)
                    .map(|tx| (P2PTransaction::LegacyTransaction(tx), remainder)), // TODO: check if this is a real case scenario
                // EIP2930
                0x1 => EIP2930Transaction::decode(tx_encoding)
                    .map(|tx| (P2PTransaction::EIP2930Transaction(tx), remainder)),
                // EIP1559
                0x2 => EIP1559Transaction::decode(tx_encoding)
                    .map(|tx| (P2PTransaction::EIP1559Transaction(tx), remainder)),
                // EIP4844
                0x3 => WrappedEIP4844Transaction::decode(tx_encoding)
                    .map(|tx| (P2PTransaction::EIP4844TransactionWithBlobs(tx), remainder)),
                // EIP7702
                0x4 => EIP7702Transaction::decode(tx_encoding)
                    .map(|tx| (P2PTransaction::EIP7702Transaction(tx), remainder)),
                // FeeToken
                0x7d => FeeTokenTransaction::decode(tx_encoding)
                    .map(|tx| (P2PTransaction::FeeTokenTransaction(tx), remainder)),
                ty => Err(RLPDecodeError::Custom(format!(
                    "Invalid transaction type: {ty}"
                ))),
            }
        } else {
            // LegacyTransaction
            LegacyTransaction::decode_unfinished(rlp)
                .map(|(tx, rem)| (P2PTransaction::LegacyTransaction(tx), rem))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedEIP4844Transaction {
    pub tx: EIP4844Transaction,
    pub wrapper_version: Option<u8>,
    pub blobs_bundle: BlobsBundle,
}

impl RLPEncode for WrappedEIP4844Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let encoder = Encoder::new(buf);
        encoder
            .encode_field(&self.tx)
            .encode_optional_field(&self.wrapper_version)
            .encode_field(&self.blobs_bundle.blobs)
            .encode_field(&self.blobs_bundle.commitments)
            .encode_field(&self.blobs_bundle.proofs)
            .finish();
    }
}

impl RLPDecode for WrappedEIP4844Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(WrappedEIP4844Transaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let Ok((tx, decoder)) = decoder.decode_field("tx") else {
            // Handle the case of blobless transaction
            let (tx, rest) = EIP4844Transaction::decode_unfinished(rlp)?;
            return Ok((
                WrappedEIP4844Transaction {
                    tx,
                    wrapper_version: None,
                    // Empty blobs bundles are not valid
                    blobs_bundle: BlobsBundle::empty(),
                },
                rest,
            ));
        };

        let (wrapper_version, decoder) = decoder.decode_optional_field();
        let (blobs, decoder) = decoder.decode_field("blobs")?;
        let (commitments, decoder) = decoder.decode_field("commitments")?;
        let (proofs, decoder) = decoder.decode_field("proofs")?;

        let wrapped = WrappedEIP4844Transaction {
            tx,
            wrapper_version,
            blobs_bundle: BlobsBundle {
                blobs,
                commitments,
                proofs,
                version: wrapper_version.unwrap_or_default(),
            },
        };
        Ok((wrapped, decoder.finish()?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub struct LegacyTransaction {
    pub nonce: u64,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub gas_price: U256,
    pub gas: u64,
    /// The recipient of the transaction.
    /// Create transactions contain a [`null`](RLP_NULL) value in this field.
    pub to: TxKind,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub value: U256,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub data: Bytes,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub v: U256,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub r: U256,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub s: U256,
    #[rkyv(with=rkyv::with::Skip)]
    pub inner_hash: OnceCell<H256>,
    #[rkyv(with=rkyv::with::Skip)]
    pub sender_cache: OnceCell<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub struct EIP2930Transaction {
    pub chain_id: u64,
    pub nonce: u64,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub gas_price: U256,
    pub gas_limit: u64,
    pub to: TxKind,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub value: U256,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub data: Bytes,
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::AccessListItemWrapper>)]
    pub access_list: AccessList,
    pub signature_y_parity: bool,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_r: U256,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_s: U256,
    #[rkyv(with=rkyv::with::Skip)]
    pub inner_hash: OnceCell<H256>,
    #[rkyv(with=rkyv::with::Skip)]
    pub sender_cache: OnceCell<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub struct EIP1559Transaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    pub to: TxKind,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub value: U256,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub data: Bytes,
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::AccessListItemWrapper>)]
    pub access_list: AccessList,
    pub signature_y_parity: bool,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_r: U256,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_s: U256,
    #[rkyv(with=rkyv::with::Skip)]
    pub inner_hash: OnceCell<H256>,
    #[rkyv(with=rkyv::with::Skip)]
    pub sender_cache: OnceCell<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub struct EIP4844Transaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas: u64,
    #[rkyv(with=crate::rkyv_utils::H160Wrapper)]
    pub to: Address,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub value: U256,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub data: Bytes,
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::AccessListItemWrapper>)]
    pub access_list: AccessList,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub max_fee_per_blob_gas: U256,
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::H256Wrapper>)]
    pub blob_versioned_hashes: Vec<H256>,
    pub signature_y_parity: bool,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_r: U256,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_s: U256,
    #[rkyv(with=rkyv::with::Skip)]
    pub inner_hash: OnceCell<H256>,
    #[rkyv(with=rkyv::with::Skip)]
    pub sender_cache: OnceCell<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub struct EIP7702Transaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    #[rkyv(with=crate::rkyv_utils::H160Wrapper)]
    pub to: Address,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub value: U256,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub data: Bytes,
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::AccessListItemWrapper>)]
    pub access_list: AccessList,
    pub authorization_list: AuthorizationList,
    pub signature_y_parity: bool,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_r: U256,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_s: U256,
    #[rkyv(with=rkyv::with::Skip)]
    pub inner_hash: OnceCell<H256>,
    #[rkyv(with=rkyv::with::Skip)]
    pub sender_cache: OnceCell<Address>,
}
#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub struct PrivilegedL2Transaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    pub to: TxKind,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub value: U256,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub data: Bytes,
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::AccessListItemWrapper>)]
    pub access_list: AccessList,
    #[rkyv(with=crate::rkyv_utils::H160Wrapper)]
    pub from: Address,
    #[rkyv(with=rkyv::with::Skip)]
    pub inner_hash: OnceCell<H256>,
    #[rkyv(with=rkyv::with::Skip)]
    pub sender_cache: OnceCell<Address>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TxType {
    #[default]
    Legacy = 0x00,
    EIP2930 = 0x01,
    EIP1559 = 0x02,
    EIP4844 = 0x03,
    EIP7702 = 0x04,
    FeeToken = 0x7d,
    // We take the same approach as Optimism to define the privileged tx prefix
    // https://github.com/ethereum-optimism/specs/blob/c6903a3b2cad575653e1f5ef472debb573d83805/specs/protocol/deposits.md#the-deposited-transaction-type
    Privileged = 0x7e,
}

impl From<TxType> for u8 {
    fn from(val: TxType) -> Self {
        match val {
            TxType::Legacy => 0x00,
            TxType::EIP2930 => 0x01,
            TxType::EIP1559 => 0x02,
            TxType::EIP4844 => 0x03,
            TxType::EIP7702 => 0x04,
            TxType::FeeToken => 0x7d,
            TxType::Privileged => 0x7e,
        }
    }
}

impl Display for TxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxType::Legacy => write!(f, "Legacy"),
            TxType::EIP2930 => write!(f, "EIP2930"),
            TxType::EIP1559 => write!(f, "EIP1559"),
            TxType::EIP4844 => write!(f, "EIP4844"),
            TxType::EIP7702 => write!(f, "EIP7702"),
            TxType::Privileged => write!(f, "Privileged"),
            TxType::FeeToken => write!(f, "FeeToken"),
        }
    }
}

impl Transaction {
    pub fn tx_type(&self) -> TxType {
        match self {
            Transaction::LegacyTransaction(_) => TxType::Legacy,
            Transaction::EIP2930Transaction(_) => TxType::EIP2930,
            Transaction::EIP1559Transaction(_) => TxType::EIP1559,
            Transaction::EIP4844Transaction(_) => TxType::EIP4844,
            Transaction::EIP7702Transaction(_) => TxType::EIP7702,
            Transaction::FeeTokenTransaction(_) => TxType::FeeToken,
            Transaction::PrivilegedL2Transaction(_) => TxType::Privileged,
        }
    }

    fn calc_effective_gas_price(&self, base_fee_per_gas: Option<u64>) -> Option<U256> {
        let base_fee = base_fee_per_gas?;
        let max_fee = self.max_fee_per_gas()?;
        if max_fee < base_fee {
            // This is invalid, can't calculate
            return None;
        }

        let priority_fee_per_gas = min(self.max_priority_fee()?, max_fee.saturating_sub(base_fee));
        Some(U256::from(priority_fee_per_gas) + U256::from(base_fee))
    }

    pub fn effective_gas_price(&self, base_fee_per_gas: Option<u64>) -> Option<U256> {
        match self.tx_type() {
            TxType::Legacy => Some(self.gas_price()),
            TxType::EIP2930 => Some(self.gas_price()),
            TxType::EIP1559 => self.calc_effective_gas_price(base_fee_per_gas),
            TxType::EIP4844 => self.calc_effective_gas_price(base_fee_per_gas),
            TxType::EIP7702 => self.calc_effective_gas_price(base_fee_per_gas),
            TxType::FeeToken => self.calc_effective_gas_price(base_fee_per_gas),
            TxType::Privileged => Some(self.gas_price()),
        }
    }

    pub fn cost_without_base_fee(&self) -> Option<U256> {
        let price = match self.tx_type() {
            TxType::Legacy => self.gas_price(),
            TxType::EIP2930 => self.gas_price(),
            TxType::EIP1559 => U256::from(self.max_fee_per_gas()?),
            TxType::EIP4844 => U256::from(self.max_fee_per_gas()?),
            TxType::EIP7702 => U256::from(self.max_fee_per_gas()?),
            TxType::FeeToken => U256::from(self.max_fee_per_gas()?),
            TxType::Privileged => self.gas_price(),
        };

        Some(U256::saturating_add(
            U256::saturating_mul(price, self.gas_limit().into()),
            self.value(),
        ))
    }

    pub fn fee_token(&self) -> Option<Address> {
        if let Transaction::FeeTokenTransaction(tx) = self {
            Some(tx.fee_token)
        } else {
            None
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
            tx => <[u8] as RLPEncode>::encode(&tx.encode_canonical_to_vec(), buf),
        };
    }
}

impl RLPDecode for Transaction {
    /// Transactions can be encoded in the following formats:
    /// A) Legacy transactions: rlp(LegacyTransaction)
    /// B) Non legacy transactions: rlp(Bytes) where Bytes represents the canonical encoding for the transaction as a bytes object.
    /// Checkout [Transaction::decode_canonical] for more information
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let (is_list, payload, remainder) = decode_rlp_item(rlp)?;
        if !is_list {
            let tx_type = payload.first().ok_or(RLPDecodeError::InvalidLength)?;
            let tx_encoding = &payload.get(1..).ok_or(RLPDecodeError::InvalidLength)?;
            // Look at the first byte to check if it corresponds to a TransactionType
            match *tx_type {
                // Legacy
                0x0 => LegacyTransaction::decode(tx_encoding)
                    .map(|tx| (Transaction::LegacyTransaction(tx), remainder)), // TODO: check if this is a real case scenario
                // EIP2930
                0x1 => EIP2930Transaction::decode(tx_encoding)
                    .map(|tx| (Transaction::EIP2930Transaction(tx), remainder)),
                // EIP1559
                0x2 => EIP1559Transaction::decode(tx_encoding)
                    .map(|tx| (Transaction::EIP1559Transaction(tx), remainder)),
                // EIP4844
                0x3 => EIP4844Transaction::decode(tx_encoding)
                    .map(|tx| (Transaction::EIP4844Transaction(tx), remainder)),
                // EIP7702
                0x4 => EIP7702Transaction::decode(tx_encoding)
                    .map(|tx| (Transaction::EIP7702Transaction(tx), remainder)),
                // FeeToken
                0x7d => FeeTokenTransaction::decode(tx_encoding)
                    .map(|tx| (Transaction::FeeTokenTransaction(tx), remainder)),
                // PrivilegedL2
                0x7e => PrivilegedL2Transaction::decode(tx_encoding)
                    .map(|tx| (Transaction::PrivilegedL2Transaction(tx), remainder)),
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
#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub enum TxKind {
    Call(#[rkyv(with=crate::rkyv_utils::H160Wrapper)] Address),
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

impl RLPEncode for EIP7702Transaction {
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
            .encode_field(&self.authorization_list)
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
            .encode_field(&self.from)
            .finish()
    }
}

impl RLPEncode for FeeTokenTransaction {
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
            .encode_field(&self.fee_token)
            .encode_field(&self.signature_y_parity)
            .encode_field(&self.signature_r)
            .encode_field(&self.signature_s)
            .finish()
    }
}

impl PayloadRLPEncode for Transaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            Transaction::LegacyTransaction(tx) => tx.encode_payload(buf),
            Transaction::EIP1559Transaction(tx) => tx.encode_payload(buf),
            Transaction::EIP2930Transaction(tx) => tx.encode_payload(buf),
            Transaction::EIP4844Transaction(tx) => tx.encode_payload(buf),
            Transaction::EIP7702Transaction(tx) => tx.encode_payload(buf),
            Transaction::PrivilegedL2Transaction(tx) => tx.encode_payload(buf),
            Transaction::FeeTokenTransaction(tx) => tx.encode_payload(buf),
        }
    }
}

impl PayloadRLPEncode for LegacyTransaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.nonce)
            .encode_field(&self.gas_price)
            .encode_field(&self.gas)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .finish();
    }
}

impl PayloadRLPEncode for EIP1559Transaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
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
            .finish();
    }
}

impl PayloadRLPEncode for EIP2930Transaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.nonce)
            .encode_field(&self.gas_price)
            .encode_field(&self.gas_limit)
            .encode_field(&self.to)
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.access_list)
            .finish();
    }
}

impl PayloadRLPEncode for EIP4844Transaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
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
            .finish();
    }
}

impl PayloadRLPEncode for EIP7702Transaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
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
            .encode_field(&self.authorization_list)
            .finish();
    }
}

impl PayloadRLPEncode for PrivilegedL2Transaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
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
            .encode_field(&self.from)
            .finish();
    }
}

impl PayloadRLPEncode for FeeTokenTransaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
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
            .encode_field(&self.fee_token)
            .finish();
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
        let inner_hash = OnceCell::new();
        let sender_cache = OnceCell::new();

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
            inner_hash,
            sender_cache,
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
        let inner_hash = OnceCell::new();
        let sender_cache = OnceCell::new();

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
            inner_hash,
            sender_cache,
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
        let inner_hash = OnceCell::new();
        let sender_cache = OnceCell::new();

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
            inner_hash,
            sender_cache,
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
        let inner_hash = OnceCell::new();
        let sender_cache = OnceCell::new();

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
            inner_hash,
            sender_cache,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for EIP7702Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(EIP7702Transaction, &[u8]), RLPDecodeError> {
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
        let (authorization_list, decoder) = decoder.decode_field("authorization_list")?;
        let (signature_y_parity, decoder) = decoder.decode_field("signature_y_parity")?;
        let (signature_r, decoder) = decoder.decode_field("signature_r")?;
        let (signature_s, decoder) = decoder.decode_field("signature_s")?;
        let inner_hash = OnceCell::new();
        let sender_cache = OnceCell::new();

        let tx = EIP7702Transaction {
            chain_id,
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            to,
            value,
            data,
            access_list,
            authorization_list,
            signature_y_parity,
            signature_r,
            signature_s,
            inner_hash,
            sender_cache,
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
        let (gas_limit, decoder) = decoder.decode_field::<u64>("gas_limit")?;
        let (to, decoder) = decoder.decode_field("to")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let (access_list, decoder) = decoder.decode_field("access_list")?;
        let (from, decoder) = decoder.decode_field("from")?;
        let inner_hash = OnceCell::new();
        let sender_cache = OnceCell::new();

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
            from,
            inner_hash,
            sender_cache,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for FeeTokenTransaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(FeeTokenTransaction, &[u8]), RLPDecodeError> {
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
        let (fee_token, decoder) = decoder.decode_field("fee_token")?;
        let (signature_y_parity, decoder) = decoder.decode_field("signature_y_parity")?;
        let (signature_r, decoder) = decoder.decode_field("signature_r")?;
        let (signature_s, decoder) = decoder.decode_field("signature_s")?;
        let inner_hash = OnceCell::new();
        let sender_cache = OnceCell::new();

        let tx = FeeTokenTransaction {
            chain_id,
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            to,
            value,
            data,
            access_list,
            fee_token,
            signature_y_parity,
            signature_r,
            signature_s,
            inner_hash,
            sender_cache,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl Transaction {
    pub fn sender(&self) -> Result<Address, EcdsaError> {
        let sender_cache = match self {
            Transaction::LegacyTransaction(tx) => &tx.sender_cache,
            Transaction::EIP2930Transaction(tx) => &tx.sender_cache,
            Transaction::EIP1559Transaction(tx) => &tx.sender_cache,
            Transaction::EIP4844Transaction(tx) => &tx.sender_cache,
            Transaction::EIP7702Transaction(tx) => &tx.sender_cache,
            Transaction::PrivilegedL2Transaction(tx) => &tx.sender_cache,
            Transaction::FeeTokenTransaction(tx) => &tx.sender_cache,
        };
        sender_cache
            .get_or_try_init(|| self.compute_sender())
            .copied()
    }

    fn compute_sender(&self) -> Result<Address, EcdsaError> {
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
                let mut sig = [0u8; 65];
                sig[..32].copy_from_slice(&tx.r.to_big_endian());
                sig[32..64].copy_from_slice(&tx.s.to_big_endian());
                sig[64] = signature_y_parity as u8;
                recover_address_from_message(Signature::from_slice(&sig), &Bytes::from(buf))
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
                let mut sig = [0u8; 65];
                sig[..32].copy_from_slice(&tx.signature_r.to_big_endian());
                sig[32..64].copy_from_slice(&tx.signature_s.to_big_endian());
                sig[64] = tx.signature_y_parity as u8;
                recover_address_from_message(Signature::from_slice(&sig), &Bytes::from(buf))
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
                let mut sig = [0u8; 65];
                sig[..32].copy_from_slice(&tx.signature_r.to_big_endian());
                sig[32..64].copy_from_slice(&tx.signature_s.to_big_endian());
                sig[64] = tx.signature_y_parity as u8;
                recover_address_from_message(Signature::from_slice(&sig), &Bytes::from(buf))
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
                let mut sig = [0u8; 65];
                sig[..32].copy_from_slice(&tx.signature_r.to_big_endian());
                sig[32..64].copy_from_slice(&tx.signature_s.to_big_endian());
                sig[64] = tx.signature_y_parity as u8;
                recover_address_from_message(Signature::from_slice(&sig), &Bytes::from(buf))
            }
            Transaction::EIP7702Transaction(tx) => {
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
                    .encode_field(&tx.authorization_list)
                    .finish();
                let mut sig = [0u8; 65];
                sig[..32].copy_from_slice(&tx.signature_r.to_big_endian());
                sig[32..64].copy_from_slice(&tx.signature_s.to_big_endian());
                sig[64] = tx.signature_y_parity as u8;
                recover_address_from_message(Signature::from_slice(&sig), &Bytes::from(buf))
            }
            Transaction::PrivilegedL2Transaction(tx) => Ok(tx.from),
            Transaction::FeeTokenTransaction(tx) => {
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
                    .encode_field(&tx.fee_token)
                    .finish();
                let mut sig = [0u8; 65];
                sig[..32].copy_from_slice(&tx.signature_r.to_big_endian());
                sig[32..64].copy_from_slice(&tx.signature_s.to_big_endian());
                sig[64] = tx.signature_y_parity as u8;
                recover_address_from_message(Signature::from_slice(&sig), &Bytes::from(buf))
            }
        }
    }

    pub fn gas_limit(&self) -> u64 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.gas,
            Transaction::EIP2930Transaction(tx) => tx.gas_limit,
            Transaction::EIP1559Transaction(tx) => tx.gas_limit,
            Transaction::EIP7702Transaction(tx) => tx.gas_limit,
            Transaction::EIP4844Transaction(tx) => tx.gas,
            Transaction::PrivilegedL2Transaction(tx) => tx.gas_limit,
            Transaction::FeeTokenTransaction(tx) => tx.gas_limit,
        }
    }

    //TODO: It's not very correct to return gas price for legacy and eip-2930 txs but return the max fee per gas for the others, make necessary changes for it to be technically correct.
    pub fn gas_price(&self) -> U256 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.gas_price,
            Transaction::EIP2930Transaction(tx) => tx.gas_price,
            Transaction::EIP1559Transaction(tx) => U256::from(tx.max_fee_per_gas),
            Transaction::EIP7702Transaction(tx) => U256::from(tx.max_fee_per_gas),
            Transaction::EIP4844Transaction(tx) => U256::from(tx.max_fee_per_gas),
            Transaction::PrivilegedL2Transaction(tx) => U256::from(tx.max_fee_per_gas),
            Transaction::FeeTokenTransaction(tx) => U256::from(tx.max_fee_per_gas),
        }
    }

    pub fn to(&self) -> TxKind {
        match self {
            Transaction::LegacyTransaction(tx) => tx.to.clone(),
            Transaction::EIP2930Transaction(tx) => tx.to.clone(),
            Transaction::EIP1559Transaction(tx) => tx.to.clone(),
            Transaction::EIP4844Transaction(tx) => TxKind::Call(tx.to),
            Transaction::EIP7702Transaction(tx) => TxKind::Call(tx.to),
            Transaction::PrivilegedL2Transaction(tx) => tx.to.clone(),
            Transaction::FeeTokenTransaction(tx) => tx.to.clone(),
        }
    }

    pub fn value(&self) -> U256 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.value,
            Transaction::EIP2930Transaction(tx) => tx.value,
            Transaction::EIP1559Transaction(tx) => tx.value,
            Transaction::EIP4844Transaction(tx) => tx.value,
            Transaction::EIP7702Transaction(tx) => tx.value,
            Transaction::PrivilegedL2Transaction(tx) => tx.value,
            Transaction::FeeTokenTransaction(tx) => tx.value,
        }
    }

    pub fn max_priority_fee(&self) -> Option<u64> {
        match self {
            Transaction::LegacyTransaction(_tx) => None,
            Transaction::EIP2930Transaction(_tx) => None,
            Transaction::EIP1559Transaction(tx) => Some(tx.max_priority_fee_per_gas),
            Transaction::EIP4844Transaction(tx) => Some(tx.max_priority_fee_per_gas),
            Transaction::EIP7702Transaction(tx) => Some(tx.max_priority_fee_per_gas),
            Transaction::PrivilegedL2Transaction(tx) => Some(tx.max_priority_fee_per_gas),
            Transaction::FeeTokenTransaction(tx) => Some(tx.max_priority_fee_per_gas),
        }
    }

    pub fn chain_id(&self) -> Option<u64> {
        match self {
            Transaction::LegacyTransaction(tx) => derive_legacy_chain_id(tx.v),
            Transaction::EIP2930Transaction(tx) => Some(tx.chain_id),
            Transaction::EIP1559Transaction(tx) => Some(tx.chain_id),
            Transaction::EIP4844Transaction(tx) => Some(tx.chain_id),
            Transaction::EIP7702Transaction(tx) => Some(tx.chain_id),
            Transaction::PrivilegedL2Transaction(tx) => Some(tx.chain_id),
            Transaction::FeeTokenTransaction(tx) => Some(tx.chain_id),
        }
    }

    pub fn access_list(&self) -> &AccessList {
        static EMPTY_ACCESS_LIST: AccessList = Vec::new();
        match self {
            Transaction::LegacyTransaction(_tx) => &EMPTY_ACCESS_LIST,
            Transaction::EIP2930Transaction(tx) => &tx.access_list,
            Transaction::EIP1559Transaction(tx) => &tx.access_list,
            Transaction::EIP4844Transaction(tx) => &tx.access_list,
            Transaction::EIP7702Transaction(tx) => &tx.access_list,
            Transaction::PrivilegedL2Transaction(tx) => &tx.access_list,
            Transaction::FeeTokenTransaction(tx) => &tx.access_list,
        }
    }
    pub fn authorization_list(&self) -> Option<&AuthorizationList> {
        match self {
            Transaction::LegacyTransaction(_) => None,
            Transaction::EIP2930Transaction(_) => None,
            Transaction::EIP1559Transaction(_) => None,
            Transaction::EIP4844Transaction(_) => None,
            Transaction::EIP7702Transaction(tx) => Some(&tx.authorization_list),
            Transaction::PrivilegedL2Transaction(_) => None,
            Transaction::FeeTokenTransaction(_) => None,
        }
    }

    pub fn nonce(&self) -> u64 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.nonce,
            Transaction::EIP2930Transaction(tx) => tx.nonce,
            Transaction::EIP1559Transaction(tx) => tx.nonce,
            Transaction::EIP4844Transaction(tx) => tx.nonce,
            Transaction::EIP7702Transaction(tx) => tx.nonce,
            Transaction::PrivilegedL2Transaction(tx) => tx.nonce,
            Transaction::FeeTokenTransaction(tx) => tx.nonce,
        }
    }

    pub fn data(&self) -> &Bytes {
        match self {
            Transaction::LegacyTransaction(tx) => &tx.data,
            Transaction::EIP2930Transaction(tx) => &tx.data,
            Transaction::EIP1559Transaction(tx) => &tx.data,
            Transaction::EIP4844Transaction(tx) => &tx.data,
            Transaction::EIP7702Transaction(tx) => &tx.data,
            Transaction::PrivilegedL2Transaction(tx) => &tx.data,
            Transaction::FeeTokenTransaction(tx) => &tx.data,
        }
    }

    pub fn blob_versioned_hashes(&self) -> Vec<H256> {
        match self {
            Transaction::LegacyTransaction(_) => Vec::new(),
            Transaction::EIP2930Transaction(_) => Vec::new(),
            Transaction::EIP1559Transaction(_) => Vec::new(),
            Transaction::EIP4844Transaction(tx) => tx.blob_versioned_hashes.clone(),
            Transaction::EIP7702Transaction(_) => Vec::new(),
            Transaction::PrivilegedL2Transaction(_) => Vec::new(),
            Transaction::FeeTokenTransaction(_) => Vec::new(),
        }
    }

    pub fn max_fee_per_blob_gas(&self) -> Option<U256> {
        match self {
            Transaction::LegacyTransaction(_) => None,
            Transaction::EIP2930Transaction(_) => None,
            Transaction::EIP1559Transaction(_) => None,
            Transaction::EIP4844Transaction(tx) => Some(tx.max_fee_per_blob_gas),
            Transaction::EIP7702Transaction(_) => None,
            Transaction::PrivilegedL2Transaction(_) => None,
            Transaction::FeeTokenTransaction(_) => None,
        }
    }

    pub fn is_contract_creation(&self) -> bool {
        match &self {
            Transaction::LegacyTransaction(t) => matches!(t.to, TxKind::Create),
            Transaction::EIP2930Transaction(t) => matches!(t.to, TxKind::Create),
            Transaction::EIP1559Transaction(t) => matches!(t.to, TxKind::Create),
            Transaction::EIP4844Transaction(_) => false,
            Transaction::EIP7702Transaction(_) => false,
            Transaction::PrivilegedL2Transaction(t) => matches!(t.to, TxKind::Create),
            Transaction::FeeTokenTransaction(t) => matches!(t.to, TxKind::Create),
        }
    }

    pub fn is_privileged(&self) -> bool {
        matches!(self, Transaction::PrivilegedL2Transaction(_))
    }

    pub fn max_fee_per_gas(&self) -> Option<u64> {
        match self {
            Transaction::LegacyTransaction(_tx) => None,
            Transaction::EIP2930Transaction(_tx) => None,
            Transaction::EIP1559Transaction(tx) => Some(tx.max_fee_per_gas),
            Transaction::EIP4844Transaction(tx) => Some(tx.max_fee_per_gas),
            Transaction::EIP7702Transaction(tx) => Some(tx.max_fee_per_gas),
            Transaction::PrivilegedL2Transaction(tx) => Some(tx.max_fee_per_gas),
            Transaction::FeeTokenTransaction(tx) => Some(tx.max_fee_per_gas),
        }
    }

    fn compute_hash(&self) -> H256 {
        if let Transaction::PrivilegedL2Transaction(tx) = self {
            return tx.get_privileged_hash().unwrap_or_default();
        }
        crate::utils::keccak(self.encode_canonical_to_vec())
    }

    pub fn hash(&self) -> H256 {
        let inner_hash = match self {
            Transaction::LegacyTransaction(tx) => &tx.inner_hash,
            Transaction::EIP2930Transaction(tx) => &tx.inner_hash,
            Transaction::EIP1559Transaction(tx) => &tx.inner_hash,
            Transaction::EIP4844Transaction(tx) => &tx.inner_hash,
            Transaction::EIP7702Transaction(tx) => &tx.inner_hash,
            Transaction::PrivilegedL2Transaction(tx) => &tx.inner_hash,
            Transaction::FeeTokenTransaction(tx) => &tx.inner_hash,
        };

        *inner_hash.get_or_init(|| self.compute_hash())
    }

    pub fn gas_tip_cap(&self) -> u64 {
        self.max_priority_fee().unwrap_or(self.gas_price().as_u64())
    }

    pub fn gas_fee_cap(&self) -> u64 {
        self.max_fee_per_gas().unwrap_or(self.gas_price().as_u64())
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

pub fn recover_address_from_message(
    signature: Signature,
    message: &Bytes,
) -> Result<Address, EcdsaError> {
    // Hash message
    let payload = keccak(message);
    recover_address(signature, payload).map_err(EcdsaError::from)
}

// Half the secp256k1 curve order (n/2), i.e. the upper bound for a valid `s` value per EIP-2.
const SECP256K1_N_HALF: [u8; 32] =
    hex!("7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0");

fn signature_has_high_s(signature_bytes: &[u8; 65]) -> bool {
    signature_bytes[32..64] > SECP256K1_N_HALF[..]
}

#[cfg(all(
    not(feature = "zisk"),
    not(feature = "risc0"),
    not(feature = "sp1"),
    feature = "secp256k1"
))]
pub fn recover_address(signature: Signature, payload: H256) -> Result<Address, secp256k1::Error> {
    // Create signature
    let signature_bytes = signature.to_fixed_bytes();
    // EIP-2: reject high-s signatures (s > secp256k1n/2).
    if signature_has_high_s(&signature_bytes) {
        return Err(secp256k1::Error::InvalidSignature);
    }
    let signature = secp256k1::ecdsa::RecoverableSignature::from_compact(
        &signature_bytes[..64],
        secp256k1::ecdsa::RecoveryId::try_from(signature_bytes[64] as i32)?,
    )?;
    // Recover public key
    let public = secp256k1::SECP256K1.recover_ecdsa(
        &secp256k1::Message::from_digest(payload.to_fixed_bytes()),
        &signature,
    )?;
    // Hash public key to obtain address
    let hash = ethrex_crypto::keccak::keccak_hash(&public.serialize_uncompressed()[1..]);
    Ok(Address::from_slice(&hash[12..]))
}

#[cfg(any(
    feature = "zisk",
    feature = "risc0",
    feature = "sp1",
    not(feature = "secp256k1")
))]
pub fn recover_address(signature: Signature, payload: H256) -> Result<Address, k256::ecdsa::Error> {
    use sha2::Digest;
    use sha3::Keccak256;

    // Create signature
    let signature_bytes = signature.to_fixed_bytes();
    // EIP-2: signatures must use "low-s" (s <= secp256k1n/2).
    // Standard k256 rejects high-s signatures by default but it's best to leave this for 3 reasons:
    // 1. Make it more explicit
    // 2. Sometimes it can happen that the zkVM patch can have a different behavior than the original crate (shouldn't happen, but has happened). So we put this just in case.
    // 3. Fail fast
    if signature_has_high_s(&signature_bytes) {
        return Err(k256::ecdsa::Error::from_source("High-s signature"));
    }

    let signature = k256::ecdsa::Signature::from_slice(&signature_bytes[..64])?;

    let recovery_id_byte = signature_bytes[64];
    let recovery_id = k256::ecdsa::RecoveryId::from_byte(recovery_id_byte).ok_or(
        k256::ecdsa::Error::from_source("Failed to parse recovery id"),
    )?;

    // Recover public key
    let public = k256::ecdsa::VerifyingKey::recover_from_prehash(
        payload.as_bytes(),
        &signature,
        recovery_id,
    )?;

    let uncompressed = public.to_encoded_point(false);

    let mut uncompressed = uncompressed.to_bytes();

    let xy = &mut uncompressed[1..65];

    let hash = Keccak256::digest(xy);

    Ok(Address::from_slice(&hash[12..]))
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
            0x04 => Some(Self::EIP7702),
            0x7d => Some(Self::FeeToken),
            0x7e => Some(Self::Privileged),
            _ => None,
        }
    }
}

impl PrivilegedL2Transaction {
    /// Returns the formatted hash of the privileged transaction,
    /// or None if the transaction is not a privileged transaction.
    /// The hash is computed as keccak256(chain_id || from || to || transaction_id  || value || gas_limit || keccak256(calldata))
    pub fn get_privileged_hash(&self) -> Option<H256> {
        // Should this function be changed?
        let to = match self.to {
            TxKind::Call(to) => to,
            _ => return None,
        };

        let value = self.value.to_big_endian();

        // The nonce should be a U256,
        // in solidity the transactionId is a U256.
        let u256_nonce = U256::from(self.nonce);
        let nonce = u256_nonce.to_big_endian();

        Some(crate::utils::keccak(
            [
                U256::from(self.chain_id).to_big_endian().as_ref(),
                self.from.as_bytes(),
                to.as_bytes(),
                &nonce,
                &value,
                &U256::from(self.gas_limit).to_big_endian(),
                keccak(&self.data).as_bytes(),
            ]
            .concat(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub struct FeeTokenTransaction {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    pub to: TxKind,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub value: U256,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub data: Bytes,
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::AccessListItemWrapper>)]
    pub access_list: AccessList,
    #[rkyv(with=crate::rkyv_utils::H160Wrapper)]
    pub fee_token: Address,
    pub signature_y_parity: bool,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_r: U256,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub signature_s: U256,
    #[rkyv(with=rkyv::with::Skip)]
    pub inner_hash: OnceCell<H256>,
    #[rkyv(with=rkyv::with::Skip)]
    pub sender_cache: OnceCell<Address>,
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
                        // EIP7702
                        0x4 => EIP7702Transaction::decode(tx_bytes)
                            .map(Transaction::EIP7702Transaction),
                        // FeeTokenTransaction
                        0x7d => FeeTokenTransaction::decode(tx_bytes)
                            .map(Transaction::FeeTokenTransaction),
                        // PrivilegedL2Transaction
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
                Transaction::EIP7702Transaction(t) => t.encode(buf),
                Transaction::FeeTokenTransaction(t) => t.encode(buf),
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

    impl P2PTransaction {
        pub fn tx_type(&self) -> TxType {
            match self {
                P2PTransaction::LegacyTransaction(_) => TxType::Legacy,
                P2PTransaction::EIP2930Transaction(_) => TxType::EIP2930,
                P2PTransaction::EIP1559Transaction(_) => TxType::EIP1559,
                P2PTransaction::EIP4844TransactionWithBlobs(_) => TxType::EIP4844,
                P2PTransaction::EIP7702Transaction(_) => TxType::EIP7702,
                P2PTransaction::FeeTokenTransaction(_) => TxType::FeeToken,
            }
        }

        pub fn encode_canonical(&self, buf: &mut dyn bytes::BufMut) {
            match self {
                // Legacy transactions don't have a prefix
                P2PTransaction::LegacyTransaction(_) => {}
                _ => buf.put_u8(self.tx_type() as u8),
            }
            match self {
                P2PTransaction::LegacyTransaction(t) => t.encode(buf),
                P2PTransaction::EIP2930Transaction(t) => t.encode(buf),
                P2PTransaction::EIP1559Transaction(t) => t.encode(buf),
                P2PTransaction::EIP4844TransactionWithBlobs(t) => t.encode(buf),
                P2PTransaction::EIP7702Transaction(t) => t.encode(buf),
                P2PTransaction::FeeTokenTransaction(t) => t.encode(buf),
            };
        }

        pub fn encode_canonical_to_vec(&self) -> Vec<u8> {
            let mut buf = Vec::new();
            self.encode_canonical(&mut buf);
            buf
        }

        pub fn compute_hash(&self) -> H256 {
            match self {
                P2PTransaction::LegacyTransaction(t) => {
                    Transaction::LegacyTransaction(t.clone()).compute_hash()
                }
                P2PTransaction::EIP2930Transaction(t) => {
                    Transaction::EIP2930Transaction(t.clone()).compute_hash()
                }
                P2PTransaction::EIP1559Transaction(t) => {
                    Transaction::EIP1559Transaction(t.clone()).compute_hash()
                }
                P2PTransaction::EIP4844TransactionWithBlobs(t) => {
                    Transaction::EIP4844Transaction(t.tx.clone()).compute_hash()
                }
                P2PTransaction::EIP7702Transaction(t) => {
                    Transaction::EIP7702Transaction(t.clone()).compute_hash()
                }
                P2PTransaction::FeeTokenTransaction(t) => {
                    Transaction::FeeTokenTransaction(t.clone()).compute_hash()
                }
            }
        }
    }
}

// Serialization
// This is used for RPC messaging and passing data into a RISC-V zkVM

mod serde_impl {
    use ethereum_types::H160;
    use serde::Deserialize;
    use serde::{Deserializer, de::Error};
    use serde_json::Value;
    use std::{collections::HashMap, str::FromStr};

    #[cfg(feature = "c-kzg")]
    use crate::types::BYTES_PER_BLOB;
    use crate::types::{AccessListItem, AuthorizationTuple, BlobsBundleError};

    use super::*;

    impl Serialize for TxKind {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            match self {
                TxKind::Call(address) => serializer.serialize_str(&format!("{address:#x}")),
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

    impl From<&AccessListItem> for AccessListEntry {
        fn from(value: &AccessListItem) -> AccessListEntry {
            AccessListEntry {
                address: value.0,
                storage_keys: value.1.clone(),
            }
        }
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct AuthorizationTupleEntry {
        pub chain_id: U256,
        pub address: Address,
        #[serde(default, with = "crate::serde_utils::u64::hex_str")]
        pub nonce: u64,
        pub y_parity: U256,
        pub r: U256,
        pub s: U256,
    }

    impl From<&AuthorizationTuple> for AuthorizationTupleEntry {
        fn from(value: &AuthorizationTuple) -> AuthorizationTupleEntry {
            AuthorizationTupleEntry {
                chain_id: value.chain_id,
                address: value.address,
                nonce: value.nonce,
                y_parity: value.y_parity,
                r: value.r_signature,
                s: value.s_signature,
            }
        }
    }

    impl From<AuthorizationTupleEntry> for AuthorizationTuple {
        fn from(entry: AuthorizationTupleEntry) -> AuthorizationTuple {
            AuthorizationTuple {
                chain_id: entry.chain_id,
                address: entry.address,
                nonce: entry.nonce,
                y_parity: entry.y_parity,
                r_signature: entry.r,
                s_signature: entry.s,
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

    impl Serialize for EIP7702Transaction {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut struct_serializer = serializer.serialize_struct("Eip7702Transaction", 15)?;
            struct_serializer.serialize_field("type", &TxType::EIP7702)?;
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
            struct_serializer.serialize_field(
                "authorizationList",
                &self
                    .authorization_list
                    .iter()
                    .map(AuthorizationTupleEntry::from)
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

    impl Serialize for PrivilegedL2Transaction {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut struct_serializer = serializer.serialize_struct("Eip1559Transaction", 14)?;
            struct_serializer.serialize_field("type", &TxType::Privileged)?;
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
            struct_serializer.serialize_field("sender", &self.from)?;
            struct_serializer.end()
        }
    }

    impl Serialize for FeeTokenTransaction {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut struct_serializer = serializer.serialize_struct("FeeTokenTransaction", 15)?;
            struct_serializer.serialize_field("type", &TxType::FeeToken)?;
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
            struct_serializer.serialize_field("feeToken", &format!("{:#x}", self.fee_token))?;
            struct_serializer.serialize_field("chainId", &format!("{:#x}", self.chain_id))?;
            struct_serializer
                .serialize_field("yParity", &format!("{:#x}", self.signature_y_parity as u8))?;
            struct_serializer
                .serialize_field("v", &format!("{:#x}", self.signature_y_parity as u8))?;
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
                serde_json::from_value::<TxType>(map.remove("type").unwrap_or(Value::default()))
                    .unwrap_or_else(|_| {
                        if map.contains_key("tx_type") {
                            return TxType::Privileged;
                        }
                        TxType::EIP1559
                    });

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
                TxType::EIP7702 => {
                    EIP7702Transaction::deserialize(serde::de::value::MapDeserializer::new(iter))
                        .map(Transaction::EIP7702Transaction)
                        .map_err(|e| {
                            serde::de::Error::custom(format!("Couldn't Deserialize EIP7702 {e}"))
                        })
                }
                TxType::Privileged => PrivilegedL2Transaction::deserialize(
                    serde::de::value::MapDeserializer::new(iter),
                )
                .map(Transaction::PrivilegedL2Transaction)
                .map_err(|e| {
                    serde::de::Error::custom(format!("Couldn't Deserialize Privileged: {e}"))
                }),
                TxType::FeeToken => {
                    FeeTokenTransaction::deserialize(serde::de::value::MapDeserializer::new(iter))
                        .map(Transaction::FeeTokenTransaction)
                        .map_err(|e| {
                            serde::de::Error::custom(format!("Couldn't Deserialize FeeToken {e}"))
                        })
                }
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

    fn deserialize_field<'de, T, D>(
        map: &mut HashMap<String, serde_json::Value>,
        key: &str,
    ) -> Result<T, D::Error>
    where
        D: serde::Deserializer<'de>,
        T: serde::de::DeserializeOwned,
    {
        map.remove(key)
            .ok_or_else(|| D::Error::custom(format!("Missing field: {key}")))
            .and_then(|value| {
                serde_json::from_value(value).map_err(|err| D::Error::custom(err.to_string()))
            })
    }

    impl<'de> Deserialize<'de> for LegacyTransaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;

            Ok(LegacyTransaction {
                nonce: deserialize_field::<U256, D>(&mut map, "nonce")?.as_u64(),
                gas_price: deserialize_field::<U256, D>(&mut map, "gasPrice")?,
                gas: deserialize_field::<U256, D>(&mut map, "gas")?.as_u64(),
                to: deserialize_field::<TxKind, D>(&mut map, "to")?,
                value: deserialize_field::<U256, D>(&mut map, "value")?,
                data: deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?,
                v: deserialize_field::<U256, D>(&mut map, "v")?,
                r: deserialize_field::<U256, D>(&mut map, "r")?,
                s: deserialize_field::<U256, D>(&mut map, "s")?,
                ..Default::default()
            })
        }
    }

    impl<'de> Deserialize<'de> for EIP2930Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;

            Ok(EIP2930Transaction {
                chain_id: deserialize_field::<U256, D>(&mut map, "chainId")?.as_u64(),
                nonce: deserialize_field::<U256, D>(&mut map, "nonce")?.as_u64(),
                gas_price: deserialize_field::<U256, D>(&mut map, "gasPrice")?,
                gas_limit: deserialize_field::<U256, D>(&mut map, "gas")?.as_u64(),
                to: deserialize_field::<TxKind, D>(&mut map, "to")?,
                value: deserialize_field::<U256, D>(&mut map, "value")?,
                data: deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?,
                access_list: deserialize_field::<Vec<AccessListEntry>, D>(&mut map, "accessList")?
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                signature_y_parity: u8::from_str_radix(
                    deserialize_field::<String, D>(&mut map, "yParity")?.trim_start_matches("0x"),
                    16,
                )
                .map_err(serde::de::Error::custom)?
                    != 0,
                signature_r: deserialize_field::<U256, D>(&mut map, "r")?,
                signature_s: deserialize_field::<U256, D>(&mut map, "s")?,
                ..Default::default()
            })
        }
    }

    impl<'de> Deserialize<'de> for EIP1559Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;
            Ok(EIP1559Transaction {
                chain_id: deserialize_field::<U256, D>(&mut map, "chainId")?.as_u64(),
                nonce: deserialize_field::<U256, D>(&mut map, "nonce")?.as_u64(),
                max_priority_fee_per_gas: deserialize_field::<U256, D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?
                .as_u64(),
                max_fee_per_gas: deserialize_field::<U256, D>(&mut map, "maxFeePerGas")?.as_u64(),
                gas_limit: deserialize_field::<U256, D>(&mut map, "gas")?.as_u64(),
                to: deserialize_field::<TxKind, D>(&mut map, "to")?,
                value: deserialize_field::<U256, D>(&mut map, "value")?,
                data: deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?,
                access_list: deserialize_field::<Vec<AccessListEntry>, D>(&mut map, "accessList")?
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                signature_y_parity: u8::from_str_radix(
                    deserialize_field::<String, D>(&mut map, "yParity")?.trim_start_matches("0x"),
                    16,
                )
                .map_err(serde::de::Error::custom)?
                    != 0,
                signature_r: deserialize_field::<U256, D>(&mut map, "r")?,
                signature_s: deserialize_field::<U256, D>(&mut map, "s")?,
                ..Default::default()
            })
        }
    }

    impl<'de> Deserialize<'de> for EIP4844Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;

            Ok(EIP4844Transaction {
                chain_id: deserialize_field::<U256, D>(&mut map, "chainId")?.as_u64(),
                nonce: deserialize_field::<U256, D>(&mut map, "nonce")?.as_u64(),
                max_priority_fee_per_gas: deserialize_field::<U256, D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?
                .as_u64(),
                max_fee_per_gas: deserialize_field::<U256, D>(&mut map, "maxFeePerGas")?.as_u64(),
                gas: deserialize_field::<U256, D>(&mut map, "gas")?.as_u64(),
                to: deserialize_field::<Address, D>(&mut map, "to")?,
                value: deserialize_field::<U256, D>(&mut map, "value")?,
                data: deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?,
                access_list: deserialize_field::<Vec<AccessListEntry>, D>(&mut map, "accessList")?
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                max_fee_per_blob_gas: deserialize_field::<U256, D>(&mut map, "maxFeePerBlobGas")?,
                blob_versioned_hashes: deserialize_field::<Vec<H256>, D>(
                    &mut map,
                    "blobVersionedHashes",
                )?,
                signature_y_parity: u8::from_str_radix(
                    deserialize_field::<String, D>(&mut map, "yParity")?.trim_start_matches("0x"),
                    16,
                )
                .map_err(serde::de::Error::custom)?
                    != 0,
                signature_r: deserialize_field::<U256, D>(&mut map, "r")?,
                signature_s: deserialize_field::<U256, D>(&mut map, "s")?,
                ..Default::default()
            })
        }
    }

    impl<'de> Deserialize<'de> for EIP7702Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;

            Ok(EIP7702Transaction {
                chain_id: deserialize_field::<U256, D>(&mut map, "chainId")?.as_u64(),
                nonce: deserialize_field::<U256, D>(&mut map, "nonce")?.as_u64(),
                max_priority_fee_per_gas: deserialize_field::<U256, D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?
                .as_u64(),
                max_fee_per_gas: deserialize_field::<U256, D>(&mut map, "maxFeePerGas")?.as_u64(),
                gas_limit: deserialize_field::<U256, D>(&mut map, "gas")?.as_u64(),
                to: deserialize_field::<Address, D>(&mut map, "to")?,
                value: deserialize_field::<U256, D>(&mut map, "value")?,
                data: deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?,
                access_list: deserialize_field::<Vec<AccessListEntry>, D>(&mut map, "accessList")?
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                authorization_list: deserialize_field::<Vec<AuthorizationTupleEntry>, D>(
                    &mut map,
                    "authorizationList",
                )?
                .into_iter()
                .map(AuthorizationTuple::from)
                .collect::<Vec<_>>(),
                signature_y_parity: u8::from_str_radix(
                    deserialize_field::<String, D>(&mut map, "yParity")?.trim_start_matches("0x"),
                    16,
                )
                .map_err(serde::de::Error::custom)?
                    != 0,
                signature_r: deserialize_field::<U256, D>(&mut map, "r")?,
                signature_s: deserialize_field::<U256, D>(&mut map, "s")?,
                ..Default::default()
            })
        }
    }

    impl<'de> Deserialize<'de> for PrivilegedL2Transaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;

            Ok(PrivilegedL2Transaction {
                chain_id: deserialize_field::<U256, D>(&mut map, "chainId")?.as_u64(),
                nonce: deserialize_field::<U256, D>(&mut map, "nonce")?.as_u64(),
                max_priority_fee_per_gas: deserialize_field::<U256, D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?
                .as_u64(),
                max_fee_per_gas: deserialize_field::<U256, D>(&mut map, "maxFeePerGas")?.as_u64(),
                gas_limit: deserialize_field::<U256, D>(&mut map, "gas")?.as_u64(),
                to: deserialize_field::<TxKind, D>(&mut map, "to")?,
                value: deserialize_field::<U256, D>(&mut map, "value")?,
                data: deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?,
                access_list: deserialize_field::<Vec<AccessListEntry>, D>(&mut map, "accessList")?
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                from: deserialize_field::<Address, D>(&mut map, "sender")?,
                ..Default::default()
            })
        }
    }

    impl<'de> Deserialize<'de> for FeeTokenTransaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;

            Ok(FeeTokenTransaction {
                chain_id: deserialize_field::<U256, D>(&mut map, "chainId")?.as_u64(),
                nonce: deserialize_field::<U256, D>(&mut map, "nonce")?.as_u64(),
                max_priority_fee_per_gas: deserialize_field::<U256, D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?
                .as_u64(),
                max_fee_per_gas: deserialize_field::<U256, D>(&mut map, "maxFeePerGas")?.as_u64(),
                gas_limit: deserialize_field::<U256, D>(&mut map, "gas")?.as_u64(),
                to: deserialize_field::<TxKind, D>(&mut map, "to")?,
                value: deserialize_field::<U256, D>(&mut map, "value")?,
                data: deserialize_input_field(&mut map).map_err(serde::de::Error::custom)?,
                access_list: deserialize_field::<Vec<AccessListEntry>, D>(&mut map, "accessList")?
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                fee_token: deserialize_field::<Address, D>(&mut map, "feeToken")?,
                signature_y_parity: u8::from_str_radix(
                    deserialize_field::<String, D>(&mut map, "yParity")?.trim_start_matches("0x"),
                    16,
                )
                .map_err(serde::de::Error::custom)?
                    != 0,
                signature_r: deserialize_field::<U256, D>(&mut map, "r")?,
                signature_s: deserialize_field::<U256, D>(&mut map, "s")?,
                ..Default::default()
            })
        }
    }

    #[derive(Debug, thiserror::Error)]
    pub enum GenericTransactionError {
        #[error("Invalid transaction type: {0}")]
        InvalidTxType(TxType),
        #[error("Blob bundle error: {0}")]
        BlobBundleError(#[from] BlobsBundleError),
        #[error("Missing field: {0}")]
        MissingField(String),
    }

    /// Unsigned Transaction struct generic to all types which may not contain all required transaction fields
    /// Used to perform gas estimations and access list creation
    #[derive(Deserialize, Debug, PartialEq, Clone, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct GenericTransaction {
        #[serde(default)]
        pub r#type: TxType,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub nonce: Option<u64>,
        pub to: TxKind,
        #[serde(default)]
        pub from: Address,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub gas: Option<u64>,
        #[serde(default)]
        pub value: U256,
        #[serde(default, with = "crate::serde_utils::u64::hex_str")]
        pub gas_price: u64,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub max_priority_fee_per_gas: Option<u64>,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub max_fee_per_gas: Option<u64>,
        pub max_fee_per_blob_gas: Option<U256>,
        #[serde(default)]
        pub access_list: Vec<AccessListEntry>,
        #[serde(default)]
        pub fee_token: Option<Address>,
        #[serde(default)]
        pub authorization_list: Option<Vec<AuthorizationTupleEntry>>,
        #[serde(default)]
        pub blob_versioned_hashes: Vec<H256>,
        pub wrapper_version: Option<u8>,
        #[serde(default, with = "crate::serde_utils::bytes::vec")]
        pub blobs: Vec<Bytes>,
        #[serde(default, with = "crate::serde_utils::u64::hex_str_opt")]
        pub chain_id: Option<u64>,
        // rename is needed here so we dont attempt to deserialize the `input` field rather than the remainder of the fields
        #[serde(
            flatten,
            rename = "input_or_data",
            deserialize_with = "deserialize_input",
            serialize_with = "crate::serde_utils::bytes::serialize"
        )]
        pub input: Bytes,
    }
    /// Custom deserialization function to parse either `data` or `input` fields, or both as long as they have the same value
    pub fn deserialize_input<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
    where
        D: Deserializer<'de>,
    {
        // The input field can be named either input or data
        // In case we have both fields both should be named the same
        let variables = HashMap::<String, Value>::deserialize(deserializer)?;
        let data = variables.get("data");
        let input = variables.get("input");
        let value = match (data, input) {
            // This replaces `default` attribute for this custom implementation
            (None, None) => return Ok(Bytes::new()),
            (None, Some(val)) => val,
            (Some(val), None) => val,
            (Some(val_a), Some(val_b)) => {
                if val_a == val_b {
                    val_a
                } else {
                    return Err(D::Error::custom(
                        "Transaction has both `data` and `input` fields with different values",
                    ));
                }
            }
        };
        let value = String::deserialize(value).map_err(D::Error::custom)?;
        let bytes = hex::decode(value.trim_start_matches("0x"))
            .map_err(|e| D::Error::custom(e.to_string()))?;
        Ok(Bytes::from(bytes))
    }

    impl From<EIP1559Transaction> for GenericTransaction {
        fn from(value: EIP1559Transaction) -> Self {
            Self {
                r#type: TxType::EIP1559,
                nonce: Some(value.nonce),
                to: value.to,
                gas: Some(value.gas_limit),
                value: value.value,
                input: value.data.clone(),
                gas_price: value.max_fee_per_gas,
                max_priority_fee_per_gas: Some(value.max_priority_fee_per_gas),
                max_fee_per_gas: Some(value.max_fee_per_gas),
                max_fee_per_blob_gas: None,
                access_list: value
                    .access_list
                    .iter()
                    .map(AccessListEntry::from)
                    .collect(),
                fee_token: None,
                authorization_list: None,
                blob_versioned_hashes: vec![],
                blobs: vec![],
                wrapper_version: None,
                chain_id: Some(value.chain_id),
                from: Address::default(),
            }
        }
    }

    impl TryFrom<GenericTransaction> for EIP1559Transaction {
        type Error = GenericTransactionError;

        fn try_from(value: GenericTransaction) -> Result<Self, Self::Error> {
            if value.r#type != TxType::EIP1559 {
                return Err(GenericTransactionError::InvalidTxType(value.r#type));
            }

            Ok(Self {
                nonce: value.nonce.unwrap_or_default(),
                to: value.to,
                gas_limit: value.gas.unwrap_or_default(),
                value: value.value,
                data: value.input.clone(),
                max_priority_fee_per_gas: value.max_priority_fee_per_gas.unwrap_or_default(),
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(value.gas_price),
                access_list: value
                    .access_list
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                chain_id: value.chain_id.unwrap_or_default(),
                ..Default::default()
            })
        }
    }

    impl From<EIP4844Transaction> for GenericTransaction {
        fn from(value: EIP4844Transaction) -> Self {
            Self {
                r#type: TxType::EIP4844,
                nonce: Some(value.nonce),
                to: TxKind::Call(value.to),
                gas: Some(value.gas),
                value: value.value,
                input: value.data.clone(),
                gas_price: value.max_fee_per_gas,
                max_priority_fee_per_gas: Some(value.max_priority_fee_per_gas),
                max_fee_per_gas: Some(value.max_fee_per_gas),
                max_fee_per_blob_gas: Some(value.max_fee_per_blob_gas),
                access_list: value
                    .access_list
                    .iter()
                    .map(AccessListEntry::from)
                    .collect(),
                fee_token: None,
                authorization_list: None,
                blob_versioned_hashes: value.blob_versioned_hashes,
                blobs: vec![],
                wrapper_version: None,
                chain_id: Some(value.chain_id),
                from: Address::default(),
            }
        }
    }

    #[cfg(feature = "c-kzg")]
    impl TryFrom<GenericTransaction> for WrappedEIP4844Transaction {
        type Error = GenericTransactionError;

        fn try_from(value: GenericTransaction) -> Result<Self, Self::Error> {
            let blobs = value
                .blobs
                .iter()
                .map(|bytes| {
                    let slice = bytes.as_ref();
                    let mut blob = [0u8; BYTES_PER_BLOB];
                    blob.copy_from_slice(slice);
                    blob
                })
                .collect();

            let wrapper_version = value.wrapper_version;
            Ok(Self {
                tx: value.try_into()?,
                wrapper_version,
                blobs_bundle: BlobsBundle::create_from_blobs(&blobs, wrapper_version)?,
            })
        }
    }

    impl TryFrom<GenericTransaction> for EIP4844Transaction {
        type Error = GenericTransactionError;

        fn try_from(value: GenericTransaction) -> Result<Self, Self::Error> {
            if value.r#type != TxType::EIP4844 {
                return Err(GenericTransactionError::InvalidTxType(value.r#type));
            }
            Ok(Self {
                nonce: value.nonce.unwrap_or_default(),
                to: match value.to {
                    TxKind::Call(to) => to,
                    _ => H160::default(),
                },
                gas: value.gas.unwrap_or_default(),
                value: value.value,
                data: value.input.clone(),
                max_priority_fee_per_gas: value.max_priority_fee_per_gas.unwrap_or_default(),
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(value.gas_price),
                max_fee_per_blob_gas: value.max_fee_per_blob_gas.unwrap_or_default(),
                access_list: value
                    .access_list
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                blob_versioned_hashes: value.blob_versioned_hashes,
                chain_id: value.chain_id.unwrap_or_default(),
                ..Default::default()
            })
        }
    }

    impl From<EIP7702Transaction> for GenericTransaction {
        fn from(value: EIP7702Transaction) -> Self {
            Self {
                r#type: TxType::EIP7702,
                nonce: Some(value.nonce),
                to: TxKind::Call(value.to),
                gas: Some(value.gas_limit),
                value: value.value,
                input: value.data.clone(),
                gas_price: value.max_fee_per_gas,
                max_priority_fee_per_gas: Some(value.max_priority_fee_per_gas),
                max_fee_per_gas: Some(value.max_fee_per_gas),
                max_fee_per_blob_gas: None,
                access_list: value
                    .access_list
                    .iter()
                    .map(AccessListEntry::from)
                    .collect(),
                fee_token: None,
                authorization_list: Some(
                    value
                        .authorization_list
                        .iter()
                        .map(AuthorizationTupleEntry::from)
                        .collect(),
                ),
                blob_versioned_hashes: vec![],
                blobs: vec![],
                wrapper_version: None,
                chain_id: Some(value.chain_id),
                from: Address::default(),
            }
        }
    }

    impl TryFrom<GenericTransaction> for EIP7702Transaction {
        type Error = GenericTransactionError;

        fn try_from(value: GenericTransaction) -> Result<Self, Self::Error> {
            if value.r#type != TxType::EIP7702 {
                return Err(GenericTransactionError::InvalidTxType(value.r#type));
            }
            let TxKind::Call(to) = value.to else {
                return Err(GenericTransactionError::MissingField("to".to_owned()));
            };
            Ok(Self {
                chain_id: value.chain_id.unwrap_or_default(),
                nonce: value.nonce.unwrap_or_default(),
                max_priority_fee_per_gas: value.max_priority_fee_per_gas.unwrap_or_default(),
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(value.gas_price),
                gas_limit: value.gas.unwrap_or_default(),
                to,
                value: value.value,
                data: value.input,
                access_list: value
                    .access_list
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                authorization_list: value
                    .authorization_list
                    .unwrap_or_default()
                    .into_iter()
                    .map(AuthorizationTuple::from)
                    .collect(),
                ..Default::default()
            })
        }
    }

    impl From<PrivilegedL2Transaction> for GenericTransaction {
        fn from(value: PrivilegedL2Transaction) -> Self {
            Self {
                r#type: TxType::Privileged,
                nonce: Some(value.nonce),
                to: value.to,
                gas: Some(value.gas_limit),
                value: value.value,
                input: value.data.clone(),
                gas_price: value.max_fee_per_gas,
                max_priority_fee_per_gas: Some(value.max_priority_fee_per_gas),
                max_fee_per_gas: Some(value.max_fee_per_gas),
                max_fee_per_blob_gas: None,
                access_list: value
                    .access_list
                    .iter()
                    .map(AccessListEntry::from)
                    .collect(),
                fee_token: None,
                authorization_list: None,
                blob_versioned_hashes: vec![],
                blobs: vec![],
                wrapper_version: None,
                chain_id: Some(value.chain_id),
                from: value.from,
            }
        }
    }

    impl TryFrom<GenericTransaction> for PrivilegedL2Transaction {
        type Error = GenericTransactionError;

        fn try_from(value: GenericTransaction) -> Result<Self, Self::Error> {
            if value.r#type != TxType::Privileged {
                return Err(GenericTransactionError::InvalidTxType(value.r#type));
            }
            Ok(Self {
                nonce: value.nonce.unwrap_or_default(),
                to: value.to,
                gas_limit: value.gas.unwrap_or_default(),
                value: value.value,
                data: value.input.clone(),
                max_priority_fee_per_gas: value.max_priority_fee_per_gas.unwrap_or_default(),
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(value.gas_price),
                access_list: value
                    .access_list
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                chain_id: value.chain_id.unwrap_or_default(),
                from: value.from,
                ..Default::default()
            })
        }
    }

    impl From<FeeTokenTransaction> for GenericTransaction {
        fn from(value: FeeTokenTransaction) -> Self {
            Self {
                r#type: TxType::FeeToken,
                nonce: Some(value.nonce),
                to: value.to,
                gas: Some(value.gas_limit),
                value: value.value,
                input: value.data.clone(),
                gas_price: value.max_fee_per_gas,
                max_priority_fee_per_gas: Some(value.max_priority_fee_per_gas),
                max_fee_per_gas: Some(value.max_fee_per_gas),
                max_fee_per_blob_gas: None,
                access_list: value
                    .access_list
                    .iter()
                    .map(AccessListEntry::from)
                    .collect(),
                fee_token: Some(value.fee_token),
                authorization_list: None,
                blob_versioned_hashes: vec![],
                blobs: vec![],
                chain_id: Some(value.chain_id),
                from: Address::default(),
                wrapper_version: None,
            }
        }
    }

    impl TryFrom<GenericTransaction> for FeeTokenTransaction {
        type Error = GenericTransactionError;

        fn try_from(value: GenericTransaction) -> Result<Self, Self::Error> {
            if value.r#type != TxType::FeeToken {
                return Err(GenericTransactionError::InvalidTxType(value.r#type));
            }

            Ok(Self {
                nonce: value.nonce.unwrap_or_default(),
                to: value.to,
                gas_limit: value.gas.unwrap_or_default(),
                value: value.value,
                data: value.input.clone(),
                max_priority_fee_per_gas: value.max_priority_fee_per_gas.unwrap_or_default(),
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(value.gas_price),
                access_list: value
                    .access_list
                    .into_iter()
                    .map(|v| (v.address, v.storage_keys))
                    .collect::<Vec<_>>(),
                fee_token: value
                    .fee_token
                    .ok_or(GenericTransactionError::MissingField(
                        "fee token".to_owned(),
                    ))?,
                chain_id: value.chain_id.unwrap_or_default(),
                ..Default::default()
            })
        }
    }

    impl From<LegacyTransaction> for GenericTransaction {
        fn from(value: LegacyTransaction) -> Self {
            Self {
                r#type: TxType::Legacy,
                nonce: Some(value.nonce),
                to: value.to,
                from: Address::default(),
                gas: Some(value.gas),
                value: value.value,
                gas_price: value.gas_price.as_u64(),
                max_priority_fee_per_gas: None,
                max_fee_per_gas: None,
                max_fee_per_blob_gas: None,
                access_list: vec![],
                fee_token: None,
                authorization_list: None,
                blob_versioned_hashes: vec![],
                blobs: vec![],
                wrapper_version: None,
                chain_id: None,
                input: value.data,
            }
        }
    }

    impl From<EIP2930Transaction> for GenericTransaction {
        fn from(value: EIP2930Transaction) -> Self {
            Self {
                r#type: TxType::EIP2930,
                nonce: Some(value.nonce),
                to: value.to,
                from: Address::default(),
                gas: Some(value.gas_limit),
                value: value.value,
                gas_price: value.gas_price.as_u64(),
                max_priority_fee_per_gas: None,
                max_fee_per_gas: None,
                max_fee_per_blob_gas: None,
                access_list: value
                    .access_list
                    .into_iter()
                    .map(|(address, storage_keys)| AccessListEntry {
                        address,
                        storage_keys,
                    })
                    .collect(),
                fee_token: None,
                authorization_list: None,
                blob_versioned_hashes: vec![],
                blobs: vec![],
                wrapper_version: None,
                chain_id: Some(value.chain_id),
                input: value.data,
            }
        }
    }

    impl From<Transaction> for GenericTransaction {
        fn from(value: Transaction) -> Self {
            match value {
                Transaction::LegacyTransaction(tx) => tx.into(),
                Transaction::EIP2930Transaction(tx) => tx.into(),
                Transaction::EIP1559Transaction(tx) => tx.into(),
                Transaction::EIP4844Transaction(tx) => tx.into(),
                Transaction::EIP7702Transaction(tx) => tx.into(),
                Transaction::PrivilegedL2Transaction(tx) => tx.into(),
                Transaction::FeeTokenTransaction(tx) => tx.into(),
            }
        }
    }
}

mod mempool {
    use super::*;
    use std::{
        cmp::Ordering,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MempoolTransaction {
        // Unix timestamp (in microseconds) created once the transaction reached the MemPool
        timestamp: u128,
        sender: Address,
        inner: Arc<Transaction>,
    }

    impl MempoolTransaction {
        pub fn new(tx: Transaction, sender: Address) -> Self {
            Self {
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Invalid system time")
                    .as_micros(),
                sender,
                inner: Arc::new(tx),
            }
        }
        pub fn time(&self) -> u128 {
            self.timestamp
        }

        pub fn sender(&self) -> Address {
            self.sender
        }

        pub fn transaction(&self) -> &Transaction {
            &self.inner
        }
    }

    impl RLPEncode for MempoolTransaction {
        fn encode(&self, buf: &mut dyn bytes::BufMut) {
            Encoder::new(buf)
                .encode_field(&self.timestamp)
                .encode_field(&*self.inner)
                .finish();
        }
    }

    impl RLPDecode for MempoolTransaction {
        fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
            let decoder = Decoder::new(rlp)?;
            let (timestamp, decoder) = decoder.decode_field("timestamp")?;
            let (sender, decoder) = decoder.decode_field("sender")?;
            let (inner, decoder) = decoder.decode_field("inner")?;
            Ok((
                Self {
                    timestamp,
                    sender,
                    inner: Arc::new(inner),
                },
                decoder.finish()?,
            ))
        }
    }

    impl std::ops::Deref for MempoolTransaction {
        type Target = Transaction;

        fn deref(&self) -> &Self::Target {
            &self.inner
        }
    }

    // Orders transactions by lowest nonce, if the nonce is equal, orders by highest timestamp
    impl Ord for MempoolTransaction {
        fn cmp(&self, other: &Self) -> Ordering {
            match (self.tx_type(), other.tx_type()) {
                (TxType::Privileged, TxType::Privileged) => {
                    return self.nonce().cmp(&other.nonce());
                }
                (TxType::Privileged, _) => return Ordering::Less,
                (_, TxType::Privileged) => return Ordering::Greater,
                _ => (),
            };
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
    use crate::types::{
        AuthorizationTuple, BlockBody, Receipt, compute_receipts_root, compute_transactions_root,
    };
    use ethereum_types::H160;
    use hex_literal::hex;
    use serde_impl::{AccessListEntry, GenericTransaction};
    use std::str::FromStr;

    #[test]
    fn test_compute_transactions_root() {
        let mut body = BlockBody::empty();
        let tx = LegacyTransaction {
            nonce: 0,
            gas_price: U256::from(0x0a),
            gas: 0x05f5e100,
            to: TxKind::Call(hex!("1000000000000000000000000000000000000000").into()),
            value: 0.into(),
            data: Default::default(),
            v: U256::from(0x1b),
            r: U256::from_big_endian(&hex!(
                "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
            )),
            s: U256::from_big_endian(&hex!(
                "5f6e3f188e3e6eab7d7d3b6568f5eac7d687b08d307d3154ccd8c87b4630509b"
            )),
            ..Default::default()
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
            gas_price: U256::from(0x2dbf1f9a_u64),
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
            ..Default::default()
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
            gas_price: U256::from(1001000000u64),
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
            ..Default::default()
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
            ..Default::default()
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
            "input":"0x010203040506",
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
            nonce: Some(2),
            to: TxKind::Create,
            from: Address::from_slice(
                &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
            ),
            gas: Some(0x5208),
            value: U256::from(1),
            input: Bytes::from(hex::decode("010203040506").unwrap()),
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
            fee_token: None,
            blob_versioned_hashes: Default::default(),
            blobs: Default::default(),
            wrapper_version: None,
            chain_id: Default::default(),
            authorization_list: None,
        };
        assert_eq!(
            deserialized_generic_transaction,
            serde_json::from_str(generic_transaction).unwrap()
        )
    }

    #[test]
    fn deserialize_generic_transaction_with_data_and_input_fields() {
        let generic_transaction = r#"{
            "type":"0x01",
            "nonce":"0x02",
            "to":"",
            "from":"0x6177843db3138ae69679A54b95cf345ED759450d",
            "gas":"0x5208",
            "value":"0x01",
            "input":"0x010203040506",
            "data":"0x010203040506",
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
            nonce: Some(2),
            to: TxKind::Create,
            from: Address::from_slice(
                &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
            ),
            gas: Some(0x5208),
            value: U256::from(1),
            input: Bytes::from(hex::decode("010203040506").unwrap()),
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
            fee_token: None,
            blob_versioned_hashes: Default::default(),
            blobs: Default::default(),
            wrapper_version: None,
            chain_id: Default::default(),
            authorization_list: None,
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
            ..Default::default()
        };

        assert_eq!(
            deserialized_eip4844_transaction,
            serde_json::from_str(eip4844_transaction).unwrap()
        )
    }

    #[test]
    fn serialize_deserialize_transaction() {
        let eip1559 = EIP1559Transaction {
            chain_id: 65536999,
            nonce: 1,
            max_priority_fee_per_gas: 1000,
            max_fee_per_gas: 2000,
            gas_limit: 21000,
            to: TxKind::Call(H160::from_str("0x000a52D537c4150ec274dcE3962a0d179B7E71B0").unwrap()),
            value: U256::from(100000),
            data: Bytes::from_static(b"03"),
            access_list: vec![(
                H160::from_str("0x000a52D537c4150ec274dcE3962a0d179B7E71B3").unwrap(),
                vec![H256::zero()],
            )],
            signature_y_parity: true,
            signature_r: U256::one(),
            signature_s: U256::zero(),
            ..Default::default()
        };
        let tx_to_serialize = Transaction::EIP1559Transaction(eip1559.clone());
        let serialized = serde_json::to_string(&tx_to_serialize).expect("Failed to serialize");

        let deserialized_tx: Transaction =
            serde_json::from_str(&serialized).expect("Failed to deserialize");

        assert!(deserialized_tx.tx_type() == TxType::EIP1559);

        if let Transaction::EIP1559Transaction(tx) = deserialized_tx {
            assert_eq!(tx, eip1559);
        }
    }

    #[test]
    fn serialize_deserialize_eip7702transaction() {
        let eip7702 = EIP7702Transaction {
            chain_id: 65536999,
            nonce: 1,
            max_priority_fee_per_gas: 1000,
            max_fee_per_gas: 2000,
            gas_limit: 21000,
            to: Address::from_str("0x000a52D537c4150ec274dcE3962a0d179B7E71B0").unwrap(),
            value: U256::from(100000),
            data: Bytes::from_static(b"03"),
            access_list: vec![],
            signature_y_parity: true,
            signature_r: U256::one(),
            signature_s: U256::zero(),
            authorization_list: vec![AuthorizationTuple {
                chain_id: U256::from(65536999),
                address: H160::from_str("0x000a52D537c4150ec274dcE3962a0d179B7E71B1").unwrap(),
                nonce: 2,
                y_parity: U256::one(),
                r_signature: U256::from(22),
                s_signature: U256::from(37),
            }],
            ..Default::default()
        };
        let tx_to_serialize = Transaction::EIP7702Transaction(eip7702.clone());
        let serialized = serde_json::to_string(&tx_to_serialize).expect("Failed to serialize");

        let deserialized_tx: Transaction =
            serde_json::from_str(&serialized).expect("Failed to deserialize");

        assert!(deserialized_tx.tx_type() == TxType::EIP7702);

        if let Transaction::EIP7702Transaction(tx) = deserialized_tx {
            assert_eq!(tx, eip7702);
        }
    }

    #[test]
    fn serialize_deserialize_privileged_l2_transaction() -> Result<(), RLPDecodeError> {
        let privileged_l2 = PrivilegedL2Transaction {
            chain_id: 65536999,
            nonce: 0,
            max_priority_fee_per_gas: 875000000,
            max_fee_per_gas: 875000000,
            gas_limit: 42000u64,
            to: TxKind::Call(
                Address::from_str("0x8943545177806ed17b9f23f0a21ee5948ecaa776").unwrap(),
            ),
            value: U256::from(500000000000000000000000000u128),
            data: Bytes::new(),
            access_list: vec![],
            from: Address::from_str("0x8943545177806ed17b9f23f0a21ee5948ecaa776").unwrap(),
            ..Default::default()
        };

        let encoded = PrivilegedL2Transaction::encode_to_vec(&privileged_l2);
        println!("encoded length: {}", encoded.len());
        assert_eq!(encoded.len(), privileged_l2.length());

        let deserialized_tx = PrivilegedL2Transaction::decode(&encoded)?;

        assert_eq!(deserialized_tx, privileged_l2);

        Ok(())
    }

    #[test]
    fn test_legacy_transaction_into_generic() {
        let legacy_tx = LegacyTransaction {
            nonce: 1,
            gas_price: U256::from(20_000_000_000u64),
            gas: 21000,
            to: TxKind::Call(
                Address::from_str("0x742d35Cc6634C0532925a3b844Bc454e4438f44e").unwrap(),
            ),
            value: U256::from(1_000_000_000_000_000_000u64),
            data: Bytes::default(),
            v: U256::from(27),
            r: U256::from(1),
            s: U256::from(1),
            ..Default::default()
        };

        let generic_tx: GenericTransaction = legacy_tx.into();
        assert_eq!(generic_tx.r#type, TxType::Legacy);
        assert_eq!(generic_tx.nonce, Some(1));
        assert_eq!(generic_tx.gas_price, 20_000_000_000);
        assert_eq!(generic_tx.gas, Some(21000));
        assert_eq!(generic_tx.max_priority_fee_per_gas, None);
        assert_eq!(generic_tx.max_fee_per_gas, None);
        assert_eq!(generic_tx.access_list.len(), 0);
        assert_eq!(generic_tx.chain_id, None);
    }

    #[test]
    fn test_eip2930_transaction_into_generic() {
        let access_list = vec![(
            Address::from_str("0x742d35Cc6634C0532925a3b844Bc454e4438f44e").unwrap(),
            vec![
                H256::from_str(
                    "0x1234567890123456789012345678901234567890123456789012345678901234",
                )
                .unwrap(),
            ],
        )];

        let eip2930_tx = EIP2930Transaction {
            chain_id: 1,
            nonce: 1,
            gas_price: U256::from(20_000_000_000u64),
            gas_limit: 21000,
            to: TxKind::Call(
                Address::from_str("0x742d35Cc6634C0532925a3b844Bc454e4438f44e").unwrap(),
            ),
            value: U256::from(1_000_000_000_000_000_000u64),
            data: Bytes::default(),
            access_list: access_list.clone(),
            signature_y_parity: false,
            signature_r: U256::from(1),
            signature_s: U256::from(1),
            ..Default::default()
        };

        let generic_tx: GenericTransaction = eip2930_tx.into();
        assert_eq!(generic_tx.r#type, TxType::EIP2930);
        assert_eq!(generic_tx.nonce, Some(1));
        assert_eq!(generic_tx.gas_price, 20_000_000_000);
        assert_eq!(generic_tx.gas, Some(21000));
        assert_eq!(generic_tx.max_priority_fee_per_gas, None);
        assert_eq!(generic_tx.max_fee_per_gas, None);
        assert_eq!(generic_tx.chain_id, Some(1));
        assert_eq!(generic_tx.access_list.len(), 1);
        assert_eq!(generic_tx.access_list[0].address, access_list[0].0);
        assert_eq!(generic_tx.access_list[0].storage_keys, access_list[0].1);
    }

    #[test]
    fn recover_address_rejects_high_s_signatures() {
        use k256::ecdsa::SigningKey;

        // 1. Setup: Create a signer and a message
        // A random private key for testing
        let private_key = hex!("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318");
        let signing_key = SigningKey::from_bytes(&private_key.into()).expect("Valid private key");

        // The message we want to sign
        let msg = b"Test message for high-s signature rejection";
        // Calculate the Keccak256 hash of the message (the payload)
        let payload = keccak(msg);

        // 2. Generate a valid low-s signature
        // k256's sign_prehash_recoverable produces canonical low-s signatures by default.
        // We use the pre-calculated hash (payload).
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(payload.as_bytes())
            .expect("Signing failed");

        // 3. Construct the signature bytes expected by recover_address
        // Format: [r (32 bytes), s (32 bytes), v (1 byte)]
        let mut sig_bytes = [0u8; 65];
        sig_bytes[..64].copy_from_slice(&signature.to_bytes());
        sig_bytes[64] = recovery_id.to_byte();

        // 4. Verify that the valid low-s signature recovers the correct address
        // Calculate the expected address from the public key
        let uncompressed_pub = signing_key.verifying_key().to_encoded_point(false);
        let pub_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed_pub.as_bytes()[1..]);
        let expected_address = Address::from_slice(&pub_hash[12..]);

        let recovered = recover_address(Signature::from_slice(&sig_bytes), payload)
            .expect("Valid low-s signature should recover successfully");
        assert_eq!(recovered, expected_address, "Recovered address mismatch");

        // 5. Create a high-s signature: s' = N - s
        // The curve order N for secp256k1
        let n = U256::from_big_endian(&hex!(
            "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141"
        ));
        let s = U256::from_big_endian(&sig_bytes[32..64]);

        // Ensure the generated signature was indeed low-s (standard requirement)
        let half_n = n / 2;
        assert!(
            s <= half_n,
            "Generated signature was not low-s, cannot test high-s rejection"
        );

        // Calculate high-s
        let s_high = n - s;

        let mut sig_high_bytes = sig_bytes;
        // Replace s with s_high
        sig_high_bytes[32..64].copy_from_slice(&s_high.to_big_endian());
        // When flipping s to -s mod N, we must also flip the recovery ID (v) to maintain validity of the point R
        sig_high_bytes[64] ^= 1;

        // 6. Verify that the high-s signature is rejected
        // EIP-2 requires rejecting s > N/2 to prevent malleability
        assert!(
            recover_address(Signature::from_slice(&sig_high_bytes), payload).is_err(),
            "High-s signature should be rejected (EIP-2 compliance)"
        );
    }

    #[test]
    fn encode_decode_low_size_tx() {
        let tx = Transaction::EIP2930Transaction(EIP2930Transaction::default());
        let encoded = tx.encode_to_vec();
        let decoded_tx = Transaction::decode(&encoded).unwrap();
        assert_eq!(tx, decoded_tx);
    }

    #[test]
    fn test_eip1559_simple_transfer_size() {
        let tx = Transaction::EIP1559Transaction(EIP1559Transaction::default());
        assert_eq!(tx.encode_to_vec().len(), EIP1559_DEFAULT_SERIALIZED_LENGTH);
    }
}
