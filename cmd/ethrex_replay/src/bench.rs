use serde_json::json;
use std::fs::File;
use tracing::info;

pub fn write_benchmark_file(gas_used: f64, elapsed: f64) {
    let rate = gas_used / 1e6 / elapsed;

    let backend = if cfg!(feature = "sp1") {
        "SP1"
    } else if cfg!(feature = "risc0") {
        "Risc0"
    } else {
        "Exec"
    };

    let processor = if cfg!(feature = "ci") {
        "RTX A6000"
    } else if cfg!(feature = "gpu") {
        "GPU"
    } else {
        "CPU"
    };

    let benchmark_json = &json!([{
        "name": format!("{backend}, {}", processor),
        "unit": "Mgas/s",
        "value": rate
    }]);
    let file = File::create("bench_latest.json").expect("failed to create bench_latest.json");
    serde_json::to_writer(file, benchmark_json).expect("failed to write to bench_latest.json");
}

pub async fn run_and_measure(
    run: impl Future<Output = eyre::Result<f64>>,
    write_to_file: bool,
) -> eyre::Result<()> {
    info!("Starting prover program");
    let now = std::time::Instant::now();
    let gas_used = run.await?;
    let elapsed = now.elapsed().as_secs();
    if write_to_file {
        write_benchmark_file(gas_used, elapsed as f64);
    }
    Ok(())
}
