//! EXECUTE precompile for Native Rollups (EIP-8079 PoC).
//!
//! Verifies L2 state transitions by re-executing them inside the L1 EVM.
//! The precompile receives individual block fields, transactions (RLP), an
//! execution witness (JSON), and an L1 anchor (Merkle root of consumed L1
//! messages), re-executes the transactions, and verifies the resulting state
//! root and receipts root.
//!
//! This implements the `apply_body` variant from the native rollups spec:
//! individual execution parameters are provided instead of a full block.
//!
//! Before executing regular transactions, the precompile writes the L1 anchor
//! to the L1Anchor predeploy's storage slot 0 (system transaction). L2
//! contracts (e.g., L2Bridge) verify individual L1 messages via Merkle proofs
//! against this anchored root. The state root check at the end implicitly
//! guarantees correct message processing.
//!
//! Withdrawals are handled via state root proofs — the L2Bridge writes
//! withdrawal hashes to its `sentMessages` mapping, and the L1 contract
//! verifies them via MPT proofs against the post-state root. No event
//! scanning or custom Merkle trees needed.

use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, ELASTICITY_MULTIPLIER, Fork, Log, Receipt, Transaction,
        block_execution_witness::{ExecutionWitness, GuestProgramState},
        calculate_base_fee_per_gas,
    },
};
use ethrex_rlp::decode::RLPDecode;
use std::cmp::min;
use std::sync::Arc;

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

/// Address of the L2 bridge predeploy (handles L1 messages and withdrawals).
/// Must match the deployed address in the L2 genesis state.
/// Exported for test use (L2 genesis setup).
pub const L2_BRIDGE: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfd,
]);

/// Address of the L1Anchor predeploy (one above L2Bridge).
/// The EXECUTE precompile writes the L1 messages Merkle root here before
/// executing regular transactions.
pub const L1_ANCHOR: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xfe,
]);

/// Input to the EXECUTE precompile (`apply_body` variant).
///
/// Individual block fields are provided instead of a full RLP-encoded block.
/// Transactions are decoded from RLP and the witness provides stateless
/// execution data (state tries, storage tries, code, block headers).
pub struct ExecutePrecompileInput {
    pub pre_state_root: H256,
    pub post_state_root: H256,
    pub post_receipts_root: H256,
    pub block_number: u64,
    pub block_gas_limit: u64,
    pub coinbase: Address,
    pub prev_randao: H256,
    pub timestamp: u64,
    pub parent_base_fee: u64,
    pub parent_gas_limit: u64,
    pub parent_gas_used: u64,
    pub l1_anchor: H256,
    pub transactions: Vec<Transaction>,
    pub execution_witness: ExecutionWitness,
}

/// Entrypoint matching the precompile function signature.
///
/// Parses ABI-encoded calldata with 14 slots (12 static + 2 dynamic):
/// ```text
/// abi.encode(
///     bytes32 preStateRoot,           // slot 0
///     bytes32 postStateRoot,          // slot 1
///     bytes32 postReceiptsRoot,       // slot 2
///     uint256 blockNumber,            // slot 3
///     uint256 blockGasLimit,          // slot 4
///     address coinbase,               // slot 5 (ABI-padded to 32 bytes)
///     bytes32 prevRandao,             // slot 6
///     uint256 timestamp,              // slot 7
///     uint256 parentBaseFee,          // slot 8
///     uint256 parentGasLimit,         // slot 9
///     uint256 parentGasUsed,          // slot 10
///     bytes32 l1Anchor,               // slot 11
///     bytes   transactions,           // slot 12 (dynamic offset pointer)
///     bytes   witnessJson             // slot 13 (dynamic offset pointer)
/// )
/// ```
///
/// Transactions are RLP-encoded as a list. ExecutionWitness uses JSON because
/// it doesn't have RLP support (it uses serde/rkyv instead).
/// The l1Anchor is a Merkle root computed by NativeRollup.advance() on L1 from
/// stored L1 message hashes. The precompile writes it to the L1Anchor predeploy
/// on L2 before executing transactions, allowing L2 contracts to verify individual
/// messages via Merkle proofs.
///
/// Returns `abi.encode(bytes32 postStateRoot, uint256 blockNumber, uint256 gasUsed, uint256 burnedFees, uint256 baseFeePerGas)` -- 160 bytes.
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

/// Decode an RLP-encoded transaction list into a `Vec<Transaction>`.
fn decode_transactions_from_rlp(rlp_bytes: &[u8]) -> Result<Vec<Transaction>, VMError> {
    Vec::<Transaction>::decode(rlp_bytes)
        .map_err(|e| custom_err(format!("Failed to RLP-decode transactions: {e}")))
}

/// Read a uint256 from calldata as a u64, failing if the value overflows.
fn read_u64_slot(calldata: &[u8], offset: &mut usize) -> Result<u64, VMError> {
    let bytes = read_calldata(calldata, offset, 32)?;
    let val = U256::from_big_endian(bytes);
    val.try_into()
        .map_err(|_| custom_err("ABI uint256 value overflows u64".to_string()))
}

/// Parse ABI-encoded calldata into an [`ExecutePrecompileInput`].
///
/// The head is 14 x 32 = 448 bytes:
///   - slots 0-11: static fields (bytes32, uint256, address)
///   - slot 12: offset to transactions RLP bytes (dynamic)
///   - slot 13: offset to witness JSON bytes (dynamic)
fn parse_abi_calldata(calldata: &[u8]) -> Result<ExecutePrecompileInput, VMError> {
    let mut offset: usize = 0;

    // Slot 0: preStateRoot (bytes32)
    let pre_state_root = H256::from_slice(read_calldata(calldata, &mut offset, 32)?);

    // Slot 1: postStateRoot (bytes32)
    let post_state_root = H256::from_slice(read_calldata(calldata, &mut offset, 32)?);

    // Slot 2: postReceiptsRoot (bytes32)
    let post_receipts_root = H256::from_slice(read_calldata(calldata, &mut offset, 32)?);

    // Slot 3: blockNumber (uint256 -> u64)
    let block_number = read_u64_slot(calldata, &mut offset)?;

    // Slot 4: blockGasLimit (uint256 -> u64)
    let block_gas_limit = read_u64_slot(calldata, &mut offset)?;

    // Slot 5: coinbase (address, left-padded to 32 bytes)
    let coinbase_bytes = read_calldata(calldata, &mut offset, 32)?;
    let coinbase = Address::from_slice(
        coinbase_bytes
            .get(12..)
            .ok_or(InternalError::Custom("coinbase slot too short".to_string()))?,
    );

    // Slot 6: prevRandao (bytes32)
    let prev_randao = H256::from_slice(read_calldata(calldata, &mut offset, 32)?);

    // Slot 7: timestamp (uint256 -> u64)
    let timestamp = read_u64_slot(calldata, &mut offset)?;

    // Slot 8: parentBaseFee (uint256 -> u64)
    let parent_base_fee = read_u64_slot(calldata, &mut offset)?;

    // Slot 9: parentGasLimit (uint256 -> u64)
    let parent_gas_limit = read_u64_slot(calldata, &mut offset)?;

    // Slot 10: parentGasUsed (uint256 -> u64)
    let parent_gas_used = read_u64_slot(calldata, &mut offset)?;

    // Slot 11: l1Anchor (bytes32) — Merkle root of consumed L1 messages
    let l1_anchor = H256::from_slice(read_calldata(calldata, &mut offset, 32)?);

    // Slot 12: offset to transactions (dynamic)
    let txs_offset_bytes = read_calldata(calldata, &mut offset, 32)?;
    let txs_offset: usize = U256::from_big_endian(txs_offset_bytes)
        .try_into()
        .map_err(|_| custom_err("Transactions offset too large".to_string()))?;

    // Slot 13: offset to witnessJson (dynamic)
    let witness_offset_bytes = read_calldata(calldata, &mut offset, 32)?;
    let witness_offset: usize = U256::from_big_endian(witness_offset_bytes)
        .try_into()
        .map_err(|_| custom_err("Witness offset too large".to_string()))?;

    // Read transactions RLP bytes and decode
    let txs_rlp = read_abi_bytes(calldata, txs_offset)?;
    let transactions = decode_transactions_from_rlp(txs_rlp)?;

    // Read witness JSON bytes and deserialize
    let witness_bytes = read_abi_bytes(calldata, witness_offset)?;
    let execution_witness: ExecutionWitness = serde_json::from_slice(witness_bytes)
        .map_err(|e| custom_err(format!("Failed to deserialize ExecutionWitness JSON: {e}")))?;

    Ok(ExecutePrecompileInput {
        pre_state_root,
        post_state_root,
        post_receipts_root,
        block_number,
        block_gas_limit,
        coinbase,
        prev_randao,
        timestamp,
        parent_base_fee,
        parent_gas_limit,
        parent_gas_used,
        l1_anchor,
        transactions,
        execution_witness,
    })
}

fn custom_err(msg: String) -> VMError {
    VMError::Internal(InternalError::Custom(msg))
}

/// Core logic, separated so tests can call it directly with a structured input.
///
/// Implements the `apply_body` variant: receives individual block fields,
/// builds a synthetic block header internally, re-executes, and verifies.
///
/// Returns `abi.encode(bytes32 postStateRoot, uint256 blockNumber, uint256 gasUsed, uint256 burnedFees, uint256 baseFeePerGas)` -- 160 bytes.
/// The post-state root is verified against the actual computed state root after
/// execution. The state root captures all L2 state including pending withdrawals
/// (written to L2Bridge.sentMessages by withdraw()). The burned fees are
/// `base_fee_per_gas * block_gas_used` (EIP-1559 base fees are constant per
/// block). The base fee per gas is returned so the L1 contract can track it
/// on-chain for the next block.
pub fn execute_inner(input: ExecutePrecompileInput) -> Result<Bytes, VMError> {
    let ExecutePrecompileInput {
        pre_state_root,
        post_state_root: expected_post_state_root,
        post_receipts_root,
        block_number,
        block_gas_limit,
        coinbase,
        prev_randao,
        timestamp,
        parent_base_fee,
        parent_gas_limit,
        parent_gas_used,
        l1_anchor,
        transactions,
        execution_witness,
    } = input;

    // 1. Build GuestProgramState from witness
    let guest_state: GuestProgramState = execution_witness
        .try_into()
        .map_err(|e| custom_err(format!("Failed to build GuestProgramState: {e}")))?;

    // Initialize block header hashes from witness headers (empty blocks slice
    // since we no longer have a full block — the witness already contains the
    // relevant headers for BLOCKHASH).
    guest_state
        .initialize_block_header_hashes(&[])
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

    // 3. Compute base fee from explicit parent fields (EIP-1559)
    let base_fee_per_gas = calculate_base_fee_per_gas(
        block_gas_limit,
        parent_gas_limit,
        parent_gas_used,
        parent_base_fee,
        ELASTICITY_MULTIPLIER,
    )
    .ok_or_else(|| {
        custom_err("Base fee calculation failed (gas limit out of bounds)".to_string())
    })?;

    // 4. Build synthetic block header from individual fields
    let parent_hash = guest_state.parent_block_header.compute_block_hash();
    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);

    let header = ethrex_common::types::BlockHeader {
        parent_hash,
        number: block_number,
        gas_limit: block_gas_limit,
        coinbase,
        prev_randao,
        timestamp,
        base_fee_per_gas: Some(base_fee_per_gas),
        receipts_root: post_receipts_root,
        state_root: expected_post_state_root,
        transactions_root,
        difficulty: U256::zero(),
        withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(&[])),
        ..Default::default()
    };

    let block = Block {
        header,
        body: ethrex_common::types::BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    // 5. Write l1_anchor to L1Anchor predeploy storage (system transaction)
    //    This anchors the L1 messages Merkle root on L2 before executing regular
    //    transactions, allowing L2 contracts to verify messages via Merkle proofs.
    let db = Arc::new(GuestProgramStateDb::new(guest_state));
    {
        use ethrex_common::types::AccountUpdate;
        use rustc_hash::FxHashMap;

        let mut storage = FxHashMap::default();
        storage.insert(
            H256::zero(), // slot 0
            U256::from_big_endian(l1_anchor.as_bytes()),
        );

        let anchor_update = AccountUpdate {
            address: L1_ANCHOR,
            added_storage: storage,
            ..Default::default()
        };

        db.state
            .lock()
            .map_err(|e| custom_err(format!("Lock poisoned: {e}")))?
            .apply_account_updates(&[anchor_update])
            .map_err(|e| custom_err(format!("Failed to write L1Anchor storage: {e}")))?;
    }

    // 6. Execute the block
    let (_all_logs, block_gas_used) = {
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

    // 7. Verify final state root
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

    // 8. Compute burned fees: base_fee_per_gas * block_gas_used (EIP-1559)
    let burned_fees = U256::from(base_fee_per_gas)
        .checked_mul(U256::from(block_gas_used))
        .ok_or(VMError::Internal(InternalError::Overflow))?;

    // 9. Return abi.encode(postStateRoot, blockNumber, gasUsed, burnedFees, baseFeePerGas) -- 160 bytes
    let mut result = Vec::with_capacity(160);
    result.extend_from_slice(expected_post_state_root.as_bytes());
    // block_number as uint256: 24 zero bytes + 8-byte big-endian
    result.extend_from_slice(&[0u8; 24]);
    result.extend_from_slice(&block_number.to_be_bytes());
    // gasUsed as uint256: 24 zero bytes + 8-byte big-endian
    result.extend_from_slice(&[0u8; 24]);
    result.extend_from_slice(&block_gas_used.to_be_bytes());
    // burnedFees as uint256: 32 bytes big-endian
    result.extend_from_slice(&burned_fees.to_big_endian());
    // baseFeePerGas as uint256: 24 zero bytes + 8-byte big-endian
    result.extend_from_slice(&[0u8; 24]);
    result.extend_from_slice(&base_fee_per_gas.to_be_bytes());
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
