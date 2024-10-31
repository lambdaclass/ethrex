RISC0_DEV_MODE?=1
RUST_LOG?="debug"
perf_test_proving:
	@echo "Using RISC0_DEV_MODE: ${RISC0_DEV_MODE}"
	RISC0_DEV_MODE=${RISC0_DEV_MODE} RUST_LOG=${RUST_LOG} cargo test --release --test perf_zkvm --features build_zkvm -- --show-output
.PHONY: perf_test_proving

perf_gpu:
	RUSTFLAGS="-C target-cpu=native" RISC0_DEV_MODE=0 RUST_LOG="debug" cargo test --release --test perf_zkvm --features "build_zkvm,gpu" -- --show-output
.PHONY: perf_gpu
