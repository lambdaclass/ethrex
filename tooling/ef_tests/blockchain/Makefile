.PHONY: download-test-vectors clean-vectors test test-levm test-stateless zkevm-vectors zkevm-benchmark-vectors

VECTORS_ROOT := vectors
FIXTURES_FILE := ../.fixtures_url

# Retrying downloader: release downloads fail transiently often enough to be the
# top cause of red CI. See the script header for what each guard catches.
DOWNLOAD := ../../../.github/scripts/download.sh

SPECTEST_ARTIFACT := tests.tar.gz
SPECTEST_VECTORS_DIR := $(VECTORS_ROOT)/eest
SPECTEST_URL := $(shell cat $(FIXTURES_FILE))

LEGACYTEST_ARTIFACT := legacy-tests.tar.gz
LEGACYTEST_VECTORS_DIR := $(VECTORS_ROOT)/legacy
LEGACYTEST_COMMIT := 1f581b8ccdc4c63acf5f2c5c1b155c690c32a8eb
LEGACYTEST_URL := https://github.com/ethereum/legacytests/archive/$(LEGACYTEST_COMMIT).tar.gz
LEGACYTEST_ARCHIVE_ROOT := legacytests-$(LEGACYTEST_COMMIT)/Cancun/BlockchainTests

# The zkevm bundle is the Amsterdam fixtures plus the EIP-8025 stateless witness
# fields (executionWitness / statelessInputBytes / statelessOutputBytes). Its
# releases track the base fixture release they are filled from -- `tests-zkevm`
# now shares the `tests@vA.B.C` numbering -- so bump it in step with
# `.fixtures_url` above: when the two drift, the bundles describe slightly
# different chains.
# Both bundles extract a `for_amsterdam/` subtree, so we keep the zkevm bundle in
# a separate root that only the stateless harness reads, to avoid overlaying the
# regular Amsterdam fixtures.
ZKEVM_VECTORS_ROOT := vectors_zkevm
ZKEVM_VECTORS_DIR := $(ZKEVM_VECTORS_ROOT)/eest
ZKEVM_FIXTURES_FILE := ../.fixtures_url_zkevm
ZKEVM_ARTIFACT := zkevm-tests.tar.gz
ZKEVM_URL := $(shell cat $(ZKEVM_FIXTURES_FILE))

# tests-zkevm-benchmark@v0.8.2 — Amsterdam compute benchmarks at 10M/30M/60M gas.
# Filled by the EEST pipeline at execution-specs 117dd1cf with go-ethereum's
# `evm` as t8n, not the EELS t8n that produced tests-zkevm@v0.8.0 (0695c34c);
# the two pins move independently despite the shared "zkevm" name.
#
# Kept in its own root and NOT a `download-test-vectors` prerequisite: 519MB
# compressed and ~3.9GB extracted, and only the zkevm bench reads it. Fetch on
# demand with `make zkevm-benchmark-vectors`.
ZKEVM_BENCH_VECTORS_ROOT := vectors_zkevm_benchmark
ZKEVM_BENCH_VECTORS_DIR := $(ZKEVM_BENCH_VECTORS_ROOT)/eest
ZKEVM_BENCH_FIXTURES_FILE := ../.fixtures_url_zkevm_benchmark
ZKEVM_BENCH_ARTIFACT := zkevm-benchmark-tests.tar.gz
ZKEVM_BENCH_URL := $(shell cat $(ZKEVM_BENCH_FIXTURES_FILE))

VECTORS_TARGETS := $(SPECTEST_VECTORS_DIR) $(LEGACYTEST_VECTORS_DIR)

$(SPECTEST_ARTIFACT): $(FIXTURES_FILE)
	$(MAKE) clean-vectors
	$(DOWNLOAD) $(SPECTEST_URL) $(SPECTEST_ARTIFACT)

$(SPECTEST_VECTORS_DIR): $(SPECTEST_ARTIFACT)
	rm -rf $(SPECTEST_VECTORS_DIR)
	mkdir -p $(SPECTEST_VECTORS_DIR)
	tar -xzf $(SPECTEST_ARTIFACT) --strip-components=2 -C $(SPECTEST_VECTORS_DIR) fixtures/blockchain_tests

$(LEGACYTEST_ARTIFACT):
	$(DOWNLOAD) $(LEGACYTEST_URL) $(LEGACYTEST_ARTIFACT)

$(LEGACYTEST_VECTORS_DIR): $(LEGACYTEST_ARTIFACT)
	rm -rf $(LEGACYTEST_VECTORS_DIR)
	mkdir -p $(LEGACYTEST_VECTORS_DIR)
	tar -xzf $(LEGACYTEST_ARTIFACT) --strip-components=3 -C $(LEGACYTEST_VECTORS_DIR) \
		$(LEGACYTEST_ARCHIVE_ROOT)/GeneralStateTests \
		$(LEGACYTEST_ARCHIVE_ROOT)/ValidBlocks

$(ZKEVM_ARTIFACT): $(ZKEVM_FIXTURES_FILE)
	$(DOWNLOAD) $(ZKEVM_URL) $(ZKEVM_ARTIFACT)

$(ZKEVM_VECTORS_DIR): $(ZKEVM_ARTIFACT)
	rm -rf $(ZKEVM_VECTORS_DIR)
	mkdir -p $(ZKEVM_VECTORS_DIR)
	tar -xzf $(ZKEVM_ARTIFACT) --strip-components=2 -C $(ZKEVM_VECTORS_DIR) fixtures/blockchain_tests/for_amsterdam

$(ZKEVM_BENCH_ARTIFACT): $(ZKEVM_BENCH_FIXTURES_FILE)
	$(DOWNLOAD) $(ZKEVM_BENCH_URL) $(ZKEVM_BENCH_ARTIFACT)

# Only the subtrees the manifest references, not all 339 fixture files: the
# whole tree unpacks to ~3.9GB for the 15 workloads actually run.
ZKEVM_BENCH_SUBTREES := \
  compute/instruction/keccak \
  compute/precompile/alt_bn128 \
  compute/precompile/p256verify \
  compute/eip7928_block_level_access_lists/block_access_list \
  compute/scenario/mix_operations
ZKEVM_BENCH_TIERS := for_amsterdam_at_0010M for_amsterdam_at_0030M for_amsterdam_at_0060M

# Keyed on the release URL, and written only after a
# successful untar. Two things this buys: `clean-vectors` runs from the
# `$(SPECTEST_ARTIFACT)` rule on any `.fixtures_url` bump, which would otherwise
# silently discard this opt-in multi-GB download; and an interrupted extraction
# leaves no stamp, so the next run redoes it instead of reporting "nothing to be
# done" while workloads fail on missing files.
ZKEVM_BENCH_STAMP := $(ZKEVM_BENCH_VECTORS_DIR)/.zkevm_benchmark_url

zkevm-benchmark-vectors: ## 📥 Download the zkEVM benchmark fixtures (519MB compressed, ~3.9GB extracted)
	@have=$$(cat $(ZKEVM_BENCH_STAMP) 2>/dev/null); \
	if [ "$$have" = "$(ZKEVM_BENCH_URL)" ]; then \
		echo "zkEVM benchmark fixtures already extracted from $(ZKEVM_BENCH_URL)"; \
	else \
		$(MAKE) $(ZKEVM_BENCH_ARTIFACT) && \
		rm -rf $(ZKEVM_BENCH_VECTORS_DIR) && \
		mkdir -p $(ZKEVM_BENCH_VECTORS_DIR) && \
		members=""; \
		for tier in $(ZKEVM_BENCH_TIERS); do \
			for sub in $(ZKEVM_BENCH_SUBTREES); do \
				members="$$members fixtures/blockchain_tests/$$tier/$$sub"; \
			done; \
		done; \
		tar -xzf $(ZKEVM_BENCH_ARTIFACT) --strip-components=2 -C $(ZKEVM_BENCH_VECTORS_DIR) $$members && \
		echo "$(ZKEVM_BENCH_URL)" > $(ZKEVM_BENCH_STAMP); \
	fi

zkevm-vectors: $(ZKEVM_VECTORS_DIR)

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

download-test-vectors: $(VECTORS_TARGETS) zkevm-vectors ## 📥 Download test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT) $(ZKEVM_VECTORS_ROOT) $(ZKEVM_BENCH_VECTORS_ROOT)
	rm -f $(SPECTEST_ARTIFACT) $(LEGACYTEST_ARTIFACT) $(ZKEVM_ARTIFACT) $(ZKEVM_BENCH_ARTIFACT)

test-levm: $(VECTORS_TARGETS) ## 🧪 Run blockchain tests with LEVM
	cargo test --profile release-fast

test-stateless: zkevm-vectors
	cargo test --profile release-fast --features stateless

test: ## 🧪 Run blockchain tests with LEVM both with state and stateless
	$(MAKE) test-levm
	$(MAKE) test-stateless
