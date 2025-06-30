use keccak_hash::H256;
use lambdaworks_crypto::merkle_tree::{merkle::MerkleTree, traits::IsMerkleTreeBackend};
use sha3::{Digest, Keccak256};

// We use a newtype wrapper around `H256` because Rust's orphan rule
// prevents implementing a foreign trait (`IsMerkleTreeBackend`) for a foreign type (`H256`).
#[derive(Default, Debug, PartialEq, Eq)]
struct TreeData(pub H256);

// Code from https://github.com/yetanotherco/aligned_layer/blob/8a3a6448c974d09c645f3b74d4c9ff9d2dd27249/batcher/aligned-sdk/src/aggregation_layer/types.rs to build a merkle tree with commutative Keccak256 hashes
impl IsMerkleTreeBackend for TreeData {
    type Data = TreeData;
    type Node = [u8; 32];

    /// We don't have to hash the data, as its already hashed
    fn hash_data(leaf: &Self::Data) -> Self::Node {
        leaf.0.to_fixed_bytes()
    }

    /// Computes a commutative Keccak256 hash, ensuring H(a, b) == H(b, a).
    ///
    /// See: https://docs.openzeppelin.com/contracts/5.x/api/utils#Hashes
    ///
    /// Source: https://github.com/OpenZeppelin/openzeppelin-contracts/blob/1a87de932664d9b905612f4d9d1655fd27a41722/contracts/utils/cryptography/Hashes.sol#L17-L19
    ///
    /// Compliant with OpenZeppelin's `verify` function from MerkleProof.sol.
    ///
    /// See: https://docs.openzeppelin.com/contracts/5.x/api/utils#MerkleProof
    ///
    /// Source: https://github.com/OpenZeppelin/openzeppelin-contracts/blob/1a87de932664d9b905612f4d9d1655fd27a41722/contracts/utils/cryptography/MerkleProof.sol#L114-L128
    fn hash_new_parent(child_1: &Self::Node, child_2: &Self::Node) -> Self::Node {
        let mut hasher = Keccak256::new();
        if child_1 < child_2 {
            hasher.update(child_1);
            hasher.update(child_2);
        } else {
            hasher.update(child_2);
            hasher.update(child_1);
        }
        hasher.finalize().into()
    }
}

pub fn compute_merkle_root(hashes: &[H256]) -> H256 {
    let hashes = hashes
        .iter()
        .map(|hash| TreeData(*hash))
        .collect::<Vec<_>>();
    // Merkle tree build only returns None when hashes is empty
    let Some(tree) = MerkleTree::<TreeData>::build(&hashes) else {
        return H256::zero();
    };
    H256::from(tree.root)
}

pub fn compute_merkle_proof(hashes: &[H256], index: usize) -> Option<Vec<H256>> {
    let hashes = hashes
        .iter()
        .map(|hash| TreeData(*hash))
        .collect::<Vec<_>>();
    Some(
        MerkleTree::<TreeData>::build(&hashes)?
            .get_proof_by_pos(index)?
            .merkle_path
            .iter()
            .map(H256::from)
            .collect(),
    )
}
