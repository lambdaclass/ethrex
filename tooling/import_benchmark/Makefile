BENCH_NUM ?= 1
NETWORK ?= hoodi

run-bench:
	rm -rf ~/.local/share/ethrex
	cp -r ~/.local/share/ethrex_$(BENCH_NUM)_bench/ethrex ~/.local/share/ethrex
	cd ../.. && cargo r --release -- --network $(BENCH_NUM) import-bench ~/.local/share/ethrex_$(BENCH_NUM)_bench/chain2.rlp | tee bench-$(BENCH_NUM).log
