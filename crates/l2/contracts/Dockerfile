FROM rust:1.87 AS chef

RUN apt-get update && apt-get install -y \
    build-essential \
    libclang-dev \
    libc6 \
    libssl-dev \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef

WORKDIR /ethrex

FROM chef AS planner

COPY . .
# Determine the crates that need to be built from dependencies
RUN cargo chef prepare --bin ethrex_l2_l1_deployer --recipe-path recipe.json

FROM chef AS builder

COPY --from=planner /ethrex/recipe.json recipe.json
# Build dependencies only, these remain cached
RUN cargo chef cook --release --recipe-path recipe.json --manifest-path crates/l2/contracts/Cargo.toml --bin ethrex_l2_l1_deployer

# Install solc for build scripts
RUN curl -L -o /usr/bin/solc https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-static-linux \
    && chmod +x /usr/bin/solc

# Install git for build scripts
RUN apt-get update && apt-get install -y git && rm -rf /var/lib/apt/lists/*

COPY . .
RUN cargo build --release --manifest-path crates/l2/contracts/Cargo.toml

FROM --platform=${BUILDPLATFORM} ubuntu:24.04

WORKDIR /usr/local/bin

RUN apt-get update && apt-get -y install git gnupg software-properties-common curl
RUN add-apt-repository ppa:ethereum/ethereum
RUN curl -L -o /usr/bin/solc https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-static-linux
RUN chmod +x /usr/bin/solc
COPY --from=builder ethrex/target/release/ethrex_l2_l1_deployer .

EXPOSE 1729
ENTRYPOINT [ "./ethrex_l2_l1_deployer" ]
