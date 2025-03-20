.PHONY: bench ethrex-bench cita-bench build-ethrex-bench build-cita-bench

build-ethrex-bench:
	@cargo build --bench ethrex_trie_bench

build-cita-bench:
	@cargo build --bench cita_trie_bench

ethrex-bench: build-ethrex-bench
	@cargo bench --bench ethrex_trie_bench

cita-bench: build-cita-bench
	@cargo bench --bench cita_trie_bench

bench-all: ethrex-bench cita-bench

compare: build-ethrex-bench build-cita-bench
	@$(MAKE) ethrex-bench | grep -v 'make\|cargo\|Entering\|Leaving' > ethrex.txt
	@$(MAKE) cita-bench | grep -v 'make\|cargo\|Entering\|Leaving' > cita.txt
	@bash ./benches/formatter.sh ethrex.txt cita.txt > compare.md
	@cat compare.md
