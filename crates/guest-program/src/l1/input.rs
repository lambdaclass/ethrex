use ethrex_common::types::{Block, block_execution_witness::ExecutionWitness};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

/// Input for the L1 stateless validation program.
#[derive(Default, Serialize, Deserialize, RDeserialize, RSerialize, Archive)]
pub struct ProgramInput {
    /// Blocks to execute.
    pub blocks: Vec<Block>,
    /// Database containing all the data necessary to execute.
    pub execution_witness: ExecutionWitness,
}

impl ProgramInput {
    /// Creates a new ProgramInput with the given blocks and execution witness.
    pub fn new(blocks: Vec<Block>, execution_witness: ExecutionWitness) -> Self {
        Self {
            blocks,
            execution_witness,
        }
    }
}
