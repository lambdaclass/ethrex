automata-dcap-qpl:
	git clone https://github.com/automata-network/automata-dcap-qpl

build:
	cargo build --release
	cd automata-dcap-qpl/automata-dcap-qpl-tool; cargo build --release

run: build
	CHAIN_ID=$(shell cast chain-id --rpc-url $(RPC_URL)) cargo run --release

.PHONY: build run
