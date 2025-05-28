.PHONY = start_geth_holesky start_lighthouse_holesky gen_jwt

ETHREX_DIR ?= "/home/admin/sync-levm/ethrex"
NETWORK ?= holesky
EVM ?= levm
NODE_NAME ?= ethrex
ENGINE_PORT ?= 8551
CHECKPOINT_SYNC_URL ?= https://checkpoint-sync.holesky.ethpandaops.io
LIGHTHOUSE_PORT ?= 9099
LIGHTHOUSE_DISCOVERY_PORT ?= 9999
BOOTNODES=enode://a37d916374b92440247c20434389904f0f1f4ddf65e00ba7d482b551824e094db99a90b89124cc6ed98b89a9d5549afc3c307443e109c366a8e060bf27430c88@65.21.237.42:30303

create_data_dir:
	mkdir -p $(NETWORK)_data

gen_jwt: create_data_dir
	openssl rand -hex 32 | tr -d "\n" | sudo tee $(NETWORK)_data/jwt.hex

start_geth: create_data_dir
	mkdir -p $(NETWORK)_data
	./start_geth.sh $(NETWORK)

sync_levm: create_data_dir
	cd $(ETHREX_DIR) && git checkout main
	mkdir -p $(NETWORK)_data
#	samply record --unstable-presymbolicate --save-only -- 
	make start_ethrex EVM=levm >> ./logs/ethrex-sync-$(NETWORK)-levm.log

sync_revm: create_data_dir
	cd $(ETHREX_DIR) && git checkout main 
	mkdir -p $(NETWORK)_data                                       
#	samply record --unstable-presymbolicate --save-only --        
	make start_ethrex EVM=revm >> ./logs/ethrex-sync-$(NETWORK)-revm.log	

flamegraph-main:
	cd $(ETHREX_DIR) && git checkout main && make flamegraph >> logs/ethrex-$(NETWORK)-$(EVM)-flamegraph-$(shell date +'%y.%m.%d-%H:%M:%S')-main-$(LOGNAME).log	

flamegraph-branch: 
	cd $(ETHREX_DIR) && git checkout $(BRANCH) && make flamegraph >> logs/ethrex-$(NETWORK)-$(EVM)-flamegraph-$(shell date +'%y.%m.%d-%H:%M:%S')-$(BRANCH)-$(LOGNAME).log

flamegraph:
	CARGO_PROFILE_RELEASE_DEBUG=true RUST_LOG=3 cargo flamegraph --features libmdbx --bin ethrex -- \
		--http.port 8545 \
		--authrpc.port 8551 \
		--p2p.port 30303\
		--discovery.port 30303 \
		--network $(NETWORK) \
		--datadir "/home/admin/.local/share/${EVM}_${NETWORK}_data/ethrex" \
		--authrpc.jwtsecret "/home/admin/sync-levm/holesky_data/jwt.hex" \
		--bootnodes $(BOOTNODES) \
		--evm $(EVM) && cp ethrex/flamegraph.svg flamegraphs/ethrex-$(NETWORK)-$(EVM)--flamegraph-$(shell date +'%y.%m.%d-%H:%M:%S')-main$(LOGNAME).svg	

backup-db-levm:
	mkdir /home/admin/.local/share/levm_$(NETWORK)_data/ethrex/backups
	mkdir /home/admin/.local/share/levm_$(NETWORK)_data/ethrex/backups/db_backup_$(shell date +'%y.%m.%d')
	rsync -ah --info=progress2 /home/admin/.local/share/levm_$(NETWORK)_data/ethrex/mdbx.* /home/admin/.local/share/levm_$(NETWORK)_data/ethrex/backups/db_backup_levm_$(shell date +'%y.%m.%d')	
	cp ./logs/ethrex-sync-holesky-levm.log /home/admin/.local/share/levm_$(NETWORK)_data/ethrex/backups/db_backup_levm_$(shell date +'%y.%m.%d')/ethrex_sync_holesky_levm.log

backup-db-revm:
	mkdir /home/admin/.local/share/levm_$(NETWORK)_data/ethrex/backups
	mkdir /home/admin/.local/share/revm_$(NETWORK)_data/ethrex/backups/db_backup_revm_$(shell date +'%y.%m.%d')
	rsync -ah --info=progress2 /home/admin/.local/share/revm_$(NETWORK)_data/ethrex/mdbx.* /home/admin/.local/share/revm_$(NETWORK)_data/ethrex/backups/db_backup_revm_$(shell date +'%y.%m.%d')	
	cp ./logs/ethrex-sync-holesky-revm.log /home/admin/.local/share/revm_$(NETWORK)_data/ethrex/backups/db_backup_revm_$(shell date +'%y.%m.%d')/ethrex_sync_holesky_revm.log

start_ethrex:
	cd $(ETHREX_DIR) && RUST_LOG=3 cargo run --release --features libmdbx --bin ethrex -- \
    		--http.port 8545 \
    		--authrpc.port 8551 \
    		--p2p.port 30303\
    		--discovery.port 30303 \
    		--network $(NETWORK) \
    		--datadir "/home/admin/.local/share/${EVM}_${NETWORK}_data/ethrex" \
    		--authrpc.jwtsecret "/home/admin/sync-levm/${NETWORK}_data/jwt.hex" \
    		--bootnodes $(BOOTNODES) \
    		--evm $(EVM)


tail-syncing-logs:
	tail -f logs/ethrex-sync-$(NETWORK)-$(EVM).log -n 200 | grep -e "SYNCING"

tail-metrics-logs:
	tail -f logs/ethrex-sync-$(NETWORK)-$(EVM).log -n 2000 | grep -A4 -e "METRICS"
