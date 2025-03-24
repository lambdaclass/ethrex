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

ENV PATH="/root/.sp1/bin:$PATH"
ENV PATH="/root/.cargo/bin:$PATH"

WORKDIR /ethrex

FROM chef AS planner

COPY . .
# Determine the crates that need to be built from dependencies
RUN cargo chef prepare --bin ethrex_prover --recipe-path recipe.json

FROM chef AS builder
ENV RUSTFLAGS=$RUSTFLAGS
COPY --from=planner /ethrex/recipe.json recipe.json
# Build dependencies only, these remained cached
RUN cargo chef cook --release --recipe-path recipe.json --manifest-path crates/l2/prover/Cargo.toml --bin ethrex_prover --features exec

COPY . .
RUN cargo build --release --manifest-path crates/l2/prover/Cargo.toml --features exec

FROM --platform=${BUILDPLATFORM} ubuntu:24.04
ENV RUSTFLAGS=$RUSTFLAGS
WORKDIR /usr/local/bin

COPY --from=builder ethrex/target/release/ethrex_prover .

ENTRYPOINT [ "./ethrex_prover" ]
