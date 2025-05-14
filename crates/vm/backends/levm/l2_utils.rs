use std::{collections::HashMap, str::FromStr};

use ethrex_common::{
    types::{BlockHeader, Log, Receipt, Transaction, TxKind, SAFE_BYTES_PER_BLOB},
    H256,
};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_storage::AccountUpdate;

use crate::{constants::COMMON_BRIDGE_L2_ADDRESS, state_diff::prepare_state_diff, EvmError};

use super::LEVM;

// pub fn calc_modified_accounts_size(
//     account_updates: &[AccountUpdate],
//     db: &GeneralizedDatabase,
// ) -> Result<usize, EvmError> {
//     let mut modified_accounts_size: usize = 2; // 2bytes | modified_accounts_len(u16)

//     for account_update in account_updates {
//         modified_accounts_size += 1 + 20; // 1byte + 20bytes | r#type(u8) + address(H160)
//         if account_update.info.is_some() {
//             modified_accounts_size += 32; // 32bytes | new_balance(U256)
//         }

//         if has_nonce_changed(account_update, db)? {
//             modified_accounts_size += 2; // 2bytes | nonce_diff(u16)
//         }
//         // for each added_storage: 32bytes + 32bytes | key(H256) + value(U256)
//         modified_accounts_size += account_update.added_storage.len() * 2 * 32;

//         modified_accounts_size += 2; // 2bytes | bytecode_len(u16)
//         if let Some(bytecode) = &account_update.code {
//             modified_accounts_size += bytecode.len(); // (len)bytes | bytecode(Bytes)
//         }
//     }
//     Ok(modified_accounts_size)
// }

// pub fn has_nonce_changed(
//     account_update: &AccountUpdate,
//     db: &GeneralizedDatabase,
// ) -> Result<bool, EvmError> {
//     // Get previous nonce
//     let prev_nonce = db
//         .in_memory_db
//         .get(&account_update.address)
//         .ok_or_else(|| EvmError::Custom("Failed to get account".to_owned()))?
//         .info
//         .nonce;
//     // Get current nonce
//     let new_nonce = if let Some(info) = &account_update.info {
//         prev_nonce == info.nonce
//     } else {
//         false
//     };
//     Ok(new_nonce)
// }

pub fn get_nonce_diff(
    account_update: &AccountUpdate,
    db: &GeneralizedDatabase,
) -> Result<u16, EvmError> {
    // Get previous nonce
    let prev_nonce = db
        .in_memory_db
        .get(&account_update.address)
        .ok_or_else(|| EvmError::Custom("Failed to get account".to_owned()))?
        .info
        .nonce;
    // Get current nonce
    let new_nonce = if let Some(info) = &account_update.info {
        info.nonce - prev_nonce
    } else {
        0
    };
    new_nonce
        .try_into()
        .map_err(|_| EvmError::Custom("Invalid nonce diff".to_owned()))
}

/// Calculates the size of the current `StateDiff` of the block.
/// If the current size exceeds the blob size limit, returns `Err(EvmError::StateDiffSizeError)`.
/// If there is still space in the blob, returns `Ok(())`.
/// Updates the following mutable variable in the process:
/// - `acc_state_diff_size`: Set to current total state diff size if within limit
///
///  StateDiff:
/// +-------------------+
/// | Version           |
/// | HeaderFields      |
/// | AccountsStateDiff |
/// | Withdrawals       |
/// | Deposits          |
/// +-------------------+
pub fn update_state_diff_size(
    acc_state_diff_size: &mut Option<usize>,
    tx: &Transaction,
    logs: &[Log],
    db: &GeneralizedDatabase,
    payload: &[Transaction],
    receipts: &[Receipt],
) -> Result<(), EvmError> {
    if acc_state_diff_size.is_none() {
        return Ok(());
    }
    let mut withdrawals = vec![];
    let mut deposits = vec![];
    for (payload_tx, receipt) in payload.iter().zip(receipts.iter()) {
        if is_withdrawal_l2(payload_tx, &receipt.logs)? {
            withdrawals.push((payload_tx.compute_hash(), payload_tx.clone()));
        }
        if let Transaction::PrivilegedL2Transaction(privileged_tx) = payload_tx {
            deposits.push(privileged_tx.clone())
        }
    }
    if is_withdrawal_l2(tx, logs)? {
        withdrawals.push((tx.compute_hash(), tx.clone()));
    }
    if let Transaction::PrivilegedL2Transaction(privileged_tx) = tx {
        deposits.push(privileged_tx.clone())
    }

    let account_updates = LEVM::get_state_transitions_no_drain(db)?;

    let mut nonce_diffs = HashMap::new();
    for account_update in account_updates.iter() {
        let nonce_diff = get_nonce_diff(account_update, db)?;
        nonce_diffs.insert(account_update.address, nonce_diff);
    }

    let new_state_diff_size = prepare_state_diff(
        BlockHeader::default(),
        &withdrawals,
        &deposits,
        account_updates,
        nonce_diffs,
    )
    .map_err(|_| EvmError::Custom("Error on creating state diff".to_owned()))?
    .encode()
    .map_err(|e| EvmError::Custom(format!("Encoding error: {}", e)))?
    .len();
    if new_state_diff_size > SAFE_BYTES_PER_BLOB {
        return Err(EvmError::StateDiffSizeError);
    }
    *acc_state_diff_size = Some(new_state_diff_size); // update the accumulated size
    Ok(())
}

fn is_withdrawal_l2(tx: &Transaction, logs: &[Log]) -> Result<bool, EvmError> {
    // WithdrawalInitiated(address,address,uint256)
    let withdrawal_event_selector: H256 =
        H256::from_str("bb2689ff876f7ef453cf8865dde5ab10349d222e2e1383c5152fbdb083f02da2")
            .map_err(|e| EvmError::Custom(e.to_string()))?;

    let is_withdrawal = match tx.to() {
        TxKind::Call(to) if to == *COMMON_BRIDGE_L2_ADDRESS => logs.iter().any(|log| {
            log.topics
                .iter()
                .any(|topic| *topic == withdrawal_event_selector)
        }),
        _ => false,
    };
    Ok(is_withdrawal)
}

// fn is_deposit_l2(tx: &Transaction) -> bool {
//     matches!(tx, Transaction::PrivilegedL2Transaction(_tx))
// }
