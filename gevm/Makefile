FIXTURES_DIR := $(CURDIR)/tests/fixtures/ethereum-tests
EEST_DIR := $(CURDIR)/tests/fixtures/execution-spec-tests
EEST_FIXTURES := $(EEST_DIR)/fixtures
EEST_VERSION := v5.4.0

.PHONY: test test-unit test-spec eest-fixtures

# Run all fixture tests (GeneralStateTests, BlockchainTests, TransactionTests, EEST)
test: test-unit test-spec

# Run unit tests (no fixtures needed)
test-unit:
	go test ./internal/... -count=1

# Run all ethereum spec fixture tests
test-spec: eest-fixtures
	GEVM_TESTS_DIR=$(FIXTURES_DIR)/GeneralStateTests \
	GEVM_BLOCKCHAIN_TESTS_DIR=$(FIXTURES_DIR)/BlockchainTests \
	GEVM_TRANSACTION_TESTS_DIR=$(FIXTURES_DIR)/TransactionTests \
	GEVM_EEST_DIR=$(EEST_FIXTURES)/state_tests \
	go test ./tests/spec/... -count=1 -timeout=30m -failfast

# Download EEST fixtures from GitHub release
eest-fixtures:
	@if [ ! -d "$(EEST_FIXTURES)/state_tests" ]; then \
		echo "Downloading EEST fixtures $(EEST_VERSION)..."; \
		curl -sL https://github.com/ethereum/execution-spec-tests/releases/download/$(EEST_VERSION)/fixtures_stable.tar.gz | \
		tar xz -C $(EEST_DIR); \
		echo "EEST fixtures extracted to $(EEST_FIXTURES)"; \
	fi
