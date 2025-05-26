use crate::cache::Cache;
use ethrex_common::types::ELASTICITY_MULTIPLIER;
use ethrex_prover_lib::ProveOutput;
use zkvm_interface::io::ProgramInput;

pub async fn exec(cache: Cache) -> eyre::Result<ProveOutput> {
    let Cache {
        block,
        parent_block_header,
        db,
    } = cache;
    let out = ethrex_prover_lib::execution_program(ProgramInput {
        blocks: vec![block],
        parent_block_header,
        db,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
    })
    .map_err(|e| eyre::Error::msg(e.to_string()))?;
    Ok(ProveOutput(out))
}

pub async fn prove(cache: Cache) -> eyre::Result<ProveOutput> {
    let Cache {
        block,
        parent_block_header,
        db,
    } = cache;
    ethrex_prover_lib::prove(ProgramInput {
        blocks: vec![block],
        parent_block_header,
        db,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
    })
    .map_err(|e| eyre::Error::msg(e.to_string()))
}
