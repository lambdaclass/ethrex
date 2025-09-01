.PHONY = help create_data_dir gen_jwt sync start_geth_holesky flamegraph-main  flamegraph \
flamegraph-branch flamegraph-inner flamegraph-mainnet flamegraph-sepolia flamegraph-holesky \
flamegraph-hoodi start-lighthouse start-ethrex backup-db start-mainnet-metrics-docker \
start-sepolia-metrics-docker start-holesky-metrics-docker start-hoodi-metrics-docker \
start-metrics-docker tail-syncing-logs tail-metrics-logs copy_flamegraph import-with-metrics

ETHREX_DIR ?= "../.."
EVM ?= levm
NODE_NAME ?= ethrex
ENGINE_PORT ?= 8551
LIGHTHOUSE_PORT ?= 9099
LIGHTHOUSE_DISCOVERY_PORT ?= 9999
CURRENT_DATETIME = $(shell date +'%y.%m.%d-%H.%M.%S')
BATCH_SIZE ?= 1024
OS = $(shell uname)
ifeq ($(OS), Darwin)
	DATA_PATH = ~/Library/Application\ Support
else 
	DATA_PATH = ~/.local/share
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

create_data_dir: ## Create the data folder for the network. If NETWORK environment variable is not provided, mainnet will be used as default.
	mkdir -p $(DATA_PATH)/$(NETWORK)_data

gen-jwt: create_data_dir ## Create the jwt for a given network. NETWORK environment variable required.
	openssl rand -hex 32 | tr -d "\n" | tee $(DATA_PATH)/$(NETWORK)_data/jwt.hex

sync: create_data_dir ## Run the sync for a given network. SYNC_BLOCK_NUM environment variable required (for block to start from).  If NETWORK environment variable is not provided, mainnet will be used as default. EVM can also be set to select the evm to use.
ifndef SYNC_BLOCK_NUM
	$(error "Sync block number not set")
endif	
#	samply record --unstable-presymbolicate --save-only -- 
	mkdir -p logs
	make start-ethrex-$(NETWORK) >> ./logs/ethrex-sync-$(NETWORK)-$(EVM).log


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
	cd $(ETHREX_DIR) && CARGO_PROFILE_RELEASE_DEBUG=true RUST_LOG=3 cargo flamegraph --features "libmdbx sync-test" --bin ethrex -- \
		--http.port 8545 \
		--authrpc.port 8551 \
		--p2p.tcp.port 30303\
		--p2p.udp.port 30303 \
		--network $(NETWORK) \
		--datadir ${NETWORK}_data/ethrex/$(EVM) \
		--authrpc.jwtsecret $(DATA_PATH)/${NETWORK}_data/jwt.hex \
		$(BOOTNODES_FLAG) \
		--evm $(EVM) \

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
	cd $(ETHREX_DIR) && cargo run --release --features "metrics" --bin ethrex -- \
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
		--disable-deposit-contract-sync --port $(LIGHTHOUSE_PORT) --discovery-port $(LIGHTHOUSE_DISCOVERY_PORT) --http-port 5053

start-ethrex: ## Start ethrex for the network given by NETWORK.
	cd $(ETHREX_DIR) && RUST_LOG=3 cargo run --release --features "libmdbx sync-test metrics" --bin ethrex -- \
    		--http.addr 0.0.0.0 \
    		--http.port 8545 \
    		--authrpc.port 8551 \
    		--p2p.tcp.port 30303\
    		--p2p.udp.port 30303 \
    		--metrics \
    		--metrics.port 3701 \
    		--network $(NETWORK) \
    		--datadir "${NETWORK}_data/ethrex/$(EVM)" \
    		--authrpc.jwtsecret $(DATA_PATH)/${NETWORK}_data/jwt.hex \
    		$(BOOTNODES_FLAG) \
    		--evm $(EVM)
