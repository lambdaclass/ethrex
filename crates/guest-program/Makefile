.PHONY: sp1 risc0 zisk openvm l2-sp1 l2-risc0 l2-zisk l2-openvm clean help

# Build flags
CARGO_FLAGS := --release -p ethrex-guest-program

# Environment for reproducible builds
ifdef REPRODUCIBLE
	ENV_PREFIX := PROVER_REPRODUCIBLE_BUILD=true
endif

# Default target
help:
	@echo "Usage: make <target>"
	@echo ""
	@echo "L1 Targets:"
	@echo "  sp1       Build SP1 guest"
	@echo "  risc0     Build RISC0 guest"
	@echo "  zisk      Build ZisK guest"
	@echo "  openvm    Build OpenVM guest"
	@echo ""
	@echo "L2 Targets:"
	@echo "  l2-sp1    Build SP1 guest with L2 support"
	@echo "  l2-risc0  Build RISC0 guest with L2 support"
	@echo "  l2-zisk   Build ZisK guest with L2 support"
	@echo "  l2-openvm Build OpenVM guest with L2 support"
	@echo ""
	@echo "Options:"
	@echo "  REPRODUCIBLE=1    Use Docker for reproducible builds"
	@echo ""
	@echo "Examples:"
	@echo "  make sp1                  # Local build"
	@echo "  make l2-sp1               # Local build with L2 support"
	@echo "  make sp1 REPRODUCIBLE=1   # Reproducible build with Docker"

# L1 targets
sp1:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features sp1

risc0:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features risc0

zisk:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features zisk

openvm:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features openvm

# L2 targets
l2-sp1:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features sp1,l2

l2-risc0:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features risc0,l2

l2-zisk:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features zisk,l2

l2-openvm:
	$(ENV_PREFIX) cargo check $(CARGO_FLAGS) --features openvm,l2

clean:
	rm -rf bin/sp1/out bin/risc0/out bin/zisk/out bin/openvm/out
