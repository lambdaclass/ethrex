//! Protobuf message definitions for the CometBFT `0x67` light-block validation
//! precompile.
//!
//! These mirror **greenfield-cometbft v1.3.2** (the fork bsc-geth vendors), not
//! vanilla cometbft. The fork adds three consensus-relevant fields that change
//! hashing/decoding: `Header.randao_mix` (15), `Validator.bls_key` (5) /
//! `relayer_address` (6), and `SimpleValidator.bls_key` (3) / `relayer_address`
//! (4). `CanonicalVote`/`CanonicalBlockID` are unforked.
//!
//! Field numbers and wire types are taken verbatim from the fork's
//! `proto/tendermint/types/{types,canonical,validator}.proto`,
//! `proto/tendermint/version/types.proto`, `crypto/keys.proto`, and the WKT
//! `google.protobuf.Timestamp`.

use prost::Message;

/// `google.protobuf.Timestamp` (well-known type).
#[derive(Clone, PartialEq, Message)]
pub struct Timestamp {
    #[prost(int64, tag = "1")]
    pub seconds: i64,
    #[prost(int32, tag = "2")]
    pub nanos: i32,
}

/// `tendermint.version.Consensus`.
#[derive(Clone, PartialEq, Message)]
pub struct Consensus {
    #[prost(uint64, tag = "1")]
    pub block: u64,
    #[prost(uint64, tag = "2")]
    pub app: u64,
}

/// `tendermint.crypto.PublicKey` — only the ed25519 oneof arm (field 1) is
/// modelled; BSC validator keys are always ed25519. Encoding a `Some` at tag 1
/// reproduces the oneof `ed25519` wire bytes exactly.
#[derive(Clone, PartialEq, Message)]
pub struct PublicKey {
    #[prost(bytes = "vec", optional, tag = "1")]
    pub ed25519: Option<Vec<u8>>,
}

/// `tendermint.types.PartSetHeader`.
#[derive(Clone, PartialEq, Message)]
pub struct PartSetHeader {
    #[prost(uint32, tag = "1")]
    pub total: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
}

/// `tendermint.types.BlockID`.
#[derive(Clone, PartialEq, Message)]
pub struct BlockId {
    #[prost(bytes = "vec", tag = "1")]
    pub hash: Vec<u8>,
    #[prost(message, optional, tag = "2")]
    pub part_set_header: Option<PartSetHeader>,
}

/// `tendermint.types.Header` (greenfield fork — includes `randao_mix` at 15).
#[derive(Clone, PartialEq, Message)]
pub struct Header {
    #[prost(message, optional, tag = "1")]
    pub version: Option<Consensus>,
    #[prost(string, tag = "2")]
    pub chain_id: String,
    #[prost(int64, tag = "3")]
    pub height: i64,
    #[prost(message, optional, tag = "4")]
    pub time: Option<Timestamp>,
    #[prost(message, optional, tag = "5")]
    pub last_block_id: Option<BlockId>,
    #[prost(bytes = "vec", tag = "6")]
    pub last_commit_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "7")]
    pub data_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "8")]
    pub validators_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "9")]
    pub next_validators_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "10")]
    pub consensus_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "11")]
    pub app_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "12")]
    pub last_results_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "13")]
    pub evidence_hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "14")]
    pub proposer_address: Vec<u8>,
    #[prost(bytes = "vec", tag = "15")]
    pub randao_mix: Vec<u8>,
}

/// `tendermint.types.CommitSig`. `block_id_flag` is the `BlockIDFlag` enum
/// (absent = 1, commit = 2, nil = 3) encoded as a varint.
#[derive(Clone, PartialEq, Message)]
pub struct CommitSig {
    #[prost(int32, tag = "1")]
    pub block_id_flag: i32,
    #[prost(bytes = "vec", tag = "2")]
    pub validator_address: Vec<u8>,
    #[prost(message, optional, tag = "3")]
    pub timestamp: Option<Timestamp>,
    #[prost(bytes = "vec", tag = "4")]
    pub signature: Vec<u8>,
}

/// `tendermint.types.Commit`.
#[derive(Clone, PartialEq, Message)]
pub struct Commit {
    #[prost(int64, tag = "1")]
    pub height: i64,
    #[prost(int32, tag = "2")]
    pub round: i32,
    #[prost(message, optional, tag = "3")]
    pub block_id: Option<BlockId>,
    #[prost(message, repeated, tag = "4")]
    pub signatures: Vec<CommitSig>,
}

/// `tendermint.types.SignedHeader`.
#[derive(Clone, PartialEq, Message)]
pub struct SignedHeader {
    #[prost(message, optional, tag = "1")]
    pub header: Option<Header>,
    #[prost(message, optional, tag = "2")]
    pub commit: Option<Commit>,
}

/// `tendermint.types.Validator` (greenfield fork — `bls_key` (5),
/// `relayer_address` (6)).
#[derive(Clone, PartialEq, Message)]
pub struct Validator {
    #[prost(bytes = "vec", tag = "1")]
    pub address: Vec<u8>,
    #[prost(message, optional, tag = "2")]
    pub pub_key: Option<PublicKey>,
    #[prost(int64, tag = "3")]
    pub voting_power: i64,
    #[prost(int64, tag = "4")]
    pub proposer_priority: i64,
    #[prost(bytes = "vec", tag = "5")]
    pub bls_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "6")]
    pub relayer_address: Vec<u8>,
}

/// `tendermint.types.ValidatorSet`.
#[derive(Clone, PartialEq, Message)]
pub struct ValidatorSet {
    #[prost(message, repeated, tag = "1")]
    pub validators: Vec<Validator>,
    #[prost(message, optional, tag = "2")]
    pub proposer: Option<Validator>,
    #[prost(int64, tag = "3")]
    pub total_voting_power: i64,
}

/// `tendermint.types.LightBlock`.
#[derive(Clone, PartialEq, Message)]
pub struct LightBlock {
    #[prost(message, optional, tag = "1")]
    pub signed_header: Option<SignedHeader>,
    #[prost(message, optional, tag = "2")]
    pub validator_set: Option<ValidatorSet>,
}

/// `tendermint.types.SimpleValidator` (greenfield fork — the validator-set
/// Merkle **leaf**; includes `bls_key` (3) / `relayer_address` (4)).
#[derive(Clone, PartialEq, Message)]
pub struct SimpleValidator {
    #[prost(message, optional, tag = "1")]
    pub pub_key: Option<PublicKey>,
    #[prost(int64, tag = "2")]
    pub voting_power: i64,
    #[prost(bytes = "vec", tag = "3")]
    pub bls_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub relayer_address: Vec<u8>,
}

// ── Canonical vote (sign-bytes) — unforked, identical to upstream cometbft ────

/// `tendermint.types.CanonicalPartSetHeader`.
#[derive(Clone, PartialEq, Message)]
pub struct CanonicalPartSetHeader {
    #[prost(uint32, tag = "1")]
    pub total: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
}

/// `tendermint.types.CanonicalBlockID`.
#[derive(Clone, PartialEq, Message)]
pub struct CanonicalBlockId {
    #[prost(bytes = "vec", tag = "1")]
    pub hash: Vec<u8>,
    #[prost(message, optional, tag = "2")]
    pub part_set_header: Option<CanonicalPartSetHeader>,
}

/// `tendermint.types.CanonicalVote`. `height`/`round` are `sfixed64`
/// (8-byte little-endian). `block_id` is omitted entirely for a nil vote.
#[derive(Clone, PartialEq, Message)]
pub struct CanonicalVote {
    /// `SignedMsgType` (PrecommitType = 2), varint.
    #[prost(int32, tag = "1")]
    pub r#type: i32,
    #[prost(sfixed64, tag = "2")]
    pub height: i64,
    #[prost(sfixed64, tag = "3")]
    pub round: i64,
    #[prost(message, optional, tag = "4")]
    pub block_id: Option<CanonicalBlockId>,
    #[prost(message, optional, tag = "5")]
    pub timestamp: Option<Timestamp>,
    #[prost(string, tag = "6")]
    pub chain_id: String,
}

// ── gogoproto well-known wrappers, used by `cdcEncode` in the header hash ──────

/// `google.protobuf.StringValue`.
#[derive(Clone, PartialEq, Message)]
pub struct StringValue {
    #[prost(string, tag = "1")]
    pub value: String,
}

/// `google.protobuf.Int64Value`.
#[derive(Clone, PartialEq, Message)]
pub struct Int64Value {
    #[prost(int64, tag = "1")]
    pub value: i64,
}

/// `google.protobuf.BytesValue`.
#[derive(Clone, PartialEq, Message)]
pub struct BytesValue {
    #[prost(bytes = "vec", tag = "1")]
    pub value: Vec<u8>,
}

/// `BlockIDFlag` values (`tendermint.types`).
pub const BLOCK_ID_FLAG_COMMIT: i32 = 2;

/// `SignedMsgType::Precommit`.
pub const PRECOMMIT_TYPE: i32 = 2;

/// Marshal a message to its proto3 bytes.
pub fn marshal<M: Message>(m: &M) -> Vec<u8> {
    m.encode_to_vec()
}
