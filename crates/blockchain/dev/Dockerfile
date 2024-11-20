FROM rust:1.80 AS chef

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
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /ethrex/recipe.json recipe.json
# Build dependencies only, these remained cached
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --features dev

FROM ubuntu:24.04
WORKDIR /usr/local/bin

COPY --from=builder ethrex/target/release/ethrex .
EXPOSE 8545
ENTRYPOINT [ "./ethrex" ]
