.PHONY: all test clippy fmt usage lint eth-tests run-evm-ef-tests flamegraph-run-ef-tests samply-run-ef-tests render-benches

all: test clippy fmt ## 🚀 Runs all tests, linter and formatter

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

test: ## 🧪 Runs all tests except Ethereum tests
	cargo test -p ethrex-levm

lint: ## 🧹 Linter check
	cargo clippy --all-targets --all-features -- -D warnings

fmt: ## 📄 Runs rustfmt
	cargo fmt --all

###### EF Tests ######
download-evm-ef-tests: ## 📥 Download EF Tests
	cd ../../../cmd/ef_tests/state && \
	make download-evm-ef-tests flags="$(flags)"

run-evm-ef-tests: ## 🏃‍♂️ Run EF Tests
	cd ../../../cmd/ef_tests/state && \
	make run-evm-ef-tests flags="$(flags)"

###### Benchmarks ######
REPETITIONS := 10000
REPETITIONS_SLOW := 200

BENCH_FACT_ITERATIONS := 57
BENCH_FACT_REC_ITERATIONS := 57
BENCH_FIB_ITERATIONS := 57
BENCH_FIB_REC_ITERATIONS := 15
BENCH_HASHES_ITERATIONS := 57
BENCH_BUBBLESORT_ITERATIONS := 100 # Size of the array to sort
BENCH_MINT_ITERATIONS := 500
BENCH_TRANSFER_ITERATIONS := 500
BENCH_APPROVAL_ITERATIONS := 500
BENCH_PUSH_ITERATIONS := 0 # unused, fixed size array to use stack
BENCH_MSTOREBENCH_ITERATIONS := 0 # unused, fixed size array to use stack

# For manually testing you could add --show-output to hyperfine
define run_benchmark
	mkdir -p ./bench-results
	@printf "%s\n" "revm_$(1)"
	@target/release/benchmark revm $(1) 1 $($(3))
	@printf "%s\n" "levm_$(1)"
	@target/release/benchmark levm $(1) 1 $($(3))
	hyperfine -w 5 -r 10 -N --export-markdown ./bench-results/$(1).md --export-json ./bench-results/$(1).json \
		-n "revm_$(1)" "target/release/benchmark revm $(1) $($(2)) $($(3))" \
		-n "levm_$(1)" "target/release/benchmark levm $(1) $($(2)) $($(3))"
	@echo
endef

define run_benchmark_ci
	@printf "%s\n" "main_revm$(1)"
	@../../../main/benchmark revm $(1) 1 $($(3))
	@printf "%s\n" "main_levm$(1)"
	@../../../main/benchmark levm $(1) 1 $($(3))
	@printf "%s\n" "pr_revm$(1)"
	@../../../pr/benchmark revm $(1) 1 $($(3))
	@printf "%s\n" "pr_levm_$(1)"
	@../../../pr/benchmark levm $(1) 1 $($(3))
	hyperfine -w 5 -r 10 -N --export-markdown ../../../benchmark_comparison_results/$(1).md \
		-n "main_revm_$(1)" "../../../main/benchmark revm $(1) $($(2)) $($(3))" \
		-n "main_levm_$(1)" "../../../main/benchmark levm $(1) $($(2)) $($(3))" \
		-n "pr_revm_$(1)" "../../../pr/benchmark revm $(1) $($(2)) $($(3))" \
		-n "pr_levm_$(1)" "../../../pr/benchmark levm $(1) $($(2)) $($(3))"
	@echo
endef

compile-contracts:
	cd ../../../ && \
	cargo run --package revm_comparison \
	--bin compile \
	--manifest-path crates/vm/levm/bench/revm_comparison/Cargo.toml

revm-comparison: compile-contracts ## 📊 Run benchmarks of fibonacci and factorial for both REVM and LEVM
	$(MAKE) build-revm-comparison
	$(call run_benchmark,MstoreBench,REPETITIONS,BENCH_MSTOREBENCH_ITERATIONS)
	$(call run_benchmark,Fibonacci,REPETITIONS,BENCH_FIB_ITERATIONS)
	$(call run_benchmark,FibonacciRecursive,REPETITIONS_SLOW,BENCH_FIB_REC_ITERATIONS)
	$(call run_benchmark,Factorial,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_benchmark,FactorialRecursive,REPETITIONS,BENCH_FACT_REC_ITERATIONS)
	$(call run_benchmark,Push,REPETITIONS,BENCH_PUSH_ITERATIONS)
	$(call run_benchmark,ManyHashes,REPETITIONS_SLOW,BENCH_HASHES_ITERATIONS)
	$(call run_benchmark,BubbleSort,REPETITIONS_SLOW,BENCH_BUBBLESORT_ITERATIONS)
	$(call run_benchmark,ERC20Approval,REPETITIONS_SLOW,BENCH_APPROVAL_ITERATIONS)
	$(call run_benchmark,ERC20Transfer,REPETITIONS_SLOW,BENCH_TRANSFER_ITERATIONS)
	$(call run_benchmark,ERC20Mint,REPETITIONS_SLOW,BENCH_MINT_ITERATIONS)

revm-comparison-ci: compile-contracts
	mkdir -p ../../../benchmark_comparison_results
	$(call run_benchmark_ci,Fibonacci,REPETITIONS,BENCH_FIB_ITERATIONS)
	$(call run_benchmark_ci,FibonacciRecursive,REPETITIONS_SLOW,BENCH_FIB_REC_ITERATIONS)
	$(call run_benchmark_ci,Factorial,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_benchmark_ci,FactorialRecursive,REPETITIONS,BENCH_FACT_ITERATIONS)
	$(call run_benchmark_ci,Push,REPETITIONS,BENCH_PUSH_ITERATIONS)
	$(call run_benchmark_ci,MstoreBench,REPETITIONS,BENCH_MSTOREBENCH_ITERATIONS)
	$(call run_benchmark_ci,ManyHashes,REPETITIONS_SLOW,BENCH_HASHES_ITERATIONS)
	$(call run_benchmark_ci,BubbleSort,REPETITIONS_SLOW,BENCH_BUBBLESORT_ITERATIONS)
	$(call run_benchmark_ci,ERC20Approval,REPETITIONS_SLOW,BENCH_APPROVAL_ITERATIONS)
	$(call run_benchmark_ci,ERC20Transfer,REPETITIONS_SLOW,BENCH_TRANSFER_ITERATIONS)
	$(call run_benchmark_ci,ERC20Mint,REPETITIONS_SLOW,BENCH_MINT_ITERATIONS)

render-benches:
	python3 ./bench/revm_comparison/create_bench_page.py > report.html

flamegraph-levm-benchmark:
	$(MAKE) flamegraph-levm-bench-generic name=Fibonacci repetitions=$(REPETITIONS) iterations=$(BENCH_FIB_ITERATIONS)
	$(MAKE) flamegraph-levm-bench-generic name=FibonacciRecursive repetitions=$(REPETITIONS_SLOW) iterations=$(BENCH_FIB_REC_ITERATIONS)
	$(MAKE) flamegraph-levm-bench-generic name=Factorial repetitions=$(REPETITIONS) iterations=$(BENCH_FACT_ITERATIONS)
	$(MAKE) flamegraph-levm-bench-generic name=FactorialRecursive repetitions=$(REPETITIONS) iterations=$(BENCH_FACT_REC_ITERATIONS)
	$(MAKE) flamegraph-levm-bench-generic name=ManyHashes repetitions=$(REPETITIONS_SLOW) iterations=$(BENCH_HASHES_ITERATIONS)
	$(MAKE) flamegraph-levm-bench-generic name=BubbleSort repetitions=$(REPETITIONS_SLOW) iterations=$(BENCH_BUBBLESORT_ITERATIONS)
	$(MAKE) flamegraph-levm-bench-generic name=ERC20Approval repetitions=$(REPETITIONS_SLOW) iterations=$(BENCH_APPROVAL_ITERATIONS)
	$(MAKE) flamegraph-levm-bench-generic name=ERC20Transfer repetitions=$(REPETITIONS_SLOW) iterations=$(BENCH_TRANSFER_ITERATIONS)
	$(MAKE) flamegraph-levm-bench-generic name=ERC20Mint repetitions=$(REPETITIONS_SLOW) iterations=$(BENCH_MINT_ITERATIONS)
	$(MAKE) flamegraph-levm-bench-generic name=Push repetitions=$(REPETITIONS_SLOW) iterations=$(BENCH_PUSH_ITERATIONS)

flamegraph-levm-bench-generic:
	mkdir -p flamegraph_levm_benchmark
	cargo flamegraph --root --package revm_comparison --bin benchmark --output flamegraph_levm_benchmark/flamegraph_levm_$(name).svg -- levm $(name) $(repetitions) $(iterations)

build-revm-comparison:
	cd bench/revm_comparison && \
		CARGO_TARGET_DIR=../../target \
		cargo build --release --bin benchmark

###### Build Client with LEVM ######
EFTEST_DIR := ../../../cmd/ef_tests/state
VECTORS_DIR := $(EFTEST_DIR)/vectors

STAMP_FILE := ../../../.docker_build_stamp
$(STAMP_FILE):
	cd ../../../
	$(shell find crates cmd -type f -name '*.rs') Cargo.toml Dockerfile
	docker build -t ethrex .
	touch $(STAMP_FILE)

build-image-levm: $(STAMP_FILE) ## 🐳 Build the Docker image

SIM_PARALLELISM := 48
run-hive-levm: build-image-levm ## 🐝 Run Hive with LEVM and Build report
	$(MAKE) -C ../../../ setup-hive
	$(MAKE) -C ../../../ clean-hive-logs
	cd ../../../hive && ./hive --ethrex.flags "--evm levm" --sim ethereum/rpc-compat --client-file ../fixtures/network/hive_clients/ethrex.yml --client ethrex --sim.limit "$(TEST_PATTERN)" --sim.parallelism $(SIM_PARALLELISM) || exit 0
	cd ../../../hive && ./hive --ethrex.flags "--evm levm" --sim devp2p --client-file ../fixtures/network/hive_clients/ethrex.yml --client ethrex --sim.limit "$(TEST_PATTERN)" --sim.parallelism $(SIM_PARALLELISM) || exit 0
	cd ../../../hive && ./hive --ethrex.flags "--evm levm" --sim ethereum/engine --client-file ../fixtures/network/hive_clients/ethrex.yml --client ethrex --sim.limit "$(TEST_PATTERN)" --sim.parallelism $(SIM_PARALLELISM) || exit 0
	cd ../../../hive && ./hive --ethrex.flags "--evm levm" --sim ethereum/sync --client-file ../fixtures/network/hive_clients/ethrex.yml --client ethrex --sim.limit "$(TEST_PATTERN)" --sim.parallelism $(SIM_PARALLELISM) || exit 0
	cd ../../../ && cargo run --release -p hive_report

download-state-tests: ## 📥 Download and setup state tests fixtures
	cd $(EFTEST_DIR) &&\
	$(MAKE) download-evm-ef-tests

flamegraph-run-ef-tests: ## 🔥 Run EF tests and create a flamegraph per test folder
	cd $(EFTEST_DIR) &&\
	$(MAKE) flamegraph-run-ef-tests

samply-run-ef-tests: ## ⚡️ Run EF tests and create a samply profiling file per test folder
	cd $(EFTEST_DIR) && \
	$(MAKE) samply-run-ef-tests
