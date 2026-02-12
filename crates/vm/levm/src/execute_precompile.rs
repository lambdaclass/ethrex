//! EXECUTE precompile for Native Rollups (EIP-8079 PoC).
//!
//! Verifies L2 state transitions by re-executing them inside the L1 EVM.
//! The precompile receives an execution witness, a block, and deposit data,
//! re-executes the block, and verifies the resulting state root matches.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Block, Fork, Transaction,
        block_execution_witness::{ExecutionWitness, GuestProgramState},
    },
};
use std::cmp::min;
use std::sync::Arc;

use crate::{
    db::{gen_db::GeneralizedDatabase, guest_program_state_db::GuestProgramStateDb},
    environment::{EVMConfig, Environment},
    errors::{InternalError, TxValidationError, VMError},
    precompiles::increase_precompile_consumed_gas,
    tracing::LevmCallTracer,
    utils::get_base_fee_per_blob_gas,
    vm::{VM, VMType},
};

/// Fixed gas cost for the PoC. Real cost TBD in the EIP.
const EXECUTE_GAS_COST: u64 = 100_000;

/// A deposit: credit `amount` to `address` before block execution.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Deposit {
    pub address: Address,
    pub amount: U256,
}

/// Input to the EXECUTE precompile.
///
/// Serialized with JSON (serde_json) for the PoC. A future version will
/// define a proper ABI encoding/decoding scheme.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ExecutePrecompileInput {
    pub pre_state_root: H256,
    pub post_state_root: H256,
    pub deposits: Vec<Deposit>,
    pub execution_witness: ExecutionWitness,
    pub block: Block,
}

/// Entrypoint matching the precompile function signature.
///
/// Deserializes input from calldata (JSON) and delegates to [`execute_inner`].
pub fn execute_precompile(
    calldata: &Bytes,
    gas_remaining: &mut u64,
    _fork: Fork,
) -> Result<Bytes, VMError> {
    increase_precompile_consumed_gas(EXECUTE_GAS_COST, gas_remaining)?;

    let input: ExecutePrecompileInput = serde_json::from_slice(calldata.as_ref())
        .map_err(|e| custom_err(format!("Failed to deserialize EXECUTE calldata: {e}")))?;
    execute_inner(input)
}

fn custom_err(msg: String) -> VMError {
    VMError::Internal(InternalError::Custom(msg))
}

/// Core logic, separated so tests can call it directly with a structured input.
pub fn execute_inner(input: ExecutePrecompileInput) -> Result<Bytes, VMError> {
    let ExecutePrecompileInput {
        pre_state_root,
        post_state_root,
        deposits,
        execution_witness,
        block,
    } = input;

    // 1. Build GuestProgramState from witness
    let mut guest_state: GuestProgramState = execution_witness
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

    // 3. Apply deposits (anchor): credit balances in the state trie
    for deposit in &deposits {
        apply_deposit(&mut guest_state, deposit).map_err(|e| {
            custom_err(format!(
                "Failed to apply deposit to {}: {e}",
                deposit.address
            ))
        })?;
    }

    // 4. Execute the block
    let db = Arc::new(GuestProgramStateDb::new(guest_state));

    {
        let db_dyn: Arc<dyn crate::db::Database> = db.clone();
        let mut gen_db = GeneralizedDatabase::new(db_dyn);

        execute_block(&block, &mut gen_db)?;

        // Apply state transitions back to the GuestProgramState
        let account_updates = gen_db
            .get_state_transitions()
            .map_err(|e| custom_err(format!("Failed to get state transitions: {e}")))?;

        db.state
            .lock()
            .map_err(|e| custom_err(format!("Lock poisoned: {e}")))?
            .apply_account_updates(&account_updates)
            .map_err(|e| custom_err(format!("Failed to apply account updates: {e}")))?;
    }

    // 5. Verify final state root
    let final_root = db
        .state
        .lock()
        .map_err(|e| custom_err(format!("Lock poisoned: {e}")))?
        .state_trie_root()
        .map_err(|e| custom_err(format!("Failed to compute final state root: {e}")))?;

    if final_root != post_state_root {
        return Err(custom_err(format!(
            "Final state root mismatch: expected {post_state_root:?}, got {final_root:?}"
        )));
    }

    // 6. Success
    Ok(Bytes::from(vec![0x01]))
}

/// Execute a block's transactions and process withdrawals.
fn execute_block(block: &Block, db: &mut GeneralizedDatabase) -> Result<(), VMError> {
    let chain_config = db.store.get_chain_config()?;
    let config = EVMConfig::new_from_chain_config(&chain_config, &block.header);

    let block_excess_blob_gas = block.header.excess_blob_gas.map(U256::from);
    let base_blob_fee = get_base_fee_per_blob_gas(block_excess_blob_gas, &config)?;

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

    let mut cumulative_gas_used = 0_u64;
    let mut block_gas_used = 0_u64;

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
            base_blob_fee_per_gas: base_blob_fee,
            gas_price,
            block_excess_blob_gas,
            block_blob_gas_used: block.header.blob_gas_used.map(U256::from),
            tx_blob_hashes: tx.blob_versioned_hashes(),
            tx_max_priority_fee_per_gas: tx.max_priority_fee().map(U256::from),
            tx_max_fee_per_gas: tx.max_fee_per_gas().map(U256::from),
            tx_max_fee_per_blob_gas: tx.max_fee_per_blob_gas(),
            tx_nonce: tx.nonce(),
            block_gas_limit: block.header.gas_limit,
            difficulty: block.header.difficulty,
            is_privileged: matches!(tx, Transaction::PrivilegedL2Transaction(_)),
            fee_token: tx.fee_token(),
        };

        let mut vm = VM::new(env, db, tx, LevmCallTracer::disabled(), VMType::L1)?;
        let report = vm.execute()?;

        cumulative_gas_used = cumulative_gas_used
            .checked_add(report.gas_spent)
            .ok_or(VMError::Internal(InternalError::Overflow))?;
        block_gas_used = block_gas_used
            .checked_add(report.gas_used)
            .ok_or(VMError::Internal(InternalError::Overflow))?;
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
    ethrex_common::validate_gas_used(block_gas_used, &block.header)
        .map_err(|e| custom_err(format!("Gas validation failed: {e}")))?;

    Ok(())
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

/// Apply a single deposit by crediting the recipient's balance in the state trie.
fn apply_deposit(
    state: &mut GuestProgramState,
    deposit: &Deposit,
) -> Result<(), Box<dyn std::error::Error>> {
    use ethrex_common::types::AccountState;
    use ethrex_rlp::decode::RLPDecode;
    use ethrex_rlp::encode::RLPEncode;

    let hashed_address = state
        .account_hashes_by_address
        .entry(deposit.address)
        .or_insert_with(|| {
            ethrex_crypto::keccak::keccak_hash(deposit.address.to_fixed_bytes()).to_vec()
        });

    let mut account_state = match state.state_trie.get(hashed_address)? {
        Some(encoded) => AccountState::decode(&encoded)?,
        None => AccountState::default(),
    };

    account_state.balance = account_state
        .balance
        .checked_add(deposit.amount)
        .ok_or("Deposit would overflow balance")?;

    state
        .state_trie
        .insert(hashed_address.clone(), account_state.encode_to_vec())?;

    Ok(())
}
