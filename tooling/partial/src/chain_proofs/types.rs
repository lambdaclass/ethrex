//! EIP-8440 containers: the public input a chain proof commits to, the gossiped proof, and
//! the mock proof shape test and devnet proof engines accept.

use ethrex_common::H256;
use ethrex_common::types::stateless_ssz::STATELESS_INPUT_SCHEMA_ID;
use libssz::{SszDecode, SszEncode};
use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha2Hasher, merkleize_progressive, mix_in_active_fields};
use libssz_types::SszList;
use serde::{Deserialize, Deserializer};

/// `MAX_PROOF_SIZE`: 4 MiB.
pub const MAX_PROOF_SIZE: usize = 4_194_304;

/// The opaque proof bytes.
pub type ProofData = SszList<u8, MAX_PROOF_SIZE>;

/// `ProofType`: a proof system, guest program and version.
pub type ProofType = u8;

/// `PublicInput`, a `ProgressiveContainer(active_fields=[1, 1, 1, 1])`.
///
/// `origin_block_root` and `head_block_root` come from the proof; `chain_id` and `schema_id`
/// are constants of the chain the node follows, never read from the proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq, SszEncode, SszDecode)]
pub struct PublicInput {
    /// Beacon block root at which the recursion started.
    pub origin_block_root: [u8; 32],
    /// Beacon block root whose payload the proof's last step proved.
    pub head_block_root: [u8; 32],
    /// `DEPOSIT_CHAIN_ID`.
    pub chain_id: u64,
    /// `STATELESS_INPUT_SCHEMA_ID`.
    pub schema_id: u16,
}

impl PublicInput {
    /// The public input for a proof over `chain_id`, with the current stateless schema.
    pub const fn new(origin_block_root: H256, head_block_root: H256, chain_id: u64) -> Self {
        Self {
            origin_block_root: origin_block_root.0,
            head_block_root: head_block_root.0,
            chain_id,
            schema_id: STATELESS_INPUT_SCHEMA_ID,
        }
    }

    /// `hash_tree_root`: what the proof system is verified against.
    pub fn root(&self) -> H256 {
        H256(self.hash_tree_root(&Sha2Hasher))
    }
}

impl HashTreeRoot for PublicInput {
    fn hash_tree_root(&self, hasher: &impl libssz_merkle::Sha256Hasher) -> libssz_merkle::Node {
        // Destructured without `..` so a new field cannot be left out of the root.
        let Self {
            origin_block_root,
            head_block_root,
            chain_id,
            schema_id,
        } = self;
        let roots = [
            origin_block_root.hash_tree_root(hasher),
            head_block_root.hash_tree_root(hasher),
            chain_id.hash_tree_root(hasher),
            schema_id.hash_tree_root(hasher),
        ];
        let root = merkleize_progressive(hasher, &roots);
        mix_in_active_fields(hasher, &root, &[true; 4])
    }
}

/// `ExecutionProof` as gossiped.
#[derive(Clone, Debug, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct ExecutionProof {
    /// The proof bytes.
    pub proof_data: ProofData,
    /// The proof type.
    pub proof_type: ProofType,
    /// Where the recursion started.
    pub origin_block_root: [u8; 32],
    /// The block whose payload is proved.
    pub head_block_root: [u8; 32],
}

impl ExecutionProof {
    /// The public input this proof must verify against on `chain_id`.
    pub const fn public_input(&self, chain_id: u64) -> PublicInput {
        PublicInput::new(
            H256(self.origin_block_root),
            H256(self.head_block_root),
            chain_id,
        )
    }

    /// Whether this proof starts a recursion.
    pub fn is_origin(&self) -> bool {
        self.origin_block_root == self.head_block_root
    }
}

/// `SignedExecutionProof`. The signature only gates gossip; validity of the proof does not
/// depend on who signed it, so this node does not check it.
#[derive(Clone, Debug, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct SignedExecutionProof {
    /// The proof.
    pub message: ExecutionProof,
    /// The prover.
    pub validator_index: u64,
    /// The prover's signature.
    pub signature: [u8; 96],
}

/// The JSON shape of a signed proof on the beacon API (`GET /eth/v1/beacon/execution_proofs`).
#[derive(Clone, Debug, Deserialize)]
pub struct SignedExecutionProofJson {
    /// The proof.
    pub message: ExecutionProofJson,
    /// The prover.
    #[serde(deserialize_with = "de_u64")]
    pub validator_index: u64,
    /// The prover's signature.
    #[serde(deserialize_with = "de_hex")]
    pub signature: Vec<u8>,
}

/// The JSON shape of a proof.
#[derive(Clone, Debug, Deserialize)]
pub struct ExecutionProofJson {
    /// The proof bytes, hex.
    #[serde(deserialize_with = "de_hex")]
    pub proof_data: Vec<u8>,
    /// The proof type.
    #[serde(deserialize_with = "de_u8")]
    pub proof_type: u8,
    /// Where the recursion started.
    #[serde(deserialize_with = "de_h256")]
    pub origin_block_root: H256,
    /// The block whose payload is proved.
    #[serde(deserialize_with = "de_h256")]
    pub head_block_root: H256,
}

impl TryFrom<SignedExecutionProofJson> for SignedExecutionProof {
    type Error = String;

    fn try_from(json: SignedExecutionProofJson) -> Result<Self, Self::Error> {
        let proof_data: ProofData = json
            .message
            .proof_data
            .try_into()
            .map_err(|_| "proof data over 4 MiB".to_string())?;
        let signature: [u8; 96] = json
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| "signature is not 96 bytes".to_string())?;
        Ok(Self {
            message: ExecutionProof {
                proof_data,
                proof_type: json.message.proof_type,
                origin_block_root: json.message.origin_block_root.0,
                head_block_root: json.message.head_block_root.0,
            },
            validator_index: json.validator_index,
            signature,
        })
    }
}

/// The reply of `GET /eth/v1/beacon/execution_proofs/{block_id}`.
#[derive(Clone, Debug, Deserialize)]
pub struct ExecutionProofsResponse {
    /// The proofs known for the block.
    pub data: Vec<SignedExecutionProofJson>,
}

/// The mock proof devnet proof engines produce: the public values a real verifier would
/// return, plus random bytes in place of a proof. Accepted only by the mock engine.
#[derive(Clone, Debug, PartialEq, Eq, SszEncode, SszDecode)]
pub struct MockProof {
    /// The SSZ `PublicInput` the proof claims.
    pub public_values: SszList<u8, MAX_PROOF_SIZE>,
    /// Random bytes.
    pub proof: SszList<u8, MAX_PROOF_SIZE>,
}

impl MockProof {
    /// A mock proof claiming `public_input`.
    pub fn claiming(public_input: &PublicInput) -> Option<Self> {
        Some(Self {
            public_values: public_input.to_ssz().try_into().ok()?,
            proof: vec![0x4d, 0x4f, 0x43, 0x4b].try_into().ok()?,
        })
    }

    /// The SSZ bytes.
    pub fn encode(&self) -> Vec<u8> {
        self.to_ssz()
    }

    /// Decodes `bytes` as a mock proof.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        Self::from_ssz_bytes(bytes).ok()
    }
}

fn de_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let s = String::deserialize(d)?;
    s.parse().map_err(serde::de::Error::custom)
}

fn de_u8<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Num {
        Str(String),
        Int(u64),
    }
    let value = match Num::deserialize(d)? {
        Num::Str(s) => s.parse::<u64>().map_err(serde::de::Error::custom)?,
        Num::Int(i) => i,
    };
    u8::try_from(value).map_err(serde::de::Error::custom)
}

fn de_hex<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let s = String::deserialize(d)?;
    hex::decode(s.trim_start_matches("0x")).map_err(serde::de::Error::custom)
}

fn de_h256<'de, D: Deserializer<'de>>(d: D) -> Result<H256, D::Error> {
    let bytes = de_hex(d)?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map(H256)
        .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use libssz_merkle::{hash_nodes, merkleize};

    /// The progressive root of four chunks, computed by hand per EIP-7916: the first
    /// chunk sits in a 1-leaf subtree, the next 4 chunks in a 4-leaf subtree, and the
    /// remainder (here empty) hangs to the right; then active fields are mixed in.
    #[test]
    fn public_input_root_matches_a_hand_computation() {
        let input = PublicInput::new(H256::repeat_byte(1), H256::repeat_byte(2), 11155111);
        let hasher = Sha2Hasher;
        let chunks = [
            input.origin_block_root,
            input.head_block_root,
            input.chain_id.hash_tree_root(&hasher),
            input.schema_id.hash_tree_root(&hasher),
        ];
        // merkleize_progressive(chunks) = hash(subtree, rest): the subtree of size 1 holds
        // chunks[0]; the rest recurses with base 4, where the subtree of 4 holds chunks[1..4]
        // (3 chunks, padded) and its own remainder is empty (zero).
        let zero = [0u8; 32];
        let inner_rest = hash_nodes(&hasher, &merkleize(&hasher, &chunks[1..], Some(4)), &zero);
        let progressive = hash_nodes(&hasher, &chunks[0], &inner_rest);
        let expected = mix_in_active_fields(&hasher, &progressive, &[true; 4]);
        assert_eq!(input.root(), H256(expected));
        assert_eq!(input.schema_id, 0x1501);
        assert_eq!(input.to_ssz().len(), 32 + 32 + 8 + 2);
    }

    #[test]
    fn mock_proof_round_trips_and_json_proof_converts() {
        let input = PublicInput::new(H256::repeat_byte(1), H256::repeat_byte(2), 1);
        let mock = MockProof::claiming(&input).unwrap();
        let decoded = MockProof::decode(&mock.encode()).unwrap();
        assert_eq!(decoded, mock);
        assert_eq!(
            PublicInput::from_ssz_bytes(decoded.public_values.as_ref()).unwrap(),
            input
        );

        let json = serde_json::json!({
            "message": {
                "proof_data": format!("0x{}", hex::encode(mock.encode())),
                "proof_type": "3",
                "origin_block_root": format!("{:#x}", H256::repeat_byte(1)),
                "head_block_root": format!("{:#x}", H256::repeat_byte(2)),
            },
            "validator_index": "42",
            "signature": format!("0x{}", hex::encode([7u8; 96])),
        });
        let parsed: SignedExecutionProofJson = serde_json::from_value(json).unwrap();
        let proof = SignedExecutionProof::try_from(parsed).unwrap();
        assert_eq!(proof.message.proof_type, 3);
        assert_eq!(proof.validator_index, 42);
        assert_eq!(proof.message.public_input(1), input);
        assert!(!proof.message.is_origin());
    }
}
