BENCH_ID ?= 1
NETWORK ?= hoodi
NUM_REPETITIONS ?= 3

help:
	@printf "run-bench: ## Runs a bench for the current pr. \nParameters:\n -BENCH_ID: number for the log file where it will\
	 be saved with the format bench-BENCH_ID.log\n -NETWORK: which network to access (hoodi, mainnet)\nRequirements:\n This tool assumes we are running on Linux,\
	 and that we have a valid db in ~/.local/share/ethrex_NETWORK_bench/ethrex\n with a valid state and a list of blocks for import\
	 in ~/.local/share/ethrex_NETWORK_bench/chain.rlp\n\n"
	@printf "run-benches: ## Runs multiple repetitions of the bench for the current pr. \nParameters:\n -NUM_REPETITIONS: number of runs\n -NETWORK: which network to access (hoodi, mainnet)\nRequirements:\n This tool assumes we are running on Linux,\
	 and that we have a valid db in ~/.local/share/ethrex_NETWORK_bench/ethrex\n with a valid state and a list of blocks for import\
	 in ~/.local/share/ethrex_NETWORK_bench/chain.rlp\n\n"
	@printf "python3 parse_bench.py <bench_file_1> <bench_file_2> <...> : ## Parses the given bench log files to find average ggas\nRequirements\n\
	 This script assumes we have the bench logs on the ethrex folder\n\n"

run-bench: ## Runs a bench for the current pr. parameters -BENCH_ID: number for the log file where it will be saved -NETWORK: which network to access
	./benchmark.sh $(NETWORK) 1 $(BENCH_ID)

run-benches: ## Runs multiple repetitions of the bench for the current pr. parameters -NUM_REPETITIONS: number of runs -NETWORK: which network to access
	./benchmark.sh $(NETWORK) $(NUM_REPETITIONS)
