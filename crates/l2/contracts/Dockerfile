FROM rust:1.81 AS chef

RUN apt-get update && apt-get install -y \
    build-essential \
    libclang-dev \
    libc6 \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef

WORKDIR /ethrex

FROM chef AS planner

COPY . .
# Determine the crates that need to be built from dependencies
RUN cargo chef prepare --bin ethrex_l2_l1_deployer --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /ethrex/recipe.json recipe.json
# Build dependencies only, these remained cached
RUN cargo chef cook --release --recipe-path recipe.json --manifest-path crates/l2/contracts/Cargo.toml --bin ethrex_l2_l1_deployer

COPY . .
RUN cargo build --release --manifest-path crates/l2/contracts/Cargo.toml

FROM --platform=${BUILDPLATFORM} ubuntu:24.04
WORKDIR /usr/local/bin

RUN apt-get update && apt-get -y install git gnupg software-properties-common
RUN add-apt-repository ppa:ethereum/ethereum
RUN apt-get update && apt-get -y install solc

COPY --from=builder ethrex/target/release/ethrex_l2_l1_deployer .

EXPOSE 1729
ENTRYPOINT [ "./ethrex_l2_l1_deployer" ]
