.PHONY: sp1

sp1:
	SP1_PROVER=local cargo r -r --features build_sp1 -- --rpc-url ${RPC_URL}

sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "build_sp1,gpu" -- --rpc-url ${RPC_URL}

risc0:
	cargo r -r --features build_risc0 -- --rpc-url ${RPC_URL}

risc0-gpu:
	cargo r -r --features "build_risc0,gpu" -- --rpc-url ${RPC_URL}
