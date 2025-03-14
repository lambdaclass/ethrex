# risc0

.PHONY: perf-risc0 perf-risc0-gpu 
RUST_LOG?="info"

perf-risc0:
	RUST_LOG=${RUST_LOG} cargo test --release --test perf_zkvm --features "risc0,l2" -- test_performance_zkvm --exact --show-output

perf-risc0-gpu:
	RUSTFLAGS="-C target-cpu=native" RISC0_DEV_MODE=0 RUST_LOG=${RUST_LOG} cargo test --release --test perf_zkvm --features "risc0,l2,gpu" -- test_performance_zkvm --exact --show-output

# sp1

.PHONY: perf-sp1 perf-sp1-gpu 
RUST_LOG?="info"

perf-sp1:
	@echo "Using SP1_PROVER: ${SP1_PROVER}"
	RUST_LOG=${RUST_LOG} cargo test --release --test perf_zkvm --features "sp1,l2" -- test_performance_zkvm --exact --show-output
	
perf-sp1-gpu:
	@echo "Using SP1_PROVER: ${SP1_PROVER}"
	RUST_LOG=${RUST_LOG} cargo test --release --test perf_zkvm --features "sp1,l2,gpu" -- test_performance_zkvm --exact --show-output

# pico

.PHONY: perf-pico perf-pico-gpu 
RUST_LOG?="info"

perf-pico:
	RUST_LOG=${RUST_LOG} cargo +nightly-2024-11-27 test --release --test perf_zkvm --features "pico,l2" -- test_performance_zkvm --exact --show-output
	
perf-pico-gpu:
	RUST_LOG=${RUST_LOG} cargo +nightly-2024-11-27 test --release --test perf_zkvm --features "pico,l2,gpu" -- test_performance_zkvm --exact --show-output
