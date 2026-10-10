.PHONY: inventory ethrex-deploy ethrex-development

help:
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {if ($$1 ~ /^__/) printf "\033[33m%-50s\033[0m %s\n", $$1, $$2; else printf "\033[36m%-50s\033[0m %s\n", $$1, $$2}'

inventory: ## Generates inventory for ansible
	@if [ -n "$TARGET" ]; then \
		truncate -s0 inventory.ini; \
		echo "[all]" >> inventory.ini; \
		for ip in $$(echo $(TARGET) | tr ',' ' '); do echo "$$ip ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
		echo "" >> inventory.ini; \
	else \
		echo "Error: TARGET variable was not set." 1>&2; exit 1; \
	fi

ethrex-deploy: ## Deploys Ethrex nodes (NETWORK optional, defaults to mainnet)
	NETWORK=$(NETWORK) ansible-playbook -i inventory.ini ethrex-deploy.yml

ethrex-development: ## Sets up Ethrex development environment (requires BRANCH and NETWORK (defaults for mainnet))
	@if [ -z "$(BRANCH)" ]; then \
		echo "Error: BRANCH variable was not set."; \
		exit 1; \
	fi
	BRANCH=$(BRANCH) NETWORK=$(NETWORK) ansible-playbook -i inventory.ini ethrex-development.yml
