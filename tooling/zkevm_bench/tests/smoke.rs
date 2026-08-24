//! End-to-end smoke test for the `ethrex-zkevm-bench` binary.
//!
//! This is only meaningful when the binary under test was built with
//! `--features zisk-elf` (so the zisk guest ELF is actually embedded); the
//! test itself doesn't set that feature, since `cargo test` builds the crate
//! once and this file only shells out to whatever binary Cargo already
//! produced (via `CARGO_BIN_EXE_ethrex-zkevm-bench`). Run
//! `cargo test -p ethrex-zkevm-bench --features zisk-elf` to exercise it for
//! real.
//!
//! It also skips (does not fail) unless `ziskemu` is installed and on
//! `PATH`, so `cargo test` stays green on machines without the ZisK
//! toolchain.

use std::process::Command;

#[test]
fn smoke_run_one_light_block() {
    if Command::new("ziskemu").arg("--version").output().is_err() {
        eprintln!("skipping smoke: ziskemu not installed");
        return;
    }

    let bin = env!("CARGO_BIN_EXE_ethrex-zkevm-bench");
    let out_path = std::env::temp_dir().join("zkevm_bench_smoke.json");
    let status = Command::new(bin)
        .args([
            "run",
            "--workloads",
            "fixtures/manifest.toml",
            "--filter",
            "light",
            "--out",
            out_path.to_str().unwrap(),
        ])
        .status()
        .expect("run the bench binary");
    assert!(status.success(), "bench run exited non-zero");

    let json = std::fs::read_to_string(&out_path).expect("read report");
    let report: serde_json::Value = serde_json::from_str(&json).unwrap();
    let wl = &report["workloads"][0];
    assert_eq!(wl["guest_output_ok"], serde_json::json!(true));
    assert!(wl["air_cost"]["total"].as_u64().unwrap() > 0);
    assert!(wl["steps"].as_u64().unwrap() > 0);
}
