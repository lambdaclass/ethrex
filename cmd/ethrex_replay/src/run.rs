use crate::cache::Cache;
use ethrex_common::types::ELASTICITY_MULTIPLIER;
use ethrex_prover_lib::ProveOutput;
use zkvm_interface::io::ProgramInput;

pub async fn exec(cache: Cache) -> eyre::Result<String> {
    let Cache {
        block,
        parent_block_header,
        db,
    } = cache;
    if cfg!(feature = "sp1") || cfg!(feature = "risc0") || cfg!(feature = "pico") {
        ethrex_prover_lib::execute(ProgramInput {
            blocks: vec![block],
            parent_block_header,
            db,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
        })
        .map_err(|e| eyre::Error::msg(e.to_string()))?;
        Ok("".to_string())
    } else {
        let out = ethrex_prover_lib::execution_program(ProgramInput {
            blocks: vec![block],
            parent_block_header,
            db,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
        })
        .map_err(|e| eyre::Error::msg(e.to_string()))?;
        Ok(serde_json::to_string(&out)?)
    }
}

pub async fn prove(cache: Cache) -> eyre::Result<String> {
    let Cache {
        block,
        parent_block_header,
        db,
    } = cache;
    let out = ethrex_prover_lib::prove(ProgramInput {
        blocks: vec![block],
        parent_block_header,
        db,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
    })
    .map_err(|e| eyre::Error::msg(e.to_string()))?;
    if cfg!(feature = "sp1") {
        Ok(format!("{out:#?}"))
    } else if cfg!(feature = "risc0") {
        todo!()
    } else if cfg!(feature = "pico") {
        todo!()
    } else {
        Err(eyre::Error::msg("Exec can't prove."))
    }
}
