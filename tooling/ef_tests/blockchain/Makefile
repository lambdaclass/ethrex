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
# fields (executionWitness / statelessInputBytes / statelessOutputBytes). Each
# release is filled from a base `tests@` release, named in its release notes,
# and that base must match `.fixtures_url` above: when the two drift, the bundles
# describe slightly different chains. The zkevm line shares the base's major and
# minor numbers but ships patch releases of its own, so a zkevm-only bump such as
# v21.0.1, on the unchanged tests@v21.0.0 base, leaves `.fixtures_url` alone.
# Both bundles extract a `for_amsterdam/` subtree, so we keep the zkevm bundle in
# a separate root that only the stateless harness reads, to avoid overlaying the
# regular Amsterdam fixtures.
ZKEVM_VECTORS_ROOT := vectors_zkevm
ZKEVM_VECTORS_DIR := $(ZKEVM_VECTORS_ROOT)/eest
# The same bundle's Engine API fixtures. Since tests-zkevm@v21.0.1 their payloads
# carry stateless bytes too, including the payload mutations (block access list,
# slot number) that have no RLP-block form, so they cover strictly more invalid
# inputs than the blockchain fixtures do. A separate root keeps the blockchain
# runner from parsing them.
ZKEVM_ENGINE_VECTORS_ROOT := vectors_zkevm_engine
ZKEVM_ENGINE_VECTORS_DIR := $(ZKEVM_ENGINE_VECTORS_ROOT)/eest
# Subtrees extracted from both fixture kinds. `for_bpo2toamsterdamattime15k` holds
# the fork-activation fixtures, whose witnesses carry pre-Amsterdam headers, so
# omitting it leaves the guest untested across the fork boundary.
ZKEVM_SUBTREES := for_amsterdam for_bpo2toamsterdamattime15k
ZKEVM_FIXTURES_FILE := ../.fixtures_url_zkevm
ZKEVM_ARTIFACT := zkevm-tests.tar.gz
ZKEVM_URL := $(shell cat $(ZKEVM_FIXTURES_FILE))

# tests-zkevm-benchmark: Amsterdam compute benchmarks at fixed gas values, filled
# by the EEST pipeline with go-ethereum's `evm` as t8n. The release this pins is
# built against the same stateless wire format as `.fixtures_url_zkevm`, but the
# two pins move independently despite the shared "zkevm" name.
#
# Since v21.0.5 each gas value ships as its own tarball,
# `fixtures_zkevm-benchmark_<gas>.tar.gz`, because all of them together exceed
# GitHub's 2 GiB asset limit. So the pin file holds the release's download URL,
# and the tarball names come from ZKEVM_BENCH_GAS.
#
# Kept in its own root and NOT a `download-test-vectors` prerequisite: ~2.4GB
# compressed across the five tarballs, and only the zkevm bench reads it. Fetch on
# demand with `make zkevm-benchmark-vectors`.
ZKEVM_BENCH_VECTORS_ROOT := vectors_zkevm_benchmark
ZKEVM_BENCH_VECTORS_DIR := $(ZKEVM_BENCH_VECTORS_ROOT)/eest
ZKEVM_BENCH_FIXTURES_FILE := ../.fixtures_url_zkevm_benchmark
ZKEVM_BENCH_RELEASE_URL := $(shell cat $(ZKEVM_BENCH_FIXTURES_FILE))
ZKEVM_BENCH_GAS := 0030M 0060M 0100M 0150M 0200M
ZKEVM_BENCH_ARTIFACTS := $(foreach gas,$(ZKEVM_BENCH_GAS),zkevm-benchmark-tests_$(gas).tar.gz)

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
	rm -rf $(ZKEVM_VECTORS_DIR) $(ZKEVM_ENGINE_VECTORS_DIR)
	mkdir -p $(ZKEVM_VECTORS_DIR) $(ZKEVM_ENGINE_VECTORS_DIR)
	tar -xzf $(ZKEVM_ARTIFACT) --strip-components=2 -C $(ZKEVM_VECTORS_DIR) \
		$(addprefix fixtures/blockchain_tests/,$(ZKEVM_SUBTREES))
	tar -xzf $(ZKEVM_ARTIFACT) --strip-components=2 -C $(ZKEVM_ENGINE_VECTORS_DIR) \
		$(addprefix fixtures/blockchain_tests_engine/,$(ZKEVM_SUBTREES))

zkevm-benchmark-tests_%.tar.gz: $(ZKEVM_BENCH_FIXTURES_FILE)
	$(DOWNLOAD) $(ZKEVM_BENCH_RELEASE_URL)/fixtures_zkevm-benchmark_$*.tar.gz $@

# Only the subtrees the manifest references, not all 339 fixture files: the
# whole tree unpacks to ~3.9GB for the 15 workloads actually run.
ZKEVM_BENCH_SUBTREES := \
  compute/instruction/keccak \
  compute/precompile/alt_bn128 \
  compute/precompile/p256verify \
  compute/eip7928_block_level_access_lists/block_access_list \
  compute/scenario/mix_operations

# Keyed on the release URL and the gas values, and written only after a
# successful untar. Two things this buys: `clean-vectors` runs from the
# `$(SPECTEST_ARTIFACT)` rule on any `.fixtures_url` bump, which would otherwise
# silently discard this opt-in multi-GB download; and an interrupted extraction
# leaves no stamp, so the next run redoes it instead of reporting "nothing to be
# done" while workloads fail on missing files.
ZKEVM_BENCH_STAMP := $(ZKEVM_BENCH_VECTORS_DIR)/.zkevm_benchmark_url

zkevm-benchmark-vectors: ## 📥 Download the zkEVM benchmark fixtures (~2.4GB compressed, five tarballs)
	@want="$(ZKEVM_BENCH_RELEASE_URL) $(ZKEVM_BENCH_GAS)"; \
	have=$$(cat $(ZKEVM_BENCH_STAMP) 2>/dev/null); \
	if [ "$$have" = "$$want" ]; then \
		echo "zkEVM benchmark fixtures already extracted from $(ZKEVM_BENCH_RELEASE_URL)"; \
	else \
		$(MAKE) $(ZKEVM_BENCH_ARTIFACTS) && \
		rm -rf $(ZKEVM_BENCH_VECTORS_DIR) && \
		mkdir -p $(ZKEVM_BENCH_VECTORS_DIR) && \
		for gas in $(ZKEVM_BENCH_GAS); do \
			members=""; \
			for sub in $(ZKEVM_BENCH_SUBTREES); do \
				members="$$members fixtures/blockchain_tests/for_amsterdam_at_$$gas/$$sub"; \
			done; \
			tar -xzf zkevm-benchmark-tests_$$gas.tar.gz --strip-components=2 \
				-C $(ZKEVM_BENCH_VECTORS_DIR) $$members || exit 1; \
		done && \
		echo "$$want" > $(ZKEVM_BENCH_STAMP); \
	fi

zkevm-vectors: $(ZKEVM_VECTORS_DIR)

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

download-test-vectors: $(VECTORS_TARGETS) zkevm-vectors ## 📥 Download test vectors

clean-vectors: ## 🗑️  Clean test vectors
	rm -rf $(VECTORS_ROOT) $(ZKEVM_VECTORS_ROOT) $(ZKEVM_ENGINE_VECTORS_ROOT) $(ZKEVM_BENCH_VECTORS_ROOT)
	rm -f $(SPECTEST_ARTIFACT) $(LEGACYTEST_ARTIFACT) $(ZKEVM_ARTIFACT) $(ZKEVM_BENCH_ARTIFACTS)

test-levm: $(VECTORS_TARGETS) ## 🧪 Run blockchain tests with LEVM
	cargo test --profile release-fast

test-stateless: zkevm-vectors
	cargo test --profile release-fast --features stateless

test: ## 🧪 Run blockchain tests with LEVM both with state and stateless
	$(MAKE) test-levm
	$(MAKE) test-stateless
