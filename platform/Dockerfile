# Platform Docker images for Tokamak Appchain deployments.
#
# Extends pre-built ethrex images with the files needed for
# pull-mode deployments (genesis, keys, contracts, verification keys).
#
# Usage:
#   docker build -f platform/Dockerfile --target l1  -t tokamak-appchain:l1  .
#   docker build -f platform/Dockerfile --target l2  -t tokamak-appchain:l2  .
#   docker build -f platform/Dockerfile --target sp1 -t tokamak-appchain:sp1 .

# ---- L1 image ----
FROM ghcr.io/tokamak-network/ethrex:main AS l1

# Genesis files
COPY fixtures/genesis/l1.json /genesis/l1.json

# ---- L2 image (standard, no SP1) ----
FROM ghcr.io/tokamak-network/ethrex:main-l2 AS l2

WORKDIR /usr/local/bin

# Genesis files (multiple paths for deployer + L2 node)
COPY fixtures/genesis/l1.json /genesis/l1.json
COPY fixtures/genesis/l2.json /genesis/l2.json
COPY fixtures/genesis/l1.json ./fixtures/genesis/l1.json
COPY fixtures/genesis/l2.json ./fixtures/genesis/l2.json

# Keys
COPY fixtures/keys/private_keys_l1.txt ./fixtures/keys/private_keys_l1.txt

# Contracts source (for deployer COMPILE_CONTRACTS=true)
COPY crates/l2/contracts ./contracts

ENTRYPOINT [ "./ethrex" ]

# ---- SP1 image (L2 + SP1 prover + verification keys) ----
FROM ghcr.io/tokamak-network/ethrex:main-l2 AS sp1

WORKDIR /usr/local/bin

# Genesis files
COPY fixtures/genesis/l1.json /genesis/l1.json
COPY fixtures/genesis/l2.json /genesis/l2.json
COPY fixtures/genesis/l2-zk-dex.json /genesis/l2-zk-dex.json
COPY fixtures/genesis/l1.json ./fixtures/genesis/l1.json
COPY fixtures/genesis/l2.json ./fixtures/genesis/l2.json
COPY fixtures/genesis/l2-zk-dex.json ./fixtures/genesis/l2-zk-dex.json

# Keys
COPY fixtures/keys/private_keys_l1.txt ./fixtures/keys/private_keys_l1.txt

# Contracts source
COPY crates/l2/contracts ./contracts

# Verification keys (SP1 / RISC0)
COPY crates/guest-program/bin/sp1/out/riscv32im-succinct-zkvm-vk-u32 ./riscv32im-succinct-zkvm-vk-u32
COPY crates/guest-program/bin/sp1/out/riscv32im-succinct-zkvm-vk-bn254 ./riscv32im-succinct-zkvm-vk-bn254
COPY crates/guest-program/bin/risc0/out/riscv32im-risc0-vk ./riscv32im-risc0-vk
COPY crates/guest-program/bin/sp1/out/ ./sp1_out

# Guest program configs
COPY crates/l2/programs-zk-dex.toml /etc/ethrex/programs-zk-dex.toml

ENTRYPOINT [ "./ethrex" ]
