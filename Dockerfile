# syntax=docker/dockerfile:1.10

# --- Chef base ---
# Slim rust image + apt deps needed to compile native crates (rocksdb, openssl-sys, bindgen).
FROM rust:1.91-slim-bookworm AS chef

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        libclang-dev \
        libssl-dev \
        pkg-config \
        ca-certificates \
        curl \
        git

# Force cargo to fetch git deps via the git CLI instead of libgit2. The bundled
# libgit2 hangs on some hosts/networks inside containers; the CLI also supports
# single-commit fetches for rev-pinned deps.
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true

# Install cargo-chef via prebuilt binary (cargo-binstall) — avoids ~2 min source build.
# cargo-binstall pinned for reproducibility; bump deliberately.
ARG CARGO_BINSTALL_VERSION=v1.19.1
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    # uname -m (NOT $TARGETARCH): cargo-chef is a build-platform tool, must match
    # the stage's execution arch, not the target image arch.
    curl -fsSL https://github.com/cargo-bins/cargo-binstall/releases/download/${CARGO_BINSTALL_VERSION}/cargo-binstall-$(uname -m)-unknown-linux-musl.tgz \
      | tar -xz -C /usr/local/cargo/bin \
    && cargo binstall --no-confirm cargo-chef

WORKDIR /ethrex


# --- Planner ---
# Compute the dependency recipe. Fast, invalidated on any source change.
FROM chef AS planner

COPY --link benches ./benches
COPY --link crates ./crates
COPY --link metrics ./metrics
COPY --link cmd ./cmd
COPY --link test ./test
COPY --link tooling/repl ./tooling/repl
COPY --link tooling/monitor ./tooling/monitor
COPY --link Cargo.toml Cargo.lock ./
COPY --link .cargo ./.cargo

RUN cargo chef prepare --recipe-path recipe.json


# --- Builder ---
# Cook deps first (cached unless recipe.json changes), then build the app.
FROM chef AS builder

ARG PROFILE=release
ARG BUILD_FLAGS=""
ARG TARGETARCH

# vergen-git2 reads .git unless these env vars are set. Pass via build args
# so we don't ship the 1 GB .git directory into the build context.
ARG GIT_BRANCH=unknown
ARG GIT_SHA=unknown
ENV VERGEN_GIT_BRANCH=$GIT_BRANCH \
    VERGEN_GIT_SHA=$GIT_SHA \
    VERGEN_IDEMPOTENT=1

COPY --from=planner --link /ethrex/recipe.json recipe.json

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/ethrex/target,id=ethrex-target-${TARGETARCH} \
    cargo chef cook --profile $PROFILE --recipe-path recipe.json $BUILD_FLAGS

# Fetch solc using buildx's TARGETARCH (no shell uname).
RUN case "$TARGETARCH" in \
        arm64) SOLC_URL=https://github.com/ethereum/solidity/releases/download/v0.8.31/solc-static-linux-arm ;; \
        amd64) SOLC_URL=https://github.com/ethereum/solidity/releases/download/v0.8.31/solc-static-linux ;; \
        *) echo "unsupported TARGETARCH=$TARGETARCH" >&2; exit 1 ;; \
    esac \
    && curl -fsSL -o /usr/bin/solc "$SOLC_URL" \
    && chmod +x /usr/bin/solc

COPY --link benches ./benches
COPY --link crates ./crates
COPY --link cmd ./cmd
COPY --link metrics ./metrics
COPY --link test ./test
COPY --link tooling/repl ./tooling/repl
COPY --link tooling/monitor ./tooling/monitor
COPY --link Cargo.toml Cargo.lock ./
COPY --link .cargo ./.cargo
# Only these subdirs are referenced by include_str!/include_bytes! in workspace
# crates; the rest of fixtures/ is test data not needed at build time.
COPY --link fixtures/genesis ./fixtures/genesis
COPY --link fixtures/keys ./fixtures/keys

ENV COMPILE_CONTRACTS=true

# Combine build + extract in one RUN so the target cache mount is still mounted.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/ethrex/target,id=ethrex-target-${TARGETARCH} \
    cargo build --profile $PROFILE $BUILD_FLAGS \
    && mkdir -p /ethrex/bin \
    && cp /ethrex/target/${PROFILE}/ethrex /ethrex/bin/ethrex


# --- Runtime ---
# ubuntu:24.04 keeps glibc + libssl3 available. Network genesis/bootnodes are
# embedded into the binary via include_str!, so no extra files are needed.
FROM ubuntu:24.04

ARG GIT_SHA=unknown
ARG VERSION=dev

LABEL org.opencontainers.image.title="ethrex" \
      org.opencontainers.image.description="Rust Ethereum execution client" \
      org.opencontainers.image.source="https://github.com/lambdaclass/ethrex" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.revision="${GIT_SHA}" \
      org.opencontainers.image.version="${VERSION}"

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends \
        libssl3 \
        ca-certificates

WORKDIR /usr/local/bin

COPY --from=builder --link /ethrex/bin/ethrex /usr/local/bin/ethrex

# Common ports:
# -  8545: RPC
# -  8551: EngineAPI
# - 30303: Discovery (tcp+udp)
# -  9090: Metrics
# -  1729: L2 RPC
# -  3900: L2 Proof Coordinator
EXPOSE 8545 8551 9090 1729 3900 30303/tcp 30303/udp

ENTRYPOINT ["ethrex"]
