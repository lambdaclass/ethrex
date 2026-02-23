//! Shared types for the native rollup L2 PoC.
//!
//! These types are shared between the L1 watcher, block producer, and L1 committer
//! GenServer actors via `Arc<Mutex<VecDeque<_>>>` queues.

use ethrex_common::{Address, H256, U256};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// An L1 message recorded on the NativeRollup contract via sendL1Message().
///
/// The L1 watcher parses `L1MessageRecorded` events into this struct and pushes
/// them into the shared `PendingL1Messages` queue for the block producer.
#[derive(Clone, Debug)]
pub struct L1Message {
    pub sender: Address,
    pub to: Address,
    pub nonce: U256,
    pub value: U256,
    pub gas_limit: u64,
    pub data_hash: H256,
}

/// Thread-safe queue of L1 messages waiting to be included in an L2 block.
pub type PendingL1Messages = Arc<Mutex<VecDeque<L1Message>>>;

/// Information about a produced L2 block, used by the L1 committer to build
/// the `advance()` calldata for the NativeRollup contract.
#[derive(Clone, Debug)]
pub struct ProducedBlockInfo {
    pub block_number: u64,
    pub pre_state_root: H256,
    pub post_state_root: H256,
    pub receipts_root: H256,
    pub coinbase: Address,
    pub prev_randao: H256,
    pub timestamp: u64,
    pub transactions_rlp: Vec<u8>,
    pub witness_json: Vec<u8>,
    pub gas_used: u64,
    pub l1_messages_count: u64,
    pub l1_anchor: H256,
    pub parent_base_fee: u64,
    pub parent_gas_limit: u64,
    pub parent_gas_used: u64,
}

/// Thread-safe queue of produced blocks waiting to be committed to L1.
pub type ProducedBlocks = Arc<Mutex<VecDeque<ProducedBlockInfo>>>;
