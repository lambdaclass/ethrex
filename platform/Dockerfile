# Platform Docker images for Tokamak L2 deployments.
#
# Extends pre-built ethrex images with the files needed for
# pull-mode deployments (genesis, keys, contracts, verification keys).
#
# Usage:
#   # L1 image (extends ethrex:main)
#   docker build -f platform/Dockerfile --target l1 -t tokamak-app-l1:latest .
#
#   # L2 image (extends ethrex:main-l2)
#   docker build -f platform/Dockerfile --target l2 -t tokamak-app-l2:latest .

# ---- L1 image ----
FROM ghcr.io/tokamak-network/ethrex:main AS l1

# Genesis files
COPY fixtures/genesis/l1.json /genesis/l1.json

# ---- L2 image ----
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

# Verification keys (SP1 / RISC0)
COPY crates/guest-program/bin/sp1/out/riscv32im-succinct-zkvm-vk-u32 ./riscv32im-succinct-zkvm-vk-u32
COPY crates/guest-program/bin/sp1/out/riscv32im-succinct-zkvm-vk-bn254 ./riscv32im-succinct-zkvm-vk-bn254
COPY crates/guest-program/bin/risc0/out/riscv32im-risc0-vk ./riscv32im-risc0-vk
COPY crates/guest-program/bin/sp1/out/ ./sp1_out

ENTRYPOINT [ "./ethrex" ]
