.PHONY: help run-tests

help: ## 📚 Show help for each of the Makefile recipes
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

run-tests: ## 🧪 Run all tests with optional flags. Usage: make run-tests flags="--flag1 --flag2"
	cargo test --package ef_tests-statev2 --test all --release -- $(flags)

