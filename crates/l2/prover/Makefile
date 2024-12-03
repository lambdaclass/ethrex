.PHONY: perf_test_proving perf_gpu rsp_comparison
.PHONY: perf_test_sp1_proving

ROOT_DIRECTORY := ../../..

########################################################################
###############                 RISC0                    ###############
########################################################################
RISC0_DEV_MODE?=1
RUST_LOG?="info"
perf_test_proving:
	@echo "Using RISC0_DEV_MODE: ${RISC0_DEV_MODE}"
	RISC0_DEV_MODE=${RISC0_DEV_MODE} RUST_LOG=${RUST_LOG} cargo test --release --test perf_zkvm --features build_zkvm -- test_performance_zkvm --exact --show-output

perf_gpu:
	RUSTFLAGS="-C target-cpu=native" RISC0_DEV_MODE=0 RUST_LOG="debug" cargo test --release --test perf_zkvm --features "build_zkvm,gpu" -- test_performance_zkvm --exact --show-output


########################################################################
###############                  SP1                     ###############
########################################################################
SP1_PROVER?=mock
RUST_LOG?="debug"
perf_test_sp1_proving:
	@echo "Using SP1_PROVER: ${SP1_PROVER}"
	SP1_PROVER=${SP1_PROVER} RUST_LOG=${RUST_LOG} cargo test --release --test perf_zkvm --features build_sp1 -- test_performance_sp1_zkvm --exact --show-output

########################################################################
########## L2 Prover comparison with rsp. Uses GPU by default ##########
########################################################################
ETHREX_L2_BIN := ./target/release/ethrex_l2
RSP_BIN := ./target/release/rsp

GENESIS_FILE := $(ROOT_DIRECTORY)/test_data/genesis-l2-old.json
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
	CARGO_TARGET_DIR=target cargo build -r --manifest-path $(ROOT_DIRECTORY)/Cargo.toml --bin ethrex_l2 --features "build_zkvm,gpu,disable-dev-mode"

$(RSP_BIN):
	- git clone https://github.com/succinctlabs/rsp.git
	cd rsp; \
	CARGO_TARGET_DIR=../target cargo build -r --bin rsp --features cuda
