use ethrex_common::H256;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct WithdrawalProof {
    pub batch_number: u64,
    pub index: usize,
    pub withdrawal_hash: H256,
    pub merkle_proof: Vec<H256>,
}
