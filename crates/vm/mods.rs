use revm::{
    primitives::{EVMError, Spec},
    Context, Database, FrameResult,
};
use revm_primitives::{Address, TxKind, U256};
use tracing::info;

use crate::DEPOSIT_MAGIC_DATA;

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
