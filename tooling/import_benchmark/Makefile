BENCH_NUM ?= 1
NETWORK ?= hoodi

run-bench:
	rm -rf ~/.local/share/ethrex
	cp -r ~/.local/share/ethrex_$(NETWORK)_bench/ethrex ~/.local/share/ethrex
	cd ../.. && cargo r --release -- --network $(NETWORK) import-bench ~/.local/share/ethrex_$(NETWORK)_bench/chain2.rlp | tee bench-$(BENCH_NUM).log
