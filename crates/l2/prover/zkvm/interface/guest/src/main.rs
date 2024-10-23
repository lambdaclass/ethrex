use risc0_zkvm::guest::env;

//use ethereum_rust_blockchain::validate_gas_used;
use ethereum_rust_core::types::{Receipt, Transaction};
// We have to import the ExecutionDB.
use ethereum_rust_vm::{block_env, tx_env};

use revm::{
    db::CacheDB, inspectors::TracerEip3155, primitives::ResultAndState as RevmResultAndState,
    Evm as Revm,
};

fn main() {
    // Read the input
    let head_block_bytes = env::read::<Vec<u8>>();
    let parent_header_bytes = env::read::<Vec<u8>>();
    //let execution_db = env::read::<ExecutionDB>();

    // SetUp data from inputs
    let block = <ethereum_rust_core::types::Block as ethereum_rust_rlp::decode::RLPDecode>::decode(
        &head_block_bytes,
    )
    .unwrap();

    let parent_header =
        <ethereum_rust_core::types::BlockHeader as ethereum_rust_rlp::decode::RLPDecode>::decode(
            &parent_header_bytes,
        )
        .unwrap();

    // Make DataInputs public.
    env::commit(&block);
    env::commit(&parent_header);
    //env::commit(&execution_db);

    // SetUp CacheDB in order to use execute_block()
    //let mut cache_db = CacheDB::new(execution_db);
    println!("executing block");

    //let block_receipts = execute_block(&block, &mut cache_db).unwrap();
    // TODO
    // Handle the case in which the gas used differs and throws an error.
    // Should the zkVM panic? Should it generate a dummy proof?
    // Private function
    //let _ = validate_gas_used(&block_receipts, &block.header);

    //env::commit(&block_receipts);
}

// Modified from ethereum_rust-vm
/*
fn execute_block(
    block: &ethereum_rust_core::types::Block,
    db: &mut CacheDB<ExecutionDB>,
) -> Result<Vec<Receipt>, ethereum_rust_vm::EvmError> {
    let spec_id = revm::primitives::SpecId::CANCUN;
    let mut receipts = Vec::new();
    let mut cumulative_gas_used = 0;

    for transaction in block.body.transactions.iter() {
        let result = execute_tx(transaction, &block.header, db, spec_id)?;
        cumulative_gas_used += result.gas_used();
        let receipt = Receipt::new(
            transaction.tx_type(),
            result.is_success(),
            cumulative_gas_used,
            result.logs(),
        );
        receipts.push(receipt);
    }

    Ok(receipts)
}

// Modified from ethereum_rust-vm
fn execute_tx(
    transaction: &Transaction,
    block_header: &ethereum_rust_core::types::BlockHeader,
    db: &mut CacheDB<ExecutionDB>,
    spec_id: revm::primitives::SpecId,
) -> Result<ethereum_rust_vm::ExecutionResult, ethereum_rust_vm::EvmError> {
    let block_env = block_env(block_header);
    let tx_env = tx_env(transaction);
    run_evm(tx_env, block_env, db, spec_id)
        .map(Into::into)
        .map_err(ethereum_rust_vm::EvmError::from)
}

// Modified from ethereum_rust-vm
fn run_evm(
    tx_env: revm::primitives::TxEnv,
    block_env: revm::primitives::BlockEnv,
    db: &mut CacheDB<ExecutionDB>,
    spec_id: revm::primitives::SpecId,
) -> Result<ethereum_rust_vm::ExecutionResult, ethereum_rust_vm::EvmError> {
    // let chain_spec = db.get_chain_config()?;
    let mut evm = Revm::builder()
        .with_db(db)
        .with_block_env(block_env)
        .with_tx_env(tx_env)
        // If the chain_id is not correct, it throws:
        // Transaction(InvalidChainId)
        // TODO: do not hardcode the chain_id
        .modify_cfg_env(|cfg| cfg.chain_id = 1729)
        .with_spec_id(spec_id)
        .with_external_context(TracerEip3155::new(Box::new(std::io::stderr())).without_summary())
        .build();
    let RevmResultAndState { result, state: _ } = evm.transact().unwrap();
    Ok(result.into())
}
*/
