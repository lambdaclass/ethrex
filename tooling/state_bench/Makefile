# A/B cold-state benchmark recipe for `state-bench`.
#
# The 4-step workflow (see README.md for full detail):
#   1. `make gen-state`     — build the synthetic state fixture ONCE.
#   2. `make gen-workload`  — build the real-block workload (chain.rlp + bals.rlp)
#                             on top of that fixture, ONCE.
#   3. A/B run:
#        git checkout <branch-A>
#        make run-a          # builds state-bench from the checked-out tree,
#                             # copies it to BIN_A, runs it against the SAME
#                             # fixture + workload, writes OUT_A
#        git checkout <branch-B>
#        make run-b          # same, but into BIN_B / OUT_B
#   4. `make compare`      — diff OUT_A vs OUT_B.
#
# `gen-state` / `gen-workload` only need to run once; `run-a` / `run-b` can be
# repeated (e.g. after further branch changes) against the same fixture and
# workload as long as both sides use the same --jobs (see README "A/B caveats").
#
# All parameters are overridable via environment / `make VAR=value`.

REPO_ROOT := $(abspath ../..)
TOOLING_DIR := $(abspath ..)
CARGO_MANIFEST := $(TOOLING_DIR)/Cargo.toml
BUILT_BIN := $(TOOLING_DIR)/target/release/state-bench

# --- gen-state parameters ----------------------------------------------------
SEED ?= 42
NUM_SMALL_ACCOUNTS ?= 1000
SLOTS_PER_ACCOUNT ?= 8
# A real (non-smoke) run wants several GB in the mega account so its working
# set does not fit in RAM; lower for a quick smoke test (e.g. MEGA_GB=0.05).
MEGA_GB ?= 4.0
# RocksDB block cache (GiB) used while building. Default 1 GiB is fine for small
# fixtures; for large ones raise it toward ~half MEGA_GB so the shards' per-chunk
# re-reads stay cached (peak RSS ~= 9 GiB + this; keep a few GiB under RAM).
BLOCK_CACHE_GB ?= 1.0

# --- gen-workload parameters --------------------------------------------------
# A real run wants >= 1000 blocks so cold-read/write costs dominate; lower for
# a smoke test (e.g. NUM_BLOCKS=20).
NUM_BLOCKS ?= 2000
READS ?= 8
WRITES ?= 4
MEGA_FRACTION ?= 0.5

# --- run parameters ------------------------------------------------------------
RUNS ?= 5
# The parallel BAL merkleizer needs >= 17 worker threads or it deadlocks; `run`
# clamps anything lower and warns. Raise this on a bigger machine.
JOBS ?= 17
# checkpoint (default): reproducible, never mutates DATADIR. undo: faster/less
# disk but mutates DATADIR in place and adds cache-miss variance; a failed run
# leaves DATADIR dirty (regenerate with gen-state). See README.
RESET ?= checkpoint

# --- shared genesis + artifact paths -----------------------------------------
GENESIS ?= $(REPO_ROOT)/fixtures/genesis/l1-bal.json
DATA_DIR ?= $(HOME)/.local/share/state-bench
DATADIR ?= $(DATA_DIR)/state
CHAIN ?= $(DATA_DIR)/chain.rlp
BALS ?= $(DATA_DIR)/bals.rlp
OUT_A ?= $(DATA_DIR)/out-a.log
OUT_B ?= $(DATA_DIR)/out-b.log
BIN_A ?= $(DATA_DIR)/bin/state-bench-a
BIN_B ?= $(DATA_DIR)/bin/state-bench-b

.PHONY: help gen-state gen-workload run-a run-b compare clean

help: ## Show the A/B recipe and every parameter.
	@printf "state-bench A/B recipe:\n\n"
	@printf "  1. make gen-state       # build the synthetic fixture (once)\n"
	@printf "  2. make gen-workload    # build chain.rlp + bals.rlp on top of it (once)\n"
	@printf "  3. git checkout <A>; make run-a   # build + run branch A\n"
	@printf "     git checkout <B>; make run-b   # build + run branch B (same fixture+workload)\n"
	@printf "  4. make compare         # diff OUT_A vs OUT_B\n\n"
	@printf "Requirements: A and B must share the same ethrex-storage schema version\n"
	@printf "and the same JOBS/fixture/workload (see README.md).\n\n"
	@printf "Parameters (env or make VAR=value):\n"
	@printf "  SEED=%s NUM_SMALL_ACCOUNTS=%s SLOTS_PER_ACCOUNT=%s MEGA_GB=%s\n" "$(SEED)" "$(NUM_SMALL_ACCOUNTS)" "$(SLOTS_PER_ACCOUNT)" "$(MEGA_GB)"
	@printf "  NUM_BLOCKS=%s READS=%s WRITES=%s MEGA_FRACTION=%s\n" "$(NUM_BLOCKS)" "$(READS)" "$(WRITES)" "$(MEGA_FRACTION)"
	@printf "  RUNS=%s JOBS=%s RESET=%s GENESIS=%s\n" "$(RUNS)" "$(JOBS)" "$(RESET)" "$(GENESIS)"
	@printf "  DATADIR=%s CHAIN=%s BALS=%s\n" "$(DATADIR)" "$(CHAIN)" "$(BALS)"
	@printf "  OUT_A=%s OUT_B=%s\n" "$(OUT_A)" "$(OUT_B)"

gen-state: ## Build the synthetic state fixture (run once; shared by A and B).
	rm -rf "$(DATADIR)"
	mkdir -p "$(DATA_DIR)"
	cargo run --release --manifest-path "$(CARGO_MANIFEST)" -p state_bench --bin state-bench -- \
		gen-state \
		--datadir "$(DATADIR)" \
		--seed $(SEED) \
		--num-small-accounts $(NUM_SMALL_ACCOUNTS) \
		--slots-per-account $(SLOTS_PER_ACCOUNT) \
		--mega-account-gb $(MEGA_GB) \
		--block-cache-gb $(BLOCK_CACHE_GB) \
		--genesis "$(GENESIS)"

gen-workload: ## Build chain.rlp + bals.rlp on top of the fixture (run once; shared by A and B).
	cargo run --release --manifest-path "$(CARGO_MANIFEST)" -p state_bench --bin state-bench -- \
		gen-workload \
		--datadir "$(DATADIR)" \
		--out-chain "$(CHAIN)" \
		--out-bals "$(BALS)" \
		--num-blocks $(NUM_BLOCKS) \
		--reads-per-block $(READS) \
		--writes-per-block $(WRITES) \
		--mega-fraction $(MEGA_FRACTION) \
		--genesis "$(GENESIS)"

run-a: ## Build state-bench from the currently checked-out tree and run it as branch A.
	mkdir -p "$(dir $(BIN_A))"
	cargo build --release --manifest-path "$(CARGO_MANIFEST)" -p state_bench
	cp "$(BUILT_BIN)" "$(BIN_A)"
	"$(BIN_A)" run \
		--jobs $(JOBS) \
		--datadir "$(DATADIR)" \
		--chain "$(CHAIN)" \
		--bals "$(BALS)" \
		--genesis "$(GENESIS)" \
		--runs $(RUNS) \
		--reset $(RESET) \
		--out-log "$(OUT_A)"

run-b: ## Build state-bench from the currently checked-out tree and run it as branch B.
	mkdir -p "$(dir $(BIN_B))"
	cargo build --release --manifest-path "$(CARGO_MANIFEST)" -p state_bench
	cp "$(BUILT_BIN)" "$(BIN_B)"
	"$(BIN_B)" run \
		--jobs $(JOBS) \
		--datadir "$(DATADIR)" \
		--chain "$(CHAIN)" \
		--bals "$(BALS)" \
		--genesis "$(GENESIS)" \
		--runs $(RUNS) \
		--reset $(RESET) \
		--out-log "$(OUT_B)"

compare: ## Diff OUT_A vs OUT_B.
	cargo run --release --manifest-path "$(CARGO_MANIFEST)" -p state_bench --bin state-bench -- \
		compare "$(OUT_A)" "$(OUT_B)"

clean: ## Remove the generated fixture, workload, logs, and per-branch binaries under DATA_DIR.
	rm -rf "$(DATA_DIR)"
