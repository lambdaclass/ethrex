build:
    cargo build --workspace

lint:
    cargo clippy --all-targets --all-features --workspace -- -D warnings

test-all:
    cargo test --workspace

test crate:
    cargo test -p {{crate}}

clean:
    cargo clean

run_image: build_image
    docker run --rm -p 127.0.0.1:8545:8545 ethrex --http.addr 0.0.0.0

build_image:
    docker build -t ethrex .
