.PHONY: run-evm-ef-tests

run-evm-ef-tests: ## 🏃‍♂️ Run EF Tests
	cd ../../../crates/vm/levm && \
	make run-evm-ef-tests flags="$(flags)"

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'
