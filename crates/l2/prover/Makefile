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

########################################################################
########## L2 Prover comparison with rsp. Uses GPU by default ##########
########################################################################
ROOT_DIRECTORY := ../../..

ETHREX_L2_BIN := ./target/release/ethrex_l2
RSP_BIN := ./target/release/rsp

GENESIS_FILE := $(ROOT_DIRECTORY)/test_data/genesis-l2.json
CHAIN_FILE := $(ROOT_DIRECTORY)/test_data/l2-loadtest.rlp
RSP_CACHE := $(ROOT_DIRECTORY)/test_data/rsp

rsp_comparison: $(ETHREX_L2_BIN) $(RSP_BIN)
	@echo "rsp times (L1 Mainnet block, 24 MGas):"
	@time $(RSP_BIN) --prove \
		--chain-id 1 \
		--block-number 21272632 \
		--cache-dir $(RSP_CACHE) \
		>/dev/null
	@echo ""
	@echo "ethrex_l2 times (L2 block, 25 MGas):"
	@RISC0_DEV_MODE=false time $(ETHREX_L2_BIN) prove \
		--genesis $(GENESIS_FILE) \
		--chain $(CHAIN_FILE) \
		--block-number 2 \
		>/dev/null

$(ETHREX_L2_BIN):
	CARGO_TARGET_DIR=target cargo build -r --manifest-path $(ROOT_DIRECTORY)/Cargo.toml --bin ethrex_l2 --features "build_risc0,gpu,disable-dev-mode"

$(RSP_BIN):
	- git clone https://github.com/succinctlabs/rsp.git
	cd rsp; \
	CARGO_TARGET_DIR=../target cargo build -r --bin rsp --features cuda
