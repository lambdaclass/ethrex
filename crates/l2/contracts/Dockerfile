FROM rust:1.80 AS chef

RUN apt-get update && apt-get install -y \
    build-essential \
    libclang-dev \
    libc6 \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef

WORKDIR /ethereum_rust

FROM chef AS planner

COPY Cargo.toml ./Cargo.toml
COPY crates/blockchain/Cargo.toml crates/blockchain/Cargo.toml
COPY crates/blockchain/dev/Cargo.toml crates/blockchain/dev/Cargo.toml
COPY crates/common/Cargo.toml crates/common/Cargo.toml
COPY crates/networking/p2p/Cargo.toml crates/networking/p2p/Cargo.toml
COPY crates/networking/rpc/Cargo.toml crates/networking/rpc/Cargo.toml
COPY crates/storage/store/Cargo.toml crates/storage/store/Cargo.toml
COPY crates/vm/Cargo.toml crates/vm/Cargo.toml
COPY crates/storage/trie/Cargo.toml crates/storage/trie/Cargo.toml
COPY crates/common/rlp/Cargo.toml crates/common/rlp/Cargo.toml
COPY cmd/ethereum_rust/Cargo.toml cmd/ethereum_rust/Cargo.toml
COPY cmd/ef_tests/Cargo.toml cmd/ef_tests/Cargo.toml
COPY cmd/ef_tests/tests cmd/ef_tests/tests
COPY cmd/ethereum_rust_l2/Cargo.toml cmd/ethereum_rust_l2/Cargo.toml
COPY crates/vm/levm/Cargo.toml crates/vm/levm/Cargo.toml
COPY crates/vm/levm/bench/revm_comparison/Cargo.toml crates/vm/levm/bench/revm_comparison/Cargo.toml
COPY crates/l2/Cargo.toml crates/l2/Cargo.toml
COPY crates/l2/contracts/Cargo.toml crates/l2/contracts/Cargo.toml
COPY crates/l2/sdk/Cargo.toml crates/l2/sdk/Cargo.toml
COPY crates/l2/prover/Cargo.toml crates/l2/prover/Cargo.toml
COPY crates/l2/prover/zkvm/Cargo.toml crates/l2/prover/zkvm/Cargo.toml
COPY crates/l2/prover/zkvm/interface/Cargo.toml crates/l2/prover/zkvm/interface/Cargo.toml

# Determine the crates that need to be built from dependencies
RUN cargo chef prepare --bin ethereum_rust_l2_l1_deployer --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /ethereum_rust/recipe.json recipe.json
# Build dependencies only, these remained cached
RUN cargo chef cook --release --recipe-path recipe.json --manifest-path crates/l2/contracts/Cargo.toml --bin ethereum_rust_l2_l1_deployer

COPY . .
RUN cargo build --release --manifest-path crates/l2/contracts/Cargo.toml

FROM ubuntu:24.04
WORKDIR /usr/local/bin

RUN apt-get update && apt-get -y install git gnupg
RUN echo 'deb https://ppa.launchpadcontent.net/ethereum/ethereum/ubuntu noble main' > /etc/apt/sources.list.d/ethereum-ethereum-noble.list && \
    echo 'deb-src https://ppa.launchpadcontent.net/ethereum/ethereum/ubuntu noble main' >> /etc/apt/sources.list.d/ethereum-ethereum-noble.list
RUN apt-key adv --keyserver keyserver.ubuntu.com --recv-keys E66CB8FE57F8218E96B13446B50ABD2EE384B7D4
RUN apt-get update && apt-get -y install solc

COPY --from=builder ethereum_rust/target/release/ethereum_rust_l2_l1_deployer .

EXPOSE 1729
ENTRYPOINT [ "./ethereum_rust_l2_l1_deployer" ]
