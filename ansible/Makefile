.PHONY: all

help:
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {if ($$1 ~ /^__/) printf "\033[33m%-50s\033[0m %s\n", $$1, $$2; else printf "\033[36m%-50s\033[0m %s\n", $$1, $$2}'

all: inventory ethrex-l2

inventory: ## Generates inventory for ansible
	@if [ -n "$(DATABASE_IP)" ] || [ -n "$(EXPLORER_FRONTEND_IP)" ] || [ -n "$(EXPLORER_BACKEND_IP)" ] || [ -n "$(L1_IP)" ] || [ -n "$(L2_IP)" ] || [ -n "$(PROVER_EXEC_IP)" ] || [ -n "$(PROVER_SP1_IP)" ] || [ -n "$(METRICS_IP)" ]; then \
		truncate -s0 inventory.ini; \
		if [ -n "$(DATABASE_IP)" ]; then \
			echo "[database]" >> inventory.ini; \
			for ip in $$(echo $(DATABASE_IP) | tr ',' ' '); do echo "$$ip ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(EXPLORER_FRONTEND_IP)" ]; then \
			echo "[explorer_frontend]" >> inventory.ini; \
			for ip in $$(echo $(EXPLORER_FRONTEND_IP) | tr ',' ' '); do echo "$$ip ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(EXPLORER_BACKEND_IP)" ]; then \
			echo "[explorer_backend]" >> inventory.ini; \
			for ip in $$(echo $(EXPLORER_BACKEND_IP) | tr ',' ' '); do echo "$$ip ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(L1_IP)" ]; then \
			echo "[l1]" >> inventory.ini; \
			for ip in $$(echo $(L1_IP) | tr ',' ' '); do echo "$$ip ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(L2_IP)" ]; then \
			echo "[l2]" >> inventory.ini; \
			for ip in $$(echo $(L2_IP) | tr ',' ' '); do echo "$$ip ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(PROVER_EXEC_IP)" ]; then \
			echo "[prover_exec]" >> inventory.ini; \
			for ip in $$(echo $(PROVER_EXEC_IP) | tr ',' ' '); do echo "$$ip ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(PROVER_SP1_IP)" ]; then \
			echo "[prover_sp1]" >> inventory.ini; \
			for ip in $$(echo $(PROVER_SP1_IP) | tr ',' ' '); do echo "$$ip ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
		if [ -n "$(METRICS_IP)" ]; then \
			echo "[metrics]" >> inventory.ini; \
			for ip in $$(echo $(METRICS_IP) | tr ',' ' '); do echo "$$ip ansible_python_interpreter=/usr/bin/python3 ansible_ssh_common_args='-o ForwardAgent=yes'" >> inventory.ini; done; \
			echo "" >> inventory.ini; \
		fi; \
	else \
		echo "Error: At least one of DATABASE_IP, EXPLORER_FRONTEND_IP, EXPLORER_BACKEND_IP, L1_IP, L2_IP, PROVER_EXEC_IP, PROVER_SP1_IP, or METRICS_IP must be set." 1>&2; exit 1; \
	fi

database: ## Installs Postgres database
	PGUSER=$(PGUSER)
	PGPASSWORD=$(PGPASSWORD)
	ansible-playbook -i inventory.ini database.yml

ethrex-l1: ## Installs Ethrex L1
	NETWORK=$(NETWORK)
	EVM=$(EVM)
	BOOTNODES=$(BOOTNODES)
	ansible-playbook -i inventory.ini ethrex-l1.yml

ethrex-dev-deps: ## Installs Ethrex Dependencies
	BRANCH=$(BRANCH)
	ansible-playbook -i inventory.ini ethrex-dev-deps.yml

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

ethrex-prover-exec: ## Installs ethrex prover exec
	ETHREX_SEQUENCER_ADDRESS=$(ETHREX_SEQUENCER_ADDRESS)
	ETHREX_SEQUENCER_PORT=$(ETHREX_SEQUENCER_PORT)
	ansible-playbook -i inventory.ini ethrex-prover-exec.yml

ethrex-prover-sp1: ## Installs ethrex prover SP1
	ETHREX_SEQUENCER_ADDRESS=$(ETHREX_SEQUENCER_ADDRESS)
	ETHREX_SEQUENCER_PORT=$(ETHREX_SEQUENCER_PORT)
	ansible-playbook -i inventory.ini ethrex-prover-sp1.yml

explorer_backend: ## Installs explorer backend
	DATABASE_URL=$(DATABASE_URL)
	ETHEREUM_JSONRPC_VARIANT=$(ETHEREUM_JSONRPC_VARIANT)
	ETHEREUM_JSONRPC_HTTP_URL=$(ETHEREUM_JSONRPC_HTTP_URL)
	API_V2_ENABLED=true
	PORT=3001
	COIN=$(COIN)
	COIN_NAME=$(COIN_NAME)
	DISPLAY_TOKEN_ICONS=true
	MIX_ENV=prod
	MICROSERVICE_SC_VERIFIER_ENABLED=true
	MICROSERVICE_SC_VERIFIER_URL=http://localhost:8082/
	POOL_SIZE=25
	POOL_SIZE_API=25
	DISABLE_WEBAPP=true
	ansible-playbook -i inventory.ini explorer_backend.yml

explorer_frontend: ## Installs explorer frontend
	NEXT_PUBLIC_API_HOST=$(NEXT_PUBLIC_API_HOST)
	NEXT_PUBLIC_API_PORT=8000
	NEXT_PUBLIC_API_PROTOCOL=https
	NEXT_PUBLIC_APP_HOST=$(NEXT_PUBLIC_APP_HOST)
	NEXT_PUBLIC_APP_PORT=8001
	NEXT_PUBLIC_APP_PROTOCOL=https
	NEXT_PUBLIC_APP_ENV=production
	NEXT_PUBLIC_API_WEBSOCKET_PROTOCOL=wss
	NEXT_PUBLIC_NETWORK_NAME=$(NEXT_PUBLIC_NETWORK_NAME)
	NEXT_PUBLIC_NETWORK_ID=1
	NEXT_PUBLIC_AD_BANNER_PROVIDER=none
	NEXT_PUBLIC_AD_TEXT_PROVIDER=none
	NEXT_PUBLIC_MARKETPLACE_ENABLED=false
	NEXT_PUBLIC_PROMOTE_BLOCKSCOUT_IN_TITLE=false
	NEXT_PUBLIC_NETWORK_LOGO=$(NEXT_PUBLIC_NETWORK_LOGO)
	NEXT_PUBLIC_NETWORK_LOGO_DARK=$(NEXT_PUBLIC_NETWORK_LOGO_DARK)
	NEXT_PUBLIC_NETWORK_ICON=$(NEXT_PUBLIC_NETWORK_ICON)
	NEXT_PUBLIC_NETWORK_ICON_DARK=$(NEXT_PUBLIC_NETWORK_ICON_DARK)
	NEXT_PUBLIC_STATS_API_HOST=$(NEXT_PUBLIC_STATS_API_HOST)
	NEXT_PUBLIC_GRAPHIQL_TRANSACTION=none
	NEXT_PUBLIC_API_SPEC_URL=none
	NEXT_PUBLIC_NAVIGATION_HIDDEN_LINKS="['eth_rpc_api','rpc_api']"
	ansible-playbook -i inventory.ini explorer_frontend.yml

metrics: ## Installs grafana and metrics
	ALERTS_SLACK_CHANNEL=$(ALERTS_SLACK_CHANNEL)
	ALERTS_SLACK_TOKEN=$(ALERTS_SLACK_TOKEN)
	GRAFANA_PASSWORD=$(GRAFANA_PASSWORD)
	ansible-playbook -i inventory.ini metrics.yml
