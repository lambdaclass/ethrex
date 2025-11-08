BENCH_NUM ?= 1
NETWORK ?= hoodi

run-bench:
	rm -rf ~/.local/share/temp
	cp -r ~/.local/share/ethrex_$(NETWORK)_bench/ethrex ~/.local/share/temp
	cd ../.. && cargo r --release -- --network $(NETWORK) --datadir ~/.local/share/temp import-bench ~/.local/share/ethrex_$(NETWORK)_bench/chain.rlp | tee bench-$(BENCH_NUM).log

BLOCK_NUM ?= 1

create-single-chain-rlp:
	cd ../.. && cargo r --release -- --network $(NETWORK) export --first $(BLOCK_NUM) --last $(BLOCK_NUM) ~/.local/share/ethrex_$(NETWORK)_bench/single_block/chain-$(BLOCK_NUM).rlp

move-bench-forward:
	cd ../.. && cargo r --release -- --network $(NETWORK) --datadir ~/.local/share/temp import-bench ~/.local/share/ethrex_$(NETWORK)_bench/single_block/chain-$(BLOCK_NUM).rlp | tee bench-$(BENCH_NUM).log

run-regenerate-bench:
	cd ../.. && cargo r --relase -- --network $(NETWORK) --datadir ~/.local/share/temp import-bench doesnt_exist | tee tooling/import_benchmark/logs/bench-$(BENCH_NUM)-$(BLOCK_NUM).log
