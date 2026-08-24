use std::sync::Arc;

use crate::{
    eth::block,
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block_identifier::{BlockIdentifier, BlockIdentifierOrHash},
        transaction::{RpcTransaction, SendRawTransactionRequest},
    },
    utils::RpcErr,
};
use ethrex_blockchain::{Blockchain, vm::StoreVmDatabase};
use ethrex_common::{
    H256, U256,
    constants::{EMPTY_KECCAK_HASH, GAS_PER_BLOB},
    types::{AccessListEntry, BlockHash, BlockHeader, BlockNumber, GenericTransaction, TxKind},
};

use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;

use ethrex_vm::{ExecutionResult, backends::levm::get_max_allowed_gas_limit};
use serde::Serialize;

use serde_json::Value;
use tracing::debug;

/// Allowed upward overestimation before the estimate's binary search stops, matching
/// geth's `estimateGasErrorRatio` (`internal/ethapi/api.go`). geth's rationale is that a
/// perfect estimate is not worth extra execution when callers bump by 20-25% anyway, and
/// that for a transaction which inspects its own remaining gas the true minimum is not
/// even the useful answer. Only reached when the fast path below cannot answer.
pub const ESTIMATE_ERROR_RATIO: f64 = 0.015;

pub const CALL_STIPEND: u64 = 2_300; // Free gas given at beginning of call.
pub const TRANSACTION_GAS: u64 = 21_000; // Per transaction not creating a contract. NOTE: Not payable on data of calls between transactions.

pub struct CallRequest {
    transaction: GenericTransaction,
    block: Option<BlockIdentifierOrHash>,
}

pub struct GetTransactionByBlockNumberAndIndexRequest {
    pub block: BlockIdentifier,
    pub transaction_index: usize,
}

/// `eth_getRawTransactionByBlockHashAndIndex` / `...ByBlockNumberAndIndex`.
/// `BlockIdentifierOrHash` covers both spellings with one handler.
pub struct GetRawTransactionByBlockAndIndex {
    pub block: BlockIdentifierOrHash,
    pub transaction_index: usize,
}

pub struct GetTransactionByBlockHashAndIndexRequest {
    pub block: BlockHash,
    pub transaction_index: usize,
}

pub struct GetTransactionByHashRequest {
    pub transaction_hash: H256,
}

pub struct GetTransactionReceiptRequest {
    pub transaction_hash: H256,
}

pub struct CreateAccessListRequest {
    pub transaction: GenericTransaction,
    pub block: Option<BlockIdentifier>,
}
pub struct EstimateGasRequest {
    pub transaction: GenericTransaction,
    pub block: Option<BlockIdentifier>,
}

pub struct GetRawTransaction {
    pub transaction_hash: H256,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessListResult {
    access_list: Vec<AccessListEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    gas_used: u64,
}

impl RpcHandler for CallRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<CallRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() {
            return Err(RpcErr::BadParams("No params provided".to_owned()));
        }
        if params.len() > 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected one or two params and {} were provided",
                params.len()
            )));
        }
        let block = match params.get(1) {
            // Differentiate between missing and bad block param
            Some(value) => Some(BlockIdentifierOrHash::parse(value.clone(), 1)?),
            None => None,
        };
        Ok(CallRequest {
            transaction: serde_json::from_value(params[0].clone())?,
            block,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block = self
            .block
            .clone()
            .unwrap_or(BlockIdentifierOrHash::Identifier(BlockIdentifier::default()));
        debug!("Requested call on block: {}", block);
        let header = match block.resolve_block_header(&context.storage).await? {
            Some(header) => header,
            // Block not found
            _ => return Ok(Value::Null),
        };
        // Run transaction
        let result = simulate_tx(
            &self.transaction,
            &header,
            context.storage,
            context.blockchain,
        )?;
        serde_json::to_value(format!("0x{:#x}", result.output()))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetTransactionByBlockNumberAndIndexRequest {
    fn parse(
        params: &Option<Vec<Value>>,
    ) -> Result<GetTransactionByBlockNumberAndIndexRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected two params and {} were provided",
                params.len()
            )));
        };
        let index_as_string: String = serde_json::from_value(params[1].clone())?;
        Ok(GetTransactionByBlockNumberAndIndexRequest {
            block: BlockIdentifier::parse(params[0].clone(), 0)?,
            transaction_index: usize::from_str_radix(index_as_string.trim_start_matches("0x"), 16)
                .map_err(|error| RpcErr::BadParams(error.to_string()))?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested transaction at index: {} of block with number: {}",
            self.transaction_index, self.block,
        );
        let block_number = match self.block.resolve_block_number(&context.storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let block_body = match context.storage.get_block_body(block_number).await? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let block_header = match context.storage.get_block_header(block_number)? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let tx = match block_body.transactions.get(self.transaction_index) {
            Some(tx) => tx,
            None => return Ok(Value::Null),
        };
        let tx = RpcTransaction::build(
            tx.clone(),
            Some(block_number),
            Some(block_header.hash()),
            Some(self.transaction_index),
        )?;
        serde_json::to_value(tx).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetTransactionByBlockHashAndIndexRequest {
    fn parse(
        params: &Option<Vec<Value>>,
    ) -> Result<GetTransactionByBlockHashAndIndexRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected two param and {} were provided",
                params.len()
            )));
        };
        let index_as_string: String = serde_json::from_value(params[1].clone())?;
        Ok(GetTransactionByBlockHashAndIndexRequest {
            block: serde_json::from_value(params[0].clone())?,
            transaction_index: usize::from_str_radix(index_as_string.trim_start_matches("0x"), 16)
                .map_err(|error| RpcErr::BadParams(error.to_string()))?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested transaction at index: {} of block with hash: {:#x}",
            self.transaction_index, self.block,
        );
        let block_number = match context.storage.get_block_number(self.block).await? {
            Some(number) => number,
            _ => return Ok(Value::Null),
        };
        let block_body = match context.storage.get_block_body(block_number).await? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let tx = match block_body.transactions.get(self.transaction_index) {
            Some(tx) => tx,
            None => return Ok(Value::Null),
        };
        let tx = RpcTransaction::build(
            tx.clone(),
            Some(block_number),
            Some(self.block),
            Some(self.transaction_index),
        )?;
        serde_json::to_value(tx).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetTransactionByHashRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetTransactionByHashRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected one param and {} were provided",
                params.len()
            )));
        };
        Ok(GetTransactionByHashRequest {
            transaction_hash: serde_json::from_value(params[0].clone())?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!(
            "Requested transaction with hash: {:#x}",
            self.transaction_hash,
        );
        let transaction = if let Some((block_number, block_hash, index)) = storage
            .get_transaction_location(self.transaction_hash)
            .await?
        {
            let Some(tx) = storage
                .get_transaction_by_location(block_hash, index)
                .await?
            else {
                return Ok(Value::Null);
            };
            RpcTransaction::build(
                tx,
                Some(block_number),
                Some(block_hash),
                Some(index as usize),
            )?
        } else {
            let Some(tx) = context
                .blockchain
                .mempool
                .get_transaction_by_hash(self.transaction_hash)?
            else {
                return Ok(Value::Null);
            };
            RpcTransaction::build(tx, None, None, None)?
        };
        serde_json::to_value(transaction).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetTransactionReceiptRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetTransactionReceiptRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected one param and {} were provided",
                params.len()
            )));
        };
        Ok(GetTransactionReceiptRequest {
            transaction_hash: serde_json::from_value(params[0].clone())?,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!(
            "Requested receipt for transaction {:#x}",
            self.transaction_hash,
        );
        let (_block_number, block_hash, index) = match storage
            .get_transaction_location(self.transaction_hash)
            .await?
        {
            Some(location) => location,
            _ => return Ok(Value::Null),
        };
        let block = match storage.get_block_by_hash(block_hash).await? {
            Some(block) => block,
            None => return Ok(Value::Null),
        };
        let receipts =
            block::get_all_block_rpc_receipts(block.header, block.body, storage, Some(index))
                .await?;

        serde_json::to_value(receipts.get(index as usize))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for CreateAccessListRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<CreateAccessListRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() {
            return Err(RpcErr::BadParams("No params provided".to_owned()));
        }
        if params.len() > 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected one or two params and {} were provided",
                params.len()
            )));
        }
        let block = match params.get(1) {
            // Differentiate between missing and bad block param
            Some(value) => Some(BlockIdentifier::parse(value.clone(), 1)?),
            None => None,
        };
        Ok(CreateAccessListRequest {
            transaction: serde_json::from_value(params[0].clone())?,
            block,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let block = self.block.clone().unwrap_or_default();
        debug!("Requested access list creation for tx on block: {}", block);
        let block_number = match block.resolve_block_number(&context.storage).await? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = match context.storage.get_block_header(block_number)? {
            Some(header) => header,
            // Block not found
            _ => return Ok(Value::Null),
        };

        let vm_db = StoreVmDatabase::new(context.storage.clone(), header.clone())?;
        let mut vm = context.blockchain.new_evm(vm_db)?;

        // Run transaction and obtain access list
        let (gas_used, access_list, error) = vm.create_access_list(&self.transaction, &header)?;
        let result = AccessListResult {
            access_list: access_list
                .into_iter()
                .map(|(address, storage_keys)| AccessListEntry {
                    address,
                    storage_keys,
                })
                .collect(),
            error,
            gas_used,
        };

        serde_json::to_value(result).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetRawTransactionByBlockAndIndex {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected two params and {} were provided",
                params.len()
            )));
        };
        let index_as_string: String = serde_json::from_value(params[1].clone())?;
        Ok(GetRawTransactionByBlockAndIndex {
            block: BlockIdentifierOrHash::parse(params[0].clone(), 0)?,
            transaction_index: usize::from_str_radix(index_as_string.trim_start_matches("0x"), 16)
                .map_err(|error| RpcErr::BadParams(error.to_string()))?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested raw transaction at index {} of block {}",
            self.transaction_index, self.block
        );
        // An unknown block, or an index past the end of a known one, both yield
        // `null` rather than an error — same as the decoded by-index getters.
        let Some(block_number) = self.block.resolve_block_number(&context.storage).await? else {
            return Ok(Value::Null);
        };
        let Some(block_body) = context.storage.get_block_body(block_number).await? else {
            return Ok(Value::Null);
        };
        let Some(tx) = block_body.transactions.get(self.transaction_index) else {
            return Ok(Value::Null);
        };
        serde_json::to_value(format!("0x{}", &hex::encode(tx.encode_to_vec())))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetRawTransaction {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected one param and {} were provided",
                params.len()
            )));
        };

        let transaction_str: String = serde_json::from_value(params[0].clone())?;
        if !transaction_str.starts_with("0x") {
            return Err(RpcErr::BadHexFormat(0));
        }

        Ok(GetRawTransaction {
            transaction_hash: serde_json::from_value(params[0].clone())?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let mut tx = context
            .storage
            .get_transaction_by_hash(self.transaction_hash)
            .await?;
        if tx.is_none() {
            tx = context
                .blockchain
                .mempool
                .get_transaction_by_hash(self.transaction_hash)?;
        }
        let tx = match tx {
            Some(tx) => tx,
            _ => return Ok(Value::Null),
        };
        serde_json::to_value(format!("0x{}", &hex::encode(tx.encode_to_vec())))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for EstimateGasRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<EstimateGasRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.is_empty() {
            return Err(RpcErr::BadParams("No params provided".to_owned()));
        }
        if params.len() > 2 {
            return Err(RpcErr::BadParams(format!(
                "Expected one or two params and {} were provided",
                params.len()
            )));
        }
        let block = match params.get(1) {
            // Differentiate between missing and bad block param
            Some(value) => Some(BlockIdentifier::parse(value.clone(), 1)?),
            None => None,
        };
        Ok(EstimateGasRequest {
            transaction: serde_json::from_value(params[0].clone())?,
            block,
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        let blockchain = &context.blockchain;
        let block = self.block.clone().unwrap_or_default();
        let chain_config = storage.get_chain_config();

        debug!("Requested estimate on block: {}", block);
        let block_header = match block.resolve_block_header(storage).await? {
            Some(header) => header,
            // Block not found
            _ => return Ok(Value::Null),
        };

        let current_fork = chain_config.fork(block_header.timestamp);

        let transaction = match self.transaction.nonce {
            Some(_nonce) => self.transaction.clone(),
            None => {
                let transaction_nonce = storage
                    .get_nonce_by_account_address(block_header.number, self.transaction.from)
                    .await?;

                let mut cloned_transaction = self.transaction.clone();
                cloned_transaction.nonce = transaction_nonce;
                cloned_transaction
            }
        };

        // If the transaction is a plain value transfer, short circuit estimation.
        //
        // A transfer is a call carrying no calldata to a recipient with no code, which is
        // what geth checks (`len(call.Data) == 0 && GetCodeSize(to) == 0`), and reth and
        // nethermind equivalently. Both halves matter: calldata to a code-less account is
        // still 21000 plus its per-byte cost, and an empty call to a contract runs that
        // contract's fallback.
        if let TxKind::Call(address) = transaction.to
            && transaction.input.is_empty()
        {
            let account_info = storage
                .get_account_info(block_header.number, address)
                .await?;
            // An account absent from state has no code either, so it takes this path too.
            // Compared against the code hash rather than through `get_account_code`: the
            // hash is already in hand, and the code itself is not needed to know it is
            // empty.
            let has_code = account_info.is_some_and(|info| info.code_hash != *EMPTY_KECCAK_HASH);
            if !has_code {
                let mut value_transfer_transaction = transaction.clone();
                value_transfer_transaction.gas = Some(TRANSACTION_GAS);
                let result: Result<ExecutionResult, RpcErr> = simulate_tx(
                    &value_transfer_transaction,
                    &block_header,
                    storage.clone(),
                    blockchain.clone(),
                );
                if let Ok(ExecutionResult::Success { .. }) = result {
                    return serde_json::to_value(format!("{TRANSACTION_GAS:#x}"))
                        .map_err(|error| RpcErr::Internal(error.to_string()));
                }
            }
        }

        // Prepare binary search
        let highest_gas_limit = get_max_allowed_gas_limit(block_header.gas_limit, current_fork);
        let mut highest_gas_limit = match transaction.gas {
            Some(gas) => gas.min(highest_gas_limit),
            None => highest_gas_limit,
        };

        // The cap has to be computed against the same fee the balance check will be
        // measured against, and by the same rule: `default_hook` requires
        // `tx_max_fee_per_gas * gas_limit`, falling back to `gas_price` only when the
        // former is absent. Recapping by `gas_price` alone left a 1559 request — where the
        // legacy field is absent and deserializes to zero — uncapped at the block gas
        // limit, and the simulation then failed `InsufficientAccountFunds` for any sender
        // that could not afford the whole block's gas at its fee cap. Written as the same
        // expression rather than an equivalent one, so the two sites cannot drift: a call
        // object setting both fields (which `GenericTransaction` accepts, and which
        // `calculate_gas_price_for_generic` resolves legacy-first) would otherwise cap
        // against one fee and be checked against the other.
        let fee_cap = transaction
            .max_fee_per_gas
            .map(U256::from)
            .unwrap_or(transaction.gas_price);

        highest_gas_limit = recap_with_account_balances(
            highest_gas_limit,
            &transaction,
            fee_cap,
            storage,
            block_header.number,
        )
        .await?;

        // Check whether the execution is possible
        let mut transaction = transaction.clone();
        transaction.gas = Some(highest_gas_limit);
        let result = simulate_tx(
            &transaction,
            &block_header,
            storage.clone(),
            blockchain.clone(),
        )?;

        let gas_used = result.gas_used();
        let gas_refunded = result.gas_refunded();

        // Most transactions execute identically at their own `gas_used` as they do with
        // the whole block's gas available, and nothing can succeed below what an
        // unconstrained run consumed — so if that one re-run succeeds it *is* the minimum,
        // and the search can be skipped entirely. Two simulations, exact. Only gas-observing
        // callers (an explicit `GAS` check, or a subcall needing 63/64 headroom the
        // consumed total does not imply) fall through to the search below.
        transaction.gas = Some(gas_used);
        if let Ok(ExecutionResult::Success { .. }) = simulate_tx(
            &transaction,
            &block_header,
            storage.clone(),
            blockchain.clone(),
        ) {
            return serde_json::to_value(format!("{gas_used:#x}"))
                .map_err(|error| RpcErr::Internal(error.to_string()));
        }

        // Choose an optimistic start limit. See https://github.com/ethereum/go-ethereum/blob/a5a4fa7032bb248f5a7c40f4e8df2b131c4186a4/eth/gasestimator/gasestimator.go#L135
        let optimistic_limit = (gas_used + gas_refunded + CALL_STIPEND) * 64 / 63;
        let mut lowest_gas_limit = gas_used.saturating_sub(1);
        let mut middle_gas_limit = (optimistic_limit + lowest_gas_limit) / 2;

        // Reached only by a transaction whose result depends on the gas it is given: an
        // explicit `GAS` check, or a subcall needing 63/64 headroom the consumed total does
        // not imply. Bisect as geth does, stopping within `ESTIMATE_ERROR_RATIO` of the
        // upper bound rather than converging exactly — the same transactions geth's comment
        // says do not want their true minimum returned.
        while lowest_gas_limit + 1 < highest_gas_limit {
            if (highest_gas_limit - lowest_gas_limit) as f64 / (highest_gas_limit as f64)
                < ESTIMATE_ERROR_RATIO
            {
                break;
            };

            if middle_gas_limit > lowest_gas_limit * 2 {
                // Favor the low side, since most transactions don't need much higher gas limit than their gas used.
                middle_gas_limit = lowest_gas_limit * 2;
            }
            transaction.gas = Some(middle_gas_limit);

            let result = simulate_tx(
                &transaction,
                &block_header,
                storage.clone(),
                blockchain.clone(),
            );
            if let Ok(ExecutionResult::Success { .. }) = result {
                highest_gas_limit = middle_gas_limit;
            } else {
                lowest_gas_limit = middle_gas_limit;
            };
            middle_gas_limit = (highest_gas_limit + lowest_gas_limit) / 2;
        }

        serde_json::to_value(format!("{highest_gas_limit:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

/// The most a call object's blobs can cost, mirroring levm's `get_max_blob_gas_price`:
/// `max_fee_per_blob_gas * GAS_PER_BLOB * blobs`.
///
/// Saturates rather than erroring on overflow. A product that cannot fit a `U256` is one no
/// balance could cover, so `U256::MAX` leaves nothing available and caps the ceiling at
/// zero, which is the arithmetically correct answer rather than a fallback.
fn max_blob_gas_cost(blob_versioned_hashes: &[H256], max_fee_per_blob_gas: Option<U256>) -> U256 {
    let Some(max_fee) = max_fee_per_blob_gas else {
        return U256::zero();
    };
    U256::from(GAS_PER_BLOB)
        .checked_mul(blob_versioned_hashes.len().into())
        .and_then(|blob_gas| max_fee.checked_mul(blob_gas))
        .unwrap_or(U256::MAX)
}

/// Caps the estimation ceiling at the gas the sender can actually pay for.
///
/// `fee_cap` is the per-gas price the transaction's balance check uses: `max_fee_per_gas`
/// when the call object carries one, otherwise the legacy `gas_price`.
///
/// A zero `fee_cap` is a request that names no fee at all, or names a fee of zero, and in
/// both cases no balance bounds the gas: the ceiling is returned unchanged. Checked here
/// rather than at the call site so no caller can reach the division with a zero divisor,
/// which is the same split this function's `fee_cap` argument exists to close.
async fn recap_with_account_balances(
    highest_gas_limit: u64,
    transaction: &GenericTransaction,
    fee_cap: U256,
    storage: &Store,
    block_number: BlockNumber,
) -> Result<u64, RpcErr> {
    if fee_cap.is_zero() {
        return Ok(highest_gas_limit);
    }
    let account_balance = storage
        .get_account_info(block_number, transaction.from)
        .await?
        .map(|acc| acc.balance)
        .unwrap_or_default();
    // Blob gas is a separate market with its own fee, and `validate_sufficient_balance`
    // adds `max_fee_per_blob_gas * GAS_PER_BLOB * blobs` to what the sender must hold.
    // Balance spent there cannot also pay for execution gas, so it comes off before the
    // division — the same subtraction geth makes — or the ceiling this returns is one the
    // check will reject. `validate_sender_balance` runs ahead of `validate_4844_tx` in
    // `prepare_execution`, so a call object carrying blob fields reaches the check even
    // when it is not a well-formed blob transaction.
    let blob_cost = max_blob_gas_cost(
        &transaction.blob_versioned_hashes,
        transaction.max_fee_per_blob_gas,
    );
    let available = account_balance
        .saturating_sub(transaction.value)
        .saturating_sub(blob_cost);
    let account_gas = available / fee_cap;
    // If account_gas exceeds u64, the account can afford any gas limit.
    let account_gas = u64::try_from(account_gas).unwrap_or(highest_gas_limit);
    Ok(highest_gas_limit.min(account_gas))
}

fn simulate_tx(
    transaction: &GenericTransaction,
    block_header: &BlockHeader,
    storage: Store,
    blockchain: Arc<Blockchain>,
) -> Result<ExecutionResult, RpcErr> {
    let vm_db = StoreVmDatabase::new(storage, block_header.clone())?;
    let mut vm = blockchain.new_evm(vm_db)?;

    match vm.simulate_tx_from_generic(transaction, block_header)? {
        ExecutionResult::Revert {
            gas_used: _,
            output,
        } => Err(RpcErr::Revert {
            data: format!("0x{output:#x}"),
        }),
        ExecutionResult::Halt { reason, gas_used } => Err(RpcErr::Halt { reason, gas_used }),
        success => Ok(success),
    }
}

impl RpcHandler for SendRawTransactionRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<SendRawTransactionRequest, RpcErr> {
        let data = get_transaction_data(params)?;

        let transaction = SendRawTransactionRequest::decode_canonical(&data)
            .map_err(|error| RpcErr::BadParams(error.to_string()))?;

        if matches!(transaction, SendRawTransactionRequest::PrivilegedL2(_)) {
            return Err(RpcErr::BadParams("Invalid transaction type".to_string()));
        }

        Ok(transaction)
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // RPC submissions go through the *local* entry points so the
        // BlockchainOptions::private_mempool flag controls whether the tx is
        // propagated to peers. P2P-received txs continue to use the
        // non-local methods elsewhere.
        let hash = if let SendRawTransactionRequest::EIP4844(wrapped_blob_tx) = self {
            context
                .blockchain
                .add_local_blob_transaction_to_pool(
                    wrapped_blob_tx.tx.clone(),
                    wrapped_blob_tx.blobs_bundle.clone(),
                )
                .await
        } else {
            context
                .blockchain
                .add_local_transaction_to_pool(self.to_transaction())
                .await
        }?;
        serde_json::to_value(format!("{hash:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

fn get_transaction_data(rpc_req_params: &Option<Vec<Value>>) -> Result<Vec<u8>, RpcErr> {
    let params = rpc_req_params
        .as_ref()
        .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
    if params.len() != 1 {
        return Err(RpcErr::BadParams(format!(
            "Expected one param and {} were provided",
            params.len()
        )));
    };

    let str_data = serde_json::from_value::<String>(params[0].clone())?;
    let str_data = str_data
        .strip_prefix("0x")
        .ok_or(RpcErr::BadParams("Params are note 0x prefixed".to_owned()))?;
    hex::decode(str_data).map_err(|error| RpcErr::BadParams(error.to_string()))
}

#[cfg(test)]
mod call_nonce_tests {
    use std::str::FromStr;

    use crate::rpc::map_http_requests;
    use crate::test_utils::default_context_with_storage;
    use crate::utils::RpcRequest;
    use ethrex_common::Address;
    use ethrex_common::types::Genesis;
    use ethrex_storage::{EngineType, Store};
    use serde_json::{Value, json};

    /// Funded EOA from fixtures/genesis/l1.json.
    const SENDER: &str = "0x00000a8d3f37af8def18832962ee008d8dca4f7b";

    /// In-memory store from the l1 test genesis with `SENDER`'s account nonce
    /// bumped, so call objects can be exercised against a sender whose
    /// on-chain nonce is nonzero.
    async fn setup_store_with_sender_nonce(nonce: u64) -> Store {
        let genesis: &str = include_str!("../../../../fixtures/genesis/l1.json");
        let mut genesis: Genesis =
            serde_json::from_str(genesis).expect("Fatal: test config is invalid");
        genesis
            .alloc
            .get_mut(&Address::from_str(SENDER).unwrap())
            .expect("test sender missing from genesis")
            .nonce = nonce;
        let mut store =
            Store::new("test-store", EngineType::InMemory).expect("Fail to create in-memory db");
        store.add_initial_state(genesis).await.unwrap();
        store
    }

    async fn run_eth_call(call_object: Value) -> Value {
        let storage = setup_store_with_sender_nonce(5).await;
        let context = default_context_with_storage(storage).await;
        let request: RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [call_object, "latest"],
        }))
        .unwrap();
        map_http_requests(&request, context).await.unwrap()
    }

    /// A call object without `nonce` must not be rejected for senders whose
    /// account nonce is nonzero: the env defaults the tx nonce to 0, which the
    /// hook used to enforce against the state nonce.
    #[tokio::test]
    async fn eth_call_ignores_missing_nonce() {
        let result = run_eth_call(json!({
            "from": SENDER,
            "to": "0xc100000000000000000000000000000000000000",
            "value": "0x1",
        }))
        .await;
        assert_eq!(result, Value::String("0x".to_string()));
    }

    /// An explicit stale nonce is ignored too: no client validates the sender
    /// nonce on eth_call, even when the call object supplies one.
    #[tokio::test]
    async fn eth_call_ignores_explicit_stale_nonce() {
        let result = run_eth_call(json!({
            "from": SENDER,
            "to": "0xc100000000000000000000000000000000000000",
            "value": "0x1",
            "nonce": "0x0",
        }))
        .await;
        assert_eq!(result, Value::String("0x".to_string()));
    }
}

#[cfg(test)]
mod estimate_gas_fee_cap_tests {
    use std::str::FromStr;

    use crate::rpc::map_http_requests;
    use crate::test_utils::default_context_with_storage;
    use crate::utils::RpcRequest;
    use ethrex_common::types::Genesis;
    use ethrex_common::{Address, U256};
    use ethrex_storage::{EngineType, Store};
    use serde_json::{Value, json};

    /// Funded EOA from fixtures/genesis/l1.json.
    const SENDER: &str = "0x00000a8d3f37af8def18832962ee008d8dca4f7b";
    /// 0.01 ETH: enough for any real fill, far short of the fee cap times the
    /// genesis block gas limit (1 gwei * 25M = 0.025 ETH).
    const SENDER_BALANCE: u64 = 10_000_000_000_000_000;
    const ONE_GWEI: &str = "0x3b9aca00";

    async fn setup_store_with_poor_sender() -> Store {
        let genesis: &str = include_str!("../../../../fixtures/genesis/l1.json");
        let mut genesis: Genesis =
            serde_json::from_str(genesis).expect("Fatal: test config is invalid");
        genesis
            .alloc
            .get_mut(&Address::from_str(SENDER).unwrap())
            .expect("test sender missing from genesis")
            .balance = U256::from(SENDER_BALANCE);
        let mut store =
            Store::new("test-store", EngineType::InMemory).expect("Fail to create in-memory db");
        store.add_initial_state(genesis).await.unwrap();
        store
    }

    async fn run_estimate_gas(call_object: Value) -> Value {
        let storage = setup_store_with_poor_sender().await;
        let context = default_context_with_storage(storage).await;
        let request: RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_estimateGas",
            "params": [call_object, "latest"],
        }))
        .unwrap();
        map_http_requests(&request, context).await.unwrap()
    }

    /// A 1559 call object carries its fee cap in `maxFeePerGas`, leaving the legacy
    /// `gasPrice` to deserialize to zero. The estimation ceiling used to be recapped by
    /// `gasPrice` alone, so such a request kept the whole block's gas limit as its
    /// ceiling while the balance check measured that ceiling against `maxFeePerGas` —
    /// failing `InsufficientAccountFunds` for every sender holding less than a full
    /// block's worth of gas at its own fee cap.
    #[tokio::test]
    async fn estimate_gas_recaps_a_1559_request_by_its_max_fee_per_gas() {
        // Calldata so the plain-transfer short circuit above the recap cannot answer this
        // request: it returns TRANSACTION_GAS without ever reaching the cap, which would
        // make the test pass for a reason unrelated to what it is asserting.
        let result = run_estimate_gas(json!({
            "from": SENDER,
            "to": SENDER,
            "value": "0x1",
            "input": "0x0102030405060708",
            "maxFeePerGas": ONE_GWEI,
            "maxPriorityFeePerGas": "0x0",
        }))
        .await;
        assert_eq!(result, Value::String("0x5348".to_string()));
    }

    /// A call object may carry both fee fields — `GenericTransaction` has no conflict
    /// check, and `calculate_gas_price_for_generic` resolves `env.gas_price` legacy-first
    /// while `env.tx_max_fee_per_gas` comes straight from `maxFeePerGas`, so the two can
    /// hold different numbers. The balance check reads the max fee, so the recap must too:
    /// capping against 2 gwei while the check demands 3 gwei would leave a ceiling the
    /// sender cannot afford, which is this PR's bug in a second guise.
    #[tokio::test]
    async fn estimate_gas_recaps_by_the_max_fee_when_both_fees_are_set() {
        let result = run_estimate_gas(json!({
            "from": SENDER,
            "to": SENDER,
            "value": "0x1",
            "gasPrice": "0x77359400",      // 2 gwei
            "maxFeePerGas": "0xb2d05e00",  // 3 gwei
        }))
        .await;
        assert_eq!(result, Value::String("0x5208".to_string()));
    }

    /// The legacy path keeps recapping by `gasPrice`, which is the only fee such a
    /// request states.
    #[tokio::test]
    async fn estimate_gas_recaps_a_legacy_request_by_its_gas_price() {
        let result = run_estimate_gas(json!({
            "from": SENDER,
            "to": SENDER,
            "value": "0x1",
            "gasPrice": ONE_GWEI,
        }))
        .await;
        assert_eq!(result, Value::String("0x5208".to_string()));
    }

    /// Blob gas is priced in its own market, and `validate_sufficient_balance` adds
    /// `max_fee_per_blob_gas * GAS_PER_BLOB * blobs` to what the sender must hold. Balance
    /// committed there cannot also pay for execution gas, so the recap has to spend it
    /// too: one blob at 45 gwei is ~0.0059 ETH against this sender's 0.01, so a ceiling
    /// computed from the full balance is one the check then rejects.
    #[tokio::test]
    async fn estimate_gas_recaps_blob_cost_out_of_the_available_balance() {
        let result = run_estimate_gas(json!({
            "from": SENDER,
            "to": SENDER,
            "value": "0x1",
            "input": "0x0102030405060708",
            "maxFeePerGas": ONE_GWEI,
            "maxPriorityFeePerGas": "0x0",
            "maxFeePerBlobGas": "0xa7a358200",  // 45 gwei
            "blobVersionedHashes": [
                "0x0100000000000000000000000000000000000000000000000000000000000001"
            ],
        }))
        .await;
        assert_eq!(result, Value::String("0x5348".to_string()));
    }

    /// A call object with no blob fields must be unaffected: nothing is subtracted, so the
    /// whole balance still backs the ceiling.
    #[tokio::test]
    async fn a_request_without_blobs_loses_no_balance_to_them() {
        let result = run_estimate_gas(json!({
            "from": SENDER,
            "to": SENDER,
            "value": "0x1",
            "input": "0x0102030405060708",
            "maxFeePerGas": ONE_GWEI,
            "maxPriorityFeePerGas": "0x0",
        }))
        .await;
        assert_eq!(result, Value::String("0x5348".to_string()));
    }

    /// A transfer to an ordinary funded account is what the short circuit exists for, and
    /// what it used to miss: the recipient exists, so the old `code.is_none()` was false
    /// and every such request ran the binary search instead of answering at once.
    #[tokio::test]
    async fn a_transfer_to_a_code_less_account_short_circuits() {
        let result = run_estimate_gas(json!({
            "from": SENDER,
            "to": "0x00002132ce94eefb06eb15898c1aabd94feb0ac2",
            "value": "0x1",
        }))
        .await;
        assert_eq!(result, Value::String("0x5208".to_string()));
    }

    /// An account absent from state has no code either, so it keeps taking the same path.
    #[tokio::test]
    async fn a_transfer_to_an_absent_account_short_circuits() {
        let result = run_estimate_gas(json!({
            "from": SENDER,
            "to": "0x00000000000000000000000000000000deadbeef",
            "value": "0x1",
        }))
        .await;
        assert_eq!(result, Value::String("0x5208".to_string()));
    }

    /// Calldata is not free, so a call carrying it is not a plain transfer however
    /// code-less the recipient: 21000 would under-report by the per-byte cost.
    #[tokio::test]
    async fn calldata_is_not_a_plain_transfer() {
        let result = run_estimate_gas(json!({
            "from": SENDER,
            "to": "0x00000000000000000000000000000000deadbeef",
            "value": "0x1",
            "input": "0x0102030405060708",
        }))
        .await;
        assert_eq!(result, Value::String("0x5348".to_string()));
    }

    /// An empty call to a contract runs its fallback, which the short circuit must not
    /// price at 21000. The deposit contract from the l1 genesis reverts on an empty call,
    /// so the estimate is an error rather than a number.
    #[tokio::test]
    async fn an_empty_call_to_a_contract_is_not_a_plain_transfer() {
        let storage = setup_store_with_poor_sender().await;
        let context = default_context_with_storage(storage).await;
        let request: RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_estimateGas",
            "params": [{
                "from": SENDER,
                "to": "0x00000000219ab540356cbb839cbe05303d7705fa",
                "value": "0x0",
            }, "latest"],
        }))
        .unwrap();
        assert!(
            map_http_requests(&request, context).await.is_err(),
            "an empty call to the deposit contract must not be priced as a transfer"
        );
    }

    /// A request that states no fee at all asks not to be capped, and must not divide
    /// by a zero fee cap on the way there.
    #[tokio::test]
    async fn estimate_gas_without_any_fee_is_not_recapped() {
        let result = run_estimate_gas(json!({
            "from": SENDER,
            "to": SENDER,
            "value": "0x1",
        }))
        .await;
        assert_eq!(result, Value::String("0x5208".to_string()));
    }
}
