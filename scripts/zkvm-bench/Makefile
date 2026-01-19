# scripts/zkvm-bench/Makefile
# zkVM Benchmarking Workflow
#
# Usage:
#   make bench ZKVM=zisk BLOCK=23769082 RPC_URL=http://localhost:8545
#   make input BLOCK=23769082
#   make build ZKVM=zisk
#   make profile ZKVM=zisk BLOCK=23769082
#   make compare BASELINE=stats_before.txt CURRENT=stats_after.txt

# ==============================================================================
# Configuration
# ==============================================================================

ZKVM ?= zisk
BLOCK ?=
RPC_URL ?= http://localhost:8545
TOP_ROI ?= 25

# Directories
SCRIPT_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
REPO_ROOT := $(abspath $(SCRIPT_DIR)/../..)
INPUT_DIR := $(SCRIPT_DIR)/inputs
PROFILE_DIR := $(SCRIPT_DIR)/profiles/$(ZKVM)
GUEST_DIR := $(REPO_ROOT)/crates/l2/prover/src/guest_program/src

# Derived paths
INPUT_FILE := $(INPUT_DIR)/ethrex_mainnet_$(BLOCK)_input.bin
TIMESTAMP := $(shell date +%Y%m%d_%H%M%S)
STATS_FILE := $(PROFILE_DIR)/stats_$(BLOCK)_$(TIMESTAMP).txt

# ELF paths
ZISK_ELF := $(GUEST_DIR)/zisk/target/riscv64ima-zisk-zkvm-elf/release/zkvm-zisk-program
SP1_ELF := $(GUEST_DIR)/sp1/target/release/zkvm-sp1-program

# ==============================================================================
# Help
# ==============================================================================

.PHONY: help
help:
	@echo "zkVM Benchmarking Workflow"
	@echo ""
	@echo "Full workflow (generate input if needed, build, profile):"
	@echo "  make bench ZKVM=zisk BLOCK=23769082 RPC_URL=http://localhost:8545"
	@echo ""
	@echo "Individual steps:"
	@echo "  make input BLOCK=23769082              Generate input for block"
	@echo "  make build ZKVM=zisk                   Build guest program"
	@echo "  make profile ZKVM=zisk BLOCK=23769082  Profile execution"
	@echo "  make compare BASELINE=a.txt CURRENT=b.txt  Compare results"
	@echo ""
	@echo "Utilities:"
	@echo "  make list-inputs                       List available inputs"
	@echo "  make list-profiles                     List profiles for current ZKVM"
	@echo "  make to-json FILE=stats.txt            Convert stats to JSON"
	@echo "  make clean                             Remove generated files"
	@echo ""
	@echo "Variables:"
	@echo "  ZKVM     - Backend: zisk or sp1 (default: zisk)"
	@echo "  BLOCK    - Block number (required for input/profile/bench)"
	@echo "  RPC_URL  - RPC endpoint (default: http://localhost:8545)"
	@echo "  TOP_ROI  - Top functions to show (default: 25)"
	@echo ""
	@echo "Examples:"
	@echo "  make bench ZKVM=zisk BLOCK=23769082 RPC_URL=\$$ALCHEMY_URL"
	@echo "  make profile ZKVM=zisk BLOCK=23769082"
	@echo "  make compare BASELINE=profiles/zisk/stats_before.txt CURRENT=profiles/zisk/stats_after.txt"

# ==============================================================================
# Full Workflow
# ==============================================================================

.PHONY: bench
bench: _check-block input build profile
	@echo ""
	@echo "=========================================="
	@echo "Benchmark complete for block $(BLOCK)"
	@echo "=========================================="
	@echo ""
	@echo "Profile: $(STATS_FILE)"
	@echo ""
	@echo "To compare with a baseline:"
	@echo "  make compare BASELINE=<baseline.txt> CURRENT=$(STATS_FILE)"

# ==============================================================================
# Individual Steps
# ==============================================================================

.PHONY: input
input: _check-block
	@mkdir -p $(INPUT_DIR)
	@if [ -f "$(INPUT_FILE)" ]; then \
		echo "Input exists: $(INPUT_FILE)"; \
	else \
		echo "Generating input for block $(BLOCK)..."; \
		./generate-input.sh $(BLOCK) $(RPC_URL) $(INPUT_DIR); \
	fi

.PHONY: build
build:
	@echo "Building $(ZKVM) guest program..."
	@./build.sh $(ZKVM)

.PHONY: profile
profile: _check-block _check-input
	@mkdir -p $(PROFILE_DIR)
	@echo "Profiling $(ZKVM) execution for block $(BLOCK)..."
	@echo "Output: $(STATS_FILE)"
	@echo ""
ifeq ($(ZKVM),zisk)
	@./profile-zisk.sh $(INPUT_FILE) $(PROFILE_DIR) $(TOP_ROI) 2>&1 | tee $(STATS_FILE)
else ifeq ($(ZKVM),sp1)
	@./profile-sp1.sh $(INPUT_FILE) $(PROFILE_DIR)
else
	@echo "Error: Unknown ZKVM=$(ZKVM). Use 'zisk' or 'sp1'."
	@exit 1
endif
	@echo ""
	@echo "Profile saved: $(STATS_FILE)"

.PHONY: compare
compare:
	@if [ -z "$(BASELINE)" ] || [ -z "$(CURRENT)" ]; then \
		echo "Usage: make compare BASELINE=<file> CURRENT=<file>"; \
		echo ""; \
		echo "Example:"; \
		echo "  make compare BASELINE=profiles/zisk/stats_before.txt CURRENT=profiles/zisk/stats_after.txt"; \
		exit 1; \
	fi
	@python3 compare.py $(BASELINE) $(CURRENT)

# ==============================================================================
# Utilities
# ==============================================================================

.PHONY: list-inputs
list-inputs:
	@echo "Available inputs:"
	@ls -la $(INPUT_DIR)/*.bin 2>/dev/null || echo "  (none)"

.PHONY: list-profiles
list-profiles:
	@echo "Profiles for $(ZKVM):"
	@ls -la $(PROFILE_DIR)/*.txt 2>/dev/null || echo "  (none)"

.PHONY: to-json
to-json:
	@if [ -z "$(FILE)" ]; then \
		echo "Usage: make to-json FILE=<stats.txt>"; \
		exit 1; \
	fi
	@./to-json.sh $(FILE)

.PHONY: clean
clean:
	@echo "Removing generated files..."
	@rm -rf $(SCRIPT_DIR)/inputs
	@rm -rf $(SCRIPT_DIR)/profiles
	@rm -rf $(SCRIPT_DIR)/benchmarks
	@echo "Done."

.PHONY: clean-profiles
clean-profiles:
	@echo "Removing profiles..."
	@rm -rf $(SCRIPT_DIR)/profiles
	@echo "Done."

# ==============================================================================
# Validation Helpers
# ==============================================================================

.PHONY: _check-block
_check-block:
	@if [ -z "$(BLOCK)" ]; then \
		echo "Error: BLOCK is required"; \
		echo "Usage: make <target> BLOCK=<block_number>"; \
		exit 1; \
	fi

.PHONY: _check-input
_check-input:
	@if [ ! -f "$(INPUT_FILE)" ]; then \
		echo "Error: Input file not found: $(INPUT_FILE)"; \
		echo ""; \
		echo "Generate it first:"; \
		echo "  make input BLOCK=$(BLOCK) RPC_URL=$(RPC_URL)"; \
		exit 1; \
	fi
