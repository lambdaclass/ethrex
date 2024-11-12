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
COPY . .
# Determine the crates that need to be built from dependencies
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /ethereum_rust/recipe.json recipe.json
# Build dependencies only, these remained cached
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --features l2

FROM ubuntu:24.04
WORKDIR /usr/local/bin

COPY --from=builder ethereum_rust/target/release/ethereum_rust .
EXPOSE 1729
ENTRYPOINT [ "./ethereum_rust" ]
