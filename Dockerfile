FROM rust:1.90 AS chef

RUN apt-get update && apt-get install -y \
    build-essential \
    libclang-dev \
    libc6 \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef

WORKDIR /ethrex


# --- Planner Stage ---
# Copy all source code to calculate the dependency recipe.
# This layer is fast and will be invalidated on any source change.
FROM chef AS planner

COPY benches ./benches
COPY crates ./crates
COPY metrics ./metrics
COPY cmd ./cmd
COPY Cargo.* .
COPY .cargo/ ./.cargo

RUN cargo chef prepare --recipe-path recipe.json


# --- Builder Stage ---
# Build the dependencies. This is the most time-consuming step.
# This layer will be cached and only re-run if the recipe.json from the
# previous stage has changed, which only happens when dependencies change.
FROM chef AS builder

COPY --from=planner /ethrex/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

RUN  if [ "$(uname -m)" = aarch64 ]; \
    then \
    SOLC_URL=https://github.com/ethereum/solidity/releases/download/v0.8.31/solc-static-linux-arm;\
    else \
    SOLC_URL=https://github.com/ethereum/solidity/releases/download/v0.8.31/solc-static-linux; \
    fi \
    && curl -L -o /usr/bin/solc $SOLC_URL \
    && chmod +x /usr/bin/solc

COPY benches ./benches
COPY crates ./crates
COPY cmd ./cmd
COPY metrics ./metrics
COPY tooling ./tooling
COPY fixtures/genesis ./fixtures/genesis
COPY .git ./.git
COPY Cargo.* ./
COPY fixtures ./fixtures
COPY .git ./.git
COPY .cargo/ ./.cargo

# Optional build flags
ARG BUILD_FLAGS=""
ENV COMPILE_CONTRACTS=true
RUN cargo build --release $BUILD_FLAGS

# --- Final Image ---
# Copy the ethrex binary into a minimalist image to reduce bloat size.
# This image must have glibc and libssl
FROM ubuntu:24.04
WORKDIR /usr/local/bin

RUN apt-get update && apt-get install -y --no-install-recommends libssl3

COPY cmd/ethrex/networks ./cmd/ethrex/networks
COPY --from=builder /ethrex/target/release/ethrex .

# Common ports:
# -  8545: RPC
# -  8551: EngineAPI
# - 30303: Discovery
# -  9090: Metrics
# -  1729: L2 RPC
# -  3900: L2 Proof Coordinator
EXPOSE 8545
EXPOSE 8551
EXPOSE 30303/tcp
EXPOSE 30303/udp
EXPOSE 9090
EXPOSE 1729
EXPOSE 3900

ENTRYPOINT [ "./ethrex" ]
