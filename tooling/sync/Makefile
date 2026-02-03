.PHONY = help create_datadir gen_jwt sync start_geth_holesky flamegraph-main  flamegraph \
flamegraph-branch flamegraph-inner flamegraph-mainnet flamegraph-sepolia flamegraph-holesky \
flamegraph-hoodi start-lighthouse start-ethrex backup-db start-mainnet-metrics-docker \
start-sepolia-metrics-docker start-holesky-metrics-docker start-hoodi-metrics-docker \
start-metrics-docker tail-syncing-logs tail-metrics-logs copy_flamegraph import-with-metrics \
multisync-up multisync-down multisync-clean multisync-logs multisync-status \
multisync-restart multisync-monitor multisync-run multisync-loop multisync-loop-auto \
benchmark-parse benchmark-compare benchmark-compare-csv benchmark-compare-md benchmark-diff benchmark-latest benchmark-help \
benchmark-parse-prefix benchmark-merge

ETHREX_DIR ?= "../.."

# Frame pointers for profiling (default on for sync/profiling targets)
FRAME_POINTERS ?= 1

ifeq ($(FRAME_POINTERS),1)
PROFILING_CFG := --config .cargo/profiling.toml
endif

EVM ?= levm
NODE_NAME ?= ethrex
ENGINE_PORT ?= 8551
LIGHTHOUSE_PORT ?= 9099
LIGHTHOUSE_DISCOVERY_PORT ?= 9999
CURRENT_DATETIME = $(shell date +'%y.%m.%d-%H.%M.%S')
BATCH_SIZE ?= 1024
OS = $(shell uname)
ifeq ($(OS), Darwin)
	DATA_PATH = $(HOME)/Library
else
	DATA_PATH = $(HOME)/.local/share
endif

# Set checkpoint sync url based on network
NETWORK ?= mainnet# default network if not set
ifeq ($(NETWORK), mainnet)
CHECKPOINT_SYNC_URL ?= https://beaconstate.info
endif
ifeq ($(NETWORK), sepolia)
CHECKPOINT_SYNC_URL ?= https://checkpoint-sync.sepolia.ethpandaops.io
endif
ifeq ($(NETWORK), holesky)
CHECKPOINT_SYNC_URL ?= https://checkpoint-sync.holesky.ethpandaops.io
endif
ifeq ($(NETWORK), hoodi)
CHECKPOINT_SYNC_URL ?= https://hoodi.beaconstate.ethstaker.cc
endif

# Set bootnodes flag only if we have bootnodes
ifndef BOOTNODES
BOOTNODES_FLAG ?=
else
BOOTNODES_FLAG ?= --bootnodes ${BOOTNODES}
endif

default: help

help: ## Display help for the makefile.
	@grep -E '^[%a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

create_datadir: ## Create the data folder for the network. If NETWORK environment variable is not provided, mainnet will be used as default.
	mkdir -p $(DATA_PATH)/$(NETWORK)_data

gen-jwt: create_datadir ## Create the jwt for a given network. NETWORK environment variable required.
	openssl rand -hex 32 | tr -d "\n" | tee $(DATA_PATH)/$(NETWORK)_data/jwt.hex

sync: create_datadir ## Run the sync for a given network. SYNC_BLOCK_NUM environment variable required (for block to start from).  If NETWORK environment variable is not provided, mainnet will be used as default. EVM can also be set to select the evm to use.
ifndef SYNC_BLOCK_NUM
	$(error "Sync block number not set")
endif
#	samply record --unstable-presymbolicate --save-only --
	mkdir -p logs
	make start-ethrex >> ./logs/ethrex-sync-$(NETWORK)-$(EVM).log


flamegraph-main: ## Run flamegraph on main branch. SYNC_BLOCK_NUM environment variable required (for block to start from). If NETWORK environment variable is not provided, mainnet will be used as default. EVM can also be set to select the evm to use. Execution logs are output to log file.
ifndef SYNC_BLOCK_NUM
	$(error "Sync block number not set)
endif
	cd $(ETHREX_DIR) && git checkout main
	mkdir -p logs
	make flamegraph-inner >> logs/ethrex-$(NETWORK)-$(EVM)-flamegraph-$(CURRENT_DATETIME)-main-block-$(SYNC_BLOCK_NUM)-$(LOGNAME).log

flamegraph: ## Run flamegraph on the currently checked out branch. SYNC_BLOCK_NUM environment variable required (for block to start from).  If NETWORK environment variable is not provided, mainnet will be used as default. EVM can also be set to select the evm to use. Execution logs are output to log file.
ifndef SYNC_BLOCK_NUM
	$(error "Sync block number not set)
endif
	mkdir -p logs
	make flamegraph-inner >> logs/ethrex-$(NETWORK)-$(EVM)-flamegraph-$(CURRENT_DATETIME)-main-block-$(SYNC_BLOCK_NUM)-$(LOGNAME).log

flamegraph-branch: ## Run flamegraph on custom branch. SYNC_BLOCK_NUM environment variable required (for block to start from).  If NETWORK environment variable is not provided, mainnet will be used as default. EVM can also be set to select the evm to use. Execution logs are output to log file.
ifndef SYNC_BLOCK_NUM
	$(error "Sync block number not set")
endif
ifndef BRANCH
	$(error "Branch not specified")
endif
	cd $(ETHREX_DIR) && git checkout $(BRANCH)
	mkdir -p logs
	make flamegraph-inner >> logs/ethrex-$(NETWORK)-$(EVM)-flamegraph-$(CURRENT_DATETIME)-$(BRANCH)-block-$(SYNC_BLOCK_NUM)-$(LOGNAME).log


flamegraph-inner: # Inner target used for flamegraph-related targets. Runs flamegraph on the network given by NETWORK.
	cd $(ETHREX_DIR) && CARGO_PROFILE_RELEASE_DEBUG=true RUST_LOG=3 cargo flamegraph --features "rocksdb sync-test" --bin ethrex -- \
		--http.port 8545 \
		--authrpc.port 8551 \
		--p2p.port 30303\
		--discovery.port 30303 \
		--network $(NETWORK) \
		--datadir $(DATA_PATH)/${NETWORK}_data/ethrex/$(EVM) \
		--authrpc.jwtsecret $(DATA_PATH)/${NETWORK}_data/jwt.hex \
		$(BOOTNODES_FLAG) \

flamegraph-%: ## Run flamegraph on either mainnet, sepolia, holesky or hoodi network. For example, to run the flamgraph on hoodi, use `flamegraph-hoodi`
	$(MAKE) flamegraph-inner NETWORK=$*

backup-db: ## Back-up the store db. EVM and NETWORK environment variables need to be provided to select which DB to back up. If NETWORK environment variable is not provided, mainnet will be used as default.
	mkdir -p $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)
ifeq ($(OS), Darwin)
	rsync -ah --progress $(DATA_PATH)/$(NETWORK)_data/ethrex/$(EVM)/mdbx.* $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)
	rsync -ah --progress ./logs/ethrex-sync-$(NETWORK)-$(EVM).log $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)/ethrex-sync-$(NETWORK)-$(EVM).log
else
	rsync -ah --info=progress2 $(DATA_PATH)/$(NETWORK)_data/ethrex/$(EVM)/mdbx.* $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)
	rsync -ah --info=progress2 ./logs/ethrex-sync-$(NETWORK)-$(EVM).log $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)/ethrex-sync-$(NETWORK)-$(EVM).log
endif

import-with-metrics: ## Start L1 docker compose, lighthouse in background, and ethrex for holesky
ifndef RLP_FILE
	$(error "RLP file with blocks not provided")
endif
	@echo "Starting L1 docker compose with metrics..."
	cd $(ETHREX_DIR)/metrics && docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up -d
	@echo "Starting ethrex..."
	cd $(ETHREX_DIR) && cargo run $(PROFILING_CFG) --release --features "metrics" --bin ethrex -- \
	--network $(NETWORK) \
	--metrics \
	--metrics.port 3701 \
	import $(RLP_FILE) \
	--removedb

tail-syncing-logs: ## Tail the syncing logs for a given log file. Environment variable LOGNAME with the name of the file needs to be provided.
ifndef LOGNAME
	$(error "Log file not provided")
endif
	tail -n 100 -f ./logs/$(LOGNAME) | grep -e "SYNCING"

tail-metrics-logs: ## Tail the metrics logs for a given log file. Environment variable LOGNAME with the name of the file needs to be provided.
ifndef LOGNAME
	$(error "Log file not provided")
endif
	tail -n 2000  -f ./logs/$(LOGNAME) | grep -A4 -e "METRICS"

copy-flamegraph: ## Copy flamegraph from ethrex folder to flamegraphs folder. A name for the file can be provided with GRAPHNAME variable.
ifeq ($(OS), Darwin)
	rsync -ah --progress $(ETHREX_DIR)/flamegraph.svg flamegraphs/flamegraph-$(GRAPHNAME).svg
else
	rsync -ah --info=progress2 $(ETHREX_DIR)/flamegraph.svg flamegraphs/flamegraph-$(GRAPHNAME).svg
endif

start-%-metrics-docker: ## Start L1 docker compose, lighthouse in background, and ethrex for either mainnet, sepolia, holesky or hoodi network. For example, to run on hoodi, use `start-hoodi-metrics-docker`
	$(MAKE) start-metrics-docker NETWORK=$*

start-metrics-docker: ## Start L1 docker compose, lighthouse in background, and ethrex for the network given by NETWORK.
	@echo "Starting L1 docker compose with metrics..."
	cd $(ETHREX_DIR)/metrics && docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up -d
	@echo "Starting lighthouse in background..."
	cd $(ETHREX_DIR)/tooling/sync && nohup make start-lighthouse > /dev/null 2>&1 &
	@echo "Starting ethrex..."
	cd $(ETHREX_DIR)/tooling/sync && make start-ethrex

start-lighthouse: ## Start lighthouse for the network given by NETWORK.
	lighthouse bn \
		--network $(NETWORK) \
		--execution-endpoint http://localhost:${ENGINE_PORT} \
		--execution-jwt $(DATA_PATH)/${NETWORK}_data/jwt.hex \
		--checkpoint-sync-url $(CHECKPOINT_SYNC_URL) \
		--http \
		--http-address 0.0.0.0 \
		--http-allow-origin "*" \
		--metrics \
		--metrics-address 0.0.0.0 \
		--metrics-port 5054 \
		--datadir $(DATA_PATH)/${NETWORK}_data/lighthouse_${NODE_NAME}_$(EVM) \
		--port $(LIGHTHOUSE_PORT) --discovery-port $(LIGHTHOUSE_DISCOVERY_PORT)

start-ethrex: ## Start ethrex for the network given by NETWORK.
	@echo $(RUSTFLAGS)
	cd $(ETHREX_DIR) && RUST_LOG=3 cargo run $(PROFILING_CFG) --release --features "rocksdb sync-test metrics" --bin ethrex -- \
    		--http.addr 0.0.0.0 \
    		--http.port 8545 \
    		--authrpc.port 8551 \
    		--p2p.port 30303\
    		--discovery.port 30303 \
    		--metrics \
    		--metrics.port 3701 \
    		--network $(NETWORK) \
    		--datadir $(DATA_PATH)/${NETWORK}_data/ethrex/$(EVM) \
    		--authrpc.jwtsecret $(DATA_PATH)/${NETWORK}_data/jwt.hex \
    		$(BOOTNODES_FLAG) \

SERVER_SYNC_BRANCH ?= main
SERVER_SYNC_NETWORK ?= hoodi

ifeq ($(SERVER_SYNC_NETWORK),hoodi)
CHECKPOINT_URL = https://hoodi-checkpoint-sync.stakely.io/
else ifeq ($(SERVER_SYNC_NETWORK),sepolia)
CHECKPOINT_URL = https://checkpoint-sync.sepolia.ethpandaops.io
else ifeq ($(SERVER_SYNC_NETWORK),mainnet)
CHECKPOINT_URL = https://mainnet-checkpoint-sync.attestant.io
else
$(error Unknown network $(SERVER_SYNC_NETWORK))
endif

LOGS_FILE ?= output.log

# Use make server-sync SERVER_SYNC_BRANCH=branch_name SERVER_SYNC_NETWORK=network_name LOGS_FILE=logs_file_name  HEALING=1 MEMORY=1 FULL_SYNC=1
# SERVER_SYNC_BRANCH is the branch to checkout before syncing, SERVER_SYNC_NETWORK is the network to sync, LOGS_FILE is the file to output logs to, HEALING enables healing mode, MEMORY uses memory datadir, FULL_SYNC switches to full-sync mode.
# If you want to run metrics locally (grafana/prometheus/ethereum-metrics-exporter in docker-compose), make sure to run the following first:
# 		cd ../../metrics && docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up -d
server-sync:
	git fetch --all

	git checkout $(SERVER_SYNC_BRANCH)

	git pull

	tmux kill-session -t sync || true

	sleep 5

	tmux new-session -d -s sync -n htop "htop"

	tmux new-window -t sync:1 -n lighthouse "lighthouse bn --network $(SERVER_SYNC_NETWORK) --execution-endpoint http://localhost:8551 --execution-jwt ~/secrets/jwt.hex --http --checkpoint-sync-url $(CHECKPOINT_URL) --http-address 0.0.0.0 --purge-db-force "

	sleep 0.2

	tmux new-window -t sync:2 -n ethrex "cd ../.. && ulimit -n 1000000 && rm -rf ~/.local/share/ethrex && RUST_LOG=info,ethrex_p2p::sync=debug $(if $(DEBUG_ASSERT),RUSTFLAGS='-C debug-assertions=yes') $(if $(HEALING),SKIP_START_SNAP_SYNC=1) cargo run $(PROFILING_CFG) --release --bin ethrex --features rocksdb --  --http.addr 0.0.0.0 --metrics --metrics.port 3701 --network $(SERVER_SYNC_NETWORK) $(if $(MEMORY),--datadir memory) --authrpc.jwtsecret ~/secrets/jwt.hex $(if $(or $(FULL_SYNC),$(HEALING)),--syncmode full)  2>&1 | tee $(LOGS_FILE)"

# ==============================================================================
# Docker Compose Multi-Network Snapsync
# ==============================================================================

MULTISYNC_COMPOSE = docker compose -f docker-compose.multisync.yaml
MULTISYNC_NETWORKS ?= hoodi,sepolia,mainnet
comma := ,
MULTISYNC_NETWORK_LIST := $(subst $(comma), ,$(MULTISYNC_NETWORKS))
MULTISYNC_SERVICES := $(foreach n,$(MULTISYNC_NETWORK_LIST),setup-jwt-$(n) ethrex-$(n) consensus-$(n))

# Auto-update configuration (for multisync-loop-auto)
# Build profile: 'release-with-debug-assertions' enables state trie validation
MULTISYNC_BUILD_PROFILE ?= release-with-debug-assertions
# Local image tag for builds
MULTISYNC_LOCAL_IMAGE ?= ethrex-local:multisync
# Branch to track for auto-update mode (defaults to current branch if not set)
MULTISYNC_BRANCH ?= $(shell git rev-parse --abbrev-ref HEAD)

multisync-up: ## Start all networks specified in MULTISYNC_NETWORKS via Docker Compose.
	$(MULTISYNC_COMPOSE) up -d $(MULTISYNC_SERVICES)

multisync-down: ## Stop and remove all snapsync containers.
	$(MULTISYNC_COMPOSE) down

multisync-clean: ## Stop, remove containers AND volumes (full reset).
	$(MULTISYNC_COMPOSE) down -v

multisync-logs: ## Tail logs from all networks.
	$(MULTISYNC_COMPOSE) logs -f

multisync-logs-%: ## Tail logs for a specific network (e.g., multisync-logs-hoodi).
	$(MULTISYNC_COMPOSE) logs -f ethrex-$* consensus-$*

multisync-logs-ethrex-%: ## Tail only ethrex logs for a network (e.g., multisync-logs-ethrex-hoodi).
	$(MULTISYNC_COMPOSE) logs -f ethrex-$*

multisync-logs-consensus-%: ## Tail only consensus logs for a network (e.g., multisync-logs-consensus-hoodi).
	$(MULTISYNC_COMPOSE) logs -f consensus-$*

multisync-restart: ## Restart the cycle (clean volumes + start fresh).
	$(MULTISYNC_COMPOSE) down -v
	$(MULTISYNC_COMPOSE) up -d $(MULTISYNC_SERVICES)

multisync-monitor: ## Monitor all networks (one-shot, exits on completion).
	python3 docker_monitor.py --networks $(MULTISYNC_NETWORKS) --exit-on-success

multisync-run: ## Full run: start + monitor (one-shot, exits on completion).
	$(MULTISYNC_COMPOSE) up -d $(MULTISYNC_SERVICES)
	@echo "Waiting 10s for containers to start..."
	@sleep 10
	python3 docker_monitor.py --networks $(MULTISYNC_NETWORKS) --exit-on-success

multisync-loop: ## Continuous loop: sync all networks, restart on success, repeat forever.
	$(MULTISYNC_COMPOSE) up -d $(MULTISYNC_SERVICES)
	@echo "Waiting 10s for containers to start..."
	@sleep 10
	python3 docker_monitor.py --networks $(MULTISYNC_NETWORKS) --compose-file docker-compose.multisync.yaml --compose-dir $(CURDIR)

multisync-history: ## View the run history log.
	@if [ -f multisync_logs/run_history.log ]; then \
		cat multisync_logs/run_history.log; \
	else \
		echo "No run history found. Run 'make multisync-loop' first."; \
	fi

multisync-list-logs: ## List all saved run logs.
	@if [ -d multisync_logs ]; then \
		echo "=== Saved Run Logs ===" && \
		ls -la multisync_logs/ && \
		echo "" && \
		for dir in multisync_logs/run_*/; do \
			if [ -d "$$dir" ]; then \
				echo "$$dir:"; \
				ls "$$dir"; \
				echo ""; \
			fi; \
		done; \
	else \
		echo "No logs directory found."; \
	fi

# ==============================================================================
# Multisync with Auto-Update and Validation
# ==============================================================================
# The multisync-loop-auto target pulls latest code and rebuilds before each run.
# When using 'release-with-debug-assertions' profile, state trie validation is
# enabled (same as daily snapsync CI checks). The validation verifies:
#   - State root: traverses entire account trie, validates all node hashes
#   - Storage roots: validates each account's storage trie (parallelized)
#   - Bytecodes: verifies code exists for all accounts with code
#
# If validation fails, the node exits with code 1 and logs the error.
# ==============================================================================

multisync-loop-auto: ## Continuous loop with auto-update: pull latest, build, and validate on each run.
	@echo "Starting multisync loop with auto-update enabled..."
	@echo "Branch: $(MULTISYNC_BRANCH)"
	@echo "Build profile: $(MULTISYNC_BUILD_PROFILE)"
	@echo "Image tag: $(MULTISYNC_LOCAL_IMAGE)"
	python3 docker_monitor.py \
		--networks $(MULTISYNC_NETWORKS) \
		--compose-file docker-compose.multisync.yaml \
		--compose-dir $(CURDIR) \
		--auto-update \
		--branch "$(MULTISYNC_BRANCH)" \
		--build-profile "$(MULTISYNC_BUILD_PROFILE)" \
		--image-tag "$(MULTISYNC_LOCAL_IMAGE)" \
		--ethrex-dir "$(ETHREX_DIR)"

# ==============================================================================
# Benchmark Tools
# ==============================================================================
# Tools for measuring and comparing snap sync performance across branches.
# ==============================================================================

.PHONY: benchmark-parse benchmark-compare benchmark-diff benchmark-help

BENCHMARK_RESULTS ?= benchmark_results.json
BENCHMARK_LOGS_DIR ?= ./multisync_logs

benchmark-help: ## Show benchmark tool help.
	python3 sync_benchmark.py --help

benchmark-parse: ## Parse all runs and generate benchmark results JSON.
	python3 sync_benchmark.py parse-all $(BENCHMARK_LOGS_DIR) --output $(BENCHMARK_RESULTS)

benchmark-compare: ## Compare all benchmark results (table format).
	@if [ -f $(BENCHMARK_RESULTS) ]; then \
		python3 sync_benchmark.py compare $(BENCHMARK_RESULTS) --format table; \
	else \
		echo "No benchmark results found. Run 'make benchmark-parse' first."; \
	fi

benchmark-compare-csv: ## Compare all benchmark results (CSV format).
	@if [ -f $(BENCHMARK_RESULTS) ]; then \
		python3 sync_benchmark.py compare $(BENCHMARK_RESULTS) --format csv; \
	else \
		echo "No benchmark results found. Run 'make benchmark-parse' first."; \
	fi

benchmark-compare-md: ## Compare all benchmark results (Markdown format).
	@if [ -f $(BENCHMARK_RESULTS) ]; then \
		python3 sync_benchmark.py compare $(BENCHMARK_RESULTS) --format markdown; \
	else \
		echo "No benchmark results found. Run 'make benchmark-parse' first."; \
	fi

benchmark-diff: ## Diff two runs. Usage: make benchmark-diff RUN1=<id1> RUN2=<id2> [NETWORK=mainnet]
ifndef RUN1
	$(error RUN1 is required. Usage: make benchmark-diff RUN1=20260203_082422 RUN2=20260202_120000)
endif
ifndef RUN2
	$(error RUN2 is required. Usage: make benchmark-diff RUN1=20260203_082422 RUN2=20260202_120000)
endif
	python3 sync_benchmark.py diff $(RUN1) $(RUN2) --results $(BENCHMARK_RESULTS) --network $(or $(NETWORK),mainnet)

benchmark-latest: ## Parse logs and show comparison of recent runs.
	@make benchmark-parse
	@make benchmark-compare

benchmark-parse-prefix: ## Parse all runs with a server prefix. Usage: make benchmark-parse-prefix PREFIX=server1
ifndef PREFIX
	$(error PREFIX is required. Usage: make benchmark-parse-prefix PREFIX=server1)
endif
	python3 sync_benchmark.py parse-all $(BENCHMARK_LOGS_DIR) --prefix $(PREFIX) --output $(PREFIX)_results.json

benchmark-merge: ## Merge multiple result files. Usage: make benchmark-merge FILES="server1_results.json server2_results.json"
ifndef FILES
	$(error FILES is required. Usage: make benchmark-merge FILES="server1_results.json server2_results.json")
endif
	python3 sync_benchmark.py merge $(FILES) --output $(BENCHMARK_RESULTS)
