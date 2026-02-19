//! EXECUTE precompile for Native Rollups (EIP-8079 PoC).
//!
//! Verifies L2 state transitions by re-executing them inside the L1 EVM.
//! The precompile receives an execution witness, a block, and an L1 messages
//! rolling hash, re-executes the block, and verifies the resulting state root,
//! receipts root, and L1 message inclusion.
//!
//! After execution, it scans L1MessageProcessed events from the L2Bridge predeploy
//! to reconstruct the L1 messages rolling hash and verify it matches the one provided
//! by the L1 NativeRollup contract. It also extracts WithdrawalInitiated events
//! and computes a Merkle root for withdrawal claiming on L1.

use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, ELASTICITY_MULTIPLIER, Fork, INITIAL_BASE_FEE, Log, Receipt, Transaction,
        block_execution_witness::{ExecutionWitness, GuestProgramState},
        calculate_base_fee_per_gas,
    },
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::decode::RLPDecode;
use std::cmp::min;
use std::sync::{Arc, LazyLock};

use crate::{
    db::{gen_db::GeneralizedDatabase, guest_program_state_db::GuestProgramStateDb},
    environment::{EVMConfig, Environment},
    errors::{InternalError, TxResult, TxValidationError, VMError},
    precompiles::increase_precompile_consumed_gas,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};

/// Fixed gas cost for the PoC. Real cost TBD in the EIP.
const EXECUTE_GAS_COST: u64 = 100_000;

/// Address of the L2 bridge predeploy (handles both L1 messages and withdrawals).
/// Must match the deployed address in the L2 genesis state.
pub const L2_BRIDGE: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfd,
]);

/// Event signature: WithdrawalInitiated(address indexed from, address indexed receiver, uint256 amount, uint256 indexed messageId)
static WITHDRAWAL_INITIATED_SELECTOR: LazyLock<H256> = LazyLock::new(|| {
    H256::from(keccak_hash(
        b"WithdrawalInitiated(address,address,uint256,uint256)",
    ))
});

/// Event signature: L1MessageProcessed(address indexed from, address indexed to, uint256 value, uint256 gasLimit, bytes32 dataHash, uint256 indexed nonce)
static L1_MESSAGE_PROCESSED_SELECTOR: LazyLock<H256> =
    LazyLock::new(|| H256::from(keccak_hash(b"L1MessageProcessed(address,address,uint256,uint256,bytes32,uint256)")));

/// A withdrawal extracted from L2 block execution logs.
#[derive(Clone, Debug)]
pub struct Withdrawal {
    pub from: Address,
    pub receiver: Address,
    pub amount: U256,
    pub message_id: U256,
}

/// Input to the EXECUTE precompile.
pub struct ExecutePrecompileInput {
    pub pre_state_root: H256,
    pub l1_messages_rolling_hash: H256,
    pub execution_witness: ExecutionWitness,
    pub block: Block,
}

/// Entrypoint matching the precompile function signature.
///
/// Parses ABI-encoded calldata:
/// ```text
/// abi.encode(bytes32 preStateRoot, bytes blockRlp, bytes witnessJson, bytes32 l1MessagesRollingHash)
/// ```
///
/// ABI layout:
///   slot 0: preStateRoot            (bytes32, static)
///   slot 1: offset_to_blockRlp      (uint256, dynamic pointer -> 0x80)
///   slot 2: offset_to_witness       (uint256, dynamic pointer)
///   slot 3: l1MessagesRollingHash   (bytes32, static -- NOT a pointer)
///   tail:   block RLP data, witness JSON data
///
/// Block uses RLP encoding (already implemented in ethrex). ExecutionWitness
/// uses JSON because it doesn't have RLP support (it uses serde/rkyv instead).
/// The l1MessagesRollingHash is computed by NativeRollup.advance() on L1 from
/// stored L1 message hashes. The precompile verifies it against L1MessageProcessed
/// events emitted by the L2Bridge predeploy during block execution.
///
/// Returns `abi.encode(bytes32 postStateRoot, uint256 blockNumber, bytes32 withdrawalRoot, uint256 gasUsed)` -- 128 bytes.
pub fn execute_precompile(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    increase_precompile_consumed_gas(EXECUTE_GAS_COST, gas_remaining)?;

    let input = parse_abi_calldata(calldata)?;
    execute_inner(input)
}

/// Read `n` bytes from `calldata` starting at `offset`, advancing `offset`.
fn read_calldata<'a>(
    calldata: &'a [u8],
    offset: &mut usize,
    n: usize,
) -> Result<&'a [u8], VMError> {
    let end = offset
        .checked_add(n)
        .ok_or_else(|| custom_err("EXECUTE calldata offset overflow".to_string()))?;
    if end > calldata.len() {
        return Err(custom_err(format!(
            "EXECUTE calldata too short: need {n} more bytes at offset {}, have {}",
            *offset,
            calldata.len()
        )));
    }
    let slice = calldata
        .get(*offset..end)
        .ok_or_else(|| custom_err("EXECUTE calldata slice out of bounds".to_string()))?;
    *offset = end;
    Ok(slice)
}

/// Read the dynamic `bytes` at the given ABI offset.
///
/// In ABI encoding, a dynamic `bytes` parameter is stored as:
///   - At the offset position: uint256 length (32 bytes)
///   - Immediately after: the raw bytes (padded to 32-byte boundary)
///
/// Returns the raw bytes (without padding).
fn read_abi_bytes(calldata: &[u8], abi_offset: usize) -> Result<&[u8], VMError> {
    // Read the length word at the offset
    let mut pos = abi_offset;
    let len_bytes = read_calldata(calldata, &mut pos, 32)?;
    let len = U256::from_big_endian(len_bytes);
    let len: usize = len
        .try_into()
        .map_err(|_| custom_err("ABI bytes length too large".to_string()))?;

    // Read the actual data
    read_calldata(calldata, &mut pos, len)
}

/// Parse ABI-encoded calldata into an [`ExecutePrecompileInput`].
///
/// Format: `abi.encode(bytes32 preStateRoot, bytes blockRlp, bytes witnessJson, bytes32 l1MessagesRollingHash)`
///
/// The head is 4 x 32 = 128 bytes:
///   - slot 0: preStateRoot (bytes32, static)
///   - slot 1: offset to blockRlp (dynamic pointer)
///   - slot 2: offset to witnessJson (dynamic pointer)
///   - slot 3: l1MessagesRollingHash (bytes32, static -- NOT a pointer)
fn parse_abi_calldata(calldata: &[u8]) -> Result<ExecutePrecompileInput, VMError> {
    let mut offset: usize = 0;

    // 1. pre_state_root (bytes32, static -- 32 bytes)
    let pre_state_root = H256::from_slice(read_calldata(calldata, &mut offset, 32)?);

    // 2. Read offsets for the 2 dynamic params
    let block_offset_bytes = read_calldata(calldata, &mut offset, 32)?;
    let block_offset: usize = U256::from_big_endian(block_offset_bytes)
        .try_into()
        .map_err(|_| custom_err("Block offset too large".to_string()))?;

    let witness_offset_bytes = read_calldata(calldata, &mut offset, 32)?;
    let witness_offset: usize = U256::from_big_endian(witness_offset_bytes)
        .try_into()
        .map_err(|_| custom_err("Witness offset too large".to_string()))?;

    // 3. l1MessagesRollingHash (bytes32, static -- NOT a dynamic offset)
    let l1_messages_rolling_hash = H256::from_slice(read_calldata(calldata, &mut offset, 32)?);

    // 4. Read block RLP bytes
    let block_rlp = read_abi_bytes(calldata, block_offset)?;
    let block = Block::decode(block_rlp)
        .map_err(|e| custom_err(format!("Failed to RLP-decode block: {e}")))?;

    // 5. Read witness JSON bytes
    let witness_bytes = read_abi_bytes(calldata, witness_offset)?;
    let execution_witness: ExecutionWitness = serde_json::from_slice(witness_bytes)
        .map_err(|e| custom_err(format!("Failed to deserialize ExecutionWitness JSON: {e}")))?;

    Ok(ExecutePrecompileInput {
        pre_state_root,
        l1_messages_rolling_hash,
        execution_witness,
        block,
    })
}

fn custom_err(msg: String) -> VMError {
    VMError::Internal(InternalError::Custom(msg))
}

/// Core logic, separated so tests can call it directly with a structured input.
///
/// Returns `abi.encode(bytes32 postStateRoot, uint256 blockNumber, bytes32 withdrawalRoot, uint256 gasUsed)` -- 128 bytes.
/// The post-state root is extracted from `block.header.state_root` and verified
/// against the actual computed state root after execution. The withdrawal root is
/// computed from WithdrawalInitiated events emitted during block execution.
pub fn execute_inner(input: ExecutePrecompileInput) -> Result<Bytes, VMError> {
    let ExecutePrecompileInput {
        pre_state_root,
        l1_messages_rolling_hash,
        execution_witness,
        block,
    } = input;

    // Extract expected values from the block header
    let expected_post_state_root = block.header.state_root;
    let block_number = block.header.number;

    // 1. Build GuestProgramState from witness
    let guest_state: GuestProgramState = execution_witness
        .try_into()
        .map_err(|e| custom_err(format!("Failed to build GuestProgramState: {e}")))?;

    // Initialize block header hashes
    guest_state
        .initialize_block_header_hashes(std::slice::from_ref(&block))
        .map_err(|e| custom_err(format!("Failed to initialize block header hashes: {e}")))?;

    // 2. Verify initial state root
    let initial_root = guest_state
        .state_trie_root()
        .map_err(|e| custom_err(format!("Failed to compute initial state root: {e}")))?;
    if initial_root != pre_state_root {
        return Err(custom_err(format!(
            "Initial state root mismatch: expected {pre_state_root:?}, got {initial_root:?}"
        )));
    }

    // 3. Verify base fee against parent header (EIP-1559)
    let parent_base_fee = guest_state
        .parent_block_header
        .base_fee_per_gas
        .unwrap_or(INITIAL_BASE_FEE);
    if let Some(block_base_fee) = block.header.base_fee_per_gas {
        let expected_base_fee = calculate_base_fee_per_gas(
            block.header.gas_limit,
            guest_state.parent_block_header.gas_limit,
            guest_state.parent_block_header.gas_used,
            parent_base_fee,
            ELASTICITY_MULTIPLIER,
        )
        .ok_or_else(|| {
            custom_err("Base fee calculation failed (gas limit out of bounds)".to_string())
        })?;
        if block_base_fee != expected_base_fee {
            return Err(custom_err(format!(
                "Base fee mismatch: block header has {block_base_fee}, expected {expected_base_fee}"
            )));
        }
    }

    // 4. Execute the block and collect logs
    let db = Arc::new(GuestProgramStateDb::new(guest_state));

    let (all_logs, block_gas_used) = {
        let db_dyn: Arc<dyn crate::db::Database> = db.clone();
        let mut gen_db = GeneralizedDatabase::new(db_dyn);

        let (logs, gas_used) = execute_block(&block, &mut gen_db)?;

        // Apply state transitions back to the GuestProgramState
        let account_updates = gen_db
            .get_state_transitions()
            .map_err(|e| custom_err(format!("Failed to get state transitions: {e}")))?;

        db.state
            .lock()
            .map_err(|e| custom_err(format!("Lock poisoned: {e}")))?
            .apply_account_updates(&account_updates)
            .map_err(|e| custom_err(format!("Failed to apply account updates: {e}")))?;

        (logs, gas_used)
    };

    // 5. Verify final state root
    let final_root = db
        .state
        .lock()
        .map_err(|e| custom_err(format!("Lock poisoned: {e}")))?
        .state_trie_root()
        .map_err(|e| custom_err(format!("Failed to compute final state root: {e}")))?;

    if final_root != expected_post_state_root {
        return Err(custom_err(format!(
            "Final state root mismatch: expected {expected_post_state_root:?}, got {final_root:?}"
        )));
    }

    // 6. Verify L1 messages rolling hash against L1MessageProcessed events
    let computed_rolling_hash = compute_l1_messages_rolling_hash(&all_logs);
    if computed_rolling_hash != l1_messages_rolling_hash {
        return Err(custom_err(format!(
            "L1 messages rolling hash mismatch: expected {l1_messages_rolling_hash:?}, got {computed_rolling_hash:?}"
        )));
    }

    // 7. Extract withdrawals from logs and compute Merkle root
    let withdrawals = extract_withdrawals(&all_logs);
    let withdrawal_root = compute_withdrawals_merkle_root(&withdrawals);

    // 8. Return abi.encode(postStateRoot, blockNumber, withdrawalRoot, gasUsed) -- 128 bytes
    let mut result = Vec::with_capacity(128);
    result.extend_from_slice(expected_post_state_root.as_bytes());
    // block_number as uint256: 24 zero bytes + 8-byte big-endian
    result.extend_from_slice(&[0u8; 24]);
    result.extend_from_slice(&block_number.to_be_bytes());
    // withdrawal Merkle root as bytes32
    result.extend_from_slice(withdrawal_root.as_bytes());
    // gasUsed as uint256: 24 zero bytes + 8-byte big-endian
    result.extend_from_slice(&[0u8; 24]);
    result.extend_from_slice(&block_gas_used.to_be_bytes());
    Ok(Bytes::from(result))
}

/// Execute a block's transactions, returning all logs and total gas used.
///
/// Builds receipts for every transaction (including reverted ones) and validates
/// the receipts root against the block header.
fn execute_block(block: &Block, db: &mut GeneralizedDatabase) -> Result<(Vec<Log>, u64), VMError> {
    let chain_config = db.store.get_chain_config()?;
    let config = EVMConfig::new_from_chain_config(&chain_config, &block.header);

    // Validate transaction types before recovering senders (cheap check first).
    // Native rollup blocks only allow standard L1 transaction types.
    for tx in &block.body.transactions {
        match tx {
            Transaction::EIP4844Transaction(_) => {
                return Err(custom_err(
                    "Blob transactions (EIP-4844) are not allowed in native rollup blocks"
                        .to_string(),
                ));
            }
            Transaction::PrivilegedL2Transaction(_) => {
                return Err(custom_err(
                    "Privileged L2 transactions are not allowed in native rollup blocks"
                        .to_string(),
                ));
            }
            Transaction::FeeTokenTransaction(_) => {
                return Err(custom_err(
                    "Fee token transactions are not allowed in native rollup blocks".to_string(),
                ));
            }
            _ => {} // Legacy, EIP-2930, EIP-1559, EIP-7702 are allowed
        }
    }

    let transactions_with_sender = block.body.get_transactions_with_sender().map_err(|error| {
        VMError::Internal(InternalError::Custom(format!(
            "Couldn't recover addresses: {error}"
        )))
    })?;

    let mut all_logs = Vec::new();
    let mut receipts: Vec<Receipt> = Vec::new();
    let mut cumulative_gas_used = 0_u64;

    for (tx, tx_sender) in &transactions_with_sender {
        let gas_price = calculate_gas_price(tx, block.header.base_fee_per_gas.unwrap_or_default())?;

        let env = Environment {
            origin: *tx_sender,
            gas_limit: tx.gas_limit(),
            config,
            block_number: block.header.number.into(),
            coinbase: block.header.coinbase,
            timestamp: block.header.timestamp.into(),
            prev_randao: Some(block.header.prev_randao),
            slot_number: block
                .header
                .slot_number
                .map(U256::from)
                .unwrap_or(U256::zero()),
            chain_id: chain_config.chain_id.into(),
            base_fee_per_gas: block.header.base_fee_per_gas.unwrap_or_default().into(),
            base_blob_fee_per_gas: U256::zero(),
            gas_price,
            block_excess_blob_gas: None,
            block_blob_gas_used: None,
            tx_blob_hashes: vec![],
            tx_max_priority_fee_per_gas: tx.max_priority_fee().map(U256::from),
            tx_max_fee_per_gas: tx.max_fee_per_gas().map(U256::from),
            tx_max_fee_per_blob_gas: None,
            tx_nonce: tx.nonce(),
            block_gas_limit: block.header.gas_limit,
            difficulty: block.header.difficulty,
            is_privileged: matches!(tx, Transaction::PrivilegedL2Transaction(_)),
            fee_token: tx.fee_token(),
        };

        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), VMType::L1)?;
        let report = vm.execute()?;

        cumulative_gas_used = cumulative_gas_used
            .checked_add(report.gas_used)
            .ok_or(VMError::Internal(InternalError::Overflow))?;

        // Build receipt for every transaction (including reverted ones)
        let succeeded = matches!(report.result, TxResult::Success);
        let receipt_logs = if succeeded {
            all_logs.extend(report.logs.clone());
            report.logs
        } else {
            vec![]
        };

        receipts.push(Receipt::new(
            tx.tx_type(),
            succeeded,
            cumulative_gas_used,
            receipt_logs,
        ));
    }

    // Native rollup L2 blocks do not have validator withdrawals.
    if let Some(withdrawals) = &block.body.withdrawals
        && !withdrawals.is_empty()
    {
        return Err(custom_err(format!(
            "Native rollup blocks must not contain withdrawals, found {}",
            withdrawals.len()
        )));
    }

    // Validate gas used
    ethrex_common::validate_gas_used(cumulative_gas_used, &block.header)
        .map_err(|e| custom_err(format!("Gas validation failed: {e}")))?;

    // Validate receipts root
    ethrex_common::validate_receipts_root(&block.header, &receipts)
        .map_err(|e| custom_err(format!("Receipts root validation failed: {e}")))?;

    Ok((all_logs, cumulative_gas_used))
}

/// Calculate effective gas price for a transaction (simplified L1 version).
fn calculate_gas_price(tx: &Transaction, base_fee_per_gas: u64) -> Result<U256, VMError> {
    let Some(max_priority_fee) = tx.max_priority_fee() else {
        // Legacy transaction
        return Ok(tx.gas_price());
    };

    let max_fee_per_gas = tx.max_fee_per_gas().ok_or(VMError::TxValidation(
        TxValidationError::InsufficientMaxFeePerGas,
    ))?;

    if base_fee_per_gas > max_fee_per_gas {
        return Err(VMError::TxValidation(
            TxValidationError::InsufficientMaxFeePerGas,
        ));
    }

    Ok(min(
        max_priority_fee
            .checked_add(base_fee_per_gas)
            .ok_or(VMError::Internal(InternalError::Overflow))?,
        max_fee_per_gas,
    )
    .into())
}

// ===== L1 messages rolling hash verification =====

/// Reconstruct the L1 messages rolling hash from L1MessageProcessed events emitted
/// during block execution by the L2Bridge predeploy at [`L2_BRIDGE`].
///
/// Per-message hash (matches NativeRollup._recordL1Message() on L1):
///   `keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), nonce))`
///   = keccak256(from[20 bytes] ++ to[20 bytes] ++ value[32 bytes BE] ++ gasLimit[32 bytes BE] ++ dataHash[32 bytes] ++ nonce[32 bytes BE])
///   = 168 bytes preimage
///
/// Rolling hash (matches NativeRollup.advance() on L1):
///   `rolling_i = keccak256(abi.encodePacked(rolling_{i-1}, message_hash_i))`
///   = keccak256(rolling[32 bytes] ++ message_hash[32 bytes])
///
/// Returns `H256::zero()` when no L1MessageProcessed events are found.
fn compute_l1_messages_rolling_hash(logs: &[Log]) -> H256 {
    let mut rolling = H256::zero();

    for log in logs {
        if log.address != L2_BRIDGE
            || log.topics.first() != Some(&*L1_MESSAGE_PROCESSED_SELECTOR)
            || log.topics.len() != 4
        {
            continue;
        }

        // topics[0] = event selector
        // topics[1] = from (address, indexed -- left-padded to 32 bytes)
        // topics[2] = to (address, indexed -- left-padded to 32 bytes)
        // topics[3] = nonce (uint256, indexed)
        // data[0..32] = value (uint256, non-indexed)
        // data[32..64] = gasLimit (uint256, non-indexed)
        // data[64..96] = dataHash (bytes32, non-indexed)
        let Some(from_bytes) = log.topics.get(1).and_then(|t| t.as_bytes().get(12..32)) else {
            continue;
        };
        let Some(to_bytes) = log.topics.get(2).and_then(|t| t.as_bytes().get(12..32)) else {
            continue;
        };
        let Some(nonce_topic) = log.topics.get(3) else {
            continue;
        };
        let nonce_bytes = nonce_topic.as_bytes();

        let Some(value_bytes) = log.data.get(..32) else {
            continue;
        };
        let Some(gas_limit_bytes) = log.data.get(32..64) else {
            continue;
        };
        let Some(data_hash_bytes) = log.data.get(64..96) else {
            continue;
        };

        // Per-message hash: keccak256(from[20] ++ to[20] ++ value[32] ++ gasLimit[32] ++ dataHash[32] ++ nonce[32]) = 168 bytes
        let mut message_preimage = Vec::with_capacity(168);
        message_preimage.extend_from_slice(from_bytes);       // 20 bytes
        message_preimage.extend_from_slice(to_bytes);         // 20 bytes
        message_preimage.extend_from_slice(value_bytes);      // 32 bytes
        message_preimage.extend_from_slice(gas_limit_bytes);  // 32 bytes
        message_preimage.extend_from_slice(data_hash_bytes);  // 32 bytes
        message_preimage.extend_from_slice(nonce_bytes);      // 32 bytes
        let message_hash = H256::from(keccak_hash(&message_preimage));

        // Rolling hash: keccak256(rolling[32] ++ message_hash[32]) = 64 bytes
        let mut rolling_preimage = [0u8; 64];
        rolling_preimage[..32].copy_from_slice(rolling.as_bytes());
        rolling_preimage[32..].copy_from_slice(message_hash.as_bytes());
        rolling = H256::from(keccak_hash(rolling_preimage));
    }

    rolling
}

// ===== Withdrawal extraction and Merkle tree =====

/// Extract withdrawals from block execution logs.
///
/// Scans for `WithdrawalInitiated(address indexed from, address indexed receiver, uint256 amount, uint256 indexed messageId)`
/// events emitted by the L2 withdrawal bridge at [`L2_BRIDGE`].
fn extract_withdrawals(logs: &[Log]) -> Vec<Withdrawal> {
    logs.iter()
        .filter(|log| {
            log.address == L2_BRIDGE
                && log.topics.first() == Some(&*WITHDRAWAL_INITIATED_SELECTOR)
                && log.topics.len() == 4
        })
        .filter_map(|log| {
            // topics[0] = event selector
            // topics[1] = from (address, indexed -- left-padded to 32 bytes)
            // topics[2] = receiver (address, indexed -- left-padded to 32 bytes)
            // topics[3] = messageId (uint256, indexed)
            // data = amount (uint256, non-indexed, 32 bytes)
            let from = Address::from_slice(log.topics.get(1)?.as_bytes().get(12..32)?);
            let receiver = Address::from_slice(log.topics.get(2)?.as_bytes().get(12..32)?);
            let message_id = U256::from_big_endian(log.topics.get(3)?.as_bytes());
            let amount = U256::from_big_endian(log.data.get(..32)?);

            Some(Withdrawal {
                from,
                receiver,
                amount,
                message_id,
            })
        })
        .collect()
}

/// Compute the withdrawal hash for Merkle tree inclusion.
///
/// Format: `keccak256(abi.encodePacked(from, receiver, amount, messageId))`
///
/// Must exactly match the Solidity computation in NativeRollup.claimWithdrawal():
///   `keccak256(abi.encodePacked(_from, _receiver, _amount, _messageId))`
///
/// abi.encodePacked for address is 20 bytes, for uint256 is 32 bytes.
pub fn compute_withdrawal_hash(withdrawal: &Withdrawal) -> H256 {
    let mut data = Vec::with_capacity(104); // 20 + 20 + 32 + 32
    data.extend_from_slice(withdrawal.from.as_bytes()); // 20 bytes
    data.extend_from_slice(withdrawal.receiver.as_bytes()); // 20 bytes
    data.extend_from_slice(&withdrawal.amount.to_big_endian()); // 32 bytes
    data.extend_from_slice(&withdrawal.message_id.to_big_endian()); // 32 bytes

    H256::from(keccak_hash(&data))
}

/// Compute the Merkle root of withdrawal hashes.
fn compute_withdrawals_merkle_root(withdrawals: &[Withdrawal]) -> H256 {
    if withdrawals.is_empty() {
        return H256::zero();
    }

    let hashes: Vec<H256> = withdrawals.iter().map(compute_withdrawal_hash).collect();
    compute_merkle_root(&hashes)
}

/// Compute a Merkle root using commutative Keccak256 hashing (OpenZeppelin-compatible).
///
/// Commutative hashing ensures H(a, b) == H(b, a), which is required for
/// compatibility with OpenZeppelin's MerkleProof.verify().
///
/// See: https://docs.openzeppelin.com/contracts/5.x/api/utils#MerkleProof
pub fn compute_merkle_root(hashes: &[H256]) -> H256 {
    match hashes {
        [] => H256::zero(),
        [single] => *single,
        _ => {
            let mut current_level: Vec<[u8; 32]> = hashes.iter().map(|h| h.0).collect();
            while current_level.len() > 1 {
                current_level = merkle_next_level(&current_level);
            }
            current_level
                .first()
                .map(|h| H256::from(*h))
                .unwrap_or_default()
        }
    }
}

/// Compute a Merkle proof for the leaf at `index`.
///
/// Returns the sibling hashes from leaf to root, suitable for OpenZeppelin's
/// MerkleProof.verify().
pub fn compute_merkle_proof(hashes: &[H256], index: usize) -> Vec<H256> {
    if hashes.len() <= 1 {
        return vec![];
    }

    let mut current_level: Vec<[u8; 32]> = hashes.iter().map(|h| h.0).collect();
    let mut proof = Vec::new();
    let mut idx = index;

    while current_level.len() > 1 {
        // Add sibling to proof if it exists
        let sibling_idx = if idx.is_multiple_of(2) {
            idx.wrapping_add(1)
        } else {
            idx.wrapping_sub(1)
        };
        if let Some(sibling) = current_level.get(sibling_idx) {
            proof.push(H256::from(*sibling));
        }

        current_level = merkle_next_level(&current_level);
        idx /= 2;
    }

    proof
}

/// Build the next level of a Merkle tree from the current level.
///
/// Pairs adjacent elements and hashes them. If there's an odd element,
/// it's promoted to the next level unchanged.
fn merkle_next_level(current_level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let mut next_level = Vec::new();
    for pair in current_level.chunks(2) {
        match pair {
            [left, right] => next_level.push(commutative_hash(left, right)),
            [single] => next_level.push(*single),
            _ => {}
        }
    }
    next_level
}

/// Commutative Keccak256 hash: H(a, b) == H(b, a).
///
/// Sorts inputs so the smaller value comes first, matching OpenZeppelin's
/// `_hashPair` in MerkleProof.sol.
fn commutative_hash(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut data = [0u8; 64];
    if a <= b {
        data[..32].copy_from_slice(a);
        data[32..].copy_from_slice(b);
    } else {
        data[..32].copy_from_slice(b);
        data[32..].copy_from_slice(a);
    }
    keccak_hash(data)
}
