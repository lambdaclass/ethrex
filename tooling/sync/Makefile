.PHONY = start_geth_holesky start_lighthouse_holesky gen_jwt

ETHREX_DIR ?= "../.."
EVM ?= levm
NODE_NAME ?= ethrex
ENGINE_PORT ?= 8551
HOLESKY_CHECKPOINT_SYNC_URL ?= https://checkpoint-sync.holesky.ethpandaops.io
LIGHTHOUSE_PORT ?= 9099
LIGHTHOUSE_DISCOVERY_PORT ?= 9999
HOLESKY_BOOTNODES=enode://a37d916374b92440247c20434389904f0f1f4ddf65e00ba7d482b551824e094db99a90b89124cc6ed98b89a9d5549afc3c307443e109c366a8e060bf27430c88@65.21.237.42:30303
HOODI_CHECKPOINT_SYNC_URL ?= https://hoodi.beaconstate.ethstaker.cc
HOODI_BOOTNODES ?= enode://60203fcb3524e07c5df60a14ae1c9c5b24023ea5d47463dfae051d2c9f3219f309657537576090ca0ae641f73d419f53d8e8000d7a464319d4784acd7d2abc41@209.38.124.160:30303
CURRENT_DATETIME = $(shell date +'%y.%m.%d-%H.%M.%S')
BATCH_SIZE ?= 1024
OS = $(shell uname)
ifeq ($(OS), Darwin)
	DATA_PATH = ~/Library/Application\ Support
else 
	DATA_PATH = ~/.local/share
endif

help: ## Display help for the makefile.
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

create_data_dir: ## Create the data folder for the network. NETWORK environment variable required.
ifndef NETWORK
	$(error "Sync network not provided")
endif
	mkdir -p $(DATA_PATH)/$(NETWORK)_data

gen_jwt: create_data_dir ## Create the jwt for a given network. NETWORK environment variable required.
ifndef NETWORK
	$(error "Sync network not provided")
endif
	openssl rand -hex 32 | tr -d "\n" | tee $(DATA_PATH)/$(NETWORK)_data/jwt.hex

sync: create_data_dir ## Run the sync for a given network. NETWORK and SYNC_BLOCK_NUM environment variables required (for the network and block to start from respectively). EVM can also be set to select the evm to use.
ifndef SYNC_BLOCK_NUM
	$(error "Sync block number not set")
endif	
ifndef NETWORK
	$(error "Sync network not provided")
endif
#	samply record --unstable-presymbolicate --save-only -- 
	make start_ethrex_$(NETWORK) >> ./logs/ethrex-sync-$(NETWORK)-$(EVM).log


flamegraph-main: ## Run flamegraph on main branch. NETWORK and SYNC_BLOCK_NUM environment variables required (for the network and block to start from respectively). EVM can also be set to select the evm to use. Execution logs are output to log file.
ifndef SYNC_BLOCK_NUM
	$(error "Sync block number not set)
endif
ifndef NETWORK
	$(error "Sync network not provided")
endif
	cd $(ETHREX_DIR) && git checkout main
	make flamegraph-$(NETWORK) >> logs/ethrex-$(NETWORK)-$(EVM)-flamegraph-$(CURRENT_DATETIME)-main-block-$(SYNC_BLOCK_NUM)-$(LOGNAME).log

flamegraph-branch: ## Run flamegraph on custom branch. NETWORK and SYNC_BLOCK_NUM environment variables required (for the network and block to start from respectively). EVM can also be set to select the evm to use. xecution logs are output to log file.
ifndef SYNC_BLOCK_NUM
	$(error "Sync block number not set")
endif
ifndef BRANCH
	$(error "Branch not specified")
endif
ifndef NETWORK
	$(error "Sync network not provided")
endif
	cd $(ETHREX_DIR) && git checkout $(BRANCH)
	make flamegraph-$(NETWORK) >> logs/ethrex-$(NETWORK)-$(EVM)-flamegraph-$(CURRENT_DATETIME)-$(BRANCH)-block-$(SYNC_BLOCK_NUM)-$(LOGNAME).log

flamegraph-holesky: ## Run flamegraph on holesky.
	cd $(ETHREX_DIR) && CARGO_PROFILE_RELEASE_DEBUG=true RUST_LOG=3 cargo flamegraph --features "libmdbx sync-test" --bin ethrex -- \
		--http.port 8545 \
		--authrpc.port 8551 \
		--p2p.port 30303\
		--discovery.port 30303 \
		--network holesky \
		--datadir holesky_data/ethrex/$(EVM) \
		--authrpc.jwtsecret $(DATA_PATH)/holesky_data/jwt.hex \
		--bootnodes $(HOLESKY_BOOTNODES) \
		--evm $(EVM) \

flamegraph-hoodi: ## Run flamegraph on hoodi.
	-cd $(ETHREX_DIR) && CARGO_PROFILE_RELEASE_DEBUG=true RUST_LOG=3 cargo flamegraph --features "libmdbx sync-test" --bin ethrex -- \
		--http.port 8545 \
		--authrpc.port 8551 \
		--p2p.port 30303\
		--discovery.port 30303 \
		--network hoodi \
		--datadir hoodi_data/ethrex/$(EVM) \
		--authrpc.jwtsecret $(DATA_PATH)/hoodi_data/jwt.hex \
		--bootnodes $(HOODI_BOOTNODES) \
		--evm $(EVM)

start-lighthouse: ## Start lighthouse for a given network. NETWORK environment variable required.
ifndef NETWORK
	$(error "Sync network not provided")
endif
	make $(NETWORK)-lighthouse

holesky-lighthouse: ## Start lighthouse for holesky.
	lighthouse bn \
		--network holesky \
		--execution-endpoint http://localhost:${ENGINE_PORT} \
		--execution-jwt $(DATA_PATH)/holesky_data/jwt.hex \
		--checkpoint-sync-url $(HOLESKY_CHECKPOINT_SYNC_URL) \
		--http \
		--http-address 0.0.0.0 \
		--http-allow-origin "*" \
		--metrics \
  		--metrics-address 0.0.0.0 \
  		--metrics-port 5054 \
		--datadir $(DATA_PATH)/holesky_data/lighthouse_${NODE_NAME}_$(EVM) \
		--disable-deposit-contract-sync --port $(LIGHTHOUSE_PORT) --discovery-port $(LIGHTHOUSE_DISCOVERY_PORT) --http-port 5053

hoodi-lighthouse: ## Start lighthouse for hoodi.
	lighthouse bn \
		--network hoodi \
		--execution-endpoint http://localhost:${ENGINE_PORT} \
		--execution-jwt $(DATA_PATH)/hoodi_data/jwt.hex \
		--checkpoint-sync-url $(HOODI_CHECKPOINT_SYNC_URL) \
		--http \
		--http-address 0.0.0.0 \
		--http-allow-origin "*" \
		--metrics \
  		--metrics-address 0.0.0.0 \
  		--metrics-port 5054 \
		--datadir $(DATA_PATH)/hoodi_data/lighthouse_${NODE_NAME}_$(EVM) \
		--disable-deposit-contract-sync --port $(LIGHTHOUSE_PORT) --discovery-port $(LIGHTHOUSE_DISCOVERY_PORT) --http-port 5053

backup-db: ## Back-up the store db. EVM and NETWORK environment variables need to be provided to select which DB to back up.
ifndef NETWORK
	$(error "Sync network not provided")
endif
	mkdir -p $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)
ifeq ($(OS), Darwin)
	rsync -ah --progress $(DATA_PATH)/$(NETWORK)_data/ethrex/$(EVM)/mdbx.* $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)
	rsync -ah --progress ./logs/ethrex-sync-$(NETWORK)-$(EVM).log $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)/ethrex-sync-$(NETWORK)-$(EVM).log
else
	rsync -ah --info=progress2 $(DATA_PATH)/$(NETWORK)_data/ethrex/$(EVM)/mdbx.* $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)
	rsync -ah --info=progress2 ./logs/ethrex-sync-$(NETWORK)-$(EVM).log $(DATA_PATH)/ethrex_db_backups/$(NETWORK)/$(EVM)/db_backup_$(CURRENT_DATETIME)/ethrex-sync-$(NETWORK)-$(EVM).log
endif


start_ethrex_hoodi: ## Start ethrex for hoodi
	cd $(ETHREX_DIR) && RUST_LOG=3 cargo run --release --features "libmdbx sync-test metrics" --bin ethrex -- \
    		--http.addr 0.0.0.0 \
    		--http.port 8545 \
    		--authrpc.port 8551 \
    		--p2p.port 30303\
    		--metrics \
    		--metrics.port 3701 \
    		--discovery.port 30303 \
    		--network hoodi \
    		--datadir "hoodi_data/ethrex/$(EVM)" \
    		--authrpc.jwtsecret $(DATA_PATH)/hoodi_data/jwt.hex \
    		--bootnodes $(HOODI_BOOTNODES) \
    		--evm $(EVM)

start_ethrex_holesky: ## Start ethrex for holesky
	cd $(ETHREX_DIR) && RUST_LOG=3 cargo run --release --features "libmdbx sync-test metrics" --bin ethrex -- \
    		--http.addr 0.0.0.0 \
    		--http.port 8545 \
    		--authrpc.port 8551 \
    		--p2p.port 30303\
    		--metrics \
    		--metrics.port 3701 \
    		--discovery.port 30303 \
    		--network holesky \
    		--datadir "holesky_data/ethrex/$(EVM)" \
    		--authrpc.jwtsecret $(DATA_PATH)/holesky_data/jwt.hex \
    		--bootnodes $(HOLESKY_BOOTNODES) \
    		--evm $(EVM)

start_hoodi_metrics_docker: ## Start L1 docker compose, lighthouse in background, and ethrex for hoodi
	@echo "Starting L1 docker compose with metrics..."
	cd $(ETHREX_DIR)/crates/blockchain/metrics && docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up -d
	@echo "Starting lighthouse in background..."
	cd $(ETHREX_DIR)/tooling/sync && nohup make hoodi-lighthouse > /dev/null 2>&1 &
	@echo "Starting ethrex..."
	cd $(ETHREX_DIR)/tooling/sync && make start_ethrex_hoodi

start_hoodi_metrics: ## Start just metrics exporter docker compose, lighthouse in background, and ethrex for hoodi
	@echo "Starting L1 docker compose with metrics..."
	cd $(ETHREX_DIR)/crates/blockchain/metrics && docker compose -f docker-compose-metrics-exporter.yaml  up -d
	@echo "Starting lighthouse in background..."
	cd $(ETHREX_DIR)/tooling/sync && nohup make hoodi-lighthouse > /dev/null 2>&1 &
	@echo "Starting ethrex..."
	cd $(ETHREX_DIR)/tooling/sync && make start_ethrex_hoodi

start_holesky_metrics_docker: ## Start L1 docker compose, lighthouse in background, and ethrex for holesky
	@echo "Starting L1 docker compose with metrics..."
	cd $(ETHREX_DIR)/crates/blockchain/metrics && docker compose -f docker-compose-metrics.yaml -f docker-compose-metrics-l1.overrides.yaml up -d
	@echo "Starting lighthouse in background..."
	cd $(ETHREX_DIR)/tooling/sync && nohup make holesky-lighthouse > /dev/null 2>&1 &
	@echo "Starting ethrex..."
	cd $(ETHREX_DIR)/tooling/sync && make start_ethrex_holesky

start_holesky_metrics: ## Start just metrics exporter docker compose, lighthouse in background, and ethrex for holesky
	@echo "Starting L1 docker compose with metrics..."
	cd $(ETHREX_DIR)/crates/blockchain/metrics && docker compose -f docker-compose-metrics-exporter.yaml up -d
	@echo "Starting lighthouse in background..."
	cd $(ETHREX_DIR)/tooling/sync && nohup make holesky-lighthouse > /dev/null 2>&1 &
	@echo "Starting ethrex..."
	cd $(ETHREX_DIR)/tooling/sync && make start_ethrex_holesky

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
