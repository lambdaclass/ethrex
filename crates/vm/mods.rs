use revm::{
    primitives::{EVMError, Spec},
    Context, Database,
};

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
    if context.evm.inner.env.tx.data == *b"mint".as_slice() {
        caller_account.info.balance += context.evm.inner.env.tx.value;
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
    if context.evm.inner.env.tx.data == *b"mint".as_slice() {
        return Ok(());
    }
    revm::handler::mainnet::validate_tx_against_state::<SPEC, EXT, DB>(context)
}
