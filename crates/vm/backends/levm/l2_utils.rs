use std::str::FromStr;

use ethrex_common::{
    types::{Log, Transaction, TxKind, SAFE_BYTES_PER_BLOB},
    H256,
};
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_storage::AccountUpdate;

use crate::{
    constants::{
        COMMON_BRIDGE_L2_ADDRESS, L2_DEPOSIT_SIZE, L2_WITHDRAWAL_SIZE, LAST_HEADER_FIELDS_SIZE,
    },
    EvmError,
};

use super::LEVM;

pub fn calc_modified_accounts_size(
    account_updates: &[AccountUpdate],
    db: &GeneralizedDatabase,
) -> Result<usize, EvmError> {
    let mut modified_accounts_size: usize = 2; // 2bytes | modified_accounts_len(u16)

    for account_update in account_updates {
        modified_accounts_size += 1 + 20; // 1byte + 20bytes | r#type(u8) + address(H160)
        if account_update.info.is_some() {
            modified_accounts_size += 32; // 32bytes | new_balance(U256)
        }

        if has_nonce_changed(account_update, db)? {
            modified_accounts_size += 2; // 2bytes | nonce_diff(u16)
        }
        // for each added_storage: 32bytes + 32bytes | key(H256) + value(U256)
        modified_accounts_size += account_update.added_storage.len() * 2 * 32;

        modified_accounts_size += 2; // 2bytes | bytecode_len(u16)
        if let Some(bytecode) = &account_update.code {
            modified_accounts_size += bytecode.len(); // (len)bytes | bytecode(Bytes)
        }
    }
    Ok(modified_accounts_size)
}

pub fn has_nonce_changed(
    account_update: &AccountUpdate,
    db: &GeneralizedDatabase,
) -> Result<bool, EvmError> {
    // Get previous nonce
    let prev_nonce = db
        .in_memory_db
        .get(&account_update.address)
        .ok_or_else(|| EvmError::Custom("Failed to get account".to_owned()))?
        .info
        .nonce;
    // Get current nonce
    let new_nonce = if let Some(info) = &account_update.info {
        prev_nonce == info.nonce
    } else {
        false
    };
    Ok(new_nonce)
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
) -> Result<(), EvmError> {
    if acc_state_diff_size.is_none() {
        return Ok(());
    }
    let mut actual_size = 0;
    if is_withdrawal_l2(tx, logs)? {
        actual_size += L2_WITHDRAWAL_SIZE;
    }
    if is_deposit_l2(tx) {
        actual_size += L2_DEPOSIT_SIZE;
    }

    let account_updates = LEVM::get_state_transitions_no_drain(db)?;

    let modified_accounts_size = calc_modified_accounts_size(&account_updates, db)?;

    let current_state_diff_size =
        1 /* version (u8) */ + LAST_HEADER_FIELDS_SIZE + actual_size + modified_accounts_size;

    if current_state_diff_size > SAFE_BYTES_PER_BLOB {
        return Err(EvmError::StateDiffSizeError);
    }
    *acc_state_diff_size = Some(current_state_diff_size); // update the accumulated size
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

fn is_deposit_l2(tx: &Transaction) -> bool {
    matches!(tx, Transaction::PrivilegedL2Transaction(_tx))
}
