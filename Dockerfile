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
COPY crates ./crates
COPY tooling ./tooling
COPY cmd ./cmd
COPY Cargo.* .
# Determine the crates that need to be built from dependencies
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /ethrex/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Install solc for build scripts
RUN curl -L -o /usr/bin/solc https://github.com/ethereum/solidity/releases/download/v0.8.29/solc-static-linux \
    && chmod +x /usr/bin/solc

# Optional build flags
ARG BUILD_FLAGS=""
COPY crates ./crates
COPY cmd ./cmd
COPY Cargo.* ./
RUN cargo build --release $BUILD_FLAGS

FROM ubuntu:24.04
WORKDIR /usr/local/bin

COPY cmd/ethrex/networks ./cmd/ethrex/networks
COPY --from=builder /ethrex/target/release/ethrex .
EXPOSE 8545
ENTRYPOINT [ "./ethrex" ]
