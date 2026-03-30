# EIP-8025: Lighthouse + zkboost + ethrex Testnet

Run a local EIP-8025 testnet with Lighthouse as the consensus layer, zkboost as the proof sidecar, and ethrex as the execution layer.

## Architecture

```
Lighthouse BN ──Engine API──▶ ethrex (EL)
     │                           │
Lighthouse VC                    │
     │                           │
     └──REST API──▶ zkboost ─────┘
                    │   debug_executionWitnessByBlockHash
                    │   debug_chainConfig
                    │
                    └── mock zkVM (ethrex-zisk)
```

Lighthouse produces blocks via ethrex, then requests proofs from zkboost. zkboost fetches the execution witness from ethrex and generates a mock proof. The validator client signs the proof and gossips it.

## Prerequisites

- **Rust** (stable)
- **Docker** (for kurtosis genesis generation)
- **Kurtosis** (`curl -L https://apt.fury.io/kurtosis-tech/ | sudo apt-key add - && sudo apt install kurtosis-cli`)
- **clang** (Lighthouse needs it for leveldb-sys)

### Clone repositories

```bash
# ethrex (eip-8025-zkboost branch)
git clone https://github.com/lambdaclass/ethrex.git
cd ethrex && git checkout eip-8025-zkboost

# zkboost
git clone https://github.com/eth-act/zkboost.git

# Lighthouse (EIP-8025 fork)
git clone https://github.com/eth-act/lighthouse.git
cd lighthouse && git checkout feat/eip8025
```

### Build

```bash
# ethrex
cd ethrex
cargo build --release --features eip-8025 --bin ethrex

# zkboost
cd zkboost
cargo build --release -p zkboost-server

# Lighthouse (needs clang for leveldb-sys)
cd lighthouse
CC=clang CXX=clang++ cargo build --release --features portable --bin lighthouse
```

## Generate genesis

Lighthouse needs CL genesis files (genesis.ssz, config.yaml) that match the EL genesis. The easiest way is to use kurtosis to generate them, then extract the files.

### 1. Build docker images

Kurtosis needs docker images. Build lightweight ones from the native binaries:

```bash
# Lighthouse
mkdir -p /tmp/lh-docker
cp lighthouse/target/release/lighthouse /tmp/lh-docker/
cat > /tmp/lh-docker/Dockerfile << 'EOF'
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3t64 && rm -rf /var/lib/apt/lists/*
COPY lighthouse /usr/local/bin/lighthouse
ENTRYPOINT ["lighthouse"]
EOF
cd /tmp/lh-docker && docker build -t lighthouse:eip8025 .
```

### 2. Run kurtosis to generate genesis

```bash
mkdir -p ~/eip8025-testnet

cat > ~/eip8025-testnet/network_params.yaml << 'EOF'
participants:
  - cl_type: lighthouse
    cl_image: lighthouse:eip8025
    el_type: geth
    el_image: ethereum/client-go:latest
    supernode: true
    cl_extra_params:
      - --target-peers=3
      - --proof-engine-endpoint=http://mock/0/
    vc_extra_params:
      - --proof-engine-endpoint=http://mock/0/
    count: 1

network_params:
  fulu_fork_epoch: 0
  seconds_per_slot: 6

snooper_enabled: false
global_log_level: debug
additional_services: []
EOF

kurtosis engine start
kurtosis run --enclave eip8025 \
  "github.com/ethpandaops/ethereum-package@main" \
  --args-file ~/eip8025-testnet/network_params.yaml
```

The lighthouse container will fail (CLI flag mismatch with the ethereum-package), but the genesis files are generated.

### 3. Extract genesis files

```bash
kurtosis files download eip8025 el_cl_genesis_data ~/eip8025-testnet/genesis
kurtosis files download eip8025 jwt_file ~/eip8025-testnet/
kurtosis files download eip8025 1-lighthouse-geth-0-127 ~/eip8025-testnet/validator-keys
kurtosis enclave rm -f eip8025
```

You should have:
- `~/eip8025-testnet/genesis/genesis.json` — EL genesis
- `~/eip8025-testnet/genesis/genesis.ssz` — CL genesis state
- `~/eip8025-testnet/genesis/config.yaml` — CL config (Fulu at epoch 0)
- `~/eip8025-testnet/jwtsecret` — shared JWT secret
- `~/eip8025-testnet/validator-keys/` — validator keystores

## Configure zkboost

```bash
cat > ~/zkboost.toml << 'EOF'
el_endpoint = "http://localhost:8545"
proof_timeout_secs = 120
witness_timeout_secs = 120

[[zkvm]]
kind = "mock"
proof_type = "ethrex-zisk"
mock_proving_time_ms = 300
mock_proof_size = 1024
mock_failure = false
EOF
```

## Run

Four components, four terminals (or tmux windows).

### Terminal 1: ethrex

```bash
ethrex/target/release/ethrex \
  --network ~/eip8025-testnet/genesis/genesis.json \
  --http.port 8545 \
  --http.addr 0.0.0.0 \
  --authrpc.port 8551 \
  --authrpc.jwtsecret ~/eip8025-testnet/jwtsecret \
  --syncmode full \
  --p2p.disabled \
  --datadir /tmp/ethrex-eip8025-data
```

### Terminal 2: zkboost

```bash
RUST_LOG=info zkboost/target/release/zkboost-server --config ~/zkboost.toml
```

Wait for:
```
INFO zkboost_server::server: chain config loaded
INFO zkboost_server::server: http server listening port=3000
```

### Terminal 3: Lighthouse beacon node

```bash
lighthouse/target/release/lighthouse bn \
  --testnet-dir ~/eip8025-testnet/genesis \
  --datadir /tmp/lighthouse-eip8025 \
  --execution-endpoint http://localhost:8551 \
  --execution-jwt ~/eip8025-testnet/jwtsecret \
  --proof-engine-endpoint http://localhost:3000 \
  --supernode \
  --staking \
  --http \
  --target-peers 0 \
  --disable-discovery \
  --port 9001 \
  --enr-udp-port 9001
```

Wait for:
```
INFO Beacon chain initialized
```

### Terminal 4: Lighthouse validator client

```bash
lighthouse/target/release/lighthouse vc \
  --testnet-dir ~/eip8025-testnet/genesis \
  --datadir /tmp/lighthouse-vc \
  --beacon-nodes http://localhost:5052 \
  --init-slashing-protection \
  --proof-engine-endpoint http://localhost:3000 \
  --proof-types 2
```

`--proof-types 2` maps to `ethrex-zisk`. See the [proof type table](#proof-types) below.

## Expected output

Once all four are running:

1. **ethrex** logs `newPayload` and `forkchoiceUpdated` calls from Lighthouse
2. **zkboost** logs witness fetches and proof generation:
   ```
   INFO zkboost_server::witness: witness fetched and cached  block_hash=0x...
   INFO zkboost_server::proof: dispatching pending requests  count=1
   INFO zkboost_server::proof::worker: proving  proof_type=ethrex-zisk
   INFO zkboost_server::proof::worker: proof generated  proof_type=ethrex-zisk  proof_size=1072
   ```
3. **Lighthouse VC** logs block proposals and proof submissions:
   ```
   INFO Successfully published block  slot: 42
   INFO Completed proof signed and submitted  proof_type: 2
   ```

## Proof types

| ID | Name | EL guest | zkVM |
|----|------|----------|------|
| 0 | ethrex-risc0 | ethrex | RISC Zero |
| 1 | ethrex-sp1 | ethrex | SP1 |
| 2 | ethrex-zisk | ethrex | ZisK |
| 3 | reth-openvm | reth | OpenVM |
| 4 | reth-risc0 | reth | RISC Zero |
| 5 | reth-sp1 | reth | SP1 |
| 6 | reth-zisk | reth | ZisK |

## Clean restart

If you need a fresh start (e.g., slashing protection errors):

```bash
rm -rf /tmp/ethrex-eip8025-data /tmp/lighthouse-eip8025 /tmp/lighthouse-vc
```

Then restart all four components.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Lighthouse build fails with `-Wthread-safety` | leveldb-sys needs clang | Build with `CC=clang CXX=clang++` |
| Docker image fails with `GLIBC_2.38 not found` | Base image glibc too old | Use `debian:trixie-slim` instead of `bookworm-slim` |
| zkboost returns 400 on proof request | Proof type mismatch | Check `--proof-types` matches zkboost config (`2` for ethrex-zisk, not `6`) |
| VC logs `SlashableAttestation` errors | Stale slashing protection DB | Delete `/tmp/lighthouse-vc` and restart VC |
| BN logs `Not ready Bellatrix` | ethrex not synced yet | Wait for the first `forkchoiceUpdated` to arrive |
| zkboost logs `witness not found` | Block not yet in ethrex's DB | Increase `witness_timeout_secs` in zkboost.toml |
