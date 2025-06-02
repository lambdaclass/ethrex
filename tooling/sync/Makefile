.PHONY = start_geth_holesky start_lighthouse_holesky gen_jwt

ETHREX_DIR ?= "../.."
ifndef SYNC_NETWORK
ifndef NETWORK
$(error "Sync network not provided")
endif
endif
NETWORK ?= $(SYNC_NETWORK)
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

create_data_dir:
	mkdir -p $(NETWORK)_data

gen_jwt: create_data_dir
	openssl rand -hex 32 | tr -d "\n" | sudo tee $(NETWORK)_data/jwt.hex

start_geth: create_data_dir
	mkdir -p $(NETWORK)_data
	./start_geth.sh $(NETWORK)



sync: create_data_dir
	#cd $(ETHREX_DIR) && git checkout main
	mkdir -p $(NETWORK)_data
#	samply record --unstable-presymbolicate --save-only -- 
	make start_ethrex_$(NETWORK) >> ./logs/ethrex-sync-$(NETWORK)-$(EVM).log


flamegraph-main:
	cd $(ETHREX_DIR) && git checkout main
	#export SYNC-BLOCK-NUM = $(BLOCK-NUM)
	make flamegraph-$(NETWORK) >> logs/ethrex-$(NETWORK)-$(EVM)-flamegraph-$(shell date +'%y.%m.%d-%H:%M:%S')-main-$(LOGNAME).log	

flamegraph-branch: 
	cd $(ETHREX_DIR) && git checkout $(BRANCH)
	make flamegraph >> logs/ethrex-$(NETWORK)-$(EVM)-flamegraph-$(shell date +'%y.%m.%d-%H:%M:%S')-$(BRANCH)-$(LOGNAME).log

flamegraph-holesky:
	cd $(ETHREX_DIR) && CARGO_PROFILE_RELEASE_DEBUG=true RUST_LOG=3 cargo flamegraph --features "libmdbx sync-test" --bin ethrex -- \
		--http.port 8545 \
		--authrpc.port 8551 \
		--p2p.port 30303\
		--discovery.port 30303 \
		--network holesky \
		--datadir "${EVM}_holesky_data/ethrex" \
		--authrpc.jwtsecret "/holesky_data/jwt.hex" \
		--bootnodes $(HOLESKY_BOOTNODES) \
		--evm $(EVM) && cp ethrex/flamegraph.svg flamegraphs/ethrex-holesky-$(EVM)--flamegraph-$(shell date +'%y.%m.%d-%H:%M:%S')-main$(LOGNAME).svg	

flamegraph-hoodi:
	cd $(ETHREX_DIR) && CARGO_PROFILE_RELEASE_DEBUG=true RUST_LOG=3 cargo flamegraph --features "libmdbx sync-test" --bin ethrex -- \
		--http.port 8545 \
		--authrpc.port 8551 \
		--p2p.port 30303\
		--discovery.port 30303 \
		--network hoodi \
		--datadir "${EVM}_hoodi_data/ethrex" \
		--authrpc.jwtsecret "/hoodi_data/jwt.hex" \
		--bootnodes $(HOODI_BOOTNODES) \
		--evm $(EVM) && cp ethrex/flamegraph.svg flamegraphs/ethrex-hoodi-$(EVM)--flamegraph-$(shell date +'%y.%m.%d-%H:%M:%S')-main$(LOGNAME).svg	

start-lighthouse:
	make $(NETWORK)-lighthouse

holesky-lighthouse:
	lighthouse bn \
		--network $(NETWORK) \
		--execution-endpoint http://localhost:${ENGINE_PORT} \
		--execution-jwt ${NETWORK}_data/jwt.hex \
		--checkpoint-sync-url $(HOLESKY_CHECKPOINT_SYNC_URL) \
		--http \
		--http-address 0.0.0.0 \
		--http-allow-origin "*" \
		--datadir $(EVM)_${NETWORK}_data/lighthouse_${NODE_NAME}/ \
		--disable-deposit-contract-sync --port $(LIGHTHOUSE_PORT) --discovery-port $(LIGHTHOUSE_DISCOVERY_PORT) --http-port 5053

hoodi-lighthouse:
	lighthouse bn \
		--network $(NETWORK) \
		--execution-endpoint http://localhost:${ENGINE_PORT} \
		--execution-jwt ${NETWORK}_data/jwt.hex \
		--checkpoint-sync-url $(HOODI_CHECKPOINT_SYNC_URL) \
		--http \
		--http-address 0.0.0.0 \
		--http-allow-origin "*" \
		--datadir $(EVM)_${NETWORK}_data/lighthouse_${NODE_NAME}/ \
		--disable-deposit-contract-sync --port $(LIGHTHOUSE_PORT) --discovery-port $(LIGHTHOUSE_DISCOVERY_PORT) --http-port 5053



backup-db:
	#mkdir /home/admin/.local/share/levm_$(NETWORK)_data/ethrex/backups
	mkdir -p /home/admin/.local/share/ethrex_db_backups/db_backup_$(EVM)_$(NETWORK)_$(CURRENT_DATETIME)
	rsync -ah --info=progress2 /home/admin/.local/share/$(EVM)_$(NETWORK)_data/ethrex/mdbx.* /home/admin/.local/share/ethrex_db_backups/db_backup_$(EVM)_$(NETWORK)_$(CURRENT_DATETIME)
	cp ./logs/ethrex-sync-holesky-levm.log /home/admin/.local/share/ethrex_db_backups/db_backup_$(EVM)_$(NETWORK)_$(CURRENT_DATETIME)/ethrex_sync_holesky_levm.log


start_ethrex_hoodi:
	cd $(ETHREX_DIR) && RUST_LOG=3 cargo run --release --features "libmdbx sync-test" --bin ethrex -- \
    		--http.port 8545 \
    		--authrpc.port 8551 \
    		--p2p.port 30303\
    		--discovery.port 30303 \
    		--network hoodi \
    		--datadir "$(EVM)_hoodi_data/ethrex" \
    		--authrpc.jwtsecret "tools/sync/hoodi_data/jwt.hex" \
    		--bootnodes $(HOODI_BOOTNODES) \
    		--evm $(EVM)

start_ethrex_holesky:
	cd $(ETHREX_DIR) && RUST_LOG=3 cargo run --release --features "libmdbx sync-test" --bin ethrex -- \
    		--http.port 8545 \
    		--authrpc.port 8551 \
    		--p2p.port 30303\
    		--discovery.port 30303 \
    		--network holesky \
    		--datadir "$(EVM)_holesky_data/ethrex" \
    		--authrpc.jwtsecret "tools/sync/holesky_data/jwt.hex" \
    		--bootnodes $(HOLESKY_BOOTNODES) \
    		--evm $(EVM)


tail-syncing-logs:
	tail -f logs/$(LOGNAME) -n 200 | grep -e "SYNCING"

tail-metrics-logs:
	tail -f logs/$(LOGNAME) -n 2000 | grep -A4 -e "METRICS"

