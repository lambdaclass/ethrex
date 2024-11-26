.PHONY: perf_test_proving perf_gpu rsp-comparison build-rsp-comparison

RISC0_DEV_MODE?=1
RUST_LOG?="info"
perf_test_proving:
	@echo "Using RISC0_DEV_MODE: ${RISC0_DEV_MODE}"
	RISC0_DEV_MODE=${RISC0_DEV_MODE} RUST_LOG=${RUST_LOG} cargo test --release --test perf_zkvm --features build_zkvm -- --show-output

perf_gpu:
	RUSTFLAGS="-C target-cpu=native" RISC0_DEV_MODE=0 RUST_LOG="debug" cargo test --release --test perf_zkvm --features "build_zkvm,gpu" -- --show-output

# Benchmarks
rsp_comparison: build-rsp-comparison
	cd bench/target/release; \
	hyperfine --runs 5 --warmup 3 './ethrexl2' './rsp'

build_rsp_comparison:
	CARGO_TARGET_DIR=bench/target cargo build --manifest-path bench/Cargo.toml --release
