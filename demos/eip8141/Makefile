.PHONY: compile-contracts start-node start-backend start-frontend start-all install clean

# Compile all contracts and generate genesis.json with real bytecodes
compile-contracts:
	@echo "Compiling Solidity contracts..."
	@solc --via-ir --bin-runtime --optimize --optimize-runs 200 \
		--base-path . @solady/=contracts/deps/solady/ \
		contracts/src/MockERC20.sol \
		contracts/src/WebAuthnVerifier.sol \
		-o contracts/out --overwrite 2>/dev/null
	@echo "Compiling Yul contracts (with verbatim for EIP-8141 opcodes)..."
	@solc --strict-assembly contracts/yul/GasSponsor.yul > /dev/null 2>&1
	@solc --strict-assembly contracts/yul/WebAuthnP256Account.yul > /dev/null 2>&1
	@echo "All contracts compiled successfully."
	@echo "Run 'make genesis' to update genesis.json with compiled bytecodes."

# Generate genesis.json with compiled runtime bytecodes
genesis: compile-contracts
	@echo "Generating genesis.json..."
	@bash scripts/build-genesis.sh
	@echo "genesis.json updated with compiled bytecodes."

# Start ethrex node with demo genesis
start-node:
	@echo "Starting ethrex node with EIP-8141 demo genesis..."
	cargo run --manifest-path ../../Cargo.toml --bin ethrex -- \
		--network genesis.json \
		--http.port 8545 \
		--dev

# Start backend (Bun + Hono)
start-backend:
	@echo "Starting backend on port 3000..."
	cd backend && npx tsx src/index.ts

# Start frontend (Vite)
start-frontend:
	@echo "Starting frontend on port 5173..."
	cd frontend && npx vite

# Install all dependencies
install:
	cd backend && npm install
	cd frontend && npm install

# Start everything (run in separate terminals)
start-all:
	@echo "Run these in separate terminals:"
	@echo "  make start-node"
	@echo "  make start-backend"
	@echo "  make start-frontend"

clean:
	rm -rf contracts/out
	rm -rf backend/node_modules
	rm -rf frontend/node_modules
