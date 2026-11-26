use std::{
    cmp::min,
    fmt::Display,
    num::NonZeroUsize,
    sync::{LazyLock, Mutex},
};

use crate::utils::keccak;
use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_crypto::{Crypto, CryptoError, NativeCrypto};
use lru::LruCache;
pub use mempool::MempoolTransaction;

const MAX_SIGNER_CACHE_ENTRIES: usize = 100_000;

/// Global cache mapping transaction hash → recovered sender address.
/// Keyed by tx hash (unique per transaction), so each entry is safe to reuse.
/// Not suitable for EIP-7702 authorization tuples where the same message hash
/// can correspond to different signers (the message excludes the signature).
/// Uses LRU eviction to avoid periodic cold-start spikes from clearing all entries.
///
/// Lock with `.unwrap_or_else(|e| e.into_inner())` — a poisoned mutex just means
/// a thread panicked mid-update; the LruCache invariants are maintained by the
/// std Mutex (data is still accessible), and a missing entry only costs one
/// redundant recovery, so it's safe to keep using.
pub static GLOBAL_SIGNER_CACHE: LazyLock<Mutex<LruCache<H256, Address>>> = LazyLock::new(|| {
    Mutex::new(LruCache::new(
        NonZeroUsize::new(MAX_SIGNER_CACHE_ENTRIES).expect("MAX_SIGNER_CACHE_ENTRIES is non-zero"),
    ))
});
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Serialize, ser::SerializeStruct};
pub use serde_impl::{
    AccessListEntry, AuthorizationTupleEntry, FrameEntry, GenericTransaction,
    GenericTransactionError,
};

/// The serialized length of a default eip1559 transaction
pub const EIP1559_DEFAULT_SERIALIZED_LENGTH: usize = 15;

use ethrex_rlp::{
    constants::RLP_NULL,
    decode::{RLPDecode, decode_bytes, decode_rlp_item},
    encode::{PayloadRLPEncode, RLPEncode},
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

#[cfg(all(feature = "zisk", target_arch = "riscv64"))]
use super::unsync_cell::OnceCell;
use crate::types::{
    AccessList, AuthorizationList, BlobsBundle,
    constants::{MAX_BLOB_TX_SIZE, MAX_TX_SIZE, VERSIONED_HASH_VERSION_KZG},
};
#[cfg(not(all(feature = "zisk", target_arch = "riscv64")))]
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
    FrameTransaction(FrameTransaction),
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
    /// An EIP-8141 frame transaction in its pooled representation, with the
    /// witnesses of its claims and, when it carries blobs, its blob sidecar.
    FrameTransaction(PooledFrameTransaction),
}

impl TryInto<Transaction> for P2PTransaction {
    type Error = String;

    fn try_into(self) -> Result<Transaction, Self::Error> {
        match self {
            P2PTransaction::LegacyTransaction(itx) => Ok(Transaction::LegacyTransaction(itx)),
            P2PTransaction::EIP2930Transaction(itx) => Ok(Transaction::EIP2930Transaction(itx)),
            P2PTransaction::EIP1559Transaction(itx) => Ok(Transaction::EIP1559Transaction(itx)),
            P2PTransaction::EIP7702Transaction(itx) => Ok(Transaction::EIP7702Transaction(itx)),
            P2PTransaction::FrameTransaction(_) => Err("Can't convert a pooled frame transaction into a regular transaction. Its witnesses would be lost.".to_string()),
            _ => Err("Can't convert blob p2p transaction into regular transaction. Blob bundle would be lost.".to_string()),
        }
    }
}

impl P2PTransaction {
    /// The network form of an EIP-4844 `tx` with its `blobs_bundle`, the EIP-7594
    /// wrapper. `None` for any other transaction: a frame transaction with blobs
    /// travels in its own pooled representation, which also carries the witnesses
    /// of its claims ([`PooledFrameTransaction`]).
    pub fn with_blobs_bundle(tx: Transaction, blobs_bundle: BlobsBundle) -> Option<Self> {
        let wrapper_version = (blobs_bundle.version != 0).then_some(blobs_bundle.version);
        match tx {
            Transaction::EIP4844Transaction(tx) => Some(Self::EIP4844TransactionWithBlobs(
                WrappedEIP4844Transaction {
                    tx,
                    wrapper_version,
                    blobs_bundle,
                },
            )),
            _ => None,
        }
    }

    /// Splits a transaction received from a peer into the transaction and, for a
    /// network-wrapped EIP-4844 one, its blobs bundle. A frame transaction is not
    /// split this way: its pooled representation also carries the witnesses of
    /// its claims, and it is admitted whole.
    pub fn into_transaction_and_blobs(self) -> Result<(Transaction, Option<BlobsBundle>), String> {
        match self {
            Self::EIP4844TransactionWithBlobs(wrapped) => Ok((
                Transaction::EIP4844Transaction(wrapped.tx),
                Some(wrapped.blobs_bundle),
            )),
            other => Ok((other.try_into()?, None)),
        }
    }

    /// The versioned hashes and blobs bundle of a blob-carrying transaction in its
    /// network form.
    pub fn blob_parts(&self) -> Option<(&[H256], &BlobsBundle)> {
        match self {
            Self::EIP4844TransactionWithBlobs(wrapped) => {
                Some((&wrapped.tx.blob_versioned_hashes, &wrapped.blobs_bundle))
            }
            Self::FrameTransaction(pooled) => pooled
                .blobs_bundle
                .as_ref()
                .map(|bundle| (pooled.tx.blob_versioned_hashes.as_slice(), bundle)),
            _ => None,
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
            let tx_type = *payload.first().ok_or(RLPDecodeError::InvalidLength)?;
            let tx_encoding = payload.get(1..).ok_or(RLPDecodeError::InvalidLength)?;
            // `from_type_byte` is the single gate for valid envelope types: it rejects 0x00
            // (legacy is the bare-list branch below) and any unassigned type byte.
            let tx = match EnvelopeTxType::from_type_byte(tx_type)? {
                EnvelopeTxType::EIP2930 => {
                    P2PTransaction::EIP2930Transaction(EIP2930Transaction::decode(tx_encoding)?)
                }
                EnvelopeTxType::EIP1559 => {
                    P2PTransaction::EIP1559Transaction(EIP1559Transaction::decode(tx_encoding)?)
                }
                EnvelopeTxType::EIP4844 => P2PTransaction::EIP4844TransactionWithBlobs(
                    WrappedEIP4844Transaction::decode(tx_encoding)?,
                ),
                EnvelopeTxType::EIP7702 => {
                    P2PTransaction::EIP7702Transaction(EIP7702Transaction::decode(tx_encoding)?)
                }
                // A frame transaction travels between peers only in its pooled
                // representation (EIP-8141 §Networking); a bare `tx_body` does not
                // decode as one.
                EnvelopeTxType::Frame => {
                    P2PTransaction::FrameTransaction(PooledFrameTransaction::decode(tx_encoding)?)
                }
                EnvelopeTxType::FeeToken => {
                    P2PTransaction::FeeTokenTransaction(FeeTokenTransaction::decode(tx_encoding)?)
                }
                // Privileged (L2) transactions are never gossiped over p2p.
                EnvelopeTxType::Privileged => {
                    return Err(RLPDecodeError::Custom(
                        "privileged transactions are not sent over p2p".to_string(),
                    ));
                }
            };
            Ok((tx, remainder))
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

/// The pooled, or submission, representation of an EIP-8141 frame transaction
/// (§Networking): `rlp([tx_body, wrapper_version, witnesses, blob_sidecar])`. A
/// frame transaction has this form in `PooledTransactions` and
/// `eth_sendRawTransaction`, and never inside a block body, where it is the bare
/// `tx_body` and its witnesses sit in the block's `claim_witnesses`.
///
/// `witnesses[i]` is the witness of `tx.claims[i]`. `blob_sidecar` is the empty
/// list when the transaction carries no blobs, and `[blobs, commitments,
/// cell_proofs]` per EIP-7594 when it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PooledFrameTransaction {
    pub tx: FrameTransaction,
    /// One witness for each entry of `tx.claims`, in the same order.
    pub witnesses: Vec<Bytes>,
    /// The blob sidecar of a transaction that carries blobs.
    pub blobs_bundle: Option<BlobsBundle>,
}

impl PooledFrameTransaction {
    /// Whether a type-6 payload is the pooled representation rather than a bare
    /// `tx_body`: the pooled form's first item is the transaction body, a list,
    /// where a bare transaction's first item is its `chain_id`.
    pub fn is_pooled(rlp: &[u8]) -> Result<bool, RLPDecodeError> {
        let (is_list, payload, _) = decode_rlp_item(rlp)?;
        if !is_list {
            return Err(RLPDecodeError::UnexpectedString);
        }
        let (first_is_list, _, _) = decode_rlp_item(payload)?;
        Ok(first_is_list)
    }

    /// Total size of the witnesses, in bytes.
    pub fn witness_bytes(&self) -> usize {
        self.witnesses.iter().map(Bytes::len).sum()
    }

    /// Size in bytes of the pooled representation as it travels: the type byte
    /// followed by `rlp([tx_body, wrapper_version, witnesses, blob_sidecar])`.
    pub fn wire_len(&self) -> usize {
        1 + self.length()
    }

    /// The most bytes a node accepts for the pooled representation of this
    /// transaction ([`max_pooled_frame_tx_len`]).
    pub fn max_wire_len(&self) -> usize {
        max_pooled_frame_tx_len(&self.tx, self.blobs_bundle.is_some())
    }
}

/// What the framing of a pooled representation adds around the transaction
/// body, the witnesses and the blob sidecar: the wrapper version and the list
/// headers of the wrapper and of the sidecar, each of at most 5 bytes for
/// anything a node handles.
const POOLED_FRAME_TX_FRAMING_BYTES: usize = 16;

/// The most bytes the pooled representation of `tx` may take, type byte
/// included. It has two terms of different origin:
///
/// - The transaction itself, with its blob sidecar when it has one. EIP-8141
///   gives no limit for it, so this is node policy: `MAX_TX_SIZE`, or
///   `MAX_BLOB_TX_SIZE` for a transaction with blobs, the caps every other
///   transaction type is held to.
/// - Its witnesses. EIP-8141 §Networking fixes their number and their lengths
///   from the transaction's claims ([`FrameTransaction::max_pooled_witnesses_len`]).
///
/// A node checks it on the pooled representation as received, before any
/// witness is verified.
pub fn max_pooled_frame_tx_len(tx: &FrameTransaction, with_blobs: bool) -> usize {
    let transaction = if with_blobs {
        MAX_BLOB_TX_SIZE
    } else {
        MAX_TX_SIZE
    };
    transaction
        .saturating_add(tx.max_pooled_witnesses_len())
        .saturating_add(POOLED_FRAME_TX_FRAMING_BYTES)
}

impl RLPEncode for PooledFrameTransaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let mut blob_sidecar = Vec::new();
        match &self.blobs_bundle {
            Some(bundle) => Encoder::new(&mut blob_sidecar)
                .encode_field(&bundle.blobs)
                .encode_field(&bundle.commitments)
                .encode_field(&bundle.proofs)
                .finish(),
            None => Encoder::new(&mut blob_sidecar).finish(),
        }
        Encoder::new(buf)
            .encode_field(&self.tx)
            .encode_field(&POOLED_WRAPPER_VERSION)
            .encode_field(&self.witnesses)
            .encode_raw(&blob_sidecar)
            .finish();
    }
}

impl RLPDecode for PooledFrameTransaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(PooledFrameTransaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (tx, decoder): (FrameTransaction, _) = decoder.decode_field("tx_body")?;
        let (wrapper_version, decoder) = decoder.decode_field::<u8>("wrapper_version")?;
        if wrapper_version != POOLED_WRAPPER_VERSION {
            return Err(RLPDecodeError::Custom(format!(
                "unknown pooled wrapper version {wrapper_version}"
            )));
        }
        // EIP-8141 §Networking: the pooled representation has one witness for
        // each claim, each of the length its claim resolves to, so the body
        // already says how long the list of witnesses can be. A longer one is
        // rejected by the length its header gives, before it is read.
        let (witnesses, decoder) = decoder.get_encoded_item_ref()?;
        let max_witnesses_len = tx.max_pooled_witnesses_len();
        if witnesses.len() > max_witnesses_len {
            return Err(RLPDecodeError::Custom(format!(
                "witnesses take {} bytes, more than the {max_witnesses_len} the claims of the transaction allow",
                witnesses.len()
            )));
        }
        let witnesses = Vec::<Bytes>::decode(witnesses)?;
        let (blob_sidecar, decoder) = decoder.get_encoded_item_ref()?;
        let rest = decoder.finish()?;

        let sidecar_decoder = Decoder::new(blob_sidecar)?;
        let blobs_bundle = if sidecar_decoder.is_done() {
            sidecar_decoder.finish()?;
            None
        } else {
            let (blobs, sidecar_decoder) = sidecar_decoder.decode_field("blobs")?;
            let (commitments, sidecar_decoder) = sidecar_decoder.decode_field("commitments")?;
            let (proofs, sidecar_decoder) = sidecar_decoder.decode_field("cell_proofs")?;
            sidecar_decoder.finish()?;
            Some(BlobsBundle {
                blobs,
                commitments,
                proofs,
                // The sidecar always carries cell proofs, the EIP-7594 form.
                version: 1,
            })
        };
        Ok((
            PooledFrameTransaction {
                tx,
                witnesses,
                blobs_bundle,
            },
            rest,
        ))
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
    #[rkyv(with=rkyv::with::Skip)]
    pub cached_canonical: OnceCell<Vec<u8>>,
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
    #[rkyv(with=rkyv::with::Skip)]
    pub cached_canonical: OnceCell<Vec<u8>>,
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
    #[rkyv(with=rkyv::with::Skip)]
    pub cached_canonical: OnceCell<Vec<u8>>,
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
    #[rkyv(with=rkyv::with::Skip)]
    pub cached_canonical: OnceCell<Vec<u8>>,
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
    #[rkyv(with=rkyv::with::Skip)]
    pub cached_canonical: OnceCell<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TxType {
    #[default]
    Legacy = 0x00,
    EIP2930 = 0x01,
    EIP1559 = 0x02,
    EIP4844 = 0x03,
    EIP7702 = 0x04,
    Frame = 0x06,
    FeeToken = 0x7d,
    // We take the same approach as Optimism to define the privileged tx prefix
    // https://github.com/ethereum-optimism/specs/blob/c6903a3b2cad575653e1f5ef472debb573d83805/specs/protocol/deposits.md#the-deposited-transaction-type
    Privileged = 0x7e,
}

/// The EIP-2718 typed-transaction **envelope** types — the subset of [`TxType`] that
/// can be encoded as a `0x{type} || payload` envelope. Excludes [`TxType::Legacy`]:
/// a legacy transaction is a bare RLP list, never a typed envelope. Decoders that
/// dispatch on the leading type byte match this exhaustively, so there is no
/// impossible `Legacy` arm to handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeTxType {
    EIP2930,
    EIP1559,
    EIP4844,
    EIP7702,
    Frame,
    FeeToken,
    Privileged,
}

impl EnvelopeTxType {
    /// Resolve a typed-transaction **envelope** type byte (EIP-2718) to its type.
    ///
    /// Reuses [`TxType::from_u8`] for the byte mapping (single source of truth) and
    /// [`TxType::as_envelope`] to reject non-envelope types. **Rejects `0x00`**: per EIP-2718
    /// a legacy transaction is a bare RLP list, never a `0x00`-typed envelope, so
    /// `0x00 || rlp(..)` is non-canonical and must be rejected (matching go-ethereum) rather
    /// than silently decoded as legacy. Unassigned type bytes are rejected too.
    pub fn from_type_byte(byte: u8) -> Result<Self, RLPDecodeError> {
        TxType::from_u8(byte)
            .and_then(TxType::as_envelope)
            .ok_or_else(|| RLPDecodeError::Custom(format!("Invalid transaction type: {byte}")))
    }
}

impl From<TxType> for u8 {
    fn from(val: TxType) -> Self {
        match val {
            TxType::Legacy => 0x00,
            TxType::EIP2930 => 0x01,
            TxType::EIP1559 => 0x02,
            TxType::EIP4844 => 0x03,
            TxType::EIP7702 => 0x04,
            TxType::Frame => 0x06,
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
            TxType::Frame => write!(f, "Frame"),
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
            Transaction::FrameTransaction(_) => TxType::Frame,
        }
    }

    fn calc_effective_gas_price(&self, base_fee_per_gas: Option<u64>) -> Option<U256> {
        let base_fee = U256::from(base_fee_per_gas?);
        let max_fee = self.max_fee_per_gas()?;
        if max_fee < base_fee {
            // This is invalid, can't calculate
            return None;
        }

        let priority_fee_per_gas = min(self.max_priority_fee()?, max_fee.saturating_sub(base_fee));
        Some(priority_fee_per_gas + base_fee)
    }

    pub fn effective_gas_price(&self, base_fee_per_gas: Option<u64>) -> Option<U256> {
        match self.tx_type() {
            TxType::Legacy => Some(self.gas_price()),
            TxType::EIP2930 => Some(self.gas_price()),
            TxType::EIP1559 => self.calc_effective_gas_price(base_fee_per_gas),
            TxType::EIP4844 => self.calc_effective_gas_price(base_fee_per_gas),
            TxType::EIP7702 => self.calc_effective_gas_price(base_fee_per_gas),
            // An EIP-8141 frame transaction has no fee cap per gas: its price comes
            // from the `max_fee` budget, the blob base fee and the gas it uses (see
            // `FrameFeeBudget`).
            TxType::Frame => None,
            TxType::FeeToken => self.calc_effective_gas_price(base_fee_per_gas),
            TxType::Privileged => Some(self.gas_price()),
        }
    }

    pub fn cost_without_base_fee(&self) -> Option<U256> {
        // EIP-8141: `max_fee` bounds everything the payer can be charged, at any
        // base fees. A frame transaction moves no value at the top level.
        if let Transaction::FrameTransaction(tx) = self {
            return Some(tx.max_fee);
        }
        let price = match self.tx_type() {
            TxType::Legacy => self.gas_price(),
            TxType::EIP2930 => self.gas_price(),
            TxType::EIP1559 => self.max_fee_per_gas()?,
            TxType::EIP4844 => self.max_fee_per_gas()?,
            TxType::EIP7702 => self.max_fee_per_gas()?,
            TxType::Frame => return None, // returned above
            TxType::FeeToken => self.max_fee_per_gas()?,
            TxType::Privileged => self.gas_price(),
        };

        let base = U256::saturating_add(
            U256::saturating_mul(price, self.gas_limit().into()),
            self.value(),
        );

        // EIP-4844 blob txs pay an additional `blob_gas * max_fee_per_blob_gas`
        // upfront. Every peer client (geth, reth, nethermind, erigon, besu)
        // includes this in the balance-sufficiency check.
        if let Transaction::EIP4844Transaction(tx) = self {
            let blob_gas = U256::from(crate::constants::GAS_PER_BLOB)
                .saturating_mul(U256::from(tx.blob_versioned_hashes.len() as u64));
            let blob_cost = blob_gas.saturating_mul(tx.max_fee_per_blob_gas);
            return Some(base.saturating_add(blob_cost));
        }

        Some(base)
    }

    pub fn fee_token(&self) -> Option<Address> {
        if let Transaction::FeeTokenTransaction(tx) = self {
            Some(tx.fee_token)
        } else {
            None
        }
    }

    /// Returns a reference to the `cached_canonical` cell for non-legacy
    /// transaction types, or `None` for legacy transactions.
    fn cached_canonical_cell(&self) -> Option<&OnceCell<Vec<u8>>> {
        match self {
            Transaction::LegacyTransaction(_) => None,
            Transaction::EIP2930Transaction(t) => Some(&t.cached_canonical),
            Transaction::EIP1559Transaction(t) => Some(&t.cached_canonical),
            Transaction::EIP4844Transaction(t) => Some(&t.cached_canonical),
            Transaction::EIP7702Transaction(t) => Some(&t.cached_canonical),
            Transaction::PrivilegedL2Transaction(t) => Some(&t.cached_canonical),
            Transaction::FeeTokenTransaction(t) => Some(&t.cached_canonical),
            Transaction::FrameTransaction(t) => Some(&t.cached_canonical),
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
            _ => {
                let canonical = self.encode_canonical_to_vec();
                <[u8] as RLPEncode>::encode(canonical.as_slice(), buf)
            }
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
            let tx_type = *payload.first().ok_or(RLPDecodeError::InvalidLength)?;
            let tx_encoding = payload.get(1..).ok_or(RLPDecodeError::InvalidLength)?;
            // `from_type_byte` is the single gate for valid envelope types: it rejects 0x00
            // (a legacy tx is a bare RLP list, handled below, not a typed envelope) and any
            // unassigned type byte.
            let tx = match EnvelopeTxType::from_type_byte(tx_type)? {
                EnvelopeTxType::EIP2930 => {
                    Transaction::EIP2930Transaction(EIP2930Transaction::decode(tx_encoding)?)
                }
                EnvelopeTxType::EIP1559 => {
                    Transaction::EIP1559Transaction(EIP1559Transaction::decode(tx_encoding)?)
                }
                EnvelopeTxType::EIP4844 => {
                    Transaction::EIP4844Transaction(EIP4844Transaction::decode(tx_encoding)?)
                }
                EnvelopeTxType::EIP7702 => {
                    Transaction::EIP7702Transaction(EIP7702Transaction::decode(tx_encoding)?)
                }
                EnvelopeTxType::Frame => {
                    Transaction::FrameTransaction(FrameTransaction::decode(tx_encoding)?)
                }
                EnvelopeTxType::FeeToken => {
                    Transaction::FeeTokenTransaction(FeeTokenTransaction::decode(tx_encoding)?)
                }
                EnvelopeTxType::Privileged => Transaction::PrivilegedL2Transaction(
                    PrivilegedL2Transaction::decode(tx_encoding)?,
                ),
            };
            Ok((tx, remainder))
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
            Transaction::FrameTransaction(tx) => tx.encode_payload(buf),
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

/// Decode a transaction `nonce` field.
///
/// EEST encodes an over-`u64` nonce (a value >= 2**64, i.e. more than 8
/// significant bytes) to exercise `TransactionException.NONCE_IS_MAX`. ethrex
/// represents the nonce as a `u64`, so a plain RLP decode fails with a generic
/// field-decode error that the EEST/Hive exception mapper classifies as
/// `RLP_STRUCTURES_ENCODING` instead of `NONCE_IS_MAX`. Detect the over-`u64`
/// case and surface a "Nonce is max" message so it maps to `NONCE_IS_MAX` in
/// both the upstream Python `ethrex.py` mapper and the local Rust mapper.
fn decode_nonce_field(decoder: Decoder) -> Result<(u64, Decoder), RLPDecodeError> {
    // Wrap every decode failure exactly as `Decoder::decode_field` would, so
    // non-overflow errors keep their original wording (and mapper classification);
    // only the intentional over-u64 case below diverges.
    let wrap =
        |err| RLPDecodeError::Custom(format!("Error decoding field 'nonce' of type u64: {err}"));
    let (item, rest) = decoder.get_encoded_item_ref().map_err(wrap)?;
    // A *canonical* over-u64 nonce (>= 2**64) has more than 8 significant bytes and,
    // being canonical, no leading zero byte. EEST uses it for
    // TransactionException.NONCE_IS_MAX; surface a nonce-domain rejection rather than
    // a generic RLP field-decode error so the exception mapper maps it to NONCE_IS_MAX
    // in both the upstream Python and local Rust mappers.
    //
    // A non-canonical >8-byte encoding (leading-zero padding) is a malformed RLP
    // integer, not an over-u64 value, so it must NOT be reported as NONCE_IS_MAX. Leave
    // it to fall through to `u64::decode_unfinished` below, which rejects it as an
    // ordinary field-decode error (mapped to RLP_STRUCTURES_ENCODING) — matching strict
    // mappers like Hive.
    if let Ok((content, _)) = decode_bytes(item)
        && content.len() > 8
        && content.first() != Some(&0)
    {
        return Err(RLPDecodeError::Custom("Nonce is max".into()));
    }
    let (nonce, _) = u64::decode_unfinished(item).map_err(wrap)?;
    Ok((nonce, rest))
}

impl RLPDecode for LegacyTransaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(LegacyTransaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (nonce, decoder) = decode_nonce_field(decoder)?;
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
        let (nonce, decoder) = decode_nonce_field(decoder)?;
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
        let cached_canonical = OnceCell::new();

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
            cached_canonical,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for EIP1559Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(EIP1559Transaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (nonce, decoder) = decode_nonce_field(decoder)?;
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
        let cached_canonical = OnceCell::new();

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
            cached_canonical,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for EIP4844Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(EIP4844Transaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (nonce, decoder) = decode_nonce_field(decoder)?;
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
        let cached_canonical = OnceCell::new();

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
            cached_canonical,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for EIP7702Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(EIP7702Transaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (nonce, decoder) = decode_nonce_field(decoder)?;
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
        let cached_canonical = OnceCell::new();

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
            cached_canonical,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for PrivilegedL2Transaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(PrivilegedL2Transaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (nonce, decoder) = decode_nonce_field(decoder)?;
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
        let cached_canonical = OnceCell::new();

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
            cached_canonical,
        };
        Ok((tx, decoder.finish()?))
    }
}

impl RLPDecode for FeeTokenTransaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(FeeTokenTransaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        let (nonce, decoder) = decode_nonce_field(decoder)?;
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
        let cached_canonical = OnceCell::new();

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
            cached_canonical,
        };
        Ok((tx, decoder.finish()?))
    }
}

/// The bytes a transaction's signature covers, paired with the 65-byte
/// `r || s || v` signature itself. See [`Transaction::signing_payload`].
pub type SigningPayload = (Vec<u8>, [u8; 65]);

impl Transaction {
    pub fn sender(&self, crypto: &dyn Crypto) -> Result<Address, CryptoError> {
        // Frame transactions have explicit sender, no ECDSA recovery
        if let Transaction::FrameTransaction(tx) = self {
            return Ok(tx.sender);
        }
        let sender_cache = match self {
            Transaction::LegacyTransaction(tx) => &tx.sender_cache,
            Transaction::EIP2930Transaction(tx) => &tx.sender_cache,
            Transaction::EIP1559Transaction(tx) => &tx.sender_cache,
            Transaction::EIP4844Transaction(tx) => &tx.sender_cache,
            Transaction::EIP7702Transaction(tx) => &tx.sender_cache,
            Transaction::PrivilegedL2Transaction(tx) => &tx.sender_cache,
            Transaction::FeeTokenTransaction(tx) => &tx.sender_cache,
            Transaction::FrameTransaction(_) => unreachable!(),
        };
        sender_cache
            .get_or_try_init(|| {
                let tx_hash = self.hash(crypto);
                // Fast path: check process-level signer cache
                let mut cache = GLOBAL_SIGNER_CACHE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if let Some(&addr) = cache.get(&tx_hash) {
                    return Ok(addr);
                }
                drop(cache);
                // Slow path: actual secp256k1 recovery
                let sender = self.compute_sender(crypto)?;
                // Store in global cache for future lookups (LRU evicts oldest on overflow)
                GLOBAL_SIGNER_CACHE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .put(tx_hash, sender);
                Ok(sender)
            })
            .copied()
    }

    /// The bytes that were signed, plus the 65-byte `r || s || v` signature.
    ///
    /// `Ok(None)` for transactions that carry an explicit sender and no signature
    /// (privileged L2 and frame), for which there is nothing to recover.
    ///
    /// Exposed so callers needing the *public key* rather than the address can
    /// reuse this logic: the per-variant EIP-155 and typed-transaction handling
    /// below is subtle and must exist in exactly one place.
    pub fn signing_payload(&self) -> Result<Option<SigningPayload>, CryptoError> {
        let (buf, sig) = match self {
            Transaction::LegacyTransaction(tx) => {
                // EIP-155: valid legacy `v` is 27/28 (pre-155) or
                // {35,36} + chain_id * 2. Any other value is a malformed
                // signature and must be rejected rather than coerced into a
                // parity bit (which would recover a bogus sender). `v` is taken
                // at its full width: a chain id near 2^64 still fits in 64 bits,
                // but its `v` does not.
                let parity = match self.chain_id() {
                    Some(chain_id) => tx.v.checked_sub(U256::from(35) + U256::from(chain_id) * 2),
                    None => tx.v.checked_sub(U256::from(27)),
                };
                let signature_y_parity = match parity {
                    Some(parity) if parity.is_zero() => false,
                    Some(parity) if parity == U256::one() => true,
                    _ => return Err(CryptoError::InvalidSignature),
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
                (buf, sig)
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
                (buf, sig)
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
                (buf, sig)
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
                (buf, sig)
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
                (buf, sig)
            }
            // Explicit sender, no signature: nothing to recover from.
            Transaction::PrivilegedL2Transaction(_) | Transaction::FrameTransaction(_) => {
                return Ok(None);
            }
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
                (buf, sig)
            }
        };
        Ok(Some((buf, sig)))
    }

    fn compute_sender(&self, crypto: &dyn Crypto) -> Result<Address, CryptoError> {
        match self {
            Transaction::PrivilegedL2Transaction(tx) => return Ok(tx.from),
            Transaction::FrameTransaction(tx) => return Ok(tx.sender),
            _ => {}
        }
        let Some((buf, sig)) = self.signing_payload()? else {
            // Unreachable: the two signature-less variants are handled above.
            return Err(CryptoError::InvalidSignature);
        };
        let msg = crypto.keccak256(&buf);
        crypto.recover_signer(&sig, &msg)
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
            Transaction::FrameTransaction(tx) => tx.max_gas(),
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
            Transaction::FrameTransaction(tx) => tx.max_fee_per_gas_share(),
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
            Transaction::FrameTransaction(tx) => TxKind::Call(tx.sender),
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
            Transaction::FrameTransaction(_) => U256::zero(),
        }
    }

    /// Widened to `U256` for the same reason as [`Self::max_fee_per_gas`].
    pub fn max_priority_fee(&self) -> Option<U256> {
        match self {
            Transaction::LegacyTransaction(_tx) => None,
            Transaction::EIP2930Transaction(_tx) => None,
            Transaction::EIP1559Transaction(tx) => Some(U256::from(tx.max_priority_fee_per_gas)),
            Transaction::EIP4844Transaction(tx) => Some(U256::from(tx.max_priority_fee_per_gas)),
            Transaction::EIP7702Transaction(tx) => Some(U256::from(tx.max_priority_fee_per_gas)),
            Transaction::PrivilegedL2Transaction(tx) => {
                Some(U256::from(tx.max_priority_fee_per_gas))
            }
            Transaction::FeeTokenTransaction(tx) => Some(U256::from(tx.max_priority_fee_per_gas)),
            Transaction::FrameTransaction(tx) => Some(tx.max_priority_fee_per_gas),
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
            Transaction::FrameTransaction(tx) => Some(tx.chain_id),
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
            Transaction::FrameTransaction(_) => &EMPTY_ACCESS_LIST,
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
            Transaction::FrameTransaction(_) => None,
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
            // EIP-8250: the sequence shared by the selected nonce keys. It equals the
            // sender's account nonce only for the empty key set.
            Transaction::FrameTransaction(tx) => tx.nonce_seq,
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
            Transaction::FrameTransaction(_) => {
                static EMPTY_DATA: Bytes = Bytes::new();
                &EMPTY_DATA
            }
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
            Transaction::FrameTransaction(tx) => tx.blob_versioned_hashes.clone(),
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
            // EIP-8141 pays blob gas from the `max_fee` budget: there is no fee cap
            // per blob gas, even when the transaction carries blobs.
            Transaction::FrameTransaction(_) => None,
        }
    }

    /// Whether this transaction carries blob data, and therefore an expensive
    /// EIP-4844 sidecar. True for EIP-4844 transactions and for EIP-8141 frame
    /// transactions with a non-empty `blob_versioned_hashes`. Prefer this over
    /// matching on `EIP4844Transaction` alone when the question is "does this tx
    /// have a sidecar to re-propagate".
    pub fn is_blob_carrying(&self) -> bool {
        match self {
            Transaction::EIP4844Transaction(_) => true,
            Transaction::FrameTransaction(tx) => !tx.blob_versioned_hashes.is_empty(),
            _ => false,
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
            Transaction::FrameTransaction(_) => false,
        }
    }

    pub fn is_privileged(&self) -> bool {
        matches!(self, Transaction::PrivilegedL2Transaction(_))
    }

    /// The transaction's `max_fee_per_gas`, widened to `U256` like
    /// [`Self::max_priority_fee`]. `None` for an EIP-8141 frame transaction, whose
    /// `max_fee` caps its total fee rather than its price per gas.
    pub fn max_fee_per_gas(&self) -> Option<U256> {
        match self {
            Transaction::LegacyTransaction(_tx) => None,
            Transaction::EIP2930Transaction(_tx) => None,
            Transaction::EIP1559Transaction(tx) => Some(U256::from(tx.max_fee_per_gas)),
            Transaction::EIP4844Transaction(tx) => Some(U256::from(tx.max_fee_per_gas)),
            Transaction::EIP7702Transaction(tx) => Some(U256::from(tx.max_fee_per_gas)),
            Transaction::PrivilegedL2Transaction(tx) => Some(U256::from(tx.max_fee_per_gas)),
            Transaction::FeeTokenTransaction(tx) => Some(U256::from(tx.max_fee_per_gas)),
            Transaction::FrameTransaction(_) => None,
        }
    }

    fn compute_hash(&self, crypto: &dyn Crypto) -> H256 {
        if let Transaction::PrivilegedL2Transaction(tx) = self {
            return tx.get_privileged_hash().unwrap_or_default();
        }
        H256(crypto.keccak256(&self.encode_canonical_to_vec()))
    }

    pub fn hash(&self, crypto: &dyn Crypto) -> H256 {
        let inner_hash = match self {
            Transaction::LegacyTransaction(tx) => &tx.inner_hash,
            Transaction::EIP2930Transaction(tx) => &tx.inner_hash,
            Transaction::EIP1559Transaction(tx) => &tx.inner_hash,
            Transaction::EIP4844Transaction(tx) => &tx.inner_hash,
            Transaction::EIP7702Transaction(tx) => &tx.inner_hash,
            Transaction::PrivilegedL2Transaction(tx) => &tx.inner_hash,
            Transaction::FeeTokenTransaction(tx) => &tx.inner_hash,
            Transaction::FrameTransaction(tx) => &tx.inner_hash,
        };

        *inner_hash.get_or_init(|| self.compute_hash(crypto))
    }

    pub fn gas_tip_cap(&self) -> U256 {
        self.max_priority_fee().unwrap_or_else(|| self.gas_price())
    }

    /// The fee cap compared by the mempool's replacement rule. For an EIP-8141 frame
    /// transaction it is the `max_fee` budget, which its replacement must raise.
    pub fn gas_fee_cap(&self) -> U256 {
        if let Transaction::FrameTransaction(tx) = self {
            return tx.max_fee;
        }
        self.max_fee_per_gas().unwrap_or_else(|| self.gas_price())
    }

    /// Returns the effective tip per gas for this transaction.
    /// Returns `None` if the transaction's fee cap is below the base fee (i.e. the
    /// transaction cannot pay for its inclusion).
    ///
    /// For an EIP-8141 frame transaction the tip per gas is what the `max_fee`
    /// budget pays on each unit of `max_gas` once the base fee is covered. The blob
    /// base fee is not known here, so blob gas is left out of that requirement:
    /// the result orders transactions, and the exact fee budget is checked when the
    /// transaction executes.
    pub fn effective_gas_tip(&self, base_fee: Option<u64>) -> Option<U256> {
        if let Transaction::FrameTransaction(tx) = self {
            let base_fee = U256::from(base_fee.unwrap_or_default());
            let budget = tx.fee_budget(base_fee, U256::zero())?;
            return Some(budget.gas_price - base_fee);
        }
        let tip_cap = self.gas_tip_cap();
        let Some(base_fee) = base_fee else {
            return Some(tip_cap);
        };
        let base_fee = U256::from(base_fee);
        let fee_cap = self.gas_fee_cap();
        let tip = fee_cap.checked_sub(base_fee)?;
        Some(min(tip, tip_cap))
    }

    /// Returns whether the transaction is replay-protected.
    /// For more information check out [EIP-155](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md)
    pub fn protected(&self) -> bool {
        match self {
            Transaction::LegacyTransaction(tx) if tx.v.bits() <= 8 => {
                let v = tx.v.low_u64();
                v != 27 && v != 28 && v != 1 && v != 0
            }
            _ => true,
        }
    }
}

fn derive_legacy_chain_id(v: U256) -> Option<u64> {
    // EIP-155 encodes the chain id as `v = chain_id * 2 + 35` (or 36), so any
    // replay-protected `v` is >= 35. Pre-EIP-155 txs use v=27/28, and malformed
    // signatures (e.g. v=0 from an unsigned IL transaction) are < 35 too; none
    // carry a chain id. Guard the subtraction to avoid an underflow panic.
    // The chain id is derived from the whole `v` and must fit in 64 bits. A
    // larger one leaves the transaction without a chain id, so its `v`, which
    // is neither 27 nor 28, fails signature recovery as an invalid `v`.
    let chain_id = v.checked_sub(U256::from(35))? >> 1;
    u64::try_from(chain_id).ok()
}

impl TxType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::Legacy),
            0x01 => Some(Self::EIP2930),
            0x02 => Some(Self::EIP1559),
            0x03 => Some(Self::EIP4844),
            0x04 => Some(Self::EIP7702),
            0x06 => Some(Self::Frame),
            0x7d => Some(Self::FeeToken),
            0x7e => Some(Self::Privileged),
            _ => None,
        }
    }

    /// The typed-envelope form of this transaction type, or `None` for [`TxType::Legacy`]
    /// (a bare RLP list, not a `0x{type} || payload` envelope). Exhaustive over `TxType`
    /// with no wildcard, so adding a new variant won't compile until it's classified here —
    /// keeping envelope decoding in step with the transaction-type set.
    pub fn as_envelope(self) -> Option<EnvelopeTxType> {
        match self {
            TxType::EIP2930 => Some(EnvelopeTxType::EIP2930),
            TxType::EIP1559 => Some(EnvelopeTxType::EIP1559),
            TxType::EIP4844 => Some(EnvelopeTxType::EIP4844),
            TxType::EIP7702 => Some(EnvelopeTxType::EIP7702),
            TxType::Frame => Some(EnvelopeTxType::Frame),
            TxType::FeeToken => Some(EnvelopeTxType::FeeToken),
            TxType::Privileged => Some(EnvelopeTxType::Privileged),
            TxType::Legacy => None,
        }
    }

    /// Transaction types that only exist on the L2 rollup and must never appear in
    /// an L1 block (`FeeToken` 0x7d, `Privileged` 0x7e). Privileged transactions in
    /// particular take their sender from an unsigned, caller-chosen `from` field.
    ///
    /// This match is intentionally exhaustive (no wildcard arm): adding a new
    /// `TxType` variant will not compile until it is explicitly classified here,
    /// so an L2-only type can never be silently accepted on L1 by omission.
    pub fn is_l2_only(self) -> bool {
        match self {
            Self::Legacy
            | Self::EIP2930
            | Self::EIP1559
            | Self::EIP4844
            | Self::EIP7702
            | Self::Frame => false,
            Self::FeeToken | Self::Privileged => true,
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
    #[rkyv(with=rkyv::with::Skip)]
    pub cached_canonical: OnceCell<Vec<u8>>,
}

/// EIP-8141 Frame Transaction mode
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
#[repr(u8)]
pub enum FrameMode {
    #[default]
    Default = 0,
    Verify = 1,
    Sender = 2,
    /// EIP-7906 `ASSERT`: a read-only assertion frame. `ASSERT` frames form a
    /// trailing suffix of the frame list and are the only context in which
    /// `TXTRACE` and `EVENTDATACOPY` are valid. One that fails reverts
    /// the execution body without invalidating the transaction.
    Assert = 3,
}

impl FrameMode {
    /// Convert from the lower 8 bits of the mode field.
    /// Returns None for reserved values (4-255).
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(FrameMode::Default),
            1 => Some(FrameMode::Verify),
            2 => Some(FrameMode::Sender),
            3 => Some(FrameMode::Assert),
            _ => None,
        }
    }
}

impl From<FrameMode> for u8 {
    fn from(mode: FrameMode) -> u8 {
        match mode {
            FrameMode::Default => 0,
            FrameMode::Verify => 1,
            FrameMode::Sender => 2,
            FrameMode::Assert => 3,
        }
    }
}

/// EIP-8141 Frame: a single execution step within a frame transaction.
///
/// `mode` is the execution mode (0=DEFAULT, 1=VERIFY, 2=SENDER, 3=ASSERT under
/// EIP-7906; 4-255 reserved).
/// `flags` bits: 0-1 = APPROVE scope restriction, 2 = atomic batch flag (valid
/// on DEFAULT and SENDER frames only), 3-7 reserved (must be zero).
#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub struct Frame {
    pub mode: u8,
    pub flags: u8,
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::H160Wrapper>)]
    pub target: Option<Address>,
    /// `limits.execution` -- the frame's execution-gas budget.
    pub gas_limit: u64,
    /// `limits.state` -- the frame's EIP-8037 state-gas budget. The two
    /// dimensions are independent: neither can be spent in pursuit of the other,
    /// and unused gas in one is not available to the other.
    pub state_gas_limit: u64,
    // Per EIP-8141 the frame is a 6-tuple [mode, flags, target, limits, value, data]
    // where `limits` is itself the 2-list [execution, state]. Only SENDER frames
    // may carry a non-zero value; see `validate_static_constraints`.
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub value: U256,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub data: Bytes,
}

impl Frame {
    /// Extract the execution mode from the `mode` field.
    pub fn execution_mode(&self) -> FrameMode {
        FrameMode::from_u8(self.mode).unwrap_or_default()
    }

    /// Extract the APPROVE scope restriction from bits 0-1 of `flags`.
    pub fn scope_restriction(&self) -> u8 {
        self.flags & 0x03
    }

    /// Check if the atomic batch flag (bit 2 of `flags`) is set.
    pub fn is_atomic_batch(&self) -> bool {
        (self.flags >> 2) & 1 == 1
    }

    /// An expiry verifier frame is a VERIFY frame targeting EXPIRY_VERIFIER
    /// (EIP-8141).
    pub fn is_expiry_verifier(&self) -> bool {
        self.execution_mode() == FrameMode::Verify
            && self.target == Some(frame_tx_expiry_verifier())
    }

    /// EIP-8272 §Recent root verifier frame: a VERIFY frame to `RECENT_ROOT_ADDRESS`
    /// with no flags, no value, no state budget, and data that is one to sixteen
    /// packed 72-byte tuples. A frame to that address with any other shape is an
    /// ordinary frame, which the contract handles as such and the public mempool
    /// judges by the ordinary rules.
    pub fn is_recent_root_verifier(&self) -> bool {
        let len = self.data.len();
        self.execution_mode() == FrameMode::Verify
            && self.target == Some(frame_tx_recent_root())
            && self.flags == 0
            && self.value.is_zero()
            && self.state_gas_limit == 0
            && (FRAME_TX_RECENT_ROOT_TUPLE_BYTES
                ..=FRAME_TX_MAX_RECENT_ROOT_REFERENCES * FRAME_TX_RECENT_ROOT_TUPLE_BYTES)
                .contains(&len)
            && len.is_multiple_of(FRAME_TX_RECENT_ROOT_TUPLE_BYTES)
    }

    /// The tuples a recent root verifier frame carries, in order. Empty for any frame
    /// [`Frame::is_recent_root_verifier`] rejects.
    pub fn recent_root_tuples(&self) -> Vec<RecentRootReference> {
        if !self.is_recent_root_verifier() {
            return Vec::new();
        }
        self.data
            .chunks_exact(FRAME_TX_RECENT_ROOT_TUPLE_BYTES)
            .filter_map(RecentRootReference::from_tuple_bytes)
            .collect()
    }
}

impl RLPEncode for Frame {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        // target: encode as address or RLP null (empty bytes) for None
        let target_kind = match self.target {
            Some(addr) => TxKind::Call(addr),
            None => TxKind::Create, // RLP_NULL encodes "no target" → sender
        };
        Encoder::new(buf)
            .encode_field(&(self.mode as u64))
            .encode_field(&(self.flags as u64))
            .encode_field(&target_kind)
            .encode_field(&(self.gas_limit, self.state_gas_limit))
            .encode_field(&self.value)
            .encode_field(&self.data)
            .finish();
    }
}

impl RLPDecode for Frame {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (mode_u64, decoder): (u64, _) = decoder.decode_field("mode")?;
        let mode = u8::try_from(mode_u64)
            .map_err(|_| RLPDecodeError::Custom(format!("Frame mode too large: {mode_u64}")))?;
        let (flags_u64, decoder): (u64, _) = decoder.decode_field("flags")?;
        let flags = u8::try_from(flags_u64)
            .map_err(|_| RLPDecodeError::Custom(format!("Frame flags too large: {flags_u64}")))?;
        let (target_kind, decoder): (TxKind, _) = decoder.decode_field("target")?;
        let target = match target_kind {
            TxKind::Call(addr) => Some(addr),
            TxKind::Create => None,
        };
        let ((gas_limit, state_gas_limit), decoder): ((u64, u64), _) =
            decoder.decode_field("limits")?;
        let (value, decoder): (U256, _) = decoder.decode_field("value")?;
        let (data, decoder) = decoder.decode_field("data")?;
        let frame = Frame {
            mode,
            flags,
            target,
            gas_limit,
            state_gas_limit,
            value,
            data,
        };
        Ok((frame, decoder.finish()?))
    }
}

/// One of the three protocol verifiers of EIP-8141: reserved claim-only
/// addresses whose claims the protocol checks with a native function. A protocol
/// verifier fixes its own verification gas and witness length, which is why a
/// claim that names one declares both as zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolVerifier {
    Secp256k1,
    P256,
    LeanSphincs,
}

impl ProtocolVerifier {
    /// The protocol verifier at `address`, if it is one of the three.
    pub fn from_address(address: &Address) -> Option<Self> {
        if *address == SECP256K1_VERIFIER {
            Some(Self::Secp256k1)
        } else if *address == P256_VERIFIER {
            Some(Self::P256)
        } else if *address == LEANSPHINCS_VERIFIER {
            Some(Self::LeanSphincs)
        } else {
            None
        }
    }

    /// The gas charged for verifying one claim of this verifier.
    pub const fn verification_gas(self) -> u64 {
        match self {
            Self::Secp256k1 => SECP256K1_VERIFICATION_GAS,
            Self::P256 => P256_VERIFICATION_GAS,
            Self::LeanSphincs => LEANSPHINCS_VERIFICATION_GAS,
        }
    }

    /// The exact length in bytes of a witness of this verifier.
    pub const fn witness_len(self) -> u64 {
        match self {
            Self::Secp256k1 => SECP256K1_WITNESS_SIZE as u64,
            Self::P256 => P256_WITNESS_SIZE as u64,
            Self::LeanSphincs => LEANSPHINCS_SIGNATURE_SIZE as u64,
        }
    }
}

/// EIP-8141 claim. RLP: `[verifier, key, msg, gas_limit, witness_len]`.
///
/// A claim states that the verifier at `verifier` attests that a key authorizes a
/// message. The bytes that satisfy it, its witness, are not part of the
/// transaction: they travel beside it in the pooled representation and in the
/// block's `claim_witnesses`.
///
/// `key`: empty resolves to `tx.sender`; 20 bytes is an address; 32 bytes is used
/// as is. Either non-empty form must be non-zero.
/// `msg`: empty stands for `compute_sig_hash(tx)`; otherwise an explicit non-zero
/// 32-byte digest.
/// `gas_limit` and `witness_len` are zero for a protocol verifier, which fixes
/// both. For any other verifier they are the gas its verification is given, and
/// charged in full, and the exact length of the witness.
#[derive(Clone, Debug, PartialEq, Eq, Default, RSerialize, RDeserialize, Archive)]
pub struct Claim {
    #[rkyv(with=crate::rkyv_utils::H160Wrapper)]
    pub verifier: Address,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub key: Bytes,
    #[rkyv(with=crate::rkyv_utils::BytesWrapper)]
    pub msg: Bytes,
    pub gas_limit: u64,
    pub witness_len: u64,
}

impl Claim {
    /// The protocol verifier this claim names, if any.
    pub fn protocol_verifier(&self) -> Option<ProtocolVerifier> {
        ProtocolVerifier::from_address(&self.verifier)
    }

    /// EIP-8141 `resolve_key`: the 32-byte key the claim is about. `None` for a
    /// `key` the EIP rejects: a length other than 0, 20 or 32, or all zeros.
    pub fn resolved_key(&self, tx_sender: Address) -> Option<H256> {
        match self.key.len() {
            0 => Some(H256::from(tx_sender)),
            20 => {
                let address = Address::from_slice(&self.key);
                (!address.is_zero()).then(|| H256::from(address))
            }
            32 => {
                let key = H256::from_slice(&self.key);
                (!key.is_zero()).then_some(key)
            }
            _ => None,
        }
    }

    /// The 32-byte message the claim is about: `sig_hash` for an empty `msg`,
    /// otherwise `msg` itself. `None` for a `msg` the EIP rejects: a length other
    /// than 0 or 32, or all zeros.
    pub fn resolved_msg(&self, sig_hash: H256) -> Option<H256> {
        match self.msg.len() {
            0 => Some(sig_hash),
            32 => {
                let msg = H256::from_slice(&self.msg);
                (!msg.is_zero()).then_some(msg)
            }
            _ => None,
        }
    }

    /// EIP-8141 `resolve_claim_gas`: the gas charged for verifying the claim.
    pub fn resolved_gas(&self) -> u64 {
        self.protocol_verifier()
            .map_or(self.gas_limit, ProtocolVerifier::verification_gas)
    }

    /// EIP-8141 `resolve_witness_len`: the exact length of the claim's witness.
    pub fn resolved_witness_len(&self) -> u64 {
        self.protocol_verifier()
            .map_or(self.witness_len, ProtocolVerifier::witness_len)
    }

    /// EIP-8141 `resolve_dependency`. `None` when the claim's `key` or `msg` is
    /// one the EIP rejects.
    pub fn resolve_dependency(&self, tx_sender: Address, sig_hash: H256) -> Option<Dependency> {
        Some(Dependency {
            verifier: self.verifier,
            key: self.resolved_key(tx_sender)?,
            msg: self.resolved_msg(sig_hash)?,
            gas_limit: self.resolved_gas(),
            witness_len: self.resolved_witness_len(),
        })
    }
}

impl RLPEncode for Claim {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.verifier)
            .encode_field(&self.key)
            .encode_field(&self.msg)
            .encode_field(&self.gas_limit)
            .encode_field(&self.witness_len)
            .finish();
    }
}

impl RLPDecode for Claim {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Claim, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (verifier, decoder) = decoder.decode_field("verifier")?;
        let (key, decoder) = decoder.decode_field("key")?;
        let (msg, decoder) = decoder.decode_field("msg")?;
        let (gas_limit, decoder) = decoder.decode_field("gas_limit")?;
        let (witness_len, decoder) = decoder.decode_field("witness_len")?;
        Ok((
            Claim {
                verifier,
                key,
                msg,
                gas_limit,
                witness_len,
            },
            decoder.finish()?,
        ))
    }
}

/// EIP-8141 Frame Transaction
/// A transaction whose validity and gas payment are defined abstractly via frames.
/// No ECDSA signature — sender is explicit. Authentication happens via APPROVE opcode.
#[derive(Clone, Debug, PartialEq, Eq, RSerialize, RDeserialize, Archive)]
pub struct FrameTransaction {
    pub chain_id: u64,
    /// EIP-8250 keyed nonces, replacing EIP-8141's single `nonce`: up to sixteen
    /// strictly increasing keys sharing [`Self::nonce_seq`]. The empty set selects
    /// the sender's account nonce; any other set selects independent sequences held
    /// in the `NONCE_MANAGER` system contract, key `0` included.
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::U256Wrapper>)]
    pub nonce_keys: Vec<U256>,
    /// EIP-8250: the sequence every selected nonce key must currently hold.
    pub nonce_seq: u64,
    #[rkyv(with=crate::rkyv_utils::H160Wrapper)]
    pub sender: Address,
    /// EIP-8141 claims: what the transaction asserts about keys and messages.
    /// Every claim's witness is verified before any frame executes, and frames
    /// test for a claim with `HASCLAIM`.
    pub claims: Vec<Claim>,
    pub frames: Vec<Frame>,
    /// EIP-8141 `fees.max_fee`: one budget, in wei, for all of the transaction's
    /// execution, state and blob gas. The fee fields are bounded at 2**256, not
    /// 2**64: a frame transaction may legitimately name a fee no balance could
    /// pay, and a node still has to decode it to reject it for the balance rather
    /// than for the field width.
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub max_fee: U256,
    #[rkyv(with=crate::rkyv_utils::U256Wrapper)]
    pub max_priority_fee_per_gas: U256,
    #[rkyv(with=rkyv::with::Map<crate::rkyv_utils::H256Wrapper>)]
    pub blob_versioned_hashes: Vec<H256>,
    #[rkyv(with=rkyv::with::Skip)]
    pub inner_hash: OnceCell<H256>,
    #[rkyv(with=rkyv::with::Skip)]
    pub cached_canonical: OnceCell<Vec<u8>>,
}

impl Default for FrameTransaction {
    /// Selects the empty nonce key set, the sender's account nonce (EIP-8250).
    fn default() -> Self {
        Self {
            chain_id: 0,
            nonce_keys: Vec::new(),
            nonce_seq: 0,
            sender: Address::zero(),
            claims: Vec::new(),
            frames: Vec::new(),
            max_fee: U256::zero(),
            max_priority_fee_per_gas: U256::zero(),
            blob_versioned_hashes: Vec::new(),
            inner_hash: OnceCell::new(),
            cached_canonical: OnceCell::new(),
        }
    }
}

/// Intrinsic gas cost for frame transactions (EIP-8141)
pub const FRAME_TX_INTRINSIC_COST: u64 = 12000;
/// EIP-2780 `TX_VALUE_COST`: the intrinsic charge for a value-moving frame.
pub const TX_VALUE_COST: u64 = 6000;
/// Per-frame cost (EIP-8141): CALL context overhead (100) + G_log (375)
pub const FRAME_TX_PER_FRAME_COST: u64 = 475;
/// ENTRY_POINT address used as caller for DEFAULT/VERIFY frames per EIP-8141.
pub const FRAME_TX_ENTRY_POINT_U64: u64 = 0xaa;
/// Maximum number of frames allowed per EIP-8141 frame transaction.
pub const FRAME_TX_MAX_FRAMES: usize = 64;
/// EIP-8250 `MAX_NONCE_KEYS`: the most nonce keys a frame transaction may select.
pub const FRAME_TX_MAX_NONCE_KEYS: usize = 16;
/// EIP-8250 `MAX_NONCE_SEQ`: the bound of a nonce sequence, and the single-use
/// sequence of a keyed transaction.
pub const FRAME_TX_MAX_NONCE_SEQ: u64 = u64::MAX;
/// EIP-8250 `KEYED_NONCE_ACCESS_COST`: EIP-8038's `COLD_STORAGE_ACCESS` (2,100) plus
/// `STORAGE_WRITE` (10,000), the cold read and the write of one keyed-nonce slot. It
/// tracks the Amsterdam storage schedule in `ethrex_levm::gas_cost`, which a test pins.
pub const FRAME_TX_KEYED_NONCE_ACCESS_COST: u64 = 2_100 + 10_000;
/// EIP-8272 `MAX_RECENT_ROOT_REFERENCES`: the most `(source_id, slot, root)` tuples a
/// recent root verifier frame may carry.
pub const FRAME_TX_MAX_RECENT_ROOT_REFERENCES: usize = 16;
/// EIP-8272 `RECENT_ROOT_TUPLE_BYTES`: one packed tuple, `source_id(32) || slot(8) || root(32)`.
pub const FRAME_TX_RECENT_ROOT_TUPLE_BYTES: usize = 72;
/// EIP-7623 `STANDARD_TOKEN_COST`, and the EIP-7976 floor per token. Frame
/// transactions exist only from Hegota onward, which is after Amsterdam, so the
/// raised EIP-7976 floor always applies and neither needs a fork parameter.
const FRAME_TX_STANDARD_TOKEN_COST: u64 = 4;
const FRAME_TX_TOTAL_COST_FLOOR_PER_TOKEN: u64 = 16;
/// EIP-8141 `MAX_CLAIMS`: the most claims a frame transaction may declare.
pub const MAX_CLAIMS: usize = 64;
/// EIP-8141 `MAX_CLAIM_WITNESS_BYTES`: the longest witness a custom verifier's
/// claim may declare.
pub const MAX_CLAIM_WITNESS_BYTES: u64 = 1_048_576;
/// EIP-8141 `MAX_BLOCK_WITNESS_BYTES`: the longest `claim_witnesses` byte string
/// a block may carry (provisional).
pub const MAX_BLOCK_WITNESS_BYTES: usize = 2_097_152;
/// EIP-8141 `WITNESS_BYTE_GAS`: gas per byte of a protocol verifier's witness
/// (provisional).
pub const WITNESS_BYTE_GAS: u64 = 4;
/// EIP-8141 `CLAIM_WITNESSES_VERSION_RAW`: the version byte that opens a block's
/// `claim_witnesses` when it lists raw witnesses.
pub const CLAIM_WITNESSES_VERSION_RAW: u8 = 0x00;
/// The version byte that opens a block's `claim_witnesses` from IStar on, when
/// one aggregate proof stands for the witnesses of the block's
/// `LEANSPHINCS_VERIFIER` dependencies and the other witnesses stay raw. EIP-8141
/// §Block Witnesses leaves the byte to the aggregation upgrade and gives `0x01`
/// as its example.
pub const CLAIM_WITNESSES_VERSION_AGGREGATED: u8 = 0x01;
/// EIP-8288 `MAX_DEPENDENCY_PROOF_BYTES`: the longest aggregate proof a block's
/// `claim_witnesses` may carry.
pub const MAX_AGGREGATE_PROOF_BYTES: usize = 1_048_576;
/// EIP-8141 `POOLED_WRAPPER_VERSION`: the only wrapper version of the pooled
/// representation of a frame transaction.
pub const POOLED_WRAPPER_VERSION: u8 = 0;
/// EIP-8141 `SECP256K1_VERIFICATION_GAS`.
pub const SECP256K1_VERIFICATION_GAS: u64 = 2800;
/// EIP-8141 `P256_VERIFICATION_GAS`.
pub const P256_VERIFICATION_GAS: u64 = 6700;
/// EIP-8141 `LEANSPHINCS_VERIFICATION_GAS`.
pub const LEANSPHINCS_VERIFICATION_GAS: u64 = 13200;
/// Bytes of the witness of a `SECP256K1_VERIFIER` claim: `v || r || s`.
pub const SECP256K1_WITNESS_SIZE: usize = 65;
/// Bytes of the witness of a `P256_VERIFIER` claim: `r || s || qx || qy`.
pub const P256_WITNESS_SIZE: usize = 128;
/// EIP-8141 `LEANSPHINCS_SIGNATURE_SIZE`: bytes of a leanSPHINCS signature, the
/// witness of a `LEANSPHINCS_VERIFIER` claim.
pub const LEANSPHINCS_SIGNATURE_SIZE: usize = 7740;
/// EIP-8141 `SECP256K1_VERIFIER`, a reserved claim-only address: it names the
/// native secp256k1 verifier in a claim and is not a precompile, so calling it
/// from the EVM is calling an account without code. The EIP leaves the three
/// protocol verifier addresses to be determined; these follow `ENTRY_POINT`
/// (`0xaa`), outside the precompile range and the range EIP-7587 reserves.
pub const SECP256K1_VERIFIER: Address = ethereum_types::H160(hex_literal::hex!(
    "00000000000000000000000000000000000000ab"
));
/// EIP-8141 `P256_VERIFIER`, a reserved claim-only address (see
/// [`SECP256K1_VERIFIER`]).
pub const P256_VERIFIER: Address = ethereum_types::H160(hex_literal::hex!(
    "00000000000000000000000000000000000000ac"
));
/// EIP-8141 `LEANSPHINCS_VERIFIER`, a reserved claim-only address (see
/// [`SECP256K1_VERIFIER`]).
pub const LEANSPHINCS_VERIFIER: Address = ethereum_types::H160(hex_literal::hex!(
    "00000000000000000000000000000000000000ad"
));
/// EIP-8141 §Mempool `MAX_VERIFY_GAS`: the maximum gas
/// a public-mempool node should expend verifying claims and simulating the
/// validation prefix. Claim verification counts against this budget (rule #6),
/// so a frame tx whose `claim_verification_cost()` alone exceeds it can never
/// satisfy the prefix budget and must be rejected at admission.
pub const FRAME_TX_MAX_VERIFY_GAS: u64 = 100_000;
/// EIP-8141 `MAX_VERIFY_STATE_GAS`: the most state gas the frames of a validation
/// prefix may declare between them (structural rule 6). It bounds the state growth
/// a public-mempool transaction can buy while establishing its payer -- a deploy
/// frame, the sender-account creation an `APPROVE` may charge, and under EIP-8250
/// the keyed nonce slots it creates -- and measures no node validation work, which
/// `MAX_VERIFY_GAS` alone bounds.
pub const FRAME_TX_MAX_VERIFY_STATE_GAS: u64 = 500_000;
/// EIP-8141 APPROVE scope-restriction values (bits 0-1 of `Frame.flags`).
/// Used by VERIFY and PAY frames to declare which capabilities they grant.
pub const APPROVE_PAYMENT: u8 = 0x1;
pub const APPROVE_EXECUTION: u8 = 0x2;
pub const APPROVE_EXECUTION_AND_PAYMENT: u8 = 0x3;
/// Maximum number of pending frame txs using a non-canonical paymaster per
/// paymaster address. Per OQ1, all paymasters are currently non-canonical
/// (FRAME_CANONICAL_PAYMASTER_CODE_HASH is unresolved in the draft EIP), so
/// this de-facto limits sponsored frame txs to 1 per paymaster in the pool.
pub const FRAME_TX_MAX_PENDING_NONCANONICAL_PAYMASTER: u8 = 1;

/// Returns the ENTRY_POINT `Address` (0x…00aa) used as caller for
/// DEFAULT/VERIFY frames per EIP-8141.
pub fn frame_tx_entry_point() -> Address {
    Address::from_low_u64_be(FRAME_TX_ENTRY_POINT_U64)
}

/// EIP-8141 `EXPIRY_VERIFIER`: an ordinary contract that its keyless deployment
/// transaction creates at this address. The protocol installs nothing at activation;
/// until the contract is deployed an expiry verifier frame runs the default code.
pub const FRAME_TX_EXPIRY_VERIFIER: Address = ethereum_types::H160(hex_literal::hex!(
    "81413f0cF12e9b6a49B1D0439E081c577D57FfFf"
));
/// Required `data` length for an expiry verifier frame (8-byte BE deadline).
pub const FRAME_TX_EXPIRY_DATA_LENGTH: usize = 8;

/// Returns the EXPIRY_VERIFIER `Address` per EIP-8141.
pub fn frame_tx_expiry_verifier() -> Address {
    FRAME_TX_EXPIRY_VERIFIER
}

/// EIP-8250 `NONCE_MANAGER`, whose storage holds every sender's keyed nonce
/// sequences. An ordinary contract that its keyless deployment transaction creates at
/// this address; EIP-8250 requires it in state, deployed or in genesis, when the fork
/// activates.
pub const FRAME_TX_NONCE_MANAGER: Address = ethereum_types::H160(hex_literal::hex!(
    "8250968C12e01A19d6F667b9B2F3b3A4d0e51cB7"
));

/// Returns the `NONCE_MANAGER` `Address` per EIP-8250.
pub fn frame_tx_nonce_manager() -> Address {
    FRAME_TX_NONCE_MANAGER
}

/// EIP-8250 `nonce_slot`: the `NONCE_MANAGER` storage slot holding `sender`'s
/// sequence for `nonce_key`, `keccak256(left_pad_32(sender) || uint256_be(nonce_key))`.
pub fn keyed_nonce_slot(sender: Address, nonce_key: U256) -> H256 {
    let mut buf = [0u8; 64];
    buf[12..32].copy_from_slice(sender.as_bytes());
    buf[32..].copy_from_slice(&nonce_key.to_big_endian());
    keccak(buf)
}

/// EIP-8272 `RECENT_ROOT_ADDRESS`: an ordinary contract that its keyless deployment
/// transaction creates at this address. Until it is deployed, a recent root verifier
/// frame runs the default code, which reverts.
pub const FRAME_TX_RECENT_ROOT: Address = ethereum_types::H160(hex_literal::hex!(
    "8272D9679689Ea2f307140CdF9002D27dC00Ffff"
));

/// Returns the `RECENT_ROOT_ADDRESS` `Address` per EIP-8272.
pub fn frame_tx_recent_root() -> Address {
    FRAME_TX_RECENT_ROOT
}

/// EIP-8272 `RECENT_ROOT_LENGTH`: each root source keeps one entry per slot for the
/// last `RECENT_ROOT_LENGTH` slots, at `slot mod RECENT_ROOT_LENGTH`.
pub const FRAME_TX_RECENT_ROOT_LENGTH: u64 = 8192;
/// EIP-8272 `RECENT_ROOT_USABLE_WINDOW`: a reference is usable while
/// `current_slot - slot <= RECENT_ROOT_USABLE_WINDOW`. One less than the ring length
/// because the current slot itself is never referenceable.
pub const FRAME_TX_RECENT_ROOT_USABLE_WINDOW: u64 = 8191;

/// EIP-8272 `RECENT_ROOT_ENTRY_DOMAIN`: `keccak256("RECENT_ROOT_ENTRY")`.
pub fn recent_root_entry_domain() -> H256 {
    keccak(b"RECENT_ROOT_ENTRY")
}

/// EIP-8272 `RECENT_ROOT_STORAGE_DOMAIN`: `keccak256("RECENT_ROOT_STORAGE")`.
pub fn recent_root_storage_domain() -> H256 {
    keccak(b"RECENT_ROOT_STORAGE")
}

/// One `(source_id, slot, root)` tuple of an EIP-8272 recent root verifier frame,
/// packed in the frame's data as `source_id(32) || uint64_be(slot) || root(32)`.
/// `root` is opaque to consensus; applications bind its meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RecentRootReference {
    pub source_id: H256,
    pub slot: u64,
    pub root: H256,
}

impl RecentRootReference {
    /// Parse one packed tuple. `None` unless `bytes` is exactly
    /// `RECENT_ROOT_TUPLE_BYTES` long.
    pub fn from_tuple_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != FRAME_TX_RECENT_ROOT_TUPLE_BYTES {
            return None;
        }
        let slot = u64::from_be_bytes(bytes.get(32..40)?.try_into().ok()?);
        Some(Self {
            source_id: H256::from_slice(bytes.get(..32)?),
            slot,
            root: H256::from_slice(bytes.get(40..)?),
        })
    }

    /// EIP-8272 `entry_hash`:
    /// `keccak256(RECENT_ROOT_ENTRY_DOMAIN || source_id || uint64_be(slot) || root)`.
    /// It commits to the full slot, not its ring index, so an entry overwritten by a
    /// newer slot sharing the index can never satisfy a reference to the older one.
    pub fn entry_hash(&self) -> H256 {
        let mut buf = Vec::with_capacity(32 + 32 + 8 + 32);
        buf.extend_from_slice(recent_root_entry_domain().as_bytes());
        buf.extend_from_slice(self.source_id.as_bytes());
        buf.extend_from_slice(&self.slot.to_be_bytes());
        buf.extend_from_slice(self.root.as_bytes());
        keccak(&buf)
    }

    /// EIP-8272 `storage_key`:
    /// `keccak256(RECENT_ROOT_STORAGE_DOMAIN || source_id || uint64_be(slot mod RECENT_ROOT_LENGTH))`.
    pub fn storage_key(&self) -> H256 {
        let mut buf = Vec::with_capacity(32 + 32 + 8);
        buf.extend_from_slice(recent_root_storage_domain().as_bytes());
        buf.extend_from_slice(self.source_id.as_bytes());
        buf.extend_from_slice(&(self.slot % FRAME_TX_RECENT_ROOT_LENGTH).to_be_bytes());
        keccak(&buf)
    }
}

/// EIP-8141's fee budget of a frame transaction at an including block's base fees
/// (see [`FrameTransaction::fee_budget`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFeeBudget {
    /// `max_cost`: the escrow collected from the payer on payment approval, the
    /// worst-case base-fee charge plus the largest tip the `max_fee` budget allows.
    /// Never above `max_fee`.
    pub max_cost: U256,
    /// What `GASPRICE` returns: the base fee plus `max_priority_fee // max_gas`. The
    /// payer pays at least this per unit of gas used, blob gas aside.
    pub gas_price: U256,
}

/// EIP-8141 settlement of a frame transaction (see [`FrameFeeBudget::settle`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFeeSettlement {
    /// `base_fee_paid`: the gas used at the base fee plus the blob gas at the blob
    /// base fee, burned.
    pub base_fee_paid: U256,
    /// `priority_fee_paid`: credited to the coinbase.
    pub priority_fee_paid: U256,
    /// `payer_refund`: the rest of the escrow, returned to the payer.
    pub payer_refund: U256,
}

impl FrameFeeBudget {
    /// Split the escrow after execution: the gas used at the base fee and the blob
    /// gas at the blob base fee are burned, the coinbase receives the priority fee
    /// per gas on the gas used, capped by what the escrow holds after the burn, and
    /// the payer gets back the rest. `gas_used <= max_gas`, so the burn never
    /// exceeds `max_cost` and the refund is never negative.
    pub fn settle(
        &self,
        tx: &FrameTransaction,
        gas_used: u64,
        base_fee_per_gas: U256,
        blob_base_fee: U256,
    ) -> FrameFeeSettlement {
        let base_fee_paid = U256::from(gas_used)
            .saturating_mul(base_fee_per_gas)
            .saturating_add(U256::from(tx.blob_gas()).saturating_mul(blob_base_fee));
        let escrow_left = self.max_cost.saturating_sub(base_fee_paid);
        let priority_fee_paid = tx
            .max_priority_fee_per_gas
            .saturating_mul(U256::from(gas_used))
            .min(escrow_left);
        FrameFeeSettlement {
            base_fee_paid,
            priority_fee_paid,
            payer_refund: escrow_left - priority_fee_paid,
        }
    }
}

/// EIP-8141 dependency: a claim with its key, message, gas and witness length
/// resolved, `(verifier, key, msg, gas_limit, witness_len)`.
///
/// It states that the verifier at `verifier` attests that `key` authorizes
/// `msg`. A witness of exactly `witness_len` bytes that the verifier accepts
/// satisfies it; a block carries one witness for each of its distinct
/// dependencies.
///
/// The derived ordering is the one the EIP's `deduplicate_and_sort` uses, the
/// order of the tuple: `verifier`, `key` and `msg` compared byte by byte, then
/// `gas_limit` and `witness_len` as integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dependency {
    pub verifier: Address,
    pub key: H256,
    pub msg: H256,
    pub gas_limit: u64,
    pub witness_len: u64,
}

impl Dependency {
    /// The protocol verifier this dependency names, if any.
    pub fn protocol_verifier(&self) -> Option<ProtocolVerifier> {
        ProtocolVerifier::from_address(&self.verifier)
    }

    /// Whether a block at `fork` carries the witness of this dependency inside
    /// the aggregate proof of its `claim_witnesses` instead of raw. From IStar
    /// that is every `LEANSPHINCS_VERIFIER` dependency, the aggregatable protocol
    /// verifier of EIP-8141 §Aggregation and EIP-8288 compatibility; the
    /// dependencies of every other verifier keep a raw witness. Before IStar no
    /// dependency is aggregated.
    pub fn is_aggregated(&self, fork: super::Fork) -> bool {
        fork >= super::Fork::IStar && self.verifier == LEANSPHINCS_VERIFIER
    }

    /// The input a verifier receives: `key || msg || witness`.
    pub fn verifier_input(&self, witness: &[u8]) -> Vec<u8> {
        let mut input = Vec::with_capacity(64 + witness.len());
        input.extend_from_slice(self.key.as_bytes());
        input.extend_from_slice(self.msg.as_bytes());
        input.extend_from_slice(witness);
        input
    }
}

/// EIP-8141 `deduplicate_and_sort`.
fn deduplicate_and_sort(mut dependencies: Vec<Dependency>) -> Vec<Dependency> {
    dependencies.sort_unstable();
    dependencies.dedup();
    dependencies
}

/// EIP-8141 `block_dependencies`: the combined dependencies of a block's frame
/// transactions, sorted and deduplicated. A block carries exactly one witness
/// for each, in this order. `None` when a claim of one of the transactions has a
/// `key` or `msg` the EIP rejects, which makes that transaction, and so the
/// block, invalid.
pub fn block_dependencies(transactions: &[Transaction]) -> Option<Vec<Dependency>> {
    let mut dependencies = Vec::new();
    for tx in transactions {
        if let Transaction::FrameTransaction(frame_tx) = tx {
            dependencies.extend(frame_tx.dependencies()?);
        }
    }
    Some(deduplicate_and_sort(dependencies))
}

/// The byte string a block carries as `claim_witnesses` for `witnesses`, one for
/// each entry of its `block_dependencies` in that order (EIP-8141 §Block
/// Witnesses): empty when there are none, otherwise the raw version byte
/// followed by the RLP list of the witnesses.
pub fn encode_claim_witnesses(witnesses: &[Bytes]) -> Bytes {
    if witnesses.is_empty() {
        return Bytes::new();
    }
    // An RLP list of byte strings: the payload is the items one after another.
    let mut payload = Vec::new();
    for witness in witnesses {
        witness.encode(&mut payload);
    }
    let mut encoded = vec![CLAIM_WITNESSES_VERSION_RAW];
    ethrex_rlp::encode::encode_length(payload.len(), &mut encoded);
    encoded.extend_from_slice(&payload);
    Bytes::from(encoded)
}

/// The length in bytes of the `claim_witnesses` byte string that carries
/// `witnesses` ([`encode_claim_witnesses`]), without building it.
pub fn claim_witnesses_len<'a>(witnesses: impl IntoIterator<Item = &'a Bytes>) -> usize {
    claim_witnesses_len_for_payload(witnesses.into_iter().map(RLPEncode::length).sum())
}

/// The length in bytes of the `claim_witnesses` byte string whose witnesses,
/// each RLP-encoded as a byte string, add up to `payload_len` bytes.
pub const fn claim_witnesses_len_for_payload(payload_len: usize) -> usize {
    if payload_len == 0 {
        // A byte string is at least one byte long once encoded, so there are no
        // witnesses.
        return 0;
    }
    1 + ethrex_rlp::encode::list_length(payload_len)
}

/// The byte string a block carries as `claim_witnesses` from IStar on, for the
/// aggregate proof of its aggregated dependencies and one raw witness for each
/// of the others, in the order of `block_dependencies`
/// ([`Dependency::is_aggregated`]): empty when there is neither a proof nor a
/// witness, which is a block without dependencies, otherwise the aggregated
/// version byte followed by `rlp([aggregate_proof, [witness, ...]])`.
pub fn encode_aggregated_claim_witnesses(aggregate_proof: &[u8], witnesses: &[Bytes]) -> Bytes {
    if aggregate_proof.is_empty() && witnesses.is_empty() {
        return Bytes::new();
    }
    // The RLP list of the raw witnesses: its header, then the items one after
    // another.
    let mut witnesses_payload = Vec::new();
    for witness in witnesses {
        witness.encode(&mut witnesses_payload);
    }
    let mut witness_list = Vec::new();
    ethrex_rlp::encode::encode_length(witnesses_payload.len(), &mut witness_list);
    witness_list.extend_from_slice(&witnesses_payload);
    let mut encoded = vec![CLAIM_WITNESSES_VERSION_AGGREGATED];
    Encoder::new(&mut encoded)
        .encode_bytes(aggregate_proof)
        .encode_raw(&witness_list)
        .finish();
    Bytes::from(encoded)
}

/// The length in bytes of the `claim_witnesses` byte string that carries an
/// aggregate proof of `aggregate_proof_len` bytes and raw witnesses that, each
/// RLP-encoded as a byte string, add up to `payload_len` bytes
/// ([`encode_aggregated_claim_witnesses`]), without building it. A one-byte
/// proof is counted with a header, which it has only when its byte is above
/// `0x7f`.
pub const fn aggregated_claim_witnesses_len(
    aggregate_proof_len: usize,
    payload_len: usize,
) -> usize {
    if aggregate_proof_len == 0 && payload_len == 0 {
        return 0;
    }
    1 + ethrex_rlp::encode::list_length(
        ethrex_rlp::encode::bytes_length(aggregate_proof_len, 0xff)
            + ethrex_rlp::encode::list_length(payload_len),
    )
}

/// Why a block's `claim_witnesses` byte string does not decode.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClaimWitnessesDecodeError {
    #[error("claim witnesses are {0} bytes, more than MAX_BLOCK_WITNESS_BYTES")]
    TooLong(usize),
    /// The version byte is not the one the block's fork defines: `0x00` before
    /// IStar and `0x01` from IStar on.
    #[error("claim witnesses version {0:#04x} is not the one the block's fork defines")]
    UnknownVersion(u8),
    #[error("claim witnesses are not a list of byte strings: {0}")]
    Malformed(String),
    #[error("claim witnesses hold an empty list, which is encoded as the empty byte string")]
    EmptyList,
    #[error(
        "claim witnesses hold no aggregate proof and no witness, which is encoded as the empty byte string"
    )]
    NothingCarried,
    #[error("the aggregate proof is {0} bytes, more than MAX_AGGREGATE_PROOF_BYTES")]
    AggregateProofTooLong(usize),
}

/// What a block's `claim_witnesses` byte string holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClaimWitnesses {
    /// One proof for all the dependencies of the block that are aggregated
    /// ([`Dependency::is_aggregated`]). Empty when the block has none, which is
    /// always the case before IStar.
    pub aggregate_proof: Bytes,
    /// One raw witness for each of the block's other dependencies, in the order
    /// of `block_dependencies`.
    pub witnesses: Vec<Bytes>,
}

/// Decodes a block's `claim_witnesses` byte string into its witnesses under the
/// rules that hold before IStar ([`decode_claim_witnesses_at`]). The empty byte
/// string holds none. Anything else is the raw version byte followed by the RLP
/// list of at least one witness, with nothing after it; an empty list has the
/// empty byte string as its only encoding.
pub fn decode_claim_witnesses(
    claim_witnesses: &[u8],
) -> Result<Vec<Bytes>, ClaimWitnessesDecodeError> {
    decode_claim_witnesses_at(super::Fork::Hegota, claim_witnesses)
        .map(|claim_witnesses| claim_witnesses.witnesses)
}

/// Decodes a block's `claim_witnesses` byte string under the rules of the fork
/// the block belongs to. It is the one place that reads the version byte.
///
/// The empty byte string holds nothing, at every fork, and is the only encoding
/// of a block without dependencies. Anything else opens with the version byte
/// its fork defines, and with no other:
///
/// - before IStar, [`CLAIM_WITNESSES_VERSION_RAW`] followed by the RLP list of
///   at least one witness (EIP-8141 §Block Witnesses);
/// - from IStar, [`CLAIM_WITNESSES_VERSION_AGGREGATED`] followed by
///   `rlp([aggregate_proof, [witness, ...]])`, where the proof is at most
///   [`MAX_AGGREGATE_PROOF_BYTES`] long and the proof and the list are not both
///   empty.
///
/// Nothing follows the RLP item in either form.
pub fn decode_claim_witnesses_at(
    fork: super::Fork,
    claim_witnesses: &[u8],
) -> Result<ClaimWitnesses, ClaimWitnessesDecodeError> {
    if claim_witnesses.len() > MAX_BLOCK_WITNESS_BYTES {
        return Err(ClaimWitnessesDecodeError::TooLong(claim_witnesses.len()));
    }
    let Some((version, encoded)) = claim_witnesses.split_first() else {
        return Ok(ClaimWitnesses::default());
    };
    let malformed = |error: RLPDecodeError| ClaimWitnessesDecodeError::Malformed(error.to_string());
    match (*version, fork >= super::Fork::IStar) {
        (CLAIM_WITNESSES_VERSION_RAW, false) => {
            let witnesses = Vec::<Bytes>::decode(encoded).map_err(malformed)?;
            if witnesses.is_empty() {
                return Err(ClaimWitnessesDecodeError::EmptyList);
            }
            Ok(ClaimWitnesses {
                aggregate_proof: Bytes::new(),
                witnesses,
            })
        }
        (CLAIM_WITNESSES_VERSION_AGGREGATED, true) => {
            let (aggregate_proof, witnesses) =
                <(Bytes, Vec<Bytes>)>::decode(encoded).map_err(malformed)?;
            if aggregate_proof.len() > MAX_AGGREGATE_PROOF_BYTES {
                return Err(ClaimWitnessesDecodeError::AggregateProofTooLong(
                    aggregate_proof.len(),
                ));
            }
            if aggregate_proof.is_empty() && witnesses.is_empty() {
                return Err(ClaimWitnessesDecodeError::NothingCarried);
            }
            Ok(ClaimWitnesses {
                aggregate_proof,
                witnesses,
            })
        }
        (version, _) => Err(ClaimWitnessesDecodeError::UnknownVersion(version)),
    }
}

impl FrameTransaction {
    /// EIP-8141 `tx_dependencies`: the resolved dependency of every claim, sorted
    /// and deduplicated. `None` when a claim has a `key` or `msg` the EIP
    /// rejects; `validate_static_constraints` rejects such a transaction.
    pub fn dependencies(&self) -> Option<Vec<Dependency>> {
        if self.claims.is_empty() {
            return Some(Vec::new());
        }
        self.claim_dependencies().map(deduplicate_and_sort)
    }

    /// The resolved dependency of each claim, in the order of `claims`, so entry
    /// `i` is the dependency `witnesses[i]` of the pooled representation must
    /// satisfy. `None` under the same condition as [`Self::dependencies`].
    pub fn claim_dependencies(&self) -> Option<Vec<Dependency>> {
        let sig_hash = self.compute_sig_hash();
        self.claims
            .iter()
            .map(|claim| claim.resolve_dependency(self.sender, sig_hash))
            .collect()
    }

    /// Canonical signature hash (EIP-8141 `compute_sig_hash`):
    /// `keccak(FRAME_TX_TYPE || 0x00 || rlp(tx_body))`. The transaction carries no
    /// witness bytes, so the hash covers all of it without eliding anything, and
    /// the extra zero byte keeps it apart from the transaction hash, which is
    /// `keccak(FRAME_TX_TYPE || rlp(tx_body))`.
    pub fn compute_sig_hash(&self) -> H256 {
        let mut buf = vec![TxType::Frame as u8, 0x00];
        self.encode(&mut buf);
        keccak(&buf)
    }

    /// EIP-8141 §Networking: the most bytes the `witnesses` list of this
    /// transaction's pooled representation takes once RLP-encoded. The list has
    /// one witness for each claim, each of the length its claim resolves to.
    ///
    /// A transaction has at most `MAX_CLAIMS` claims and no witness is longer
    /// than `MAX_CLAIM_WITNESS_BYTES`. Both limits are applied here, so the
    /// result is bounded for a transaction whose static constraints nobody has
    /// checked yet, which is when a node needs it.
    pub fn max_pooled_witnesses_len(&self) -> usize {
        let payload_len = self
            .claims
            .iter()
            .take(MAX_CLAIMS)
            .fold(0usize, |total, claim| {
                let witness_len = claim.resolved_witness_len().min(MAX_CLAIM_WITNESS_BYTES);
                let witness_len = usize::try_from(witness_len).unwrap_or(usize::MAX);
                // A one-byte string takes a header when its byte is above 0x7f.
                total.saturating_add(ethrex_rlp::encode::bytes_length(witness_len, 0xff))
            });
        ethrex_rlp::encode::list_length(payload_len)
    }

    /// EIP-8141 `claim_verification_cost`: the gas of verifying every claim, a
    /// protocol verifier's fixed cost or a custom verifier's whole `gas_limit`.
    /// Saturates; `validate_static_constraints` rejects a sum past 2**64.
    pub fn claim_verification_cost(&self) -> u64 {
        self.claims
            .iter()
            .fold(0u64, |acc, claim| acc.saturating_add(claim.resolved_gas()))
    }

    /// EIP-8141 `protocol_witness_cost`: `WITNESS_BYTE_GAS` per byte of the
    /// witness of every claim that names a protocol verifier. These witnesses are
    /// not counted toward the calldata floor.
    pub fn protocol_witness_cost(&self) -> u64 {
        self.claims
            .iter()
            .filter_map(Claim::protocol_verifier)
            .fold(0u64, |acc, verifier| {
                acc.saturating_add(WITNESS_BYTE_GAS.saturating_mul(verifier.witness_len()))
            })
    }

    /// Total declared witness length of the claims that name a custom verifier.
    /// A custom verifier can accept arbitrary bytes, so its witness is priced by
    /// length as non-zero calldata and counted toward the calldata floor.
    fn custom_witness_len(&self) -> u64 {
        self.claims
            .iter()
            .filter(|claim| claim.protocol_verifier().is_none())
            .fold(0u64, |acc, claim| acc.saturating_add(claim.witness_len))
    }

    /// Iterate the EIP-8141 data fields priced by their content: each frame's
    /// `data` and each claim's `verifier`, `key` and `msg`. The RLP framing and
    /// the remaining scalar fields are not included.
    fn data_fields(&self) -> impl Iterator<Item = &[u8]> {
        self.frames.iter().map(|f| f.data.as_ref()).chain(
            self.claims
                .iter()
                .flat_map(|c| [c.verifier.as_bytes(), c.key.as_ref(), c.msg.as_ref()]),
        )
    }

    /// EIP-7623 calldata cost over the frame data, the claim metadata and
    /// EIP-8250's nonce calldata, 4 gas per zero byte and 16 per non-zero byte,
    /// plus 16 gas per byte of every custom verifier's witness whatever its value
    /// (EIP-8141 `frame_data_cost + claim_metadata_cost + custom_witness_cost`).
    pub fn data_cost(&self) -> u64 {
        let nonce_calldata = self.nonce_calldata();
        self.data_fields()
            .chain(std::iter::once(nonce_calldata.as_slice()))
            .flatten()
            .fold(0u64, |acc, byte| {
                acc.saturating_add(if *byte == 0 { 4 } else { 16 })
            })
            .saturating_add(self.custom_witness_len().saturating_mul(16))
    }

    /// EIP-7623 token count over the frame data, the claim metadata, the custom
    /// verifiers' witnesses and EIP-8250's nonce calldata, used for the calldata
    /// floor. Frame transactions exist only from Hegota onward, which is after
    /// Amsterdam, so EIP-7976's unweighted count (every byte costs
    /// `STANDARD_TOKEN_COST`) always applies.
    pub fn calldata_tokens(&self) -> u64 {
        self.data_fields()
            .fold(0u64, |acc, field| acc.saturating_add(field.len() as u64))
            .saturating_add(self.custom_witness_len())
            .saturating_add(self.nonce_calldata().len() as u64)
            .saturating_mul(FRAME_TX_STANDARD_TOKEN_COST)
    }

    /// EIP-8250 `nonce_calldata`: `rlp(nonce_keys) || rlp(nonce_seq)`, the bytes the
    /// keyed nonce fields add to the payload. They are priced exactly as frame data
    /// and claim metadata, so they enter both [`Self::data_cost`] and
    /// [`Self::calldata_tokens`]. A single-use key set (keyed, at `MAX_NONCE_SEQ`)
    /// is exempt from pricing its `nonce_seq`, so the single-use form never costs
    /// more than first use at sequence zero; the field is still encoded and signed.
    pub fn nonce_calldata(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.nonce_keys.encode(&mut buf);
        if !self.is_single_use() {
            self.nonce_seq.encode(&mut buf);
        }
        buf
    }

    /// EIP-8250 `nonce_keys_hash`: `keccak256(uint256_be(len) || uint256_be(k) for k)`.
    /// Valid keys are strictly increasing, so each key set has one digest.
    pub fn nonce_keys_hash(&self) -> H256 {
        let mut buf = Vec::with_capacity(32 * (self.nonce_keys.len() + 1));
        buf.extend_from_slice(&U256::from(self.nonce_keys.len()).to_big_endian());
        for key in &self.nonce_keys {
            buf.extend_from_slice(&key.to_big_endian());
        }
        keccak(&buf)
    }

    /// Whether the transaction selects keyed nonces (EIP-8250) rather than the
    /// sender's account nonce, which the empty key set selects. Key `0` is an
    /// ordinary key.
    pub fn is_keyed(&self) -> bool {
        !self.nonce_keys.is_empty()
    }

    /// EIP-8250's single-use form: a keyed transaction at `MAX_NONCE_SEQ`, valid only
    /// while every selected key is unused, and exhausting each of them.
    pub fn is_single_use(&self) -> bool {
        self.is_keyed() && self.nonce_seq == FRAME_TX_MAX_NONCE_SEQ
    }

    /// EIP-8272: the index of the recent root verifier frame when it sits where the
    /// public mempool requires it -- first, or second behind an expiry verifier
    /// frame. A matching frame anywhere else is not the protocol verifier and fails
    /// the structural rules ([`FrameValidationError::RecentRootFrameMisplaced`]).
    pub fn recent_root_verifier_index(&self) -> Option<usize> {
        match self.frames.as_slice() {
            [first, ..] if first.is_recent_root_verifier() => Some(0),
            [first, second, ..]
                if first.is_expiry_verifier() && second.is_recent_root_verifier() =>
            {
                Some(1)
            }
            _ => None,
        }
    }

    /// The `(source_id, slot, root)` tuples the recent root verifier frame declares,
    /// empty when the transaction carries none in the mempool position.
    pub fn recent_root_tuples(&self) -> Vec<RecentRootReference> {
        self.recent_root_verifier_index()
            .and_then(|index| self.frames.get(index))
            .map(Frame::recent_root_tuples)
            .unwrap_or_default()
    }

    /// The EIP-7623 calldata floor: the least the frame data, the claim metadata
    /// and the custom verifiers' witnesses may cost, whatever execution actually
    /// consumes.
    pub fn calldata_floor_gas(&self) -> u64 {
        self.calldata_tokens()
            .saturating_mul(FRAME_TX_TOTAL_COST_FLOOR_PER_TOKEN)
    }

    /// The mandatory costs, always charged in full: the intrinsic cost, the
    /// per-frame cost, claim verification, the protocol verifiers' witnesses,
    /// value transfers, and EIP-8250's keyed-nonce access. They are the terms
    /// EIP-8141's `frame_tx_intrinsic_gas` and `calldata_floor_gas` share, so each
    /// is charged once on either side.
    pub fn mandatory_gas(&self) -> u64 {
        FRAME_TX_INTRINSIC_COST
            .saturating_add((self.frames.len() as u64).saturating_mul(FRAME_TX_PER_FRAME_COST))
            .saturating_add(self.claim_verification_cost())
            .saturating_add(self.protocol_witness_cost())
            .saturating_add(self.value_transfer_gas())
            .saturating_add(self.keyed_nonce_access_cost())
    }

    /// EIP-8250 `keyed_nonce_access_cost`: `KEYED_NONCE_ACCESS_COST` for every key,
    /// so nothing for the empty set. EIP-8250 adds it to both
    /// `frame_tx_intrinsic_gas` and `calldata_floor_gas`, so the cold read and write
    /// of each slot is paid even when the calldata floor binds.
    pub fn keyed_nonce_access_cost(&self) -> u64 {
        (self.nonce_keys.len() as u64).saturating_mul(FRAME_TX_KEYED_NONCE_ACCESS_COST)
    }

    /// EIP-2780's `TX_VALUE_COST` per frame that moves value to an explicit target
    /// other than `tx.sender`.
    ///
    /// A frame with no explicit target resolves to the sender, and a frame paying the
    /// sender moves nothing between accounts, so neither is a transfer to charge for.
    pub fn value_transfer_gas(&self) -> u64 {
        self.frames.iter().fold(0u64, |acc, frame| {
            let moves_value =
                !frame.value.is_zero() && frame.target.is_some_and(|target| target != self.sender);
            if moves_value {
                acc.saturating_add(TX_VALUE_COST)
            } else {
                acc
            }
        })
    }

    /// EIP-8141 `standard_gas_limit`: mandatory costs + data cost + the frames'
    /// total gas budget across **both** dimensions.
    ///
    /// `total_frame_gas` sums `limits.execution + limits.state` per frame. The two
    /// dimensions are separate budgets during execution, but the transaction
    /// reserves and is charged over their sum -- so a frame's state budget counts
    /// here even though its execution budget can never be spent on it.
    pub fn standard_gas_limit(&self) -> u64 {
        self.mandatory_gas()
            .saturating_add(self.data_cost())
            .saturating_add(self.total_frame_gas())
    }

    /// Σ(`limits.execution` + `limits.state`) over every frame.
    pub fn total_frame_gas(&self) -> u64 {
        self.frames.iter().fold(0u64, |acc, f| {
            acc.saturating_add(f.gas_limit)
                .saturating_add(f.state_gas_limit)
        })
    }

    /// Σ(`limits.execution`) over every frame -- the execution half only, which is
    /// what the EIP-7825 transaction cap is measured against.
    pub fn total_frame_execution_gas(&self) -> u64 {
        self.frames
            .iter()
            .fold(0u64, |acc, f| acc.saturating_add(f.gas_limit))
    }

    /// EIP-8141 `calldata_floor_gas`: the mandatory costs plus the EIP-7623
    /// floor over every byte this transaction carries. The mandatory costs are
    /// always charged, so they sit on both sides of the `max_gas` comparison.
    pub fn calldata_floor_total(&self) -> u64 {
        self.mandatory_gas()
            .saturating_add(self.calldata_floor_gas())
    }

    /// EIP-8141 `max_gas = max(standard_gas_limit, calldata_floor_gas)`: the gas
    /// reserved from the block pool before execution and the quantity `max_cost`
    /// is charged over. A transaction whose data floor exceeds what it declared
    /// for execution reserves the floor rather than being rejected.
    pub fn max_gas(&self) -> u64 {
        // Both sides of the comparison span both dimensions: the floor prices
        // calldata in the execution dimension only, so the frames' state budget is
        // added to it. Comparing a floor without the state budget against a standard
        // limit that includes it would escrow less than a floor-bound transaction
        // goes on to spend.
        let floor_side = self
            .calldata_floor_total()
            .saturating_add(self.total_frame_state_gas());
        self.standard_gas_limit().max(floor_side)
    }

    /// EIP-8141 `blob_gas`: `GAS_PER_BLOB` per blob versioned hash.
    pub fn blob_gas(&self) -> u64 {
        (self.blob_versioned_hashes.len() as u64)
            .saturating_mul(u64::from(crate::constants::GAS_PER_BLOB))
    }

    /// `max_fee / max_gas`: the most the budget can pay per unit of gas, at a zero
    /// blob base fee. It stands in where a per-gas fee cap is expected, such as the
    /// gas price shapes RPC reports; consensus never uses it.
    pub fn max_fee_per_gas_share(&self) -> U256 {
        // `max_gas` is never zero: it includes the fixed intrinsic cost.
        self.max_fee / U256::from(self.max_gas())
    }

    /// EIP-8141 fee budget at an including block's base fees. `None` when
    /// `max_fee` is below `required_max_fee`, the worst-case base-fee charge
    /// (`max_gas` at the base fee plus the blob gas at the blob base fee): the
    /// transaction is then invalid in that block. The arithmetic is exact, so a
    /// requirement past 2**256 is one no `max_fee` can meet.
    pub fn fee_budget(
        &self,
        base_fee_per_gas: U256,
        blob_base_fee: U256,
    ) -> Option<FrameFeeBudget> {
        let max_gas = U256::from(self.max_gas());
        let required_max_fee = max_gas
            .checked_mul(base_fee_per_gas)?
            .checked_add(U256::from(self.blob_gas()).checked_mul(blob_base_fee)?)?;
        let budget_left = self.max_fee.checked_sub(required_max_fee)?;
        // `max_priority_fee_per_gas * max_gas` may exceed 2**256; whatever it is,
        // the budget left caps it, so saturating cannot change the minimum.
        let max_priority_fee = self
            .max_priority_fee_per_gas
            .saturating_mul(max_gas)
            .min(budget_left);
        Some(FrameFeeBudget {
            max_cost: required_max_fee + max_priority_fee,
            // `max_gas` is never zero: it includes the fixed intrinsic cost.
            gas_price: base_fee_per_gas + max_priority_fee / max_gas,
        })
    }

    /// Σ(`limits.state`) over every frame.
    pub fn total_frame_state_gas(&self) -> u64 {
        self.frames
            .iter()
            .fold(0u64, |acc, f| acc.saturating_add(f.state_gas_limit))
    }

    /// The expiry deadline (8-byte big-endian) of this transaction's expiry
    /// verifier frame, if one exists with well-formed data.
    pub fn expiry_deadline(&self) -> Option<u64> {
        self.frames
            .iter()
            .find(|f| f.is_expiry_verifier())
            .and_then(|f| {
                let bytes: [u8; FRAME_TX_EXPIRY_DATA_LENGTH] = f.data.as_ref().try_into().ok()?;
                Some(u64::from_be_bytes(bytes))
            })
    }

    /// Validate static constraints per EIP-8141 spec.
    /// Returns an error string if the transaction is invalid.
    pub fn validate_static_constraints(&self) -> Result<(), String> {
        // EIP-8250: the account nonce cannot be used at `MAX_NONCE_SEQ`, which only
        // a keyed set may select, as its single-use form.
        if !self.is_keyed() && self.nonce_seq == FRAME_TX_MAX_NONCE_SEQ {
            return Err("nonce_seq must be below MAX_NONCE_SEQ (2**64 - 1)".to_string());
        }
        // EIP-8250: at most sixteen strictly increasing keys.
        if self.nonce_keys.len() > FRAME_TX_MAX_NONCE_KEYS {
            return Err(format!(
                "nonce_keys count must be at most {FRAME_TX_MAX_NONCE_KEYS}"
            ));
        }
        if self.nonce_keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err("nonce_keys must be strictly increasing".to_string());
        }
        if self.frames.is_empty() || self.frames.len() > FRAME_TX_MAX_FRAMES {
            return Err(format!(
                "Frame count must be between 1 and {FRAME_TX_MAX_FRAMES}"
            ));
        }
        // Per EIP-8141, every versioned hash must carry the EIP-4844 KZG version
        // byte. Blob gas is paid from the `max_fee` budget, so there is no blob fee
        // field to constrain.
        for (i, hash) in self.blob_versioned_hashes.iter().enumerate() {
            if hash.0.first() != Some(&VERSIONED_HASH_VERSION_KZG) {
                return Err(format!("Blob versioned hash {i}: wrong version byte"));
            }
        }
        // EIP-8141 claims: at most `MAX_CLAIMS`, each with a `key` and a `msg` of
        // an accepted form. A protocol verifier fixes its own gas and witness
        // length, so its claim declares both as zero; any other verifier's claim
        // declares the gas its verification is given and the length of its witness.
        if self.claims.len() > MAX_CLAIMS {
            return Err(format!("Claim count must be at most {MAX_CLAIMS}"));
        }
        let mut total_claim_gas: u64 = 0;
        for (i, claim) in self.claims.iter().enumerate() {
            let protocol_verifier = claim.protocol_verifier();
            let Some(resolved_key) = claim.resolved_key(self.sender) else {
                return Err(format!(
                    "Claim {i}: key must be empty, or a non-zero 20-byte or 32-byte value"
                ));
            };
            // The key of a secp256k1 or P-256 claim is an address, so it has no
            // bits above the low 20 bytes.
            if matches!(
                protocol_verifier,
                Some(ProtocolVerifier::Secp256k1 | ProtocolVerifier::P256)
            ) && resolved_key
                .as_bytes()
                .iter()
                .take(12)
                .any(|byte| *byte != 0)
            {
                return Err(format!(
                    "Claim {i}: the key of a secp256k1 or P-256 claim must be an address"
                ));
            }
            match claim.msg.len() {
                0 => {}
                32 => {
                    if claim.msg.iter().all(|&b| b == 0) {
                        return Err(format!("Claim {i}: explicit msg must not be zero digest"));
                    }
                }
                other => {
                    return Err(format!(
                        "Claim {i}: msg must be empty or 32 bytes, got {other}"
                    ));
                }
            }
            if protocol_verifier.is_some() {
                if claim.gas_limit != 0 {
                    return Err(format!(
                        "Claim {i}: gas_limit must be zero for a protocol verifier"
                    ));
                }
                if claim.witness_len != 0 {
                    return Err(format!(
                        "Claim {i}: witness_len must be zero for a protocol verifier"
                    ));
                }
            } else {
                if claim.gas_limit == 0 {
                    return Err(format!(
                        "Claim {i}: gas_limit must not be zero for a custom verifier"
                    ));
                }
                if claim.witness_len > MAX_CLAIM_WITNESS_BYTES {
                    return Err(format!(
                        "Claim {i}: witness_len {} exceeds {MAX_CLAIM_WITNESS_BYTES}",
                        claim.witness_len
                    ));
                }
            }
            total_claim_gas = total_claim_gas
                .checked_add(claim.resolved_gas())
                .ok_or_else(|| format!("Claim {i}: cumulative claim gas overflow"))?;
        }
        // Tracked as u128 so the running addition itself cannot overflow; the
        // bound below rejects tx-level totals that don't fit in signed i64.
        let mut total_frame_gas: u128 = 0;
        let mut expiry_frame_count: usize = 0;
        let mut seen_assert = false;
        for (i, frame) in self.frames.iter().enumerate() {
            // Reject reserved execution modes (4-255). EIP-7906 adds ASSERT (3) to
            // the modes EIP-8141 accepts.
            if FrameMode::from_u8(frame.mode).is_none() {
                return Err(format!("Frame {i}: reserved execution mode {}", frame.mode));
            }
            // EIP-7906: ASSERT frames form a contiguous trailing suffix, so the diff
            // they observe is the transaction's final outcome.
            if frame.mode == FrameMode::Assert as u8 {
                seen_assert = true;
            } else if seen_assert {
                return Err(format!(
                    "Frame {i}: only ASSERT frames may follow an ASSERT frame"
                ));
            }
            // Reserved flag bits 3-7 must be zero
            if frame.flags >= 8 {
                return Err(format!(
                    "Frame {i}: reserved flag bits must be zero (flags={:#04x})",
                    frame.flags
                ));
            }
            // Expiry verifier frames (EIP-8141): VERIFY
            // frames targeting EXPIRY_VERIFIER must have flags == 0, value == 0
            // (already enforced by the non-SENDER value rule below), and exactly
            // EXPIRY_DATA_LENGTH bytes of data; at most one per transaction.
            if frame.is_expiry_verifier() {
                expiry_frame_count = expiry_frame_count.saturating_add(1);
                if expiry_frame_count > 1 {
                    return Err(format!("Frame {i}: more than one expiry verifier frame"));
                }
                if frame.flags != 0 {
                    return Err(format!(
                        "Frame {i}: expiry verifier frame must have flags == 0"
                    ));
                }
                if frame.data.len() != FRAME_TX_EXPIRY_DATA_LENGTH {
                    return Err(format!(
                        "Frame {i}: expiry verifier frame data must be {FRAME_TX_EXPIRY_DATA_LENGTH} bytes"
                    ));
                }
                // The expiry verifier creates no state, so its frame must declare
                // no state-gas budget.
                if frame.state_gas_limit != 0 {
                    return Err(format!(
                        "Frame {i}: expiry verifier frame must have state_gas_limit == 0 (got {})",
                        frame.state_gas_limit
                    ));
                }
            }
            // Per EIP-8141, only SENDER frames may carry a
            // non-zero value. DEFAULT and VERIFY frames with a non-zero
            // value are statically invalid.
            if frame.mode != FrameMode::Sender as u8 && !frame.value.is_zero() {
                return Err(format!(
                    "Frame {i}: non-zero value only allowed in SENDER mode (mode={}, value={})",
                    frame.mode, frame.value
                ));
            }
            // Intentional stricter-than-spec bound. EIP-8141 allows
            // gas_limit and cumulative frame gas up to 2**64-1; ethrex caps both
            // at i64::MAX (2**63-1) so the state-gas dimension (tracked as i64) can
            // never overflow downstream. This is documented as a known divergence
            // in docs/eip-8141.md. It is effectively unreachable: any gas_limit
            // >= 2**63 dwarfs every real block gas limit and is rejected by the
            // gas-limit-vs-block-limit check regardless.
            if frame.gas_limit > i64::MAX as u64 {
                return Err(format!(
                    "Frame {i}: gas_limit {} exceeds 2**63-1",
                    frame.gas_limit
                ));
            }
            if frame.state_gas_limit > i64::MAX as u64 {
                return Err(format!(
                    "Frame {i}: state_gas_limit {} exceeds 2**63-1",
                    frame.state_gas_limit
                ));
            }
            // The cumulative bound sums *both* dimensions. A state budget drives the
            // same overflow an execution budget does, and either alone can fit in 64
            // bits while their sum does not, so accumulating only execution gas lets a
            // statically invalid transaction through to execution -- where it surfaces
            // as a block gas-allowance failure instead of a format one.
            total_frame_gas = total_frame_gas
                .checked_add(frame.gas_limit as u128)
                .and_then(|t| t.checked_add(frame.state_gas_limit as u128))
                .ok_or_else(|| format!("Frame {i}: cumulative frame gas overflow"))?;
            if total_frame_gas > i64::MAX as u128 {
                return Err(format!(
                    "Frame {i}: cumulative frame gas {total_frame_gas} exceeds 2**63-1"
                ));
            }
            // Per EIP-8141, approval of execution is only allowed when the
            // frame's target is empty or `tx.sender`.
            if frame.flags & APPROVE_EXECUTION != 0
                && frame.target.is_some_and(|target| target != self.sender)
            {
                return Err(format!(
                    "Frame {i}: APPROVE_EXECUTION requires an empty target or tx.sender"
                ));
            }
            // Per EIP-8141, the atomic batch flag (bit 2 of flags) is only valid
            // on a non-VERIFY frame, requires a subsequent frame to batch with,
            // and that frame must be non-VERIFY too: batches never contain
            // VERIFY frames.
            // EIP-7906 extends this to ASSERT frames: the flag is not valid on one,
            // nor on the frame before one, so no batch contains an assertion and
            // unrolling a failed batch can never skip it.
            if frame.is_atomic_batch() {
                if frame.mode == FrameMode::Verify as u8 {
                    return Err(format!("Frame {i}: atomic batch flag on a VERIFY frame"));
                }
                if frame.mode == FrameMode::Assert as u8 {
                    return Err(format!("Frame {i}: atomic batch flag on an ASSERT frame"));
                }
                match self.frames.get(i.saturating_add(1)) {
                    None => return Err(format!("Frame {i}: atomic batch flag on last frame")),
                    Some(next) if next.mode == FrameMode::Verify as u8 => {
                        return Err(format!(
                            "Frame {i}: atomic batch flag followed by a VERIFY frame"
                        ));
                    }
                    Some(next) if next.mode == FrameMode::Assert as u8 => {
                        return Err(format!(
                            "Frame {i}: atomic batch flag followed by an ASSERT frame"
                        ));
                    }
                    Some(_) => {}
                }
            }

            // Per EIP-8141, approval scope is disallowed on every frame of an
            // atomic batch, including its terminating frame -- a frame belongs to a
            // batch when it or its predecessor carries the flag. This keeps the
            // approval context constant across a batch, so unrolling one can never
            // withdraw an execution approval later SENDER frames rely on, nor make
            // whether the transaction sets a payer depend on a batch outcome.
            // Approval can be placed in a frame preceding the batch instead.
            let predecessor_batches = i
                .checked_sub(1)
                .and_then(|prev| self.frames.get(prev))
                .is_some_and(|prev| prev.is_atomic_batch());
            if (frame.is_atomic_batch() || predecessor_batches)
                && frame.flags & APPROVE_EXECUTION_AND_PAYMENT != 0
            {
                return Err(format!(
                    "Frame {i}: approval scope on an atomic-batch frame (flags={:#04x})",
                    frame.flags
                ));
            }
        }
        Ok(())
    }

    /// Identify and return the validation prefix of this frame transaction.
    ///
    /// The validation prefix is the minimal leading subsequence of frames (ignoring
    /// expiry-verifier frames) that establishes who pays for the transaction.
    /// Four shapes are recognized (per EIP-8141 §Mempool):
    ///
    /// - `SelfVerify`: one VERIFY frame targeting `tx.sender` with scope `APPROVE_EXECUTION_AND_PAYMENT`.
    /// - `DeploySelfVerify`: one DEFAULT frame (deploy) then one VERIFY frame as above.
    /// - `OnlyVerifyPay`: one VERIFY frame (only_verify, scope `APPROVE_EXECUTION`) +
    ///   one VERIFY frame (pay, scope `APPROVE_PAYMENT`).
    /// - `DeployOnlyVerifyPay`: DEFAULT deploy frame + OnlyVerifyPay pair.
    ///
    /// Expiry-verifier frames (see `Frame::is_expiry_verifier`) are transparent; they
    /// are skipped during shape matching but their indices are NOT included in
    /// `frame_indices` (which holds only the semantically meaningful prefix frames).
    pub fn validation_prefix(&self) -> Result<ValidationPrefix, FrameValidationError> {
        // EIP-8272: a leading recent root verifier frame is a protocol frame like the
        // expiry verifier and is skipped by shape matching. Only the leading position
        // is skipped; a matching frame elsewhere stays in the sequence and fails the
        // shape or the structural rules.
        let recent_root_index = self.recent_root_verifier_index();
        // Collect the frame indices left after the protocol verifier frames, in order.
        let non_expiry: Vec<usize> = self
            .frames
            .iter()
            .enumerate()
            .filter(|(idx, f)| !f.is_expiry_verifier() && Some(*idx) != recent_root_index)
            .map(|(i, _)| i)
            .collect();

        // Helper to get a frame by its position in non_expiry.
        let frame = |pos: usize| -> Option<&Frame> {
            non_expiry.get(pos).and_then(|&idx| self.frames.get(idx))
        };

        let is_default = |pos: usize| -> bool {
            frame(pos).is_some_and(|f| f.execution_mode() == FrameMode::Default)
        };
        let is_verify = |pos: usize| -> bool {
            frame(pos).is_some_and(|f| f.execution_mode() == FrameMode::Verify)
        };
        let scope_of = |pos: usize| -> u8 { frame(pos).map_or(0, |f| f.scope_restriction()) };

        if non_expiry.is_empty() {
            return Err(FrameValidationError::UnrecognizedPrefix);
        }

        // Attempt to match each of the four shapes.
        //
        // Shape: DeployOnlyVerifyPay — DEFAULT + VERIFY(exec) + VERIFY(pay)
        if is_default(0)
            && is_verify(1)
            && scope_of(1) == APPROVE_EXECUTION
            && is_verify(2)
            && scope_of(2) == APPROVE_PAYMENT
        {
            return Ok(ValidationPrefix {
                shape: PrefixShape::DeployOnlyVerifyPay,
                frame_indices: vec![non_expiry[0], non_expiry[1], non_expiry[2]],
                deploy_index: Some(non_expiry[0]),
                pay_index: Some(non_expiry[2]),
                recent_root_index,
            });
        }

        // Shape: DeploySelfVerify — DEFAULT + VERIFY(exec+pay)
        if is_default(0) && is_verify(1) && scope_of(1) == APPROVE_EXECUTION_AND_PAYMENT {
            return Ok(ValidationPrefix {
                shape: PrefixShape::DeploySelfVerify,
                frame_indices: vec![non_expiry[0], non_expiry[1]],
                deploy_index: Some(non_expiry[0]),
                pay_index: Some(non_expiry[1]),
                recent_root_index,
            });
        }

        // Shape: OnlyVerifyPay — VERIFY(exec) + VERIFY(pay)
        if is_verify(0)
            && scope_of(0) == APPROVE_EXECUTION
            && is_verify(1)
            && scope_of(1) == APPROVE_PAYMENT
        {
            return Ok(ValidationPrefix {
                shape: PrefixShape::OnlyVerifyPay,
                frame_indices: vec![non_expiry[0], non_expiry[1]],
                deploy_index: None,
                pay_index: Some(non_expiry[1]),
                recent_root_index,
            });
        }

        // Shape: SelfVerify — VERIFY(exec+pay)
        if is_verify(0) && scope_of(0) == APPROVE_EXECUTION_AND_PAYMENT {
            return Ok(ValidationPrefix {
                shape: PrefixShape::SelfVerify,
                frame_indices: vec![non_expiry[0]],
                deploy_index: None,
                pay_index: Some(non_expiry[0]),
                recent_root_index,
            });
        }

        Err(FrameValidationError::UnrecognizedPrefix)
    }

    /// Validate structural rules for the identified validation prefix.
    ///
    /// Checks:
    /// - Deploy frame (if any) is at index 0 and uses DEFAULT execution mode.
    /// - At most one deploy frame exists in the prefix.
    /// - self_verify / only_verify / pay frames use VERIFY execution mode.
    /// - Resolved target of each self_verify / only_verify frame matches
    ///   `tx.sender` (target == None means sender). The pay frame of the
    ///   OnlyVerifyPay / DeployOnlyVerifyPay shapes has no target restriction
    ///   (EIP-8141 structural rule 4): it may target a non-sender sponsor.
    /// - Scope restriction matches the frame's role:
    ///   self_verify → `APPROVE_EXECUTION_AND_PAYMENT`, only_verify → `APPROVE_EXECUTION`,
    ///   pay → `APPROVE_PAYMENT`.
    /// - No frame in the prefix has the atomic-batch flag set.
    /// - An expiry verifier frame, if present, is the first frame of the transaction.
    /// - No VERIFY frame follows the validation prefix.
    /// - Total gas budget: Σ(prefix frame gas_limits) + claim_verification_cost() ≤ `max_verify_gas`.
    ///
    /// `max_verify_gas` is the node's `MAX_VERIFY_GAS` budget; the spec value is
    /// [`FRAME_TX_MAX_VERIFY_GAS`], but it is mempool policy and therefore
    /// operator-tunable.
    pub fn validate_prefix_structure(
        &self,
        prefix: &ValidationPrefix,
        max_verify_gas: u64,
    ) -> Result<(), FrameValidationError> {
        let mut deploy_count = 0usize;

        for &idx in &prefix.frame_indices {
            let frame = &self.frames[idx];

            if frame.is_atomic_batch() {
                return Err(FrameValidationError::AtomicBatchInPrefix { frame_index: idx });
            }

            match prefix.deploy_index {
                Some(deploy_idx) if deploy_idx == idx => {
                    // This is the deploy frame.
                    deploy_count += 1;
                    if deploy_count > 1 {
                        return Err(FrameValidationError::MultipleDeploys { frame_index: idx });
                    }
                    // The deploy must be first among prefix frames. `validation_prefix`
                    // structurally guarantees this, but the raw frame index can be
                    // non-zero when expiry-verifier frames precede the deploy — check
                    // against the first element of `frame_indices`, not the raw index.
                    if prefix.frame_indices.first() != Some(&idx) {
                        return Err(FrameValidationError::DeployNotFirst { frame_index: idx });
                    }
                    if frame.execution_mode() != FrameMode::Default {
                        return Err(FrameValidationError::DeployNotDefaultMode {
                            frame_index: idx,
                        });
                    }
                }
                _ => {
                    // VERIFY frame (self_verify, only_verify, or pay).
                    if frame.execution_mode() != FrameMode::Verify {
                        return Err(FrameValidationError::VerifyFrameNotVerifyMode {
                            frame_index: idx,
                        });
                    }

                    // EIP-8141 structural rule 3 restricts the target to
                    // tx.sender (None means sender) only for self_verify /
                    // only_verify frames. Rule 4 places no target requirement
                    // on the pay frame: it may target a non-sender sponsor,
                    // which approves payment via APPROVE(APPROVE_PAYMENT)
                    // when the frame executes.
                    // The shape guard is load-bearing: every shape populates
                    // `pay_index`, and for SelfVerify / DeploySelfVerify it points
                    // at the self_verify frame, which carries
                    // APPROVE_EXECUTION_AND_PAYMENT and does require the sender as
                    // its target. Matching on `pay_index` alone would exempt it.
                    let is_pay_frame = matches!(
                        prefix.shape,
                        PrefixShape::OnlyVerifyPay | PrefixShape::DeployOnlyVerifyPay
                    ) && prefix.pay_index == Some(idx);
                    let target_ok = match frame.target {
                        None => true,
                        Some(addr) => addr == self.sender || is_pay_frame,
                    };
                    if !target_ok {
                        return Err(FrameValidationError::VerifyTargetNotSender {
                            frame_index: idx,
                        });
                    }

                    // Scope restriction must match role.
                    let expected_scope = match prefix.shape {
                        PrefixShape::SelfVerify | PrefixShape::DeploySelfVerify => {
                            APPROVE_EXECUTION_AND_PAYMENT
                        }
                        PrefixShape::OnlyVerifyPay | PrefixShape::DeployOnlyVerifyPay => {
                            // The only_verify frame comes before the pay frame.
                            if prefix.pay_index == Some(idx) {
                                APPROVE_PAYMENT
                            } else {
                                APPROVE_EXECUTION
                            }
                        }
                    };
                    if frame.scope_restriction() != expected_scope {
                        return Err(FrameValidationError::WrongScopeRestriction {
                            frame_index: idx,
                            expected: expected_scope,
                            actual: frame.scope_restriction(),
                        });
                    }
                }
            }
        }

        // EIP-8141 §Expiry Verifier Frame: an expiry verifier frame may appear
        // only as the first frame of the frame list. Expiry frames are otherwise
        // transparent to prefix matching, so a misplaced one would silently pin
        // the transaction's validity to a deadline outside the recognized shapes.
        if let Some((frame_index, _)) = self
            .frames
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, frame)| frame.is_expiry_verifier())
        {
            return Err(FrameValidationError::ExpiryFrameNotFirst { frame_index });
        }

        // EIP-8272 §Public mempool handling: at most one recent root verifier frame,
        // immediately after the optional expiry frame and before every other frame.
        // A frame matching its shape anywhere else is rejected rather than run as an
        // ordinary frame.
        if let Some((frame_index, _)) = self.frames.iter().enumerate().find(|(idx, frame)| {
            frame.is_recent_root_verifier() && Some(*idx) != prefix.recent_root_index
        }) {
            return Err(FrameValidationError::RecentRootFrameMisplaced { frame_index });
        }

        // EIP-8141 §Structural Rules rule 8: no VERIFY frame may follow the
        // validation prefix. A reverting VERIFY frame invalidates the whole
        // transaction wherever it sits, so one placed after the prefix would make
        // validity depend on state that prefix simulation never inspects — the
        // unbounded-invalidation case the public mempool rules exist to prevent.
        if let Some(&prefix_end) = prefix.frame_indices.last()
            && let Some((frame_index, _)) = self
                .frames
                .iter()
                .enumerate()
                .skip(prefix_end.saturating_add(1))
                .find(|(_, frame)| frame.execution_mode() == FrameMode::Verify)
        {
            return Err(FrameValidationError::VerifyFrameAfterPrefix { frame_index });
        }

        // Gas budget: prefix frame gas limits + claim verification cost ≤
        // MAX_VERIFY_GAS. Every claim is verified on admission, so the cost of all
        // of them counts, whichever frames go on to test for them.
        // EIP-8272 counts the recent root verifier frame's `limits.execution` too.
        let prefix_gas: u64 = prefix
            .frame_indices
            .iter()
            .chain(prefix.recent_root_index.iter())
            .map(|&i| self.frames[i].gas_limit)
            .fold(0u64, |acc, g| acc.saturating_add(g));
        let total_verify_gas = prefix_gas.saturating_add(self.claim_verification_cost());
        if total_verify_gas > max_verify_gas {
            return Err(FrameValidationError::VerifyGasBudgetExceeded {
                actual: total_verify_gas,
                limit: max_verify_gas,
            });
        }

        // State budget (rule 6): Σ(prefix frame `limits.state`) ≤ MAX_VERIFY_STATE_GAS.
        // The whole prefix counts, deploy frame included: the cap bounds the state a
        // public-mempool transaction may create while establishing its payer. The
        // recent root verifier frame declares no state budget by definition.
        let prefix_state_gas: u64 = prefix
            .frame_indices
            .iter()
            .map(|&i| self.frames[i].state_gas_limit)
            .fold(0u64, |acc, g| acc.saturating_add(g));
        if prefix_state_gas > FRAME_TX_MAX_VERIFY_STATE_GAS {
            return Err(FrameValidationError::VerifyStateBudgetExceeded {
                actual: prefix_state_gas,
                limit: FRAME_TX_MAX_VERIFY_STATE_GAS,
            });
        }

        Ok(())
    }
}

/// The four recognized shapes of an EIP-8141 validation prefix (§Mempool).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefixShape {
    /// Single VERIFY frame: sender verifies + authorizes payment for itself.
    SelfVerify,
    /// DEFAULT (deploy) frame, then VERIFY frame that verifies + authorizes payment.
    DeploySelfVerify,
    /// VERIFY(exec) + VERIFY(pay): separate verifier and paymaster.
    OnlyVerifyPay,
    /// DEFAULT (deploy) + VERIFY(exec) + VERIFY(pay).
    DeployOnlyVerifyPay,
}

/// Identified validation prefix of an EIP-8141 frame transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationPrefix {
    /// Recognized shape of this prefix.
    pub shape: PrefixShape,
    /// Frame indices (into `FrameTransaction.frames`) that form the prefix,
    /// in order. Does not include expiry-verifier frames.
    pub frame_indices: Vec<usize>,
    /// Index of the deploy frame within `frames`, if this shape has one.
    pub deploy_index: Option<usize>,
    /// Index of the pay (or self_verify) frame within `frames`.
    pub pay_index: Option<usize>,
    /// Index of the EIP-8272 recent root verifier frame, when one leads the
    /// transaction (first, or second behind an expiry verifier). Skipped by shape
    /// matching like the expiry frame; its `limits.execution` counts toward
    /// `MAX_VERIFY_GAS`.
    pub recent_root_index: Option<usize>,
}

/// Errors produced by `FrameTransaction::validation_prefix` and
/// `FrameTransaction::validate_prefix_structure`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FrameValidationError {
    #[error("frame transaction validation prefix does not match any recognized shape")]
    UnrecognizedPrefix,
    #[error("frame {frame_index}: prefix has more than one DEFAULT deploy frame")]
    MultipleDeploys { frame_index: usize },
    #[error("frame {frame_index}: deploy frame must be at index 0")]
    DeployNotFirst { frame_index: usize },
    #[error("frame {frame_index}: deploy frame must use DEFAULT execution mode")]
    DeployNotDefaultMode { frame_index: usize },
    #[error("frame {frame_index}: prefix VERIFY frame does not use VERIFY execution mode")]
    VerifyFrameNotVerifyMode { frame_index: usize },
    #[error("frame {frame_index}: VERIFY frame target is not tx.sender")]
    VerifyTargetNotSender { frame_index: usize },
    #[error("frame {frame_index}: scope restriction is {actual:#04x}, expected {expected:#04x}")]
    WrongScopeRestriction {
        frame_index: usize,
        expected: u8,
        actual: u8,
    },
    #[error("frame {frame_index}: prefix frame has atomic-batch flag set")]
    AtomicBatchInPrefix { frame_index: usize },
    #[error("frame {frame_index}: expiry verifier frame must be the first frame")]
    ExpiryFrameNotFirst { frame_index: usize },
    #[error("frame {frame_index}: VERIFY frame follows the validation prefix")]
    VerifyFrameAfterPrefix { frame_index: usize },
    #[error("prefix gas budget exceeded: {actual} > {limit} (MAX_VERIFY_GAS)")]
    VerifyGasBudgetExceeded { actual: u64, limit: u64 },
    #[error("prefix state gas budget exceeded: {actual} > {limit} (MAX_VERIFY_STATE_GAS)")]
    VerifyStateBudgetExceeded { actual: u64, limit: u64 },
    #[error(
        "frame {frame_index}: a recent root verifier frame must be the first frame, or the second behind an expiry verifier frame"
    )]
    RecentRootFrameMisplaced { frame_index: usize },
}

impl RLPEncode for FrameTransaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.chain_id)
            .encode_field(&self.nonce_keys)
            .encode_field(&self.nonce_seq)
            .encode_field(&self.sender)
            .encode_field(&self.claims)
            .encode_field(&self.frames)
            .encode_field(&(self.max_fee, self.max_priority_fee_per_gas))
            .encode_field(&self.blob_versioned_hashes)
            .finish();
    }
}

impl RLPDecode for FrameTransaction {
    fn decode_unfinished(rlp: &[u8]) -> Result<(FrameTransaction, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (chain_id, decoder) = decoder.decode_field("chain_id")?;
        // EIP-8250: `nonce_keys` must be an RLP list of canonical integers below
        // 2**256; the U256 decoder rejects non-canonical and over-wide items.
        let (nonce_keys, decoder) = decoder.decode_field("nonce_keys")?;
        // `nonce_seq < 2**64`: a wider value is a malformed payload, rejected while
        // decoding like any other frame-transaction field too wide for its type.
        let (nonce_seq, decoder) = decoder.decode_field("nonce_seq")?;
        let (sender, decoder) = decoder.decode_field("sender")?;
        let (claims, decoder) = decoder.decode_field("claims")?;
        let (frames, decoder) = decoder.decode_field("frames")?;
        // Decode the nested `fees` list, `[max_fee, max_priority_fee_per_gas]`,
        // field by field rather than as one tuple. A fee wider than 256 bits is
        // rejected here, during decoding, and the field name is the only thing that
        // says *which* fee was malformed -- decoding the pair as a unit reports
        // `fees ... InvalidLength`, which cannot distinguish the two.
        let (fees_encoded, decoder) = decoder.get_encoded_item()?;
        let fees_decoder = Decoder::new(&fees_encoded)?;
        let (max_fee, fees_decoder): (U256, _) = fees_decoder.decode_field("max_fee")?;
        let (max_priority_fee_per_gas, fees_decoder): (U256, _) =
            fees_decoder.decode_field("max_priority_fee_per_gas")?;
        fees_decoder.finish()?;
        let (blob_versioned_hashes, decoder) = decoder.decode_field("blob_versioned_hashes")?;
        let tx = FrameTransaction {
            chain_id,
            nonce_keys,
            nonce_seq,
            sender,
            claims,
            frames,
            max_fee,
            max_priority_fee_per_gas,
            blob_versioned_hashes,
            inner_hash: OnceCell::new(),
            cached_canonical: OnceCell::new(),
        };
        Ok((tx, decoder.finish()?))
    }
}

impl PayloadRLPEncode for FrameTransaction {
    fn encode_payload(&self, buf: &mut dyn bytes::BufMut) {
        buf.put_u8(TxType::Frame as u8);
        self.encode(buf);
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
                    // Decode tx based on type. `from_type_byte` is the single gate for valid
                    // envelope types: it rejects 0x00 (legacy is the bare-list branch below) and
                    // any unassigned type byte.
                    let tx_bytes = &bytes[1..];
                    match EnvelopeTxType::from_type_byte(*tx_type)? {
                        EnvelopeTxType::EIP2930 => EIP2930Transaction::decode(tx_bytes)
                            .map(Transaction::EIP2930Transaction),
                        EnvelopeTxType::EIP1559 => EIP1559Transaction::decode(tx_bytes)
                            .map(Transaction::EIP1559Transaction),
                        EnvelopeTxType::EIP4844 => EIP4844Transaction::decode(tx_bytes)
                            .map(Transaction::EIP4844Transaction),
                        EnvelopeTxType::EIP7702 => EIP7702Transaction::decode(tx_bytes)
                            .map(Transaction::EIP7702Transaction),
                        EnvelopeTxType::Frame => {
                            FrameTransaction::decode(tx_bytes).map(Transaction::FrameTransaction)
                        }
                        EnvelopeTxType::FeeToken => FeeTokenTransaction::decode(tx_bytes)
                            .map(Transaction::FeeTokenTransaction),
                        EnvelopeTxType::Privileged => PrivilegedL2Transaction::decode(tx_bytes)
                            .map(Transaction::PrivilegedL2Transaction),
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
                Transaction::FrameTransaction(t) => t.encode(buf),
            };
        }

        /// Encodes a transaction in canonical format into a newly created buffer
        /// Based on [EIP-2718]
        /// Transactions can be encoded in the following formats:
        /// A) `TransactionType || Transaction` (Where Transaction type is an 8-bit number between 0 and 0x7f, and Transaction is an rlp encoded transaction of type TransactionType)
        /// B) `LegacyTransaction` (An rlp encoded LegacyTransaction)
        pub fn encode_canonical_to_vec(&self) -> Vec<u8> {
            if let Some(cell) = self.cached_canonical_cell() {
                return cell
                    .get_or_init(|| {
                        let mut buf = Vec::new();
                        self.encode_canonical(&mut buf);
                        buf
                    })
                    .clone();
            }
            let mut buf = Vec::new();
            self.encode_canonical(&mut buf);
            buf
        }

        /// Canonical-encoded length without allocating a buffer. Counts the
        /// 1-byte type prefix for typed txs (EIP-2718) plus the inner RLP
        /// payload length. Use this when only the size is needed (e.g.
        /// admission-time size caps) to avoid `encode_canonical_to_vec().len()`.
        pub fn encode_canonical_len(&self) -> usize {
            let prefix_len = match self {
                Transaction::LegacyTransaction(_) => 0,
                _ => 1,
            };
            let inner_len = match self {
                Transaction::LegacyTransaction(t) => t.length(),
                Transaction::EIP2930Transaction(t) => t.length(),
                Transaction::EIP1559Transaction(t) => t.length(),
                Transaction::EIP4844Transaction(t) => t.length(),
                Transaction::EIP7702Transaction(t) => t.length(),
                Transaction::FeeTokenTransaction(t) => t.length(),
                Transaction::PrivilegedL2Transaction(t) => t.length(),
                Transaction::FrameTransaction(t) => t.length(),
            };
            prefix_len + inner_len
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
                P2PTransaction::FrameTransaction(_) => TxType::Frame,
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
                P2PTransaction::FrameTransaction(t) => t.encode(buf),
            };
        }

        pub fn encode_canonical_to_vec(&self) -> Vec<u8> {
            let mut buf = Vec::new();
            self.encode_canonical(&mut buf);
            buf
        }

        /// Canonical encoded length without allocating (mirrors
        /// [`P2PTransaction::encode_canonical`]); equals
        /// `encode_canonical_to_vec().len()`. For the blob variant this includes
        /// the full sidecar, since `EIP4844TransactionWithBlobs` encodes it.
        pub fn encode_canonical_len(&self) -> usize {
            let prefix_len = match self {
                P2PTransaction::LegacyTransaction(_) => 0,
                _ => 1,
            };
            let inner_len = match self {
                P2PTransaction::LegacyTransaction(t) => t.length(),
                P2PTransaction::EIP2930Transaction(t) => t.length(),
                P2PTransaction::EIP1559Transaction(t) => t.length(),
                P2PTransaction::EIP4844TransactionWithBlobs(t) => t.length(),
                P2PTransaction::EIP7702Transaction(t) => t.length(),
                P2PTransaction::FeeTokenTransaction(t) => t.length(),
                P2PTransaction::FrameTransaction(t) => t.length(),
            };
            prefix_len + inner_len
        }

        pub fn compute_hash(&self) -> H256 {
            match self {
                P2PTransaction::LegacyTransaction(t) => {
                    Transaction::LegacyTransaction(t.clone()).compute_hash(&NativeCrypto)
                }
                P2PTransaction::EIP2930Transaction(t) => {
                    Transaction::EIP2930Transaction(t.clone()).compute_hash(&NativeCrypto)
                }
                P2PTransaction::EIP1559Transaction(t) => {
                    Transaction::EIP1559Transaction(t.clone()).compute_hash(&NativeCrypto)
                }
                P2PTransaction::EIP4844TransactionWithBlobs(t) => {
                    Transaction::EIP4844Transaction(t.tx.clone()).compute_hash(&NativeCrypto)
                }
                P2PTransaction::EIP7702Transaction(t) => {
                    Transaction::EIP7702Transaction(t.clone()).compute_hash(&NativeCrypto)
                }
                P2PTransaction::FeeTokenTransaction(t) => {
                    Transaction::FeeTokenTransaction(t.clone()).compute_hash(&NativeCrypto)
                }
                // The hash covers the transaction body alone, so it is the same
                // whatever witnesses travel with it.
                P2PTransaction::FrameTransaction(t) => {
                    Transaction::FrameTransaction(t.tx.clone()).compute_hash(&NativeCrypto)
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

    /// JSON shape of an EIP-8141 frame, named as in the execution-apis frame
    /// transaction schema.
    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct FrameEntry {
        #[serde(with = "crate::serde_utils::u64::hex_str")]
        pub mode: u64,
        #[serde(with = "crate::serde_utils::u64::hex_str")]
        pub flags: u64,
        pub target: Option<Address>,
        #[serde(with = "crate::serde_utils::u64::hex_str")]
        pub execution_gas: u64,
        #[serde(default, with = "crate::serde_utils::u64::hex_str")]
        pub state_gas: u64,
        #[serde(
            default,
            serialize_with = "serialize_u256_hex",
            deserialize_with = "crate::serde_utils::u256::deser_hex_str"
        )]
        pub value: U256,
        #[serde(with = "crate::serde_utils::bytes")]
        pub data: Bytes,
    }

    /// JSON shape of an EIP-8141 claim.
    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct ClaimEntry {
        pub verifier: Address,
        #[serde(with = "crate::serde_utils::bytes")]
        pub key: Bytes,
        #[serde(with = "crate::serde_utils::bytes")]
        pub msg: Bytes,
        #[serde(with = "crate::serde_utils::u64::hex_str")]
        pub gas_limit: u64,
        #[serde(with = "crate::serde_utils::u64::hex_str")]
        pub witness_len: u64,
    }

    impl From<&Claim> for ClaimEntry {
        fn from(value: &Claim) -> ClaimEntry {
            ClaimEntry {
                verifier: value.verifier,
                key: value.key.clone(),
                msg: value.msg.clone(),
                gas_limit: value.gas_limit,
                witness_len: value.witness_len,
            }
        }
    }

    fn serialize_u256_hex<S: serde::Serializer>(value: &U256, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{value:#x}"))
    }

    impl From<&Frame> for FrameEntry {
        fn from(value: &Frame) -> FrameEntry {
            FrameEntry {
                mode: value.mode as u64,
                flags: value.flags as u64,
                target: value.target,
                execution_gas: value.gas_limit,
                state_gas: value.state_gas_limit,
                value: value.value,
                data: value.data.clone(),
            }
        }
    }

    impl From<FrameEntry> for Frame {
        fn from(entry: FrameEntry) -> Frame {
            Frame {
                mode: entry.mode as u8,
                flags: entry.flags as u8,
                target: entry.target,
                gas_limit: entry.execution_gas,
                state_gas_limit: entry.state_gas,
                value: entry.value,
                data: entry.data,
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

    impl Serialize for FrameTransaction {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut s = serializer.serialize_struct("FrameTransaction", 10)?;
            s.serialize_field("type", &TxType::Frame)?;
            s.serialize_field("chainId", &format!("{:#x}", self.chain_id))?;
            // EIP-8250, named as in the execution-apis frame transaction schema:
            // `nonce` carries the shared sequence and `nonceKeys` the selected keys.
            s.serialize_field("nonce", &format!("{:#x}", self.nonce_seq))?;
            s.serialize_field(
                "nonceKeys",
                &self
                    .nonce_keys
                    .iter()
                    .map(|key| format!("{key:#x}"))
                    .collect::<Vec<_>>(),
            )?;
            // The sender is not serialized here: the RPC transaction object
            // carries it as `from`, as for every other transaction type.
            s.serialize_field(
                "claims",
                &self.claims.iter().map(ClaimEntry::from).collect::<Vec<_>>(),
            )?;
            s.serialize_field(
                "frames",
                &self.frames.iter().map(FrameEntry::from).collect::<Vec<_>>(),
            )?;
            // EIP-8141 `fees`: one `max_fee` budget for all execution, state and
            // blob gas, and the priority fee per gas.
            s.serialize_field("maxFee", &format!("{:#x}", self.max_fee))?;
            s.serialize_field(
                "maxPriorityFeePerGas",
                &format!("{:#x}", self.max_priority_fee_per_gas),
            )?;
            s.serialize_field("blobVersionedHashes", &self.blob_versioned_hashes)?;
            s.end()
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
                TxType::Frame => Err(serde::de::Error::custom(
                    "Frame transaction JSON deserialization not supported",
                )),
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

    fn deserialize_u64_field<'de, D>(
        map: &mut HashMap<String, serde_json::Value>,
        key: &str,
    ) -> Result<u64, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = deserialize_field::<U256, D>(map, key)?;
        u64::try_from(value).map_err(|_| D::Error::custom(format!("{key} value overflows u64")))
    }

    impl<'de> Deserialize<'de> for LegacyTransaction {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let mut map = <HashMap<String, serde_json::Value>>::deserialize(deserializer)?;

            Ok(LegacyTransaction {
                nonce: deserialize_u64_field::<D>(&mut map, "nonce")?,
                gas_price: deserialize_field::<U256, D>(&mut map, "gasPrice")?,
                gas: deserialize_u64_field::<D>(&mut map, "gas")?,
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
                chain_id: deserialize_u64_field::<D>(&mut map, "chainId")?,
                nonce: deserialize_u64_field::<D>(&mut map, "nonce")?,
                gas_price: deserialize_field::<U256, D>(&mut map, "gasPrice")?,
                gas_limit: deserialize_u64_field::<D>(&mut map, "gas")?,
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
                chain_id: deserialize_u64_field::<D>(&mut map, "chainId")?,
                nonce: deserialize_u64_field::<D>(&mut map, "nonce")?,
                max_priority_fee_per_gas: deserialize_u64_field::<D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?,
                max_fee_per_gas: deserialize_u64_field::<D>(&mut map, "maxFeePerGas")?,
                gas_limit: deserialize_u64_field::<D>(&mut map, "gas")?,
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
                chain_id: deserialize_u64_field::<D>(&mut map, "chainId")?,
                nonce: deserialize_u64_field::<D>(&mut map, "nonce")?,
                max_priority_fee_per_gas: deserialize_u64_field::<D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?,
                max_fee_per_gas: deserialize_u64_field::<D>(&mut map, "maxFeePerGas")?,
                gas: deserialize_u64_field::<D>(&mut map, "gas")?,
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
                chain_id: deserialize_u64_field::<D>(&mut map, "chainId")?,
                nonce: deserialize_u64_field::<D>(&mut map, "nonce")?,
                max_priority_fee_per_gas: deserialize_u64_field::<D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?,
                max_fee_per_gas: deserialize_u64_field::<D>(&mut map, "maxFeePerGas")?,
                gas_limit: deserialize_u64_field::<D>(&mut map, "gas")?,
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
                chain_id: deserialize_u64_field::<D>(&mut map, "chainId")?,
                nonce: deserialize_u64_field::<D>(&mut map, "nonce")?,
                max_priority_fee_per_gas: deserialize_u64_field::<D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?,
                max_fee_per_gas: deserialize_u64_field::<D>(&mut map, "maxFeePerGas")?,
                gas_limit: deserialize_u64_field::<D>(&mut map, "gas")?,
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
                chain_id: deserialize_u64_field::<D>(&mut map, "chainId")?,
                nonce: deserialize_u64_field::<D>(&mut map, "nonce")?,
                max_priority_fee_per_gas: deserialize_u64_field::<D>(
                    &mut map,
                    "maxPriorityFeePerGas",
                )?,
                max_fee_per_gas: deserialize_u64_field::<D>(&mut map, "maxFeePerGas")?,
                gas_limit: deserialize_u64_field::<D>(&mut map, "gas")?,
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
        #[error("Invalid field: {0}")]
        InvalidField(String),
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
        #[serde(default)]
        pub gas_price: U256,
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
                gas_price: U256::from(value.max_fee_per_gas),
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
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(
                    u64::try_from(value.gas_price).map_err(|_| {
                        GenericTransactionError::InvalidField("gas_price overflows u64".to_owned())
                    })?,
                ),
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
                gas_price: U256::from(value.max_fee_per_gas),
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
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(
                    u64::try_from(value.gas_price).map_err(|_| {
                        GenericTransactionError::InvalidField("gas_price overflows u64".to_owned())
                    })?,
                ),
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
                gas_price: U256::from(value.max_fee_per_gas),
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
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(
                    u64::try_from(value.gas_price).map_err(|_| {
                        GenericTransactionError::InvalidField("gas_price overflows u64".to_owned())
                    })?,
                ),
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
                gas_price: U256::from(value.max_fee_per_gas),
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
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(
                    u64::try_from(value.gas_price).map_err(|_| {
                        GenericTransactionError::InvalidField("gas_price overflows u64".to_owned())
                    })?,
                ),
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
                gas_price: U256::from(value.max_fee_per_gas),
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
                max_fee_per_gas: value.max_fee_per_gas.unwrap_or(
                    u64::try_from(value.gas_price).map_err(|_| {
                        GenericTransactionError::InvalidField("gas_price overflows u64".to_owned())
                    })?,
                ),
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
                gas_price: value.gas_price,
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
                gas_price: value.gas_price,
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
                Transaction::FrameTransaction(tx) => tx.into(),
            }
        }
    }

    impl From<FrameTransaction> for GenericTransaction {
        fn from(value: FrameTransaction) -> Self {
            Self {
                r#type: TxType::Frame,
                nonce: Some(value.nonce_seq),
                to: TxKind::Call(value.sender),
                from: value.sender,
                gas: Some(value.max_gas()),
                value: U256::zero(),
                // The frame transaction caps its total fee, not a price per gas;
                // its per-gas share of the budget stands in for the cap here.
                gas_price: value.max_fee_per_gas_share(),
                // `GenericTransaction` keeps these as `u64`, and `U256::as_u64`
                // panics rather than truncating, so saturate: this conversion feeds
                // RPC and simulation shapes, and a fee this large is unaffordable at
                // any balance, so the clamp cannot change an outcome.
                max_priority_fee_per_gas: Some(
                    u64::try_from(value.max_priority_fee_per_gas).unwrap_or(u64::MAX),
                ),
                max_fee_per_gas: Some(
                    u64::try_from(value.max_fee_per_gas_share()).unwrap_or(u64::MAX),
                ),
                // Blob gas is paid from the same budget: there is no blob fee cap.
                max_fee_per_blob_gas: None,
                access_list: vec![],
                fee_token: None,
                authorization_list: None,
                blob_versioned_hashes: value.blob_versioned_hashes,
                blobs: vec![],
                wrapper_version: None,
                chain_id: Some(value.chain_id),
                input: Bytes::new(),
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
    fn is_blob_carrying_covers_frame_txs_with_blob_hashes() {
        // An EIP-8141 frame tx carries `blob_versioned_hashes` of its own, so
        // blob-ness cannot be decided by matching on `EIP4844Transaction` alone:
        // a blob-bearing frame tx would otherwise be treated as a plain tx by the
        // mempool's replacement rules and sidecar handling.
        let plain_frame = Transaction::FrameTransaction(FrameTransaction::default());
        assert!(!plain_frame.is_blob_carrying());

        let blob_frame = Transaction::FrameTransaction(FrameTransaction {
            blob_versioned_hashes: vec![H256::from_low_u64_be(1)],
            ..Default::default()
        });
        assert!(blob_frame.is_blob_carrying());

        let blob_tx = Transaction::EIP4844Transaction(EIP4844Transaction {
            blob_versioned_hashes: vec![H256::from_low_u64_be(1)],
            ..Default::default()
        });
        assert!(blob_tx.is_blob_carrying());

        let plain = Transaction::EIP1559Transaction(EIP1559Transaction::default());
        assert!(!plain.is_blob_carrying());
    }

    #[test]
    fn nonce_over_u64_max_maps_to_nonce_is_max() {
        // RLP list [ 0x010000000000000000 ] — nonce = 2**64 (9 significant bytes),
        // one past u64::MAX. EEST expects TransactionException.NONCE_IS_MAX here.
        let rlp = hex!("ca89010000000000000000");
        let decoder = Decoder::new(&rlp).unwrap();
        let err = decode_nonce_field(decoder).unwrap_err();
        assert!(
            err.to_string().contains("Nonce is max"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn nonce_at_u64_max_decodes() {
        // RLP list [ 0xffffffffffffffff ] — nonce = u64::MAX (8 bytes), still valid.
        let rlp = hex!("c988ffffffffffffffff");
        let decoder = Decoder::new(&rlp).unwrap();
        let (nonce, _) = decode_nonce_field(decoder).unwrap();
        assert_eq!(nonce, u64::MAX);
    }

    #[test]
    fn legacy_chain_id_handles_out_of_range_v_without_underflow() {
        // A legacy transaction decodes `v` as an arbitrary U256, so a malformed tx can
        // carry any value. `derive_legacy_chain_id` must return None for the pre-EIP-155
        // values (27/28) and any v < 35 rather than underflowing `v - 35`, which panics
        // in debug and wraps to a bogus chain id in release. This is reachable on the
        // block-import path (every tx's chain id is now checked pre-execution).
        let legacy_chain_id = |v: u64| {
            Transaction::LegacyTransaction(LegacyTransaction {
                v: U256::from(v),
                ..Default::default()
            })
            .chain_id()
        };
        for v in [0u64, 1, 26, 27, 28, 34] {
            assert_eq!(legacy_chain_id(v), None, "v={v} must yield no chain id");
        }
        // EIP-155 values (v = chain_id * 2 + 35/36) still derive correctly.
        assert_eq!(legacy_chain_id(35), Some(0));
        assert_eq!(legacy_chain_id(36), Some(0));
        assert_eq!(legacy_chain_id(37), Some(1));
    }

    /// EIP-155 at the 64-bit boundary. Signed for chain id 2^64 - 1, a legacy
    /// transaction's `v` no longer fits in 64 bits but its chain id does: the
    /// transaction reports that chain id and recovers its signer, so a chain that
    /// is not 2^64 - 1 rejects it for its chain id. One chain id further, the chain
    /// id does not fit: the transaction has none and its `v` is invalid.
    #[test]
    fn legacy_chain_id_at_the_64_bit_boundary() {
        let secret = secp256k1::SecretKey::from_byte_array(&[0x11; 32]).unwrap();
        let signed_for = |chain_id: U256| {
            let mut tx = LegacyTransaction {
                nonce: 7,
                gas_price: U256::from(1_000_000_000u64),
                gas: 21_000,
                to: TxKind::Call(Address::repeat_byte(0xde)),
                value: U256::one(),
                v: U256::from(35) + chain_id * 2,
                ..Default::default()
            };
            let (payload, _) = Transaction::LegacyTransaction(tx.clone())
                .signing_payload()
                .unwrap()
                .unwrap();
            let msg = secp256k1::Message::from_digest(NativeCrypto.keccak256(&payload));
            let (recovery_id, signature) = secp256k1::SECP256K1
                .sign_ecdsa_recoverable(&msg, &secret)
                .serialize_compact();
            tx.r = U256::from_big_endian(&signature[..32]);
            tx.s = U256::from_big_endian(&signature[32..]);
            tx.v += U256::from(i32::from(recovery_id));
            Transaction::LegacyTransaction(tx)
        };
        let signer = signed_for(U256::one()).sender(&NativeCrypto).unwrap();

        let largest = signed_for(U256::from(u64::MAX));
        let Transaction::LegacyTransaction(legacy) = &largest else {
            unreachable!("signed_for builds legacy transactions");
        };
        assert!(legacy.v > U256::from(u64::MAX), "the case under test");
        assert_eq!(largest.chain_id(), Some(u64::MAX));
        assert_eq!(largest.sender(&NativeCrypto).unwrap(), signer);

        let too_large = Transaction::LegacyTransaction(LegacyTransaction {
            v: U256::from(35) + (U256::from(u64::MAX) + 1) * 2,
            ..Default::default()
        });
        assert_eq!(too_large.chain_id(), None);
        assert!(too_large.sender(&NativeCrypto).is_err());
    }

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
        let result = compute_transactions_root(&body.transactions, &ethrex_crypto::NativeCrypto);

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
        let hash = tx.compute_hash(&NativeCrypto);
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

        let result = compute_receipts_root(&[receipt], &ethrex_crypto::NativeCrypto);
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
            gas_price: U256::from(7),
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
            gas_price: U256::from(7),
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
        assert_eq!(generic_tx.gas_price, U256::from(20_000_000_000u64));
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
        assert_eq!(generic_tx.gas_price, U256::from(20_000_000_000u64));
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
        use ethrex_crypto::NativeCrypto;
        use k256::ecdsa::SigningKey;

        let crypto = NativeCrypto;

        // 1. Setup: Create a signer and a message
        // A random private key for testing
        let private_key = hex!("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318");
        let signing_key = SigningKey::from_bytes(&private_key.into()).expect("Valid private key");

        // The message we want to sign
        let msg = b"Test message for high-s signature rejection";
        // Calculate the Keccak256 hash of the message (the payload)
        let payload = keccak(msg).to_fixed_bytes();

        // 2. Generate a valid low-s signature
        // k256's sign_prehash_recoverable produces canonical low-s signatures by default.
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(&payload)
            .expect("Signing failed");

        // 3. Construct the signature bytes (r||s||v, 65 bytes)
        let mut sig_bytes = [0u8; 65];
        sig_bytes[..64].copy_from_slice(&signature.to_bytes());
        sig_bytes[64] = recovery_id.to_byte();

        // 4. Verify that the valid low-s signature recovers the correct address
        // Calculate the expected address from the public key
        let uncompressed_pub = signing_key.verifying_key().to_encoded_point(false);
        let pub_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed_pub.as_bytes()[1..]);
        let expected_address = Address::from_slice(&pub_hash[12..]);

        let recovered = crypto
            .recover_signer(&sig_bytes, &payload)
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
            crypto.recover_signer(&sig_high_bytes, &payload).is_err(),
            "High-s signature should be rejected (EIP-2 compliance)"
        );
    }

    #[test]
    fn encode_decode_low_size_tx() {
        let tx = Transaction::EIP2930Transaction(EIP2930Transaction::default());
        // Encode a separate copy so the original's cached_canonical stays uninit,
        // avoiding a false PartialEq mismatch with the decoded (uncached) tx.
        let tx_to_encode = tx.clone();
        let encoded = tx_to_encode.encode_to_vec();
        let decoded_tx = Transaction::decode(&encoded).unwrap();
        assert_eq!(tx, decoded_tx);
    }

    #[test]
    fn test_eip1559_simple_transfer_size() {
        let tx = Transaction::EIP1559Transaction(EIP1559Transaction::default());
        assert_eq!(tx.encode_to_vec().len(), EIP1559_DEFAULT_SERIALIZED_LENGTH);
    }

    // ── Frame Transaction (EIP-8141) tests ──

    fn make_test_frame_tx() -> FrameTransaction {
        FrameTransaction {
            chain_id: 1,
            nonce_keys: vec![],
            nonce_seq: 42,
            sender: Address::from_low_u64_be(0xABCD),
            frames: vec![
                Frame {
                    mode: FrameMode::Verify as u8,
                    flags: 0x03, // APPROVE_PAYMENT_AND_EXECUTION
                    target: Some(Address::from_low_u64_be(0xABCD)),
                    gas_limit: 100_000,
                    state_gas_limit: 0,
                    value: U256::zero(),
                    data: Bytes::from_static(b"verify_data"),
                },
                Frame {
                    mode: FrameMode::Sender as u8,
                    flags: 0x00,
                    target: Some(Address::from_low_u64_be(0x1234)),
                    gas_limit: 200_000,
                    state_gas_limit: 0,
                    value: U256::zero(),
                    data: Bytes::from_static(b"call_data"),
                },
            ],
            claims: vec![Claim {
                verifier: SECP256K1_VERIFIER,
                ..Default::default()
            }],
            // 0.01 ETH: a budget of 30 gwei over ~330k gas.
            max_fee: U256::from(10_000_000_000_000_000u64),
            max_priority_fee_per_gas: U256::from(1_000_000_000u64),
            blob_versioned_hashes: vec![],
            inner_hash: OnceCell::new(),
            cached_canonical: OnceCell::new(),
        }
    }

    #[test]
    fn atomic_batch_flag_valid_on_default_and_sender_frames() {
        // Per EIP-8141 the atomic batch flag is valid on DEFAULT and SENDER
        // frames, and a batch never contains a VERIFY frame.
        let mut tx = make_test_frame_tx();
        tx.frames = vec![
            Frame {
                mode: FrameMode::Default as u8,
                flags: 0x04, // atomic batch
                target: Some(Address::from_low_u64_be(0xB0B)),
                gas_limit: 21_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
            Frame {
                mode: FrameMode::Sender as u8,
                flags: 0x04, // atomic batch
                target: Some(Address::from_low_u64_be(0xB0B)),
                gas_limit: 21_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
            Frame {
                mode: FrameMode::Sender as u8,
                flags: 0x00, // terminator: no flag
                target: Some(Address::from_low_u64_be(0xCAFE)),
                gas_limit: 21_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ];
        assert!(tx.validate_static_constraints().is_ok());
    }

    #[test]
    fn atomic_batch_flag_on_last_frame_still_invalid() {
        let mut tx = make_test_frame_tx();
        tx.frames = vec![Frame {
            mode: FrameMode::Sender as u8,
            flags: 0x04,
            target: Some(Address::from_low_u64_be(0xCAFE)),
            gas_limit: 21_000,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        }];
        let err = tx.validate_static_constraints().unwrap_err();
        assert!(err.contains("atomic batch flag on last frame"), "{err}");
    }

    #[test]
    fn test_frame_rlp_roundtrip() {
        let frame = Frame {
            mode: FrameMode::Verify as u8,
            flags: 0x03,
            target: Some(Address::from_low_u64_be(0x1234)),
            gas_limit: 50_000,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::from_static(b"hello"),
        };
        let encoded = frame.encode_to_vec();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn test_frame_null_target_rlp_roundtrip() {
        let frame = Frame {
            mode: FrameMode::Default as u8,
            flags: 0x00,
            target: None,
            gas_limit: 10_000,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::from_static(b"deploy"),
        };
        let encoded = frame.encode_to_vec();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn test_frame_mode_with_flags_rlp_roundtrip() {
        // Test basic modes
        for mode_val in [0u8, 1, 2] {
            let frame = Frame {
                mode: mode_val,
                flags: if mode_val == 1 { 0x03 } else { 0x00 },
                target: Some(Address::from_low_u64_be(0x1234)),
                gas_limit: 50_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            };
            let encoded = frame.encode_to_vec();
            let decoded = Frame::decode(&encoded).unwrap();
            assert_eq!(frame, decoded);
        }
        // Test mode with scope restriction (bits 0-1 of flags) and atomic batch (bit 2 of flags)
        let frame = Frame {
            mode: 2,            // SENDER
            flags: 0x01 | 0x04, // scope=1 + atomic_batch
            target: Some(Address::from_low_u64_be(0x1234)),
            gas_limit: 50_000,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        };
        assert_eq!(frame.execution_mode(), FrameMode::Sender);
        assert_eq!(frame.scope_restriction(), 1);
        assert!(frame.is_atomic_batch());
        let encoded = frame.encode_to_vec();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn frame_rlp_roundtrip_preserves_value() {
        let frame = Frame {
            mode: FrameMode::Sender as u8,
            flags: 0x00,
            target: Some(Address::from_low_u64_be(0xCAFE)),
            gas_limit: 100_000,
            state_gas_limit: 0,
            value: U256::from(1_000_000_000_000_000u64), // 0.001 ETH
            data: Bytes::from_static(b"hello"),
        };
        let encoded = frame.encode_to_vec();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(frame, decoded);
        assert_eq!(decoded.value, U256::from(1_000_000_000_000_000u64));
    }

    #[test]
    fn test_frame_transaction_rlp_roundtrip() {
        let tx = make_test_frame_tx();
        let encoded = tx.encode_to_vec();
        let decoded = FrameTransaction::decode(&encoded).unwrap();
        assert_eq!(tx, decoded);
    }

    #[test]
    fn test_frame_transaction_canonical_roundtrip() {
        let tx = Transaction::FrameTransaction(make_test_frame_tx());
        let encoded = tx.encode_canonical_to_vec();
        let decoded = Transaction::decode_canonical(&encoded).unwrap();
        // Compare hashes since OnceCell cached state differs after encode
        assert_eq!(
            tx.hash(&ethrex_crypto::NativeCrypto),
            decoded.hash(&ethrex_crypto::NativeCrypto)
        );
        // Also verify core fields match
        assert_eq!(tx.tx_type(), decoded.tx_type());
        assert_eq!(tx.nonce(), decoded.nonce());
        assert_eq!(tx.chain_id(), decoded.chain_id());
    }

    #[test]
    fn test_frame_transaction_sig_hash_covers_all_frame_data() {
        // Updated for frame data is NO LONGER elided.
        let tx = make_test_frame_tx();
        let hash1 = tx.compute_sig_hash();

        // Changing VERIFY frame data now DOES change the sig_hash.
        let mut tx2 = tx.clone();
        tx2.frames[0].data = Bytes::from_static(b"completely_different_verify_data");
        let hash2 = tx2.compute_sig_hash();
        assert_ne!(hash1, hash2);

        // Changing SENDER frame data also produces a different hash.
        let mut tx3 = tx;
        tx3.frames[1].data = Bytes::from_static(b"different_call_data");
        let hash3 = tx3.compute_sig_hash();
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_frame_transaction_accessor_methods() {
        let frame_tx = make_test_frame_tx();
        let tx = Transaction::FrameTransaction(frame_tx);

        assert_eq!(tx.tx_type(), TxType::Frame);
        assert_eq!(tx.nonce(), 42);
        assert_eq!(tx.chain_id(), Some(1));
        assert_eq!(tx.to(), TxKind::Call(Address::from_low_u64_be(0xABCD)));
        assert_eq!(tx.value(), U256::zero());
        assert_eq!(tx.data(), &Bytes::new());
        assert!(tx.access_list().is_empty());
        assert!(tx.authorization_list().is_none());
        assert_eq!(tx.max_priority_fee(), Some(U256::from(1_000_000_000u64)));
        // EIP-8141: one `max_fee` budget, no per-gas or per-blob-gas caps.
        assert_eq!(tx.max_fee_per_gas(), None);
        assert_eq!(tx.max_fee_per_blob_gas(), None);
        assert_eq!(tx.gas_fee_cap(), U256::from(10_000_000_000_000_000u64));
        assert!(!tx.is_contract_creation());
        // sender returns explicit sender, no ECDSA
        assert_eq!(
            tx.sender(&ethrex_crypto::NativeCrypto).unwrap(),
            Address::from_low_u64_be(0xABCD)
        );
        // gas_limit = intrinsic + calldata_cost + sum(frame.gas_limit)
        assert!(tx.gas_limit() >= 300_000); // at least sum of frame gas limits
    }

    #[test]
    fn test_frame_transaction_hash_uses_oncecell() {
        let tx = Transaction::FrameTransaction(make_test_frame_tx());
        let h1 = tx.hash(&ethrex_crypto::NativeCrypto);
        let h2 = tx.hash(&ethrex_crypto::NativeCrypto);
        assert_eq!(h1, h2);
    }

    #[test]
    fn frame_transaction_rlp_roundtrip_preserves_fields() {
        let tx = make_test_frame_tx();
        let mut buf = Vec::new();
        tx.encode(&mut buf);
        let (decoded, rest) = FrameTransaction::decode_unfinished(&buf).unwrap();
        assert!(rest.is_empty());
        assert_eq!(decoded.chain_id, tx.chain_id);
        assert_eq!(decoded.nonce_keys, tx.nonce_keys);
        assert_eq!(decoded.nonce_seq, tx.nonce_seq);
        assert_eq!(decoded.sender, tx.sender);
    }

    #[test]
    fn frame_transaction_variant_is_exposed_on_transaction_enum() {
        let tx = Transaction::FrameTransaction(make_test_frame_tx());
        assert!(matches!(tx, Transaction::FrameTransaction(_)));
    }

    #[test]
    fn frame_transaction_serializes_claims_as_array() {
        // RPC must expose the full claim objects, not just a count.
        let tx = make_test_frame_tx();
        let json = serde_json::to_value(&tx).unwrap();
        let claims = json
            .get("claims")
            .expect("claims field present")
            .as_array()
            .expect("claims serialized as an array");
        assert_eq!(claims.len(), tx.claims.len());
        assert_eq!(
            claims[0].get("verifier").unwrap(),
            "0x00000000000000000000000000000000000000ab"
        );
        assert_eq!(claims[0].get("key").unwrap(), "0x");
        assert_eq!(claims[0].get("msg").unwrap(), "0x");
        assert_eq!(claims[0].get("gasLimit").unwrap(), "0x0");
        assert_eq!(claims[0].get("witnessLen").unwrap(), "0x0");
        assert!(json.get("signatures").is_none());
    }

    fn make_frame_tx_with_gas_limits(limits: Vec<u64>) -> FrameTransaction {
        let frames = limits
            .into_iter()
            .map(|gl| Frame {
                mode: FrameMode::Sender as u8,
                flags: 0x00,
                target: Some(Address::from_low_u64_be(0x1234)),
                gas_limit: gl,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            })
            .collect();
        FrameTransaction {
            chain_id: 1,
            nonce_keys: vec![],
            nonce_seq: 0,
            sender: Address::from_low_u64_be(0xABCD),
            frames,
            claims: vec![],
            max_fee: U256::from(10_000_000_000_000_000u64),
            max_priority_fee_per_gas: U256::from(1_000_000_000u64),
            blob_versioned_hashes: vec![],
            inner_hash: OnceCell::new(),
            cached_canonical: OnceCell::new(),
        }
    }

    #[test]
    fn per_frame_gas_limit_above_i64_max_is_rejected() {
        let tx = make_frame_tx_with_gas_limits(vec![(i64::MAX as u64) + 1]);
        let err = tx.validate_static_constraints().unwrap_err();
        assert!(err.contains("exceeds 2**63-1"), "unexpected error: {err}");
    }

    #[test]
    fn cumulative_frame_gas_limit_above_i64_max_is_rejected() {
        let half = (i64::MAX as u64) / 2 + 1;
        let tx = make_frame_tx_with_gas_limits(vec![half, half]);
        let err = tx.validate_static_constraints().unwrap_err();
        assert!(err.contains("cumulative"), "unexpected error: {err}");
    }

    #[test]
    fn cumulative_frame_gas_limit_equal_to_i64_max_is_accepted() {
        let a = (i64::MAX as u64) / 2;
        let b = i64::MAX as u64 - a;
        let tx = make_frame_tx_with_gas_limits(vec![a, b]);
        tx.validate_static_constraints()
            .expect("exact i64::MAX total should be accepted");
    }

    #[test]
    fn empty_frames_list_is_rejected_by_count_check_not_gas_check() {
        // The frame-count check fires before the gas-limit accumulator runs,
        // so an empty frame list surfaces the count error, not a gas error.
        let tx = make_frame_tx_with_gas_limits(vec![]);
        let err = tx.validate_static_constraints().unwrap_err();
        assert!(err.contains("between 1 and"), "unexpected error: {err}");
    }

    #[test]
    fn sig_hash_covers_frame_value() {
        // Changing `value` on any frame (SENDER or VERIFY) must change the
        // canonical signature hash (all frame data covered).
        let tx = make_test_frame_tx();
        let baseline = tx.compute_sig_hash();

        let mut with_sender_value = tx.clone();
        with_sender_value.frames[1].value = U256::from(1u64);
        assert_ne!(
            baseline,
            with_sender_value.compute_sig_hash(),
            "sig_hash must change when a SENDER frame's value changes"
        );

        let mut with_verify_value = tx.clone();
        with_verify_value.frames[0].value = U256::from(1u64);
        assert_ne!(
            baseline,
            with_verify_value.compute_sig_hash(),
            "sig_hash must change when a VERIFY frame's value changes"
        );

        // VERIFY.data is now covered too (EIP-8141 removed elision).
        let mut with_verify_data = tx;
        with_verify_data.frames[0].data = Bytes::from_static(b"different_verify_data");
        assert_ne!(
            baseline,
            with_verify_data.compute_sig_hash(),
            "VERIFY.data is now covered by sig_hash (no longer elided)"
        );
    }

    /// Helper: a minimal frame transaction carrying `frames`, valid in every
    /// respect the test in question is not exercising.
    fn frame_tx_with(frames: Vec<Frame>) -> FrameTransaction {
        FrameTransaction {
            chain_id: 1,
            nonce_keys: vec![],
            nonce_seq: 0,
            sender: Address::from_low_u64_be(0xABCD),
            frames,
            claims: vec![],
            max_fee: U256::from(10_000_000_000_000_000u64),
            max_priority_fee_per_gas: U256::from(1_000_000_000u64),
            blob_versioned_hashes: vec![],
            inner_hash: OnceCell::new(),
            cached_canonical: OnceCell::new(),
        }
    }

    #[test]
    fn validate_static_constraints_rejects_expiry_verifier_frame_with_state_gas() {
        // The expiry verifier creates no state, so its frame must declare no
        // state-gas budget. Everything else about this frame is well formed, so a
        // failure here can only come from the state budget.
        let tx = frame_tx_with(vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: 0,
            target: Some(frame_tx_expiry_verifier()),
            gas_limit: 100_000,
            state_gas_limit: 1,
            value: U256::zero(),
            data: Bytes::from(vec![0u8; FRAME_TX_EXPIRY_DATA_LENGTH]),
        }]);
        let err = tx.validate_static_constraints().unwrap_err();
        assert!(
            err.contains("expiry verifier frame must have state_gas_limit == 0"),
            "unexpected error: {err}"
        );

        // The same frame with a zero state budget passes this rule.
        let ok = frame_tx_with(vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: 0,
            target: Some(frame_tx_expiry_verifier()),
            gas_limit: 100_000,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::from(vec![0u8; FRAME_TX_EXPIRY_DATA_LENGTH]),
        }]);
        assert!(ok.validate_static_constraints().is_ok());
    }

    #[test]
    fn validate_static_constraints_sums_both_gas_dimensions() {
        // A state budget drives the cumulative overflow exactly as an execution
        // budget does. Accumulating only execution gas would let this through to
        // execution, where it surfaces as a block gas-allowance failure rather
        // than the format violation it is.
        let state_heavy = frame_tx_with(vec![Frame {
            mode: FrameMode::Default as u8,
            flags: 0,
            target: None,
            gas_limit: 100_000,
            state_gas_limit: u64::MAX,
            value: U256::zero(),
            data: Bytes::new(),
        }]);
        let err = state_heavy.validate_static_constraints().unwrap_err();
        assert!(
            err.contains("state_gas_limit") && err.contains("exceeds 2**63-1"),
            "unexpected error: {err}"
        );

        // Neither dimension exceeds the per-frame bound on its own, but together
        // across frames they cross it, so only a cross-dimension sum catches this.
        let half = (i64::MAX as u64) / 2 + 1;
        let cross = frame_tx_with(vec![
            Frame {
                mode: FrameMode::Default as u8,
                flags: 0,
                target: None,
                gas_limit: half,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
            Frame {
                mode: FrameMode::Default as u8,
                flags: 0,
                target: None,
                gas_limit: 0,
                state_gas_limit: half,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ]);
        let err = cross.validate_static_constraints().unwrap_err();
        assert!(
            err.contains("cumulative frame gas") && err.contains("exceeds 2**63-1"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_static_constraints_rejects_nonzero_value_on_non_sender_frames() {
        // VERIFY frame with non-zero value must be rejected.
        let verify_tx = FrameTransaction {
            chain_id: 1,
            nonce_keys: vec![],
            nonce_seq: 0,
            sender: Address::from_low_u64_be(0xABCD),
            frames: vec![Frame {
                mode: FrameMode::Verify as u8,
                flags: 0x01,
                target: None,
                gas_limit: 50_000,
                state_gas_limit: 0,
                value: U256::from(1u64),
                data: Bytes::new(),
            }],
            claims: vec![],
            max_fee: U256::from(10_000_000_000_000_000u64),
            max_priority_fee_per_gas: U256::from(1_000_000_000u64),
            blob_versioned_hashes: vec![],
            inner_hash: OnceCell::new(),
            cached_canonical: OnceCell::new(),
        };
        let err = verify_tx.validate_static_constraints().unwrap_err();
        assert!(
            err.contains("non-zero value only allowed in SENDER mode"),
            "unexpected error for VERIFY: {err}"
        );

        // DEFAULT frame with non-zero value must be rejected.
        let default_tx = FrameTransaction {
            frames: vec![Frame {
                mode: FrameMode::Default as u8,
                flags: 0x00,
                target: Some(Address::from_low_u64_be(0x1234)),
                gas_limit: 50_000,
                state_gas_limit: 0,
                value: U256::from(1u64),
                data: Bytes::new(),
            }],
            ..verify_tx
        };
        let err = default_tx.validate_static_constraints().unwrap_err();
        assert!(
            err.contains("non-zero value only allowed in SENDER mode"),
            "unexpected error for DEFAULT: {err}"
        );

        // SENDER frame with a non-zero value must remain valid.
        let sender_tx = FrameTransaction {
            frames: vec![Frame {
                mode: FrameMode::Sender as u8,
                flags: 0x00,
                target: Some(Address::from_low_u64_be(0x1234)),
                gas_limit: 50_000,
                state_gas_limit: 0,
                value: U256::from(1u64),
                data: Bytes::new(),
            }],
            ..default_tx
        };
        sender_tx
            .validate_static_constraints()
            .expect("SENDER frames may carry non-zero value");
    }

    // ── EIP-8141 fork-gate predicate tests ──

    fn chain_config_with_hegota(hegota_time: Option<u64>) -> crate::types::ChainConfig {
        crate::types::ChainConfig {
            hegota_time,
            ..Default::default()
        }
    }

    #[test]
    fn test_frame_tx_pre_fork_chain_config_rejects() {
        let cfg = chain_config_with_hegota(None);
        assert!(!cfg.is_hegota_activated(0));
        assert!(!cfg.is_hegota_activated(u64::MAX));
    }

    #[test]
    fn test_frame_tx_post_fork_admits() {
        let cfg = chain_config_with_hegota(Some(1000));
        assert!(cfg.is_hegota_activated(2000));
    }

    #[test]
    fn test_frame_tx_fork_boundary_admits() {
        let activation_time = 1_700_000_000u64;
        let cfg = chain_config_with_hegota(Some(activation_time));
        assert!(cfg.is_hegota_activated(activation_time));
        assert!(!cfg.is_hegota_activated(activation_time - 1));
    }

    #[test]
    fn test_frame_tx_devnet_fork_epoch_zero_admits() {
        let cfg = chain_config_with_hegota(Some(0));
        assert!(cfg.is_hegota_activated(0));
        assert!(cfg.is_hegota_activated(1));
        assert!(cfg.is_hegota_activated(u64::MAX));
    }

    #[test]
    fn test_frame_tx_pre_fork_rlp_roundtrip_still_decodes() {
        // RLP decoding is fork-unaware and must stay lossless so validators surface
        // a FrameTxPreFork error downstream rather than a corrupt-RLP error.
        let tx = make_test_frame_tx();
        let mut buf = Vec::new();
        tx.encode(&mut buf);
        let (decoded, rest) = FrameTransaction::decode_unfinished(&buf).unwrap();
        assert!(rest.is_empty());
        assert_eq!(decoded.chain_id, tx.chain_id);
        assert_eq!(decoded.nonce_keys, tx.nonce_keys);
        assert_eq!(decoded.nonce_seq, tx.nonce_seq);
        assert_eq!(decoded.sender, tx.sender);
    }

    // ── EIP-8141 expiry verifier frame tests ──

    fn expiry_frame(deadline: u64) -> Frame {
        Frame {
            mode: FrameMode::Verify as u8,
            flags: 0x00,
            target: Some(frame_tx_expiry_verifier()),
            gas_limit: 30_000,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::copy_from_slice(&deadline.to_be_bytes()),
        }
    }

    #[test]
    fn expiry_verifier_frame_passes_static_validation() {
        let mut tx = make_test_frame_tx();
        tx.frames.insert(0, expiry_frame(1_700_000_000));
        assert!(tx.validate_static_constraints().is_ok());
    }

    #[test]
    fn expiry_verifier_frame_rejects_bad_data_length() {
        let mut tx = make_test_frame_tx();
        let mut f = expiry_frame(0);
        f.data = Bytes::from_static(b"short");
        tx.frames.insert(0, f);
        let err = tx.validate_static_constraints().unwrap_err();
        assert!(err.contains("8 bytes"), "{err}");
    }

    #[test]
    fn expiry_verifier_frame_rejects_nonzero_flags() {
        let mut tx = make_test_frame_tx();
        let mut f = expiry_frame(0);
        f.flags = 0x01;
        tx.frames.insert(0, f);
        let err = tx.validate_static_constraints().unwrap_err();
        assert!(err.contains("flags == 0"), "{err}");
    }

    #[test]
    fn at_most_one_expiry_verifier_frame() {
        let mut tx = make_test_frame_tx();
        tx.frames.insert(0, expiry_frame(1));
        tx.frames.insert(0, expiry_frame(2));
        let err = tx.validate_static_constraints().unwrap_err();
        assert!(err.contains("more than one expiry"), "{err}");
    }

    #[test]
    fn verify_frame_with_zero_scope_is_now_statically_valid() {
        // EIP-8141 has no the VERIFY-needs-nonzero-scope rule.
        // make_test_frame_tx() frame[0] is VERIFY with flags=0x03 targeting sender;
        // setting flags to 0 makes it a zero-scope VERIFY (not an expiry frame,
        // since target is the sender address, not EXPIRY_VERIFIER).
        let mut tx = make_test_frame_tx();
        tx.frames[0].flags = 0x00;
        assert!(tx.validate_static_constraints().is_ok());
    }

    #[test]
    fn sig_hash_covers_expiry_deadline_and_all_verify_data() {
        // Updated for all frame data is now covered.
        let mut tx_a = make_test_frame_tx();
        tx_a.frames.insert(0, expiry_frame(100));
        let mut tx_b = make_test_frame_tx();
        tx_b.frames.insert(0, expiry_frame(200));
        // Different deadlines -> different sig hashes (expiry data covered).
        assert_ne!(tx_a.compute_sig_hash(), tx_b.compute_sig_hash());

        // Different NON-expiry VERIFY data -> different sig hash (no longer elided).
        let tx_c = make_test_frame_tx();
        let mut tx_d = make_test_frame_tx();
        tx_d.frames[0].data = Bytes::from_static(b"different_verify_data");
        assert_ne!(tx_c.compute_sig_hash(), tx_d.compute_sig_hash());
    }

    #[test]
    fn expiry_deadline_parses_or_none() {
        let mut tx = make_test_frame_tx();
        assert_eq!(tx.expiry_deadline(), None);
        tx.frames.insert(0, expiry_frame(1_700_000_123));
        assert_eq!(tx.expiry_deadline(), Some(1_700_000_123));
    }

    #[test]
    fn claim_rlp_roundtrip() {
        let claim = Claim {
            verifier: Address::from_low_u64_be(0x1234),
            key: Bytes::from(vec![5u8; 20]),
            msg: Bytes::from(vec![7u8; 32]),
            gas_limit: 50_000,
            witness_len: 128,
        };
        let mut buf = Vec::new();
        claim.encode(&mut buf);
        let (decoded, rest) = Claim::decode_unfinished(&buf).unwrap();
        assert!(rest.is_empty());
        assert_eq!(decoded, claim);

        // A claim of four or six items is not a claim, and neither is one whose
        // verifier is not 20 bytes long.
        let mut short = Vec::new();
        Encoder::new(&mut short)
            .encode_field(&claim.verifier)
            .encode_field(&claim.key)
            .encode_field(&claim.msg)
            .encode_field(&claim.gas_limit)
            .finish();
        assert!(Claim::decode(&short).is_err());
        let mut long = Vec::new();
        Encoder::new(&mut long)
            .encode_field(&claim.verifier)
            .encode_field(&claim.key)
            .encode_field(&claim.msg)
            .encode_field(&claim.gas_limit)
            .encode_field(&claim.witness_len)
            .encode_field(&0u8)
            .finish();
        assert!(Claim::decode(&long).is_err());
        let mut wide_verifier = Vec::new();
        Encoder::new(&mut wide_verifier)
            .encode_field(&H256::repeat_byte(1))
            .encode_field(&claim.key)
            .encode_field(&claim.msg)
            .encode_field(&claim.gas_limit)
            .encode_field(&claim.witness_len)
            .finish();
        assert!(Claim::decode(&wide_verifier).is_err());
    }

    #[test]
    fn frame_tx_with_claims_rlp_roundtrip() {
        let tx = make_test_frame_tx();
        assert!(!tx.claims.is_empty());
        let mut buf = Vec::new();
        tx.encode(&mut buf);
        let (decoded, rest) = FrameTransaction::decode_unfinished(&buf).unwrap();
        assert!(rest.is_empty());
        assert_eq!(decoded.claims, tx.claims);
        assert_eq!(decoded.frames, tx.frames);
        assert_eq!(decoded.nonce_keys, tx.nonce_keys);
        assert_eq!(decoded.nonce_seq, tx.nonce_seq);
    }

    #[test]
    fn p2p_frame_transaction_rlp_roundtrip() {
        // A frame tx (EIP-8141) must survive a full P2PTransaction RLP
        // encode/decode round-trip in its pooled representation, with its
        // witnesses, so it can be served over the wire on request.
        let ft = make_test_frame_tx();
        assert!(!ft.claims.is_empty());
        assert!(!ft.frames.is_empty());
        let hash = Transaction::FrameTransaction(ft.clone()).compute_hash(&NativeCrypto);
        let original = P2PTransaction::FrameTransaction(PooledFrameTransaction {
            tx: ft,
            witnesses: vec![Bytes::from(vec![7u8; SECP256K1_WITNESS_SIZE])],
            blobs_bundle: None,
        });

        let encoded = original.encode_to_vec();
        let (decoded, rest) = P2PTransaction::decode_unfinished(&encoded).unwrap();
        assert!(rest.is_empty());
        assert_eq!(decoded, original);
        assert_eq!(decoded.tx_type(), TxType::Frame);
        // The hash identifies the transaction body, whatever witnesses travel
        // with it.
        assert_eq!(decoded.compute_hash(), hash);

        // It does not convert into a bare transaction: that would drop its
        // witnesses.
        assert!(TryInto::<Transaction>::try_into(decoded).is_err());
    }

    #[test]
    fn static_validation_accepts_every_form_of_claim() {
        let mut tx = make_test_frame_tx();
        tx.claims = vec![
            // Protocol verifiers, with each accepted form of `key` and `msg`.
            Claim {
                verifier: SECP256K1_VERIFIER,
                ..Default::default()
            },
            Claim {
                verifier: P256_VERIFIER,
                key: Bytes::from(vec![5u8; 20]),
                msg: Bytes::from(vec![9u8; 32]),
                ..Default::default()
            },
            Claim {
                verifier: LEANSPHINCS_VERIFIER,
                key: Bytes::from(vec![6u8; 32]),
                ..Default::default()
            },
            // A custom verifier declares its gas and its witness length.
            Claim {
                verifier: Address::from_low_u64_be(0xC0DE),
                key: Bytes::from(vec![6u8; 32]),
                msg: Bytes::from(vec![9u8; 32]),
                gas_limit: 1,
                witness_len: MAX_CLAIM_WITNESS_BYTES,
            },
        ];
        tx.validate_static_constraints()
            .expect("every claim is well formed");
        assert_eq!(tx.claim_verification_cost(), 2800 + 6700 + 13200 + 1);
    }

    /// The error static validation reports for a transaction whose only claim is
    /// `claim`.
    fn claim_error(claim: Claim) -> String {
        let mut tx = make_test_frame_tx();
        tx.claims = vec![claim];
        tx.validate_static_constraints()
            .expect_err("the claim is malformed")
    }

    #[test]
    fn static_validation_rejects_a_malformed_key() {
        for key in [
            vec![1u8; 19],
            vec![1u8; 21],
            vec![1u8; 31],
            vec![1u8; 33],
            vec![0u8; 20],
            vec![0u8; 32],
        ] {
            let error = claim_error(Claim {
                verifier: LEANSPHINCS_VERIFIER,
                key: Bytes::from(key),
                ..Default::default()
            });
            assert!(error.contains("key must be empty"), "{error}");
        }
        // The key of a secp256k1 or P-256 claim is an address: a 32-byte key with
        // any of its upper 12 bytes set is rejected, and one without is accepted.
        let mut wide = vec![0u8; 32];
        wide[11] = 1;
        wide[31] = 1;
        for verifier in [SECP256K1_VERIFIER, P256_VERIFIER] {
            let error = claim_error(Claim {
                verifier,
                key: Bytes::from(wide.clone()),
                ..Default::default()
            });
            assert!(error.contains("must be an address"), "{error}");
            let mut tx = make_test_frame_tx();
            let mut padded = vec![0u8; 32];
            padded[31] = 1;
            tx.claims = vec![Claim {
                verifier,
                key: Bytes::from(padded),
                ..Default::default()
            }];
            tx.validate_static_constraints()
                .expect("an address in 32 bytes");
        }
        // leanSPHINCS keys and custom verifiers' keys use all 32 bytes.
        let mut tx = make_test_frame_tx();
        tx.claims = vec![Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::from(wide),
            ..Default::default()
        }];
        tx.validate_static_constraints()
            .expect("a 32-byte public key");
    }

    #[test]
    fn static_validation_rejects_bad_msg_length() {
        let error = claim_error(Claim {
            verifier: SECP256K1_VERIFIER,
            msg: Bytes::from(vec![1u8; 16]),
            ..Default::default()
        });
        assert!(error.contains("32 bytes"), "{error}");
    }

    #[test]
    fn static_validation_rejects_zero_explicit_msg() {
        let error = claim_error(Claim {
            verifier: SECP256K1_VERIFIER,
            msg: Bytes::from(vec![0u8; 32]),
            ..Default::default()
        });
        assert!(error.contains("zero digest"), "{error}");
    }

    #[test]
    fn static_validation_fixes_the_gas_and_witness_length_of_protocol_claims() {
        for verifier in [SECP256K1_VERIFIER, P256_VERIFIER, LEANSPHINCS_VERIFIER] {
            let error = claim_error(Claim {
                verifier,
                gas_limit: 1,
                ..Default::default()
            });
            assert!(error.contains("gas_limit must be zero"), "{error}");
            let error = claim_error(Claim {
                verifier,
                witness_len: 65,
                ..Default::default()
            });
            assert!(error.contains("witness_len must be zero"), "{error}");
        }
    }

    #[test]
    fn static_validation_bounds_the_gas_and_witness_length_of_custom_claims() {
        let custom = Claim {
            verifier: Address::from_low_u64_be(0xC0DE),
            gas_limit: 1,
            ..Default::default()
        };
        let error = claim_error(Claim {
            gas_limit: 0,
            ..custom.clone()
        });
        assert!(error.contains("gas_limit must not be zero"), "{error}");
        let error = claim_error(Claim {
            witness_len: MAX_CLAIM_WITNESS_BYTES + 1,
            ..custom.clone()
        });
        assert!(error.contains("witness_len"), "{error}");

        // The claims' gas adds up to less than 2**64.
        let mut tx = make_test_frame_tx();
        tx.claims = vec![
            Claim {
                gas_limit: u64::MAX,
                ..custom.clone()
            },
            Claim {
                gas_limit: 1,
                ..custom
            },
        ];
        let error = tx.validate_static_constraints().expect_err("overflow");
        assert!(error.contains("cumulative claim gas overflow"), "{error}");
    }

    #[test]
    fn static_validation_bounds_the_number_of_claims() {
        let mut tx = make_test_frame_tx();
        tx.claims = vec![tx.claims[0].clone(); MAX_CLAIMS];
        tx.validate_static_constraints().expect("MAX_CLAIMS claims");
        tx.claims.push(tx.claims[0].clone());
        let error = tx.validate_static_constraints().expect_err("too many");
        assert!(error.contains("Claim count"), "{error}");
    }

    #[test]
    fn reserved_frame_modes_are_rejected() {
        // Mode 4 is not a frame mode of this proposal: it is reserved like every
        // mode above EIP-7906's ASSERT.
        for mode in [4u8, 5, 255] {
            let mut tx = make_test_frame_tx();
            tx.frames[1].mode = mode;
            let error = tx.validate_static_constraints().expect_err("reserved");
            assert!(error.contains("reserved execution mode"), "{error}");
        }
    }

    #[test]
    fn the_signature_hash_is_domain_separated_from_the_transaction_hash() {
        let tx = make_test_frame_tx();
        let mut body = Vec::new();
        tx.encode(&mut body);

        // tx_hash = keccak(0x06 || rlp(tx_body)).
        let mut tx_preimage = vec![0x06];
        tx_preimage.extend_from_slice(&body);
        let tx_hash = Transaction::FrameTransaction(tx.clone()).compute_hash(&NativeCrypto);
        assert_eq!(tx_hash, keccak(&tx_preimage));

        // sig_hash = keccak(0x06 || 0x00 || rlp(tx_body)).
        let mut sig_preimage = vec![0x06, 0x00];
        sig_preimage.extend_from_slice(&body);
        assert_eq!(tx.compute_sig_hash(), keccak(&sig_preimage));

        assert_ne!(tx.compute_sig_hash(), tx_hash);
    }

    #[test]
    fn sig_hash_covers_frame_data_now() {
        let mut a = make_test_frame_tx();
        let mut b = make_test_frame_tx();
        // frame 0 is a VERIFY frame; its data is COVERED (not elided).
        a.frames[0].data = Bytes::from_static(b"aaaa");
        b.frames[0].data = Bytes::from_static(b"bbbb");
        assert_ne!(a.compute_sig_hash(), b.compute_sig_hash());
    }

    #[test]
    fn both_hashes_cover_every_field_of_every_claim() {
        let base = make_test_frame_tx();
        let hashes = |tx: &FrameTransaction| {
            (
                tx.compute_sig_hash(),
                Transaction::FrameTransaction(tx.clone()).compute_hash(&NativeCrypto),
            )
        };
        let (base_sig_hash, base_tx_hash) = hashes(&base);
        let custom = Claim {
            verifier: Address::from_low_u64_be(0xC0DE),
            key: Bytes::from(vec![6u8; 32]),
            msg: Bytes::from(vec![9u8; 32]),
            gas_limit: 10,
            witness_len: 20,
        };
        let variants = [
            Claim {
                verifier: Address::from_low_u64_be(0xC0DF),
                ..custom.clone()
            },
            Claim {
                key: Bytes::from(vec![7u8; 32]),
                ..custom.clone()
            },
            Claim {
                msg: Bytes::from(vec![8u8; 32]),
                ..custom.clone()
            },
            Claim {
                gas_limit: 11,
                ..custom.clone()
            },
            Claim {
                witness_len: 21,
                ..custom.clone()
            },
        ];
        let mut with_custom = base.clone();
        with_custom.claims.push(custom);
        let (custom_sig_hash, custom_tx_hash) = hashes(&with_custom);
        assert_ne!(custom_sig_hash, base_sig_hash);
        assert_ne!(custom_tx_hash, base_tx_hash);
        for variant in variants {
            let mut tx = base.clone();
            tx.claims.push(variant);
            let (sig_hash, tx_hash) = hashes(&tx);
            assert_ne!(sig_hash, custom_sig_hash);
            assert_ne!(tx_hash, custom_tx_hash);
        }
    }

    #[test]
    fn max_gas_includes_claim_costs() {
        let mut tx = make_test_frame_tx();
        let base = tx.max_gas();
        // Add a P-256 claim: its verification, its witness at `WITNESS_BYTE_GAS`
        // per byte and its 20-byte verifier as calldata.
        tx.claims.push(Claim {
            verifier: P256_VERIFIER,
            key: Bytes::from(Address::from_low_u64_be(1).as_bytes().to_vec()),
            ..Default::default()
        });
        assert_eq!(tx.claim_verification_cost(), 2800 + 6700);
        assert_eq!(tx.protocol_witness_cost(), WITNESS_BYTE_GAS * (65 + 128));
        // verifier: 19 zero bytes and one non-zero; key: the same.
        let metadata = 2 * (19 * 4 + 16);
        assert_eq!(
            tx.max_gas(),
            base + 6700 + WITNESS_BYTE_GAS * 128 + metadata
        );
    }

    #[test]
    fn golden_frame_tx_rlp_and_hashes() {
        // Regression lock for the wire format of the declarative-claims
        // proposal for EIP-8141: `claims` ahead of `frames`, each claim a
        // five-item list, the `limits = [execution, state]` frame tuple and the
        // nested `fees` list, with EIP-8250's `nonce_keys` and `nonce_seq` in
        // place of `nonce`. The expected bytes and hashes were computed with an
        // independent RLP encoder and Keccak-256 written from their
        // specifications, and must only change with a deliberate format change.
        let tx = FrameTransaction {
            chain_id: 1,
            nonce_keys: vec![U256::zero()],
            nonce_seq: 7,
            sender: Address::from_low_u64_be(0xABCD),
            claims: vec![
                Claim {
                    verifier: SECP256K1_VERIFIER,
                    ..Default::default()
                },
                Claim {
                    verifier: Address::from_low_u64_be(0xC0DE),
                    key: Bytes::from(vec![0x22u8; 32]),
                    msg: Bytes::from(vec![0x33u8; 32]),
                    gas_limit: 50_000,
                    witness_len: 100,
                },
            ],
            frames: vec![
                Frame {
                    mode: 1,
                    flags: 3,
                    target: None,
                    gas_limit: 0x5208,
                    state_gas_limit: 0x1e8480,
                    value: U256::zero(),
                    data: Bytes::from_static(&[0x11, 0x22]),
                },
                Frame {
                    mode: 2,
                    flags: 0,
                    target: Some(Address::from_low_u64_be(0x1234)),
                    gas_limit: 0x9c40,
                    state_gas_limit: 0,
                    value: U256::zero(),
                    data: Bytes::new(),
                },
            ],
            max_fee: U256::from(0x6fc23ac00u64),
            max_priority_fee_per_gas: U256::from(0x3b9aca00u64),
            blob_versioned_hashes: vec![],
            inner_hash: OnceCell::new(),
            cached_canonical: OnceCell::new(),
        };

        const GOLDEN_RLP: &str = "f8cf01c1800794000000000000000000000000000000000000abcdf877d99400000000000000000000000000000000000000ab80808080f85b94000000000000000000000000000000000000c0dea02222222222222222222222222222222222222222222222222222222222222222a0333333333333333333333333333333333333333333333333333333333333333382c35064efcf010380c7825208831e848080821122de0280940000000000000000000000000000000000001234c4829c40808080cb8506fc23ac00843b9aca00c0";
        let mut buf = Vec::new();
        tx.encode(&mut buf);
        assert_eq!(hex::encode(&buf), GOLDEN_RLP);

        // Round-trips losslessly.
        let (decoded, rest) = FrameTransaction::decode_unfinished(&buf).unwrap();
        assert!(rest.is_empty());
        assert_eq!(decoded, tx);

        assert_eq!(
            format!(
                "{:#x}",
                Transaction::FrameTransaction(tx.clone()).compute_hash(&NativeCrypto)
            ),
            "0x35e8f477d9a81bcc1f76b4448d9b4785de07a92c5795c4e5af5afd10a7b7b003",
        );
        assert_eq!(
            format!("{:#x}", tx.compute_sig_hash()),
            "0xa7d942fb1bec3f3382d28582ddc29bf1877f768105e9ce54ab623e7c6f7ec435",
        );

        // The pooled representation of the same transaction, with a witness for
        // each claim and no blob sidecar, and the claim witnesses of a block that
        // carries those two witnesses.
        let witnesses = vec![
            Bytes::from(vec![0x01u8; 65]),
            Bytes::from(vec![0x02u8; 100]),
        ];
        let witnesses_rlp = format!("f8a9b841{}b864{}", "01".repeat(65), "02".repeat(100));
        let pooled = P2PTransaction::FrameTransaction(PooledFrameTransaction {
            tx,
            witnesses: witnesses.clone(),
            blobs_bundle: None,
        });
        assert_eq!(
            hex::encode(pooled.encode_canonical_to_vec()),
            format!("06f9017e{GOLDEN_RLP}80{witnesses_rlp}c0")
        );
        let claim_witnesses = encode_claim_witnesses(&witnesses);
        assert_eq!(hex::encode(&claim_witnesses), format!("00{witnesses_rlp}"));
        assert_eq!(
            format!("{:#x}", keccak(&claim_witnesses)),
            "0x61c1c477ec626cb1b673eb94bd38e73d245556044df47a9f13cb31eb12c3265c",
        );
    }

    #[test]
    fn test_cost_without_base_fee_eip4844_includes_blob_gas() {
        // Regression test for mempool balance check: for EIP-4844 txs,
        // cost_without_base_fee() MUST include blob_gas_used * max_fee_per_blob_gas.
        // Every peer client (geth, reth, nethermind, erigon, besu) does this.
        use crate::constants::GAS_PER_BLOB;

        let max_fee_per_gas: u64 = 100;
        let gas: u64 = 21_000;
        let value = U256::from(7u64);
        let max_fee_per_blob_gas = U256::from(50u64);
        let blob_count: usize = 1;

        let tx = Transaction::EIP4844Transaction(EIP4844Transaction {
            max_fee_per_gas,
            gas,
            value,
            max_fee_per_blob_gas,
            blob_versioned_hashes: vec![H256::zero(); blob_count],
            ..Default::default()
        });

        let got = tx.cost_without_base_fee().expect("cost is computable");

        let gas_cost = U256::from(max_fee_per_gas) * U256::from(gas);
        let blob_gas = U256::from(GAS_PER_BLOB) * U256::from(blob_count as u64);
        let blob_cost = blob_gas * max_fee_per_blob_gas;
        let expected = gas_cost + blob_cost + value;

        assert_eq!(
            got, expected,
            "blob-gas term missing from cost_without_base_fee() for EIP-4844"
        );
    }

    // ── Claims and their dependencies (EIP-8141) ──

    fn custom_claim(verifier: u64, key: u8, msg: u8, gas_limit: u64, witness_len: u64) -> Claim {
        Claim {
            verifier: Address::from_low_u64_be(verifier),
            key: Bytes::from(vec![key; 32]),
            msg: Bytes::from(vec![msg; 32]),
            gas_limit,
            witness_len,
        }
    }

    #[test]
    fn a_claim_resolves_its_key_its_message_its_gas_and_its_witness_length() {
        let sender = Address::from_low_u64_be(0xABCD);
        let sig_hash = H256::repeat_byte(0x51);

        // An empty key is the sender and an empty msg the signature hash; a
        // protocol verifier fixes the gas and the witness length.
        let claim = Claim {
            verifier: SECP256K1_VERIFIER,
            ..Default::default()
        };
        assert_eq!(
            claim.resolve_dependency(sender, sig_hash),
            Some(Dependency {
                verifier: SECP256K1_VERIFIER,
                key: H256::from(sender),
                msg: sig_hash,
                gas_limit: 2800,
                witness_len: 65,
            })
        );

        // A 20-byte key is left-padded, and an explicit msg is kept.
        let other = Address::from_low_u64_be(0x77);
        let claim = Claim {
            verifier: P256_VERIFIER,
            key: Bytes::from(other.as_bytes().to_vec()),
            msg: Bytes::from(vec![9u8; 32]),
            ..Default::default()
        };
        assert_eq!(
            claim.resolve_dependency(sender, sig_hash),
            Some(Dependency {
                verifier: P256_VERIFIER,
                key: H256::from(other),
                msg: H256::repeat_byte(9),
                gas_limit: 6700,
                witness_len: 128,
            })
        );

        // A 32-byte key is used as is.
        let claim = Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::from(vec![6u8; 32]),
            ..Default::default()
        };
        assert_eq!(
            claim.resolve_dependency(sender, sig_hash),
            Some(Dependency {
                verifier: LEANSPHINCS_VERIFIER,
                key: H256::repeat_byte(6),
                msg: sig_hash,
                gas_limit: 13200,
                witness_len: 7740,
            })
        );

        // A custom verifier's claim keeps what it declares.
        assert_eq!(
            custom_claim(0xC0DE, 1, 2, 30_000, 500).resolve_dependency(sender, sig_hash),
            Some(Dependency {
                verifier: Address::from_low_u64_be(0xC0DE),
                key: H256::repeat_byte(1),
                msg: H256::repeat_byte(2),
                gas_limit: 30_000,
                witness_len: 500,
            })
        );

        // A key or a msg the EIP rejects has no dependency.
        let malformed_key = Claim {
            verifier: SECP256K1_VERIFIER,
            key: Bytes::from(vec![1u8; 5]),
            ..Default::default()
        };
        assert_eq!(malformed_key.resolve_dependency(sender, sig_hash), None);
        let zero_msg = Claim {
            verifier: SECP256K1_VERIFIER,
            msg: Bytes::from(vec![0u8; 32]),
            ..Default::default()
        };
        assert_eq!(zero_msg.resolve_dependency(sender, sig_hash), None);
    }

    #[test]
    fn dependencies_are_the_resolved_claims_sorted_and_deduplicated() {
        let mut tx = make_test_frame_tx();
        tx.claims = vec![
            custom_claim(0xC0DE, 2, 2, 10, 10),
            Claim {
                verifier: SECP256K1_VERIFIER,
                ..Default::default()
            },
            // The same claim twice, and once more with the sender spelled out:
            // three claims, one dependency.
            Claim {
                verifier: SECP256K1_VERIFIER,
                ..Default::default()
            },
            Claim {
                verifier: SECP256K1_VERIFIER,
                key: Bytes::from(tx.sender.as_bytes().to_vec()),
                ..Default::default()
            },
            custom_claim(0xC0DE, 1, 2, 10, 10),
        ];
        let sig_hash = tx.compute_sig_hash();
        let secp = Dependency {
            verifier: SECP256K1_VERIFIER,
            key: H256::from(tx.sender),
            msg: sig_hash,
            gas_limit: 2800,
            witness_len: 65,
        };
        let custom = |key: u8| Dependency {
            verifier: Address::from_low_u64_be(0xC0DE),
            key: H256::repeat_byte(key),
            msg: H256::repeat_byte(2),
            gas_limit: 10,
            witness_len: 10,
        };

        // In claim order, one per claim.
        assert_eq!(
            tx.claim_dependencies(),
            Some(vec![custom(2), secp, secp, secp, custom(1)])
        );
        // Sorted by verifier first, then by key, and deduplicated.
        assert_eq!(tx.dependencies(), Some(vec![secp, custom(1), custom(2)]));

        assert_eq!(FrameTransaction::default().dependencies(), Some(Vec::new()));

        // One malformed claim leaves the transaction without dependencies.
        tx.claims[0].key = Bytes::from(vec![1u8; 5]);
        assert_eq!(tx.dependencies(), None);
        assert_eq!(tx.claim_dependencies(), None);
    }

    #[test]
    fn dependencies_are_ordered_as_the_tuple_of_their_fields() {
        let dependency =
            |verifier: u64, key: u8, msg: u8, gas_limit: u64, witness_len: u64| Dependency {
                verifier: Address::from_low_u64_be(verifier),
                key: H256::repeat_byte(key),
                msg: H256::repeat_byte(msg),
                gas_limit,
                witness_len,
            };
        let sorted = vec![
            dependency(1, 1, 1, 1, 1),
            dependency(1, 1, 1, 1, 2),
            dependency(1, 1, 1, 2, 1),
            dependency(1, 1, 2, 1, 1),
            dependency(1, 2, 1, 1, 1),
            dependency(2, 1, 1, 1, 1),
            // Addresses compare as byte strings, not as numbers.
            dependency(0x0100, 0, 0, 0, 0),
        ];
        let mut shuffled = sorted.clone();
        shuffled.reverse();
        shuffled.sort();
        assert_eq!(shuffled, sorted);
    }

    #[test]
    fn block_dependencies_merge_the_transactions_of_a_block() {
        // Two transactions that make the same explicit-msg claim share one
        // dependency; their claims over their own signature hashes do not.
        let shared = Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::from(vec![6u8; 32]),
            msg: Bytes::from(vec![9u8; 32]),
            ..Default::default()
        };
        let own = Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::from(vec![6u8; 32]),
            ..Default::default()
        };
        let mut first = make_test_frame_tx();
        first.claims = vec![shared.clone(), own.clone()];
        let mut second = make_test_frame_tx();
        second.nonce_seq += 1;
        second.claims = vec![own, shared];
        assert_ne!(first.compute_sig_hash(), second.compute_sig_hash());

        let transactions = vec![
            Transaction::FrameTransaction(first.clone()),
            Transaction::EIP1559Transaction(EIP1559Transaction::default()),
            Transaction::FrameTransaction(second.clone()),
        ];
        let dependencies = block_dependencies(&transactions).expect("well-formed claims");

        let mut expected: Vec<Dependency> = [
            first.dependencies().unwrap(),
            second.dependencies().unwrap(),
        ]
        .concat();
        expected.sort();
        expected.dedup();
        assert_eq!(dependencies.len(), 3);
        assert_eq!(dependencies, expected);

        // A malformed claim anywhere in the block leaves it without dependencies.
        let mut malformed = second;
        malformed.claims[0].msg = Bytes::from(vec![1u8; 3]);
        let transactions = vec![
            Transaction::FrameTransaction(first),
            Transaction::FrameTransaction(malformed),
        ];
        assert_eq!(block_dependencies(&transactions), None);
    }

    // ── Gas accounting (EIP-8141 §Gas Accounting) ──

    /// `max_gas` of `tx` computed term by term from the EIP's formulas, with
    /// EIP-8250's two additions. Returns `(frame_tx_intrinsic_gas,
    /// calldata_floor_gas, max_gas)`.
    fn gas_by_the_formulas(tx: &FrameTransaction) -> (u64, u64, u64) {
        let tokens_in = |data: &[u8]| {
            let zero_bytes = data.iter().filter(|byte| **byte == 0).count() as u64;
            zero_bytes + (data.len() as u64 - zero_bytes) * 4
        };
        let floor_tokens_in = |data: &[u8]| data.len() as u64 * 4;
        let calldata_cost = |data: &[u8]| 4 * tokens_in(data);
        let is_protocol = |claim: &Claim| {
            [SECP256K1_VERIFIER, P256_VERIFIER, LEANSPHINCS_VERIFIER].contains(&claim.verifier)
        };

        let claim_verification_cost: u64 = tx.claims.iter().map(Claim::resolved_gas).sum();
        let protocol_witness_cost: u64 = tx
            .claims
            .iter()
            .filter(|claim| is_protocol(claim))
            .map(|claim| WITNESS_BYTE_GAS * claim.resolved_witness_len())
            .sum();
        let custom_witness_cost: u64 = tx
            .claims
            .iter()
            .filter(|claim| !is_protocol(claim))
            .map(|claim| 4 * claim.witness_len * 4)
            .sum();
        let claim_metadata_cost: u64 = tx
            .claims
            .iter()
            .map(|claim| {
                calldata_cost(claim.verifier.as_bytes())
                    + calldata_cost(&claim.key)
                    + calldata_cost(&claim.msg)
            })
            .sum();
        let frame_data_cost: u64 = tx
            .frames
            .iter()
            .map(|frame| calldata_cost(&frame.data))
            .sum();
        let value_transfer_cost = tx.value_transfer_gas();
        // EIP-8250 prices the nonce fields as calldata and charges each keyed
        // nonce slot.
        let nonce_calldata = tx.nonce_calldata();
        let keyed_nonce_access_cost = tx.keyed_nonce_access_cost();

        let frame_tx_intrinsic_gas = FRAME_TX_INTRINSIC_COST
            + tx.frames.len() as u64 * FRAME_TX_PER_FRAME_COST
            + frame_data_cost
            + claim_metadata_cost
            + claim_verification_cost
            + protocol_witness_cost
            + custom_witness_cost
            + value_transfer_cost
            + calldata_cost(&nonce_calldata)
            + keyed_nonce_access_cost;
        let execution_limits: u64 = tx.frames.iter().map(|frame| frame.gas_limit).sum();
        let state_limits: u64 = tx.frames.iter().map(|frame| frame.state_gas_limit).sum();
        let standard_gas_limit = frame_tx_intrinsic_gas + execution_limits + state_limits;

        let calldata_floor_tokens: u64 = tx
            .frames
            .iter()
            .map(|frame| floor_tokens_in(&frame.data))
            .sum::<u64>()
            + tx.claims
                .iter()
                .map(|claim| {
                    floor_tokens_in(claim.verifier.as_bytes())
                        + floor_tokens_in(&claim.key)
                        + floor_tokens_in(&claim.msg)
                })
                .sum::<u64>()
            + tx.claims
                .iter()
                .filter(|claim| !is_protocol(claim))
                .map(|claim| claim.witness_len * 4)
                .sum::<u64>()
            + floor_tokens_in(&nonce_calldata);
        let calldata_floor_gas = FRAME_TX_INTRINSIC_COST
            + tx.frames.len() as u64 * FRAME_TX_PER_FRAME_COST
            + claim_verification_cost
            + protocol_witness_cost
            + value_transfer_cost
            + keyed_nonce_access_cost
            + 16 * calldata_floor_tokens;

        (
            frame_tx_intrinsic_gas,
            calldata_floor_gas,
            standard_gas_limit.max(calldata_floor_gas + state_limits),
        )
    }

    #[test]
    fn gas_follows_the_formulas_for_every_kind_of_claim() {
        let leansphincs = Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::from(vec![6u8; 32]),
            ..Default::default()
        };
        let secp256k1 = Claim {
            verifier: SECP256K1_VERIFIER,
            ..Default::default()
        };
        let p256 = Claim {
            verifier: P256_VERIFIER,
            key: Bytes::from(vec![5u8; 20]),
            msg: Bytes::from(vec![9u8; 32]),
            ..Default::default()
        };
        // An explicit msg that is mostly zero bytes, which the intrinsic cost
        // prices below the non-zero ones and the floor does not.
        let mut sparse_msg = vec![0u8; 32];
        sparse_msg[31] = 7;
        let cases: Vec<Vec<Claim>> = vec![
            vec![],
            vec![secp256k1.clone()],
            vec![p256.clone()],
            vec![leansphincs.clone()],
            vec![custom_claim(0xC0DE, 1, 2, 30_000, 500)],
            // A custom witness long enough for the calldata floor to bind.
            vec![custom_claim(0xC0DE, 1, 2, 30_000, 20_000)],
            vec![
                secp256k1,
                p256,
                leansphincs,
                Claim {
                    msg: Bytes::from(sparse_msg),
                    ..custom_claim(0xC0DE, 1, 2, 30_000, 500)
                },
            ],
        ];
        for claims in cases {
            let mut tx = make_test_frame_tx();
            tx.frames[1].state_gas_limit = 50_000;
            tx.frames[1].value = U256::one();
            tx.claims = claims;
            tx.validate_static_constraints().expect("statically valid");

            let (frame_tx_intrinsic_gas, calldata_floor_gas, max_gas) = gas_by_the_formulas(&tx);
            assert_eq!(
                tx.mandatory_gas() + tx.data_cost(),
                frame_tx_intrinsic_gas,
                "frame_tx_intrinsic_gas of {:?}",
                tx.claims
            );
            assert_eq!(
                tx.calldata_floor_total(),
                calldata_floor_gas,
                "calldata_floor_gas of {:?}",
                tx.claims
            );
            assert_eq!(tx.max_gas(), max_gas, "max_gas of {:?}", tx.claims);
        }
    }

    #[test]
    fn a_leansphincs_claim_costs_its_verification_and_four_gas_per_witness_byte() {
        let plain = make_test_frame_tx();
        let mut tx = make_test_frame_tx();
        tx.claims.push(Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::from(vec![6u8; 32]),
            ..Default::default()
        });
        // 13,200 to verify, 4 * 7,740 for the witness, and the claim's metadata:
        // a 20-byte verifier with one non-zero byte and a 32-byte key.
        let metadata = (19 * 4 + 16) + 32 * 16;
        assert_eq!(
            tx.max_gas() - plain.max_gas(),
            13_200 + 4 * 7_740 + metadata
        );
        // The witness is not counted toward the calldata floor: the floor grows
        // by the fixed costs and the metadata alone.
        assert_eq!(
            tx.calldata_floor_total() - plain.calldata_floor_total(),
            13_200 + 4 * 7_740 + 64 * (20 + 32)
        );
    }

    #[test]
    fn a_custom_witness_is_priced_as_non_zero_calldata_and_enters_the_floor() {
        let mut tx = make_test_frame_tx();
        tx.claims = vec![custom_claim(0xC0DE, 1, 2, 30_000, 0)];
        let without_witness = (
            tx.mandatory_gas() + tx.data_cost(),
            tx.calldata_floor_total(),
        );
        tx.claims[0].witness_len = 1_000;
        // 16 gas per byte in the intrinsic cost, 64 in the floor, whatever the
        // bytes turn out to be.
        assert_eq!(
            tx.mandatory_gas() + tx.data_cost() - without_witness.0,
            16 * 1_000
        );
        assert_eq!(tx.calldata_floor_total() - without_witness.1, 64 * 1_000);
        // The verification gas is the claim's whole limit.
        assert_eq!(tx.claim_verification_cost(), 30_000);
        assert_eq!(tx.protocol_witness_cost(), 0);
    }

    // ── The pooled representation and a block's claim witnesses (EIP-8141) ──

    fn test_blobs_bundle() -> BlobsBundle {
        BlobsBundle {
            blobs: vec![[0x33; crate::types::BYTES_PER_BLOB]],
            commitments: vec![[0x44; 48]],
            proofs: vec![[0x55; 48]; crate::types::CELLS_PER_EXT_BLOB],
            version: 1,
        }
    }

    fn pooled_frame_tx(blobs_bundle: Option<BlobsBundle>) -> PooledFrameTransaction {
        let mut tx = make_test_frame_tx();
        tx.claims.push(Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::from(vec![6u8; 32]),
            ..Default::default()
        });
        if blobs_bundle.is_some() {
            let mut versioned_hash = [0u8; 32];
            versioned_hash[0] = VERSIONED_HASH_VERSION_KZG;
            tx.blob_versioned_hashes = vec![H256(versioned_hash)];
        }
        // The witnesses are not signatures: these tests only cover the encodings.
        PooledFrameTransaction {
            tx,
            witnesses: vec![
                Bytes::from(vec![0x11; SECP256K1_WITNESS_SIZE]),
                Bytes::from(vec![0x22; LEANSPHINCS_SIGNATURE_SIZE]),
            ],
            blobs_bundle,
        }
    }

    #[test]
    fn pooled_frame_transaction_round_trips_with_and_without_blobs() {
        for blobs_bundle in [None, Some(test_blobs_bundle())] {
            let pooled = pooled_frame_tx(blobs_bundle);
            let encoded = pooled.encode_to_vec();
            assert!(PooledFrameTransaction::is_pooled(&encoded).expect("is a list"));
            let decoded = PooledFrameTransaction::decode(&encoded).expect("decodes");
            assert_eq!(decoded, pooled);
            assert_eq!(decoded.witness_bytes(), 65 + 7740);

            // The bare transaction is told apart by its first item.
            let bare = pooled.tx.encode_to_vec();
            assert!(!PooledFrameTransaction::is_pooled(&bare).expect("is a list"));
            assert!(PooledFrameTransaction::decode(&bare).is_err());
        }
    }

    #[test]
    fn pooled_frame_transaction_is_four_items_with_a_zero_wrapper_version() {
        let pooled = pooled_frame_tx(None);
        let encode = |version: u8, sidecar: &[u8], extra: bool| {
            let mut buf = Vec::new();
            let encoder = Encoder::new(&mut buf)
                .encode_field(&pooled.tx)
                .encode_field(&version)
                .encode_field(&pooled.witnesses)
                .encode_raw(sidecar);
            if extra {
                encoder.encode_field(&0u8).finish();
            } else {
                encoder.finish();
            }
            buf
        };
        // rlp([tx_body, 0, witnesses, []]) is the encoding itself.
        assert_eq!(encode(0, &[0xc0], false), pooled.encode_to_vec());

        // Any other wrapper version is unknown.
        assert!(PooledFrameTransaction::decode(&encode(1, &[0xc0], false)).is_err());
        // A fifth item, a sidecar that is not a list, and a sidecar of two items.
        assert!(PooledFrameTransaction::decode(&encode(0, &[0xc0], true)).is_err());
        assert!(PooledFrameTransaction::decode(&encode(0, &[0x80], false)).is_err());
        assert!(PooledFrameTransaction::decode(&encode(0, &[0xc2, 0xc0, 0xc0], false)).is_err());
        // Three items: the sidecar is missing.
        let mut three = Vec::new();
        Encoder::new(&mut three)
            .encode_field(&pooled.tx)
            .encode_field(&0u8)
            .encode_field(&pooled.witnesses)
            .finish();
        assert!(PooledFrameTransaction::decode(&three).is_err());
    }

    // EIP-8141 §Networking: the pooled representation has one witness for each
    // claim, each of the length its claim resolves to, so the claims say how
    // long the list of witnesses is.
    #[test]
    fn the_claims_bound_the_witnesses_of_the_pooled_representation() {
        let pooled = pooled_frame_tx(None);
        assert_eq!(
            pooled.tx.max_pooled_witnesses_len(),
            pooled.witnesses.length()
        );

        // Each protocol verifier, and custom witnesses of 300 bytes, one byte
        // and no bytes: the bound is the length of the list they make.
        let mut tx = pooled.tx;
        tx.claims = vec![
            Claim {
                verifier: SECP256K1_VERIFIER,
                ..Default::default()
            },
            Claim {
                verifier: P256_VERIFIER,
                ..Default::default()
            },
            Claim {
                verifier: LEANSPHINCS_VERIFIER,
                key: Bytes::from(vec![6u8; 32]),
                ..Default::default()
            },
            custom_claim(0x1234, 1, 2, 5_000, 300),
            custom_claim(0x1234, 1, 2, 5_000, 1),
            custom_claim(0x1234, 1, 2, 5_000, 0),
        ];
        let witnesses: Vec<Bytes> = [65usize, 128, 7740, 300, 1, 0]
            .into_iter()
            .map(|len| Bytes::from(vec![0xff; len]))
            .collect();
        assert_eq!(tx.max_pooled_witnesses_len(), witnesses.length());

        // A transaction nobody validated may declare more claims than MAX_CLAIMS
        // and longer witnesses than MAX_CLAIM_WITNESS_BYTES. The bound stays at
        // MAX_CLAIMS witnesses of MAX_CLAIM_WITNESS_BYTES: 64 strings of
        // 1,048,576 bytes with a 4-byte header each, in a list with a 5-byte one.
        tx.claims = vec![custom_claim(0x1234, 1, 2, 5_000, u64::MAX); MAX_CLAIMS + 10];
        assert_eq!(tx.max_pooled_witnesses_len(), 64 * (1_048_576 + 4) + 5);

        tx.claims.clear();
        assert_eq!(tx.max_pooled_witnesses_len(), 1);
    }

    // A list of witnesses longer than the claims allow is rejected when the
    // pooled representation is decoded. A shorter one decodes, and the pool says
    // which witness is missing or short.
    #[test]
    fn a_pooled_representation_with_more_witness_bytes_than_its_claims_allow_does_not_decode() {
        let pooled = pooled_frame_tx(None);

        let mut one_more = pooled.clone();
        one_more.witnesses.push(Bytes::new());
        let mut longer = pooled.clone();
        longer.witnesses[1] = Bytes::from(vec![0x22; LEANSPHINCS_SIGNATURE_SIZE + 1]);
        for oversized in [one_more, longer] {
            let error = PooledFrameTransaction::decode(&oversized.encode_to_vec())
                .expect_err("more witness bytes than the claims allow");
            assert!(error.to_string().contains("witnesses take"), "{error}");
        }

        let mut shorter = pooled;
        shorter.witnesses.pop();
        let decoded = PooledFrameTransaction::decode(&shorter.encode_to_vec()).expect("decodes");
        assert_eq!(decoded, shorter);
    }

    // The pooled representation as a whole is capped at the cap of a
    // transaction, a node policy, plus the witnesses its claims fix.
    #[test]
    fn the_pooled_representation_is_capped_at_the_transaction_cap_plus_its_witnesses() {
        for blobs_bundle in [None, Some(test_blobs_bundle())] {
            let transaction_cap = if blobs_bundle.is_some() {
                crate::types::MAX_BLOB_TX_SIZE
            } else {
                crate::types::MAX_TX_SIZE
            };
            let pooled = pooled_frame_tx(blobs_bundle);
            assert_eq!(pooled.wire_len(), 1 + pooled.encode_to_vec().len());
            assert_eq!(
                pooled.max_wire_len(),
                transaction_cap + pooled.witnesses.length() + 16
            );
            assert!(pooled.wire_len() <= pooled.max_wire_len());
        }
    }

    #[test]
    fn claim_witnesses_are_empty_or_a_version_byte_and_a_list() {
        assert_eq!(encode_claim_witnesses(&[]), Bytes::new());
        assert_eq!(decode_claim_witnesses(&[]), Ok(Vec::new()));
        assert_eq!(claim_witnesses_len([]), 0);

        let witnesses = vec![
            Bytes::from(vec![0xAB; LEANSPHINCS_SIGNATURE_SIZE]),
            Bytes::from(vec![0xCD; SECP256K1_WITNESS_SIZE]),
            // A one-byte witness below 0x80 is its own encoding, and an empty
            // witness is a single byte too.
            Bytes::from(vec![0x05]),
            Bytes::new(),
        ];
        let encoded = encode_claim_witnesses(&witnesses);
        assert_eq!(encoded[0], CLAIM_WITNESSES_VERSION_RAW);
        assert_eq!(&encoded[1..], witnesses.encode_to_vec().as_slice());
        assert_eq!(decode_claim_witnesses(&encoded), Ok(witnesses.clone()));
        assert_eq!(claim_witnesses_len(&witnesses), encoded.len());
    }

    #[test]
    fn malformed_claim_witnesses_do_not_decode() {
        let witnesses = vec![Bytes::from(vec![0xAB; 65])];
        let encoded = encode_claim_witnesses(&witnesses).to_vec();

        // Another version byte.
        let mut other_version = encoded.clone();
        other_version[0] = 0x01;
        assert_eq!(
            decode_claim_witnesses(&other_version),
            Err(ClaimWitnessesDecodeError::UnknownVersion(0x01))
        );
        // The version byte alone, and the version byte with an empty list: no
        // witnesses is the empty byte string and nothing else.
        assert!(matches!(
            decode_claim_witnesses(&[0x00]),
            Err(ClaimWitnessesDecodeError::Malformed(_))
        ));
        assert_eq!(
            decode_claim_witnesses(&[0x00, 0xc0]),
            Err(ClaimWitnessesDecodeError::EmptyList)
        );
        // Bytes after the list, a truncated list, and a list item that is itself
        // a list.
        let mut trailing = encoded.clone();
        trailing.push(0x00);
        assert!(matches!(
            decode_claim_witnesses(&trailing),
            Err(ClaimWitnessesDecodeError::Malformed(_))
        ));
        assert!(matches!(
            decode_claim_witnesses(&encoded[..encoded.len() - 1]),
            Err(ClaimWitnessesDecodeError::Malformed(_))
        ));
        assert!(matches!(
            decode_claim_witnesses(&[0x00, 0xc1, 0xc0]),
            Err(ClaimWitnessesDecodeError::Malformed(_))
        ));
        // A string instead of a list.
        assert!(matches!(
            decode_claim_witnesses(&[0x00, 0x82, 0x01, 0x02]),
            Err(ClaimWitnessesDecodeError::Malformed(_))
        ));
    }

    #[test]
    fn claim_witnesses_are_bounded_by_the_block_limit() {
        // Two witnesses of the largest size a claim may declare: with the
        // version byte and the RLP headers they are over the block limit, and
        // one byte less in one of them is still over it.
        let witness = Bytes::from(vec![0xEE; MAX_CLAIM_WITNESS_BYTES as usize]);
        let over = encode_claim_witnesses(&[witness.clone(), witness]);
        assert!(over.len() > MAX_BLOCK_WITNESS_BYTES);
        assert_eq!(
            decode_claim_witnesses(&over),
            Err(ClaimWitnessesDecodeError::TooLong(over.len()))
        );

        // Exactly at the limit: one witness whose encoding fills it. The byte
        // string is the version byte, a 4-byte list header, a 4-byte string
        // header and the witness.
        let witness = Bytes::from(vec![0xEE; MAX_BLOCK_WITNESS_BYTES - 9]);
        let at_limit = encode_claim_witnesses(std::slice::from_ref(&witness));
        assert_eq!(at_limit.len(), MAX_BLOCK_WITNESS_BYTES);
        assert_eq!(decode_claim_witnesses(&at_limit), Ok(vec![witness]));
    }

    // From IStar one aggregate proof stands for the witnesses of the leanSPHINCS
    // dependencies (EIP-8141 §Aggregation and EIP-8288 compatibility): only
    // those are aggregated, and only from that fork. LStar, the fork after it,
    // keeps the rule.
    #[test]
    fn leansphincs_dependencies_are_aggregated_from_istar() {
        use crate::types::Fork;
        let dependency = |verifier: Address| Dependency {
            verifier,
            key: H256::repeat_byte(0x01),
            msg: H256::repeat_byte(0x02),
            gas_limit: 0,
            witness_len: 0,
        };
        for fork in [Fork::Amsterdam, Fork::Hegota] {
            for verifier in [SECP256K1_VERIFIER, P256_VERIFIER, LEANSPHINCS_VERIFIER] {
                assert!(!dependency(verifier).is_aggregated(fork));
            }
        }
        for fork in [Fork::IStar, Fork::LStar] {
            assert!(dependency(LEANSPHINCS_VERIFIER).is_aggregated(fork));
            for verifier in [
                SECP256K1_VERIFIER,
                P256_VERIFIER,
                Address::repeat_byte(0x7E),
            ] {
                assert!(!dependency(verifier).is_aggregated(fork));
            }
        }
    }

    #[test]
    fn aggregated_claim_witnesses_are_empty_or_a_version_byte_a_proof_and_a_list() {
        use crate::types::Fork;
        // A block without dependencies has neither a proof nor a witness.
        assert_eq!(encode_aggregated_claim_witnesses(&[], &[]), Bytes::new());
        assert_eq!(
            decode_claim_witnesses_at(Fork::IStar, &[]),
            Ok(ClaimWitnesses::default())
        );
        assert_eq!(aggregated_claim_witnesses_len(0, 0), 0);

        let proof = Bytes::from(vec![0x5A; 200_000]);
        let witnesses = vec![
            Bytes::from(vec![0xCD; SECP256K1_WITNESS_SIZE]),
            Bytes::from(vec![0x05]),
            Bytes::new(),
        ];
        // A proof and witnesses, a proof alone, and witnesses alone.
        for (proof, witnesses) in [
            (proof.clone(), witnesses.clone()),
            (proof, Vec::new()),
            (Bytes::new(), witnesses),
        ] {
            let encoded = encode_aggregated_claim_witnesses(&proof, &witnesses);
            assert_eq!(encoded[0], CLAIM_WITNESSES_VERSION_AGGREGATED);
            assert_eq!(
                &encoded[1..],
                (proof.clone(), witnesses.clone())
                    .encode_to_vec()
                    .as_slice()
            );
            assert_eq!(
                decode_claim_witnesses_at(Fork::IStar, &encoded),
                Ok(ClaimWitnesses {
                    aggregate_proof: proof.clone(),
                    witnesses: witnesses.clone(),
                })
            );
            assert_eq!(
                aggregated_claim_witnesses_len(
                    proof.len(),
                    witnesses.iter().map(Bytes::length).sum()
                ),
                encoded.len()
            );
        }

        // A proof and no witness encodes its empty list, and the other way round.
        assert_eq!(
            encode_aggregated_claim_witnesses(&[0xAA, 0xBB], &[]).as_ref(),
            [0x01, 0xc4, 0x82, 0xAA, 0xBB, 0xc0]
        );
        assert_eq!(
            encode_aggregated_claim_witnesses(&[], &[Bytes::from(vec![0xAA, 0xBB])]).as_ref(),
            [0x01, 0xc5, 0x80, 0xc3, 0x82, 0xAA, 0xBB]
        );
    }

    // Each fork defines one version byte: raw witnesses before IStar, and the
    // aggregate proof with the remaining raw witnesses from IStar on, at LStar
    // as at IStar.
    #[test]
    fn claim_witnesses_decode_only_under_the_version_of_their_fork() {
        use crate::types::Fork;
        let witnesses = vec![Bytes::from(vec![0xAB; 65])];
        let raw = encode_claim_witnesses(&witnesses);
        let aggregated = encode_aggregated_claim_witnesses(&[0x5A; 300], &witnesses);

        assert_eq!(
            decode_claim_witnesses_at(Fork::Hegota, &raw).map(|decoded| decoded.witnesses),
            Ok(witnesses.clone())
        );
        for fork in [Fork::IStar, Fork::LStar] {
            assert_eq!(
                decode_claim_witnesses_at(fork, &aggregated).map(|decoded| decoded.witnesses),
                Ok(witnesses.clone())
            );
            // The raw form in a block from IStar on.
            assert_eq!(
                decode_claim_witnesses_at(fork, &raw),
                Err(ClaimWitnessesDecodeError::UnknownVersion(0x00))
            );
        }
        // The aggregated form before IStar.
        assert_eq!(
            decode_claim_witnesses_at(Fork::Hegota, &aggregated),
            Err(ClaimWitnessesDecodeError::UnknownVersion(0x01))
        );
        // A version byte no fork defines.
        let mut unknown = aggregated.to_vec();
        unknown[0] = 0x02;
        for fork in [Fork::Hegota, Fork::IStar, Fork::LStar] {
            assert_eq!(
                decode_claim_witnesses_at(fork, &unknown),
                Err(ClaimWitnessesDecodeError::UnknownVersion(0x02))
            );
        }
    }

    #[test]
    fn malformed_aggregated_claim_witnesses_do_not_decode() {
        use crate::types::Fork;
        let decode = |bytes: &[u8]| decode_claim_witnesses_at(Fork::IStar, bytes);
        let encoded =
            encode_aggregated_claim_witnesses(&[0x5A; 300], &[Bytes::from(vec![0xAB; 65])])
                .to_vec();

        // The version byte alone.
        assert!(matches!(
            decode(&[0x01]),
            Err(ClaimWitnessesDecodeError::Malformed(_))
        ));
        // An empty proof with an empty list: a block without dependencies has
        // the empty byte string as its only encoding.
        assert_eq!(
            decode(&[0x01, 0xc2, 0x80, 0xc0]),
            Err(ClaimWitnessesDecodeError::NothingCarried)
        );
        // Bytes after the list, and a truncated list.
        let mut trailing = encoded.clone();
        trailing.push(0x00);
        assert!(matches!(
            decode(&trailing),
            Err(ClaimWitnessesDecodeError::Malformed(_))
        ));
        assert!(matches!(
            decode(&encoded[..encoded.len() - 1]),
            Err(ClaimWitnessesDecodeError::Malformed(_))
        ));
        for malformed in [
            // The list of raw witnesses of version 0x00 under the new byte.
            &[0x01, 0xc3, 0x82, 0xAA, 0xBB][..],
            // A proof alone, without the list of witnesses.
            &[0x01, 0xc1, 0x80],
            // A third item.
            &[0x01, 0xc3, 0x80, 0xc0, 0x80],
            // A proof that is a list, and witnesses that are a byte string.
            &[0x01, 0xc2, 0xc0, 0xc0],
            &[0x01, 0xc2, 0x80, 0x80],
            // A witness that is itself a list.
            &[0x01, 0xc3, 0x80, 0xc1, 0xc0],
            // A byte string instead of the outer list.
            &[0x01, 0x82, 0x01, 0x02],
        ] {
            assert!(
                matches!(
                    decode(malformed),
                    Err(ClaimWitnessesDecodeError::Malformed(_))
                ),
                "{malformed:02x?} decoded as {:?}",
                decode(malformed)
            );
        }
    }

    // EIP-8288 `MAX_DEPENDENCY_PROOF_BYTES` bounds the aggregate proof, and
    // `MAX_BLOCK_WITNESS_BYTES` still bounds the whole byte string.
    #[test]
    fn aggregated_claim_witnesses_are_bounded_by_the_proof_and_block_limits() {
        use crate::types::Fork;
        let proof_at_limit = vec![0x5A; MAX_AGGREGATE_PROOF_BYTES];
        let encoded = encode_aggregated_claim_witnesses(&proof_at_limit, &[]);
        assert_eq!(
            decode_claim_witnesses_at(Fork::IStar, &encoded).map(|decoded| decoded.aggregate_proof),
            Ok(Bytes::from(proof_at_limit))
        );
        let encoded =
            encode_aggregated_claim_witnesses(&[0x5A; MAX_AGGREGATE_PROOF_BYTES + 1], &[]);
        assert!(encoded.len() <= MAX_BLOCK_WITNESS_BYTES);
        assert_eq!(
            decode_claim_witnesses_at(Fork::IStar, &encoded),
            Err(ClaimWitnessesDecodeError::AggregateProofTooLong(
                MAX_AGGREGATE_PROOF_BYTES + 1
            ))
        );

        // A proof within its own limit beside raw witnesses that take the byte
        // string over the block's.
        let witness = Bytes::from(vec![0xEE; MAX_CLAIM_WITNESS_BYTES as usize]);
        let over = encode_aggregated_claim_witnesses(&[0x5A; 300], &[witness.clone(), witness]);
        assert!(over.len() > MAX_BLOCK_WITNESS_BYTES);
        assert_eq!(
            decode_claim_witnesses_at(Fork::IStar, &over),
            Err(ClaimWitnessesDecodeError::TooLong(over.len()))
        );
    }
}
