use crate::cache::Cache;
use ethrex_common::types::ELASTICITY_MULTIPLIER;
use zkvm_interface::io::ProgramInput;

pub async fn exec(cache: Cache) -> eyre::Result<String> {
    let Cache {
        block,
        parent_block_header,
        db,
    } = cache;
    #[cfg(any(feature = "sp1", feature = "risc0", feature = "pico"))]
    {
        ethrex_prover_lib::execute(ProgramInput {
            blocks: vec![block],
            parent_block_header,
            db,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            // The L2 specific fields (state_diff, blob_commitment, blob_proof)
            // will be filled by Default::default() if the 'l2' feature of
            // 'zkvm_interface' is active (due to workspace compilation).
            // If 'zkvm_interface' is compiled without 'l2' (e.g. standalone build),
            // these fields won't exist in ProgramInput, and ..Default::default()
            // will correctly not try to fill them.
            // A better solution would involve rethinking the `l2` feature or the
            // inclusion of this crate in the workspace.
            ..Default::default()
        })
        .map_err(|e| eyre::Error::msg(e.to_string()))?;
        Ok("".to_string())
    }
    #[cfg(not(any(feature = "sp1", feature = "risc0", feature = "pico")))]
    {
        let out = ethrex_prover_lib::execution_program(ProgramInput {
            blocks: vec![block],
            parent_block_header,
            db,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            // The L2 specific fields (state_diff, blob_commitment, blob_proof)
            // will be filled by Default::default() if the 'l2' feature of
            // 'zkvm_interface' is active (due to workspace compilation).
            // If 'zkvm_interface' is compiled without 'l2' (e.g. standalone build),
            // these fields won't exist in ProgramInput, and ..Default::default()
            // will correctly not try to fill them.
            // A better solution would involve rethinking the `l2` feature or the
            // inclusion of this crate in the workspace.
            ..Default::default()
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
            // The L2 specific fields (state_diff, blob_commitment, blob_proof)
            // will be filled by Default::default() if the 'l2' feature of
            // 'zkvm_interface' is active (due to workspace compilation).
            // If 'zkvm_interface' is compiled without 'l2' (e.g. standalone build),
            // these fields won't exist in ProgramInput, and ..Default::default()
            // will correctly not try to fill them.
            // A better solution would involve rethinking the `l2` feature or the
            // inclusion of this crate in the workspace.
            ..Default::default()
    })
    .map_err(|e| eyre::Error::msg(e.to_string()))?;
    #[cfg(feature = "sp1")]
    return Ok(format!("{out:#?}"));
    #[cfg(not(feature = "sp1"))]
    Ok(serde_json::to_string(&out.0)?)
}
