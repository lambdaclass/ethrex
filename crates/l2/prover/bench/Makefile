.PHONY: sp1

# Execution (for development)
sp1:
	SP1_PROVER=local cargo r -r --features sp1 -- --rpc-url ${RPC_URL}
sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- --rpc-url ${RPC_URL}
risc0:
	cargo r -r --features risc0 -- --rpc-url ${RPC_URL}
risc0-gpu:
	cargo r -r --features "risc0,gpu" -- --rpc-url ${RPC_URL}

# Proving
prove-sp1:
	SP1_PROVER=local cargo r -r --features sp1 -- --rpc-url ${RPC_URL} --prove
prove-sp1-gpu:
	SP1_PROVER=cuda cargo r -r --features "sp1,gpu" -- --rpc-url ${RPC_URL} --prove
prove-risc0:
	cargo r -r --features risc0 -- --rpc-url ${RPC_URL} --prove
prove-risc0-gpu:
	cargo r -r --features "risc0,gpu" -- --rpc-url ${RPC_URL} --prove
