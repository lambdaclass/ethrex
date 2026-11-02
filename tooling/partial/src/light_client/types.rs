//! Consensus-layer containers the light client reads: SSZ for roots, JSON for the beacon
//! API. Only the fields the sync protocol hashes or checks are modelled.
//!
//! `BeaconBlockHeader`, `SyncCommittee` and `SyncAggregate` are the same containers as
//! ethlambda's `beacon::containers` (`shared.rs`, `altair.rs`), spelled out here because
//! ethlambda pins `libssz` to a git revision that differs from the release ethrex builds
//! on, so its types cannot be linked without carrying a second copy of the SSZ library.
//! The light client containers themselves exist in neither client.

use ethrex_common::H256;
use ethrex_common::types::stateless_ssz::Bytes20;
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha2Hasher};
use libssz_types::{SszList, SszVector};
use serde::{Deserialize, Deserializer};

/// Validators in a sync committee.
pub const SYNC_COMMITTEE_SIZE: usize = 512;

/// A BLS public key, compressed.
pub type BlsPubkey = [u8; 48];
/// A BLS signature, compressed.
pub type BlsSignature = [u8; 96];

/// `hash_tree_root` with the default hasher.
pub fn root_of<T: HashTreeRoot>(value: &T) -> H256 {
    H256(value.hash_tree_root(&Sha2Hasher))
}

/// `BeaconBlockHeader`.
#[derive(Clone, Debug, Default, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot, Deserialize)]
pub struct BeaconBlockHeader {
    /// Slot.
    #[serde(deserialize_with = "de_u64")]
    pub slot: u64,
    /// Proposer index.
    #[serde(deserialize_with = "de_u64")]
    pub proposer_index: u64,
    /// Parent block root.
    #[serde(deserialize_with = "de_bytes32")]
    pub parent_root: [u8; 32],
    /// Post-state root.
    #[serde(deserialize_with = "de_bytes32")]
    pub state_root: [u8; 32],
    /// Body root.
    #[serde(deserialize_with = "de_bytes32")]
    pub body_root: [u8; 32],
}

impl BeaconBlockHeader {
    /// The block root.
    pub fn root(&self) -> H256 {
        root_of(self)
    }
}

/// `SyncCommittee`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SyncCommittee {
    /// The 512 member public keys.
    #[serde(deserialize_with = "de_pubkeys")]
    pub pubkeys: Vec<BlsPubkey>,
    /// Their aggregate.
    #[serde(deserialize_with = "de_bytes48")]
    pub aggregate_pubkey: BlsPubkey,
}

#[derive(SszEncode, SszDecode, HashTreeRoot)]
struct SyncCommitteeSsz {
    pubkeys: SszVector<BlsPubkey, SYNC_COMMITTEE_SIZE>,
    aggregate_pubkey: BlsPubkey,
}

impl SyncCommittee {
    /// `hash_tree_root`. A committee of the wrong size has no root.
    pub fn root(&self) -> Option<H256> {
        let pubkeys: SszVector<BlsPubkey, SYNC_COMMITTEE_SIZE> =
            self.pubkeys.clone().try_into().ok()?;
        let ssz = SyncCommitteeSsz {
            pubkeys,
            aggregate_pubkey: self.aggregate_pubkey,
        };
        Some(root_of(&ssz))
    }
}

/// `SyncAggregate`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SyncAggregate {
    /// Participation bits, `Bitvector[512]`, as 64 little-endian bytes.
    #[serde(deserialize_with = "de_bits512")]
    pub sync_committee_bits: Vec<u8>,
    /// The aggregate signature.
    #[serde(deserialize_with = "de_bytes96")]
    pub sync_committee_signature: BlsSignature,
}

impl SyncAggregate {
    /// Whether member `i` signed.
    pub fn signed(&self, i: usize) -> bool {
        self.sync_committee_bits
            .get(i / 8)
            .is_some_and(|byte| byte & (1 << (i % 8)) != 0)
    }

    /// Number of signers.
    pub fn participants(&self) -> usize {
        self.sync_committee_bits
            .iter()
            .map(|b| b.count_ones() as usize)
            .sum()
    }
}

/// The 17-field `ExecutionPayloadHeader` of Deneb through Fulu, for pre-Gloas light client
/// headers. Only its root is used.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ExecutionPayloadHeader {
    #[serde(deserialize_with = "de_bytes32")]
    parent_hash: [u8; 32],
    #[serde(deserialize_with = "de_bytes20")]
    fee_recipient: [u8; 20],
    #[serde(deserialize_with = "de_bytes32")]
    state_root: [u8; 32],
    #[serde(deserialize_with = "de_bytes32")]
    receipts_root: [u8; 32],
    #[serde(deserialize_with = "de_hex_vec")]
    logs_bloom: Vec<u8>,
    #[serde(deserialize_with = "de_bytes32")]
    prev_randao: [u8; 32],
    #[serde(deserialize_with = "de_u64")]
    block_number: u64,
    #[serde(deserialize_with = "de_u64")]
    gas_limit: u64,
    #[serde(deserialize_with = "de_u64")]
    gas_used: u64,
    #[serde(deserialize_with = "de_u64")]
    timestamp: u64,
    #[serde(deserialize_with = "de_hex_vec")]
    extra_data: Vec<u8>,
    #[serde(deserialize_with = "de_u256_le")]
    base_fee_per_gas: [u8; 32],
    /// The execution block hash.
    #[serde(deserialize_with = "de_bytes32")]
    pub block_hash: [u8; 32],
    #[serde(deserialize_with = "de_bytes32")]
    transactions_root: [u8; 32],
    #[serde(deserialize_with = "de_bytes32")]
    withdrawals_root: [u8; 32],
    #[serde(deserialize_with = "de_u64")]
    blob_gas_used: u64,
    #[serde(deserialize_with = "de_u64")]
    excess_blob_gas: u64,
}

#[derive(SszEncode, SszDecode, HashTreeRoot)]
struct ExecutionPayloadHeaderSsz {
    parent_hash: [u8; 32],
    fee_recipient: Bytes20,
    state_root: [u8; 32],
    receipts_root: [u8; 32],
    logs_bloom: SszVector<u8, 256>,
    prev_randao: [u8; 32],
    block_number: u64,
    gas_limit: u64,
    gas_used: u64,
    timestamp: u64,
    extra_data: SszList<u8, 32>,
    base_fee_per_gas: [u8; 32],
    block_hash: [u8; 32],
    transactions_root: [u8; 32],
    withdrawals_root: [u8; 32],
    blob_gas_used: u64,
    excess_blob_gas: u64,
}

impl ExecutionPayloadHeader {
    /// `hash_tree_root`, or `None` if a bounded field is out of bounds.
    pub fn root(&self) -> Option<H256> {
        let ssz = ExecutionPayloadHeaderSsz {
            parent_hash: self.parent_hash,
            fee_recipient: Bytes20(self.fee_recipient),
            state_root: self.state_root,
            receipts_root: self.receipts_root,
            logs_bloom: self.logs_bloom.clone().try_into().ok()?,
            prev_randao: self.prev_randao,
            block_number: self.block_number,
            gas_limit: self.gas_limit,
            gas_used: self.gas_used,
            timestamp: self.timestamp,
            extra_data: self.extra_data.clone().try_into().ok()?,
            base_fee_per_gas: self.base_fee_per_gas,
            block_hash: self.block_hash,
            transactions_root: self.transactions_root,
            withdrawals_root: self.withdrawals_root,
            blob_gas_used: self.blob_gas_used,
            excess_blob_gas: self.excess_blob_gas,
        };
        Some(root_of(&ssz))
    }
}

/// `LightClientHeader`, in its pre-Gloas shape (`execution` header) or its Gloas shape
/// (`execution_block_hash`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LightClientHeader {
    /// The beacon block header.
    pub beacon: BeaconBlockHeader,
    /// Deneb..Fulu: the execution payload header.
    #[serde(default)]
    pub execution: Option<ExecutionPayloadHeader>,
    /// Gloas: the latest applied execution block hash.
    #[serde(default, deserialize_with = "de_opt_bytes32")]
    pub execution_block_hash: Option<[u8; 32]>,
    /// Merkle branch for the execution data within the body.
    #[serde(default, deserialize_with = "de_bytes32_vec")]
    pub execution_branch: Vec<[u8; 32]>,
}

impl LightClientHeader {
    /// The execution block hash the header carries, whichever shape it has.
    pub fn execution_block_hash(&self) -> Option<H256> {
        self.execution_block_hash
            .map(H256)
            .or_else(|| self.execution.as_ref().map(|e| H256(e.block_hash)))
    }
}

/// `LightClientBootstrap`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LightClientBootstrap {
    /// The trusted block's header.
    pub header: LightClientHeader,
    /// The sync committee of its period.
    pub current_sync_committee: SyncCommittee,
    /// Branch for the committee within the state.
    #[serde(deserialize_with = "de_bytes32_vec")]
    pub current_sync_committee_branch: Vec<[u8; 32]>,
}

/// `LightClientUpdate`. Finality and optimistic updates are the same shape with some
/// fields absent.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LightClientUpdate {
    /// Header the sync committee attested to.
    pub attested_header: LightClientHeader,
    /// The next period's committee, if the update carries one.
    #[serde(default)]
    pub next_sync_committee: Option<SyncCommittee>,
    /// Branch for `next_sync_committee` within the attested state.
    #[serde(default, deserialize_with = "de_bytes32_vec")]
    pub next_sync_committee_branch: Vec<[u8; 32]>,
    /// The finalized header the attested state points to, if carried.
    #[serde(default)]
    pub finalized_header: Option<LightClientHeader>,
    /// Branch for the finalized root within the attested state.
    #[serde(default, deserialize_with = "de_bytes32_vec")]
    pub finality_branch: Vec<[u8; 32]>,
    /// The attestation.
    pub sync_aggregate: SyncAggregate,
    /// Slot the attestation was made in.
    #[serde(deserialize_with = "de_u64")]
    pub signature_slot: u64,
}

/// `{"version": .., "data": ..}` as the beacon API wraps light client objects.
#[derive(Clone, Debug, Deserialize)]
pub struct Versioned<T> {
    /// The fork name, lowercase.
    #[serde(default)]
    pub version: Option<String>,
    /// The object.
    pub data: T,
}

/// The reply of `GET /eth/v1/beacon/headers/{id}`.
#[derive(Clone, Debug, Deserialize)]
pub struct HeaderResponse {
    /// The envelope.
    pub data: HeaderData,
}

/// One entry of a headers reply.
#[derive(Clone, Debug, Deserialize)]
pub struct HeaderData {
    /// The block root.
    #[serde(deserialize_with = "de_bytes32")]
    pub root: [u8; 32],
    /// The signed header.
    pub header: SignedHeaderData,
}

/// A signed beacon block header, signature ignored.
#[derive(Clone, Debug, Deserialize)]
pub struct SignedHeaderData {
    /// The header.
    pub message: BeaconBlockHeader,
}

/// The reply of `GET /eth/v1/beacon/genesis`.
#[derive(Clone, Debug, Deserialize)]
pub struct GenesisResponse {
    /// The envelope.
    pub data: GenesisData,
}

/// Genesis facts.
#[derive(Clone, Debug, Deserialize)]
pub struct GenesisData {
    /// Unix time of genesis.
    #[serde(deserialize_with = "de_u64")]
    pub genesis_time: u64,
    /// Root of the genesis validator set.
    #[serde(deserialize_with = "de_bytes32")]
    pub genesis_validators_root: [u8; 32],
    /// Genesis fork version.
    #[serde(deserialize_with = "de_bytes4")]
    pub genesis_fork_version: [u8; 4],
}

// ── JSON helpers ──────────────────────────────────────────────────────────────

fn de_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let s = String::deserialize(d)?;
    s.parse().map_err(serde::de::Error::custom)
}

fn de_hex_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let s = String::deserialize(d)?;
    hex::decode(s.trim_start_matches("0x")).map_err(serde::de::Error::custom)
}

fn fixed<const N: usize, E: serde::de::Error>(bytes: Vec<u8>) -> Result<[u8; N], E> {
    <[u8; N]>::try_from(bytes.as_slice())
        .map_err(|_| E::custom(format!("expected {N} bytes, got {}", bytes.len())))
}

fn de_bytes4<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 4], D::Error> {
    fixed(de_hex_vec(d)?)
}

fn de_bytes20<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 20], D::Error> {
    fixed(de_hex_vec(d)?)
}

fn de_bytes32<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
    fixed(de_hex_vec(d)?)
}

fn de_opt_bytes32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 32]>, D::Error> {
    let s: Option<String> = Option::deserialize(d)?;
    match s {
        None => Ok(None),
        Some(s) => {
            let bytes =
                hex::decode(s.trim_start_matches("0x")).map_err(serde::de::Error::custom)?;
            fixed(bytes).map(Some)
        }
    }
}

fn de_bytes48<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 48], D::Error> {
    fixed(de_hex_vec(d)?)
}

fn de_bytes96<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 96], D::Error> {
    fixed(de_hex_vec(d)?)
}

fn de_bits512<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let bytes = de_hex_vec(d)?;
    if bytes.len() != SYNC_COMMITTEE_SIZE / 8 {
        return Err(serde::de::Error::custom(format!(
            "expected {} bitvector bytes, got {}",
            SYNC_COMMITTEE_SIZE / 8,
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn de_bytes32_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[u8; 32]>, D::Error> {
    let items: Vec<String> = Vec::deserialize(d)?;
    items
        .into_iter()
        .map(|s| {
            let bytes =
                hex::decode(s.trim_start_matches("0x")).map_err(serde::de::Error::custom)?;
            fixed(bytes)
        })
        .collect()
}

fn de_pubkeys<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[u8; 48]>, D::Error> {
    let items: Vec<String> = Vec::deserialize(d)?;
    items
        .into_iter()
        .map(|s| {
            let bytes =
                hex::decode(s.trim_start_matches("0x")).map_err(serde::de::Error::custom)?;
            fixed(bytes)
        })
        .collect()
}

/// A decimal `uint256` as the 32 little-endian bytes SSZ hashes.
fn de_u256_le<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
    let s = String::deserialize(d)?;
    let value = ethrex_common::U256::from_dec_str(&s).map_err(serde::de::Error::custom)?;
    Ok(value.to_little_endian())
}
