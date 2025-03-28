.PHONY: bench ethrex-bench cita-bench build-ethrex-bench build-cita-bench

build-bench:
	@cargo build --bench ethrex_trie_bench

bench: build-bench
	@cargo bench --bench ethrex_trie_bench
