use crate::{
    eth::block,
    types::{
        block_identifier::BlockIdentifier,
        transaction::{RpcTransaction, SendRawTransactionRequest},
    },
    utils::RpcErr,
    RpcHandler,
};
use ethereum_rust_core::{
    types::{AccessListEntry, BlockHash, BlockHeader, BlockNumber, GenericTransaction, TxKind},
    H256, U256,
};

use ethereum_rust_blockchain::mempool;
use ethereum_rust_rlp::encode::RLPEncode;
use ethereum_rust_storage::Store;

use ethereum_rust_vm::{evm_state, ExecutionResult, SpecId};
use serde::Serialize;

use serde_json::Value;
use tracing::info;

pub const ESTIMATE_ERROR_RATIO: f64 = 0.015;
pub const CALL_STIPEND: u64 = 2_300; // Free gas given at beginning of call.
pub const TRANSACTION_GAS: u64 = 21_000; // Per transaction not creating a contract. NOTE: Not payable on data of calls between transactions.

pub struct CallRequest {
    transaction: GenericTransaction,
    block: Option<BlockIdentifier>,
}

pub struct GetTransactionByBlockNumberAndIndexRequest {
    pub block: BlockIdentifier,
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
    #[serde(with = "ethereum_rust_core::serde_utils::u64::hex_str")]
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
            Some(value) => Some(BlockIdentifier::parse(value.clone(), 1)?),
            None => None,
        };
        Ok(CallRequest {
            transaction: serde_json::from_value(params[0].clone())?,
            block,
        })
    }
    fn handle(&self, storage: Store) -> Result<Value, RpcErr> {
        let block = self.block.clone().unwrap_or_default();
        info!("Requested call on block: {}", block);
        let header = match block.resolve_block_header(&storage)? {
            Some(header) => header,
            // Block not found
            _ => return Ok(Value::Null),
        };
        // Run transaction
        let result = simulate_tx(&self.transaction, &header, storage, SpecId::CANCUN)?;
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

    fn handle(&self, storage: Store) -> Result<Value, RpcErr> {
        info!(
            "Requested transaction at index: {} of block with number: {}",
            self.transaction_index, self.block,
        );
        let block_number = match self.block.resolve_block_number(&storage)? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let block_body = match storage.get_block_body(block_number)? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let block_header = match storage.get_block_header(block_number)? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let tx = match block_body.transactions.get(self.transaction_index) {
            Some(tx) => tx,
            None => return Ok(Value::Null),
        };
        let tx = RpcTransaction::build(
            tx.clone(),
            block_number,
            block_header.compute_block_hash(),
            self.transaction_index,
        );
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
    fn handle(&self, storage: Store) -> Result<Value, RpcErr> {
        info!(
            "Requested transaction at index: {} of block with hash: {:#x}",
            self.transaction_index, self.block,
        );
        let block_number = match storage.get_block_number(self.block)? {
            Some(number) => number,
            _ => return Ok(Value::Null),
        };
        let block_body = match storage.get_block_body(block_number)? {
            Some(block_body) => block_body,
            _ => return Ok(Value::Null),
        };
        let tx = match block_body.transactions.get(self.transaction_index) {
            Some(tx) => tx,
            None => return Ok(Value::Null),
        };
        let tx =
            RpcTransaction::build(tx.clone(), block_number, self.block, self.transaction_index);
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
    fn handle(&self, storage: Store) -> Result<Value, RpcErr> {
        info!(
            "Requested transaction with hash: {:#x}",
            self.transaction_hash,
        );
        let (block_number, block_hash, index) =
            match storage.get_transaction_location(self.transaction_hash)? {
                Some(location) => location,
                _ => return Ok(Value::Null),
            };

        let transaction: ethereum_rust_core::types::Transaction =
            match storage.get_transaction_by_location(block_hash, index)? {
                Some(transaction) => transaction,
                _ => return Ok(Value::Null),
            };

        let transaction =
            RpcTransaction::build(transaction, block_number, block_hash, index as usize);
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
    fn handle(&self, storage: Store) -> Result<Value, RpcErr> {
        info!(
            "Requested receipt for transaction {:#x}",
            self.transaction_hash,
        );
        let (block_number, block_hash, index) =
            match storage.get_transaction_location(self.transaction_hash)? {
                Some(location) => location,
                _ => return Ok(Value::Null),
            };
        let block = match storage.get_block_by_hash(block_hash)? {
            Some(block) => block,
            None => return Ok(Value::Null),
        };
        let receipts =
            block::get_all_block_rpc_receipts(block_number, block.header, block.body, &storage)?;
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
    fn handle(&self, storage: Store) -> Result<Value, RpcErr> {
        let block = self.block.clone().unwrap_or_default();
        info!("Requested access list creation for tx on block: {}", block);
        let block_number = match block.resolve_block_number(&storage)? {
            Some(block_number) => block_number,
            _ => return Ok(Value::Null),
        };
        let header = match storage.get_block_header(block_number)? {
            Some(header) => header,
            // Block not found
            _ => return Ok(Value::Null),
        };
        // Run transaction and obtain access list
        let (gas_used, access_list, error) = match ethereum_rust_vm::create_access_list(
            &self.transaction,
            &header,
            &mut evm_state(storage, header.compute_block_hash()),
            SpecId::CANCUN,
        )? {
            (
                ExecutionResult::Success {
                    reason: _,
                    gas_used,
                    gas_refunded: _,
                    logs: _,
                    output: _,
                },
                access_list,
            ) => (gas_used, access_list, None),
            (
                ExecutionResult::Revert {
                    gas_used,
                    output: _,
                },
                access_list,
            ) => (
                gas_used,
                access_list,
                Some("Transaction Reverted".to_string()),
            ),
            (ExecutionResult::Halt { reason, gas_used }, access_list) => {
                (gas_used, access_list, Some(reason))
            }
        };
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

    fn handle(&self, storage: Store) -> Result<Value, RpcErr> {
        let tx = storage.get_transaction_by_hash(self.transaction_hash)?;

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
    fn handle(&self, storage: Store) -> Result<Value, RpcErr> {
        let block = self.block.clone().unwrap_or_default();
        info!("Requested estimate on block: {}", block);
        let block_header = match block.resolve_block_header(&storage)? {
            Some(header) => header,
            // Block not found
            _ => return Ok(Value::Null),
        };
        let spec_id =
            ethereum_rust_vm::spec_id(&storage.get_chain_config()?, block_header.timestamp);

        // If the transaction is a plain value transfer, short circuit estimation.
        if let TxKind::Call(address) = self.transaction.to {
            let account_info = storage.get_account_info(block_header.number, address)?;
            let code = account_info.map(|info| storage.get_account_code(info.code_hash));
            if code.is_none() {
                let mut value_transfer_transaction = self.transaction.clone();
                value_transfer_transaction.gas = Some(TRANSACTION_GAS);
                let result: Result<ExecutionResult, RpcErr> = simulate_tx(
                    &value_transfer_transaction,
                    &block_header,
                    storage.clone(),
                    spec_id,
                );
                if let Ok(ExecutionResult::Success { .. }) = result {
                    return serde_json::to_value(format!("{:#x}", TRANSACTION_GAS))
                        .map_err(|error| RpcErr::Internal(error.to_string()));
                }
            }
        }

        // Prepare binary search
        let mut highest_gas_limit = match self.transaction.gas {
            Some(gas) => gas.min(block_header.gas_limit),
            None => block_header.gas_limit,
        };

        if self.transaction.gas_price != 0 {
            highest_gas_limit = recap_with_account_balances(
                highest_gas_limit,
                &self.transaction,
                &storage,
                block_header.number,
            )?;
        }

        // Check whether the execution is possible
        let mut transaction = self.transaction.clone();
        transaction.gas = Some(highest_gas_limit);
        let result = simulate_tx(&transaction, &block_header, storage.clone(), spec_id)?;

        let gas_used = result.gas_used();
        let gas_refunded = result.gas_refunded();

        // Choose an optimistic start limit. See https://github.com/ethereum/go-ethereum/blob/a5a4fa7032bb248f5a7c40f4e8df2b131c4186a4/eth/gasestimator/gasestimator.go#L135
        let optimistic_limit = (gas_used + gas_refunded + CALL_STIPEND) * 64 / 63;
        let mut lowest_gas_limit = gas_used.saturating_sub(1);
        let mut middle_gas_limit = (optimistic_limit + lowest_gas_limit) / 2;

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

            let result = simulate_tx(&transaction, &block_header, storage.clone(), spec_id);
            if let Ok(ExecutionResult::Success { .. }) = result {
                highest_gas_limit = middle_gas_limit;
            } else {
                lowest_gas_limit = middle_gas_limit;
            };
            middle_gas_limit = (highest_gas_limit + lowest_gas_limit) / 2;
        }

        serde_json::to_value(format!("{:#x}", highest_gas_limit))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

fn recap_with_account_balances(
    highest_gas_limit: u64,
    transaction: &GenericTransaction,
    storage: &Store,
    block_number: BlockNumber,
) -> Result<u64, RpcErr> {
    let account_balance = storage
        .get_account_info(block_number, transaction.from)?
        .map(|acc| acc.balance)
        .unwrap_or_default();
    let account_gas =
        account_balance.saturating_sub(transaction.value) / U256::from(transaction.gas_price);
    Ok(highest_gas_limit.min(account_gas.as_u64()))
}

fn simulate_tx(
    transaction: &GenericTransaction,
    block_header: &BlockHeader,
    storage: Store,
    spec_id: SpecId,
) -> Result<ExecutionResult, RpcErr> {
    match ethereum_rust_vm::simulate_tx_from_generic(
        transaction,
        block_header,
        &mut evm_state(storage, block_header.compute_block_hash()),
        spec_id,
    )? {
        ExecutionResult::Revert {
            gas_used: _,
            output,
        } => Err(RpcErr::Revert {
            data: format!("0x{:#x}", output),
        }),
        ExecutionResult::Halt { reason, gas_used } => Err(RpcErr::Halt { reason, gas_used }),
        success => Ok(success),
    }
}

impl RpcHandler for SendRawTransactionRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<SendRawTransactionRequest, RpcErr> {
        let params = params
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
        let data = hex::decode(str_data).map_err(|error| RpcErr::BadParams(error.to_string()))?;

        let transaction = SendRawTransactionRequest::decode_canonical(&data)
            .map_err(|error| RpcErr::BadParams(error.to_string()))?;

        Ok(transaction)
    }
    fn handle(&self, storage: Store) -> Result<Value, RpcErr> {
        let hash = if let SendRawTransactionRequest::EIP4844(wrapped_blob_tx) = self {
            mempool::add_blob_transaction(
                wrapped_blob_tx.tx.clone(),
                wrapped_blob_tx.blobs_bundle.clone(),
                storage,
            )
        } else {
            mempool::add_transaction(self.to_transaction(), storage)
        }?;
        serde_json::to_value(format!("{:#x}", hash))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}
