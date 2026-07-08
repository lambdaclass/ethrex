use ethrex_prover::backend::{ZiskAirCost, ZiskBackend};
use sha2::{Digest, Sha256};

use crate::cache::{cache_to_program_input, load_cache};
use crate::manifest::{WorkloadType, load_manifest};
use crate::report::{AirCost, Meta, Report, WorkloadResult};

fn guest_elf_sha256() -> String {
    let mut h = Sha256::new();
    h.update(ethrex_guest_program::ZKVM_ZISK_PROGRAM_ELF);
    hex::encode(h.finalize())
}

fn to_air_cost(z: &ZiskAirCost) -> AirCost {
    AirCost {
        main: z.main,
        opcodes: z.opcodes,
        precompiles: z.precompiles,
        memory: z.memory,
        base: z.base,
        total: z.total,
    }
}

pub fn run_bench(
    workloads: &str,
    filter: Option<&str>,
    out: &str,
    _elf: Option<&str>,
) -> eyre::Result<()> {
    let manifest = load_manifest(workloads)?;
    let backend = ZiskBackend::new();
    let mut results = Vec::new();

    for spec in manifest.filtered(filter) {
        let input = match spec.r#type {
            WorkloadType::RealBlock => {
                let cache = load_cache(&spec.source)?;
                cache_to_program_input(cache)?
            }
            WorkloadType::Micro => crate::micro::micro_to_program_input(&spec.source, spec.gas)?,
        };
        let (air, steps, ok) = match backend.execute_profiled(input) {
            Ok(z) => (to_air_cost(&z), z.steps, true),
            Err(e) => {
                eprintln!("workload {} failed: {e}", spec.name);
                (AirCost::default(), 0u64, false)
            }
        };
        results.push(WorkloadResult {
            name: spec.name.clone(),
            r#type: match spec.r#type {
                WorkloadType::RealBlock => "real-block",
                WorkloadType::Micro => "micro",
            }
            .into(),
            category: spec.category.clone(),
            gas: spec.gas,
            air_cost: air,
            steps,
            guest_output_ok: ok,
        });
    }

    let report = Report {
        meta: Meta {
            zisk_version: "v0.16.1".into(),
            guest_elf_sha256: guest_elf_sha256(),
            generated_by: "ethrex-zkevm-bench".into(),
            git_commit: std::env::var("GIT_COMMIT").ok(),
        },
        workloads: results,
    };
    report.write_json(out)?;
    println!("wrote {out} ({} workloads)", report.workloads.len());
    Ok(())
}
