use revm::{
    primitives::{EVMError, Spec},
    Context, Database, FrameResult,
};
use revm_primitives::{Address, TxKind, U256};
use tracing::info;

use crate::{DEPOSIT_MAGIC_DATA, WITHDRAWAL_MAGIC_DATA};

pub fn deduct_caller<SPEC: Spec, EXT, DB: Database>(
    context: &mut revm::Context<EXT, DB>,
) -> Result<(), EVMError<DB::Error>> {
    // load caller's account.
    let mut caller_account = context
        .evm
        .inner
        .journaled_state
        .load_account(context.evm.inner.env.tx.caller, &mut context.evm.inner.db)?;
    // If the transaction is a deposit with a `mint` value, add the mint value
    // in wei to the caller's balance. This should be persisted to the database
    // prior to the rest of execution.
    if context.evm.inner.env.tx.caller == Address::ZERO
        && context.evm.inner.env.tx.data == *DEPOSIT_MAGIC_DATA
    {
        info!("TX from privileged account with `mint` data");
        caller_account.info.balance = caller_account
            .info
            .balance
            // .saturating_add(context.evm.inner.env.tx.value)
            .saturating_add(U256::from(U256::MAX));
    }
    // deduct gas cost from caller's account.
    revm::handler::mainnet::deduct_caller_inner::<SPEC>(
        &mut caller_account,
        &context.evm.inner.env,
    );
    Ok(())
}

pub fn validate_tx_against_state<SPEC: Spec, EXT, DB: Database>(
    context: &mut Context<EXT, DB>,
) -> Result<(), EVMError<DB::Error>> {
    if context.evm.inner.env.tx.caller == Address::ZERO {
        return Ok(());
    }
    revm::handler::mainnet::validate_tx_against_state::<SPEC, EXT, DB>(context)
}

pub fn last_frame_return<SPEC: Spec, EXT, DB: Database>(
    context: &mut Context<EXT, DB>,
    frame_result: &mut FrameResult,
) -> Result<(), EVMError<DB::Error>> {
    match context.evm.inner.env.tx.transact_to {
        TxKind::Call(address) if address == Address::ZERO => {
            if context
                .evm
                .inner
                .env
                .tx
                .data
                .starts_with(WITHDRAWAL_MAGIC_DATA)
                && frame_result.interpreter_result().is_ok()
            {
                info!("TX to privileged account with `burn` data");
                let mut destination_account = context
                    .evm
                    .inner
                    .journaled_state
                    .load_account(address, &mut context.evm.inner.db)?;
                destination_account.info.balance = destination_account
                    .info
                    .balance
                    .saturating_sub(context.evm.inner.env.tx.value);
            }
        }
        _ => {}
    }
    revm::handler::mainnet::last_frame_return::<SPEC, EXT, DB>(context, frame_result)
}
