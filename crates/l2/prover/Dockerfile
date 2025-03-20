ARG RUSTFLAGS
ARG PROVER

FROM rust:1.82 AS chef

RUN apt-get update && apt-get install -y \
  curl \
  git \
  pkg-config \
  clang \
  build-essential \
  libclang-dev \
  libc6 \
  libssl-dev \
  ca-certificates \
  && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef
RUN curl -L https://sp1.succinct.xyz | bash && ~/.sp1/bin/sp1up --version 4.1.0

ENV PATH="/root/.sp1/bin:$PATH"
ENV PATH="/root/.cargo/bin:$PATH"

RUN cargo prove install-toolchain

WORKDIR /ethrex

FROM chef AS planner
ENV RUSTFLAGS=$RUSTFLAGS

COPY . .
# Determine the crates that need to be built from dependencies
RUN cargo chef prepare --bin ethrex_prover --recipe-path recipe.json

FROM chef AS builder
ENV RUSTFLAGS=$RUSTFLAGS
COPY --from=planner /ethrex/recipe.json recipe.json
# Build dependencies only, these remained cached
RUN cargo chef cook --release --recipe-path recipe.json --manifest-path crates/l2/prover/Cargo.toml --bin ethrex_prover --features sp1

COPY . .
RUN cargo build --release --manifest-path crates/l2/prover/Cargo.toml --features sp1

FROM --platform=${BUILDPLATFORM} ubuntu:24.04
ENV RUSTFLAGS=$RUSTFLAGS
WORKDIR /usr/local/bin

# install basic packages
RUN apt-get update && apt-get -y install curl git

# install rust
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Check cargo is visible
RUN cargo --help

COPY --from=builder ethrex/target/release/ethrex_prover .

ENTRYPOINT [ "./ethrex_prover" ]
