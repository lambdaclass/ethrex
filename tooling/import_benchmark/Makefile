BENCH_NUM ?= 1
NETWORK ?= hoodi

run-bench:
	rm -rf ~/.local/share/temp
	cp -r ~/.local/share/ethrex_$(NETWORK)_bench/ethrex ~/.local/share/temp
	cd ../.. && cargo r --release -- --network $(NETWORK) import-bench ~/.local/share/ethrex_$(NETWORK)_bench/chain.rlp | tee bench-$(BENCH_NUM).log
