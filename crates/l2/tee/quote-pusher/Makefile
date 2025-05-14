automata-dcap-qpl:
	git clone https://github.com/automata-network/automata-dcap-qpl
	cd automata-dcap-qpl/automata-dcap-qpl-tool; git clone https://github.com/automata-network/pccs-reader-rs
	rm automata-dcap-qpl/Cargo.toml
	echo "" >> automata-dcap-qpl/automata-dcap-qpl-tool/Cargo.toml
	echo "[workspace]" >> automata-dcap-qpl/automata-dcap-qpl-tool/Cargo.toml

build: automata-dcap-qpl
	cargo build --release
	cd automata-dcap-qpl/automata-dcap-qpl-tool; cargo build --release

run: build
	cargo run --release

test: build
	cargo test --release

# Assumes dev L1 is running (see `make dev` on ethrex)
integration-test:
	export $(shell cat .env | xargs); \
	cd ../contracts; \
	make deploy-deps; \
	make deploy; \
	make mkenv; \
	export $(shell cat ../contracts/.env.out | xargs); \
	cd ../quote-pusher; \
	make test

.PHONY: build run test integration-test
