use ethrex_prover::backend::{ZiskAirCost, ZiskBackend};
use sha2::{Digest, Sha256};

use crate::cache::{cache_to_program_input, load_cache};
use crate::manifest::{RunMode, WorkloadSpec, WorkloadType, load_manifest};
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

/// Scans `dir` for `*.json` / `*.json.gz` files and turns each into a
/// `stress` `WorkloadSpec` (name = filename, no category/gas/tier). `stress`
/// workloads load via the Cache loader (see `WorkloadType::Stress` in
/// `run_bench`), so `dir` must contain `generate-stress`-produced Cache-format
/// fixtures. Used by `--mode slow --stress-dir <dir>` to sweep an external
/// directory of generated stress fixtures without listing them in the
/// manifest.
fn discover_stress_workloads(dir: &str) -> eyre::Result<Vec<WorkloadSpec>> {
    let mut specs = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy().to_string();
        if !(name.ends_with(".json.gz") || name.ends_with(".json")) {
            continue;
        }
        specs.push(WorkloadSpec {
            name: name.clone(),
            r#type: WorkloadType::Stress,
            category: None,
            source: entry.path().to_string_lossy().to_string(),
            gas: None,
            tier: None,
        });
    }
    Ok(specs)
}

pub fn run_bench(
    workloads: &str,
    filter: Option<&str>,
    out: &str,
    _elf: Option<&str>,
    mode: &str,
    stress_dir: Option<&str>,
) -> eyre::Result<()> {
    let run_mode = RunMode::parse(mode)?;
    let manifest = load_manifest(workloads)?;
    let backend = ZiskBackend::new();
    let mut results = Vec::new();

    let mut specs: Vec<WorkloadSpec> = manifest
        .filtered_for_run(filter, run_mode)
        .into_iter()
        .cloned()
        .collect();
    if let (RunMode::Slow, Some(dir)) = (run_mode, stress_dir) {
        specs.extend(discover_stress_workloads(dir)?);
    }

    for spec in &specs {
        // Build the input and execute inside a single fallible closure so that a
        // bad fixture (missing/corrupt cache) or an unimplemented micro workload
        // fails just this workload (it still lands in the report with
        // `guest_output_ok: false`) instead of aborting the whole run.
        let result: eyre::Result<ZiskAirCost> = (|| {
            let input = match spec.r#type {
                WorkloadType::RealBlock => cache_to_program_input(load_cache(&spec.source)?)?,
                WorkloadType::Micro => {
                    crate::micro::micro_to_program_input(&spec.source, spec.gas)?
                }
                WorkloadType::Stress => cache_to_program_input(load_cache(&spec.source)?)?,
            };
            backend
                .execute_profiled(input)
                .map_err(|e| eyre::eyre!("{e}"))
        })();
        let (air, steps, ram, ok) = match result {
            Ok(z) => (to_air_cost(&z), z.steps, z.ram_usage, true),
            Err(e) => {
                eprintln!("workload {} failed: {e}", spec.name);
                (AirCost::default(), 0u64, 0u64, false)
            }
        };
        results.push(WorkloadResult {
            name: spec.name.clone(),
            r#type: match spec.r#type {
                WorkloadType::RealBlock => "real-block",
                WorkloadType::Micro => "micro",
                WorkloadType::Stress => "stress",
            }
            .into(),
            category: spec.category.clone(),
            gas: spec.gas,
            air_cost: air,
            steps,
            zkvm_ram_bytes: ram,
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
