.PHONY: all

help:
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {if ($$1 ~ /^__/) printf "\033[33m%-50s\033[0m %s\n", $$1, $$2; else printf "\033[36m%-50s\033[0m %s\n", $$1, $$2}'

all: inventory ethrex-l2

inventory: ## Generates inventory for ansible
	@if [ -n "$(DATABASE_IP)" ] || [ -n "$(EXPLORER_IP)" ] || [ -n "$(L1_IP)" ] || [ -n "$(L2_IP)" ] || [ -n "$(MONITORING_IP)" ]; then \
		truncate -s0 inventory.ini; \
		if [ -n "$(DATABASE_IP)" ]; then \
			echo "[database]" >> inventory.ini; \
			for ip in $$(echo $(DATABASE_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(EXPLORER_IP)" ]; then \
			echo "[explorer]" >> inventory.ini; \
			for ip in $$(echo $(EXPLORER_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=app ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(L1_IP)" ]; then \
			echo "[l1]" >> inventory.ini; \
			for ip in $$(echo $(L1_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=app ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(L2_IP)" ]; then \
			echo "[l2]" >> inventory.ini; \
			for ip in $$(echo $(L2_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=app ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(MONITORING_IP)" ]; then \
			echo "[metrics]" >> inventory.ini; \
			for ip in $$(echo $(MONITORING_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3 ansible_ssh_common_args='-o ForwardAgent=yes'" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
	else \
		echo "Error: At least one of DATABASE_IP, EXPLORER_IP, L1_IP, L2_IP, or MONITORING_IP must be set." 1>&2; exit 1; \
	fi

database: ## Installs Postgres database
	ansible-playbook -i inventory.ini database.yml

ethrex-l1: ## Installs Ethrex L1
	NETWORK=$(NETWORK)
	EVM=$(EVM)
	BOOTNODES=$(BOOTNODES)
	ansible-playbook -i inventory.ini ethrex-l1.yml

ethrex-l2: ## Installs Ethrex L2
	GENESIS_JSON_FILE=$(GENESIS_JSON_FILE)
	COMMON_BRIDGE_ADDRESS=$(COMMON_BRIDGE_ADDRESS)
	ON_CHAIN_PROPOSER_ADDRESS=$(ON_CHAIN_PROPOSER_ADDRESS)
	COMMITTER_PK=$(COMMITTER_PK)
	PROOF_SENDER_PK=$(PROOF_SENDER_PK)
	COINBASE_ADDRESS=$(COINBASE_ADDRESS)
	ETH_TESTNET_RPC=$(TESTNET_RPC)
	INFURA_API_KEY=$(INFURA_API_KEY)
	ansible-playbook -i inventory.ini ethrex-l2.yml

explorer_backend: ## Installs explorer backend
	DATABASE_URL=$(DATABASE_URL) # ecto://user:pass@127.0.0.1:5432/db
	ETHEREUM_JSONRPC_VARIANT=geth
	ETHEREUM_JSONRPC_HTTP_URL=http://localhost:8545
	API_V2_ENABLED=true
	PORT=3001
	COIN=ETH
	COIN_NAME=ETH
	DISPLAY_TOKEN_ICONS=true
	MIX_ENV=prod
	MICROSERVICE_SC_VERIFIER_ENABLED=true
	MICROSERVICE_SC_VERIFIER_URL=http://localhost:8082/
	POOL_SIZE=25
	POOL_SIZE_API=25
	DISABLE_WEBAPP=true
	ansible-playbook -i inventory.ini explorer_backend.yml

explorer_fontend: ## Installs explorer frontend
	ansible-playbook -i inventory.ini explorer_fontend.yml

metrics: ## Installs grafana and metrics
	ansible-playbook -i inventory.ini metrics.yml
