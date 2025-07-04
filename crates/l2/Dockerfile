FROM rust:1.88 AS chef

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
RUN cargo build --release --features "rollup_storage_libmdbx"

FROM ubuntu:24.04
ENV CI_ETHREX_WORKDIR=$CI_ETHREX_WORKDIR
WORKDIR $CI_ETHREX_WORKDIR

COPY --from=builder ethrex/target/release/ethrex .
EXPOSE 1729
ENTRYPOINT [ "./ethrex" ]
