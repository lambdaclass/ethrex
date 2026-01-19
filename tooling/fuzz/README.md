# Ethrex Fuzzing

This directory contains fuzz targets for ethrex using [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) and libFuzzer.

## Prerequisites

1. Install cargo-fuzz:
   ```bash
   cargo install cargo-fuzz
   ```

2. You need a nightly Rust toolchain:
   ```bash
   rustup install nightly
   ```

## Available Fuzz Targets

| Target | Description |
|--------|-------------|
| `rlp_decode` | Tests RLP decoding never panics on arbitrary input |
| `rlp_roundtrip` | Tests RLP encode/decode roundtrip consistency |
| `trie_operations` | Tests trie insert/get/remove consistency |
| `transaction_decode` | Tests transaction parsing never panics |
| `block_decode` | Tests block parsing never panics |
| `block_header_decode` | Tests block header parsing never panics |
| `precompile_all` | Tests all EVM precompiles with arbitrary calldata |
| `precompile_ecrecover` | Tests ECDSA recovery precompile |
| `precompile_modexp` | Tests modular exponentiation precompile |
| `precompile_bn254` | Tests BN254 curve operations (ecadd, ecmul, ecpairing) |
| `precompile_bls12_381` | Tests BLS12-381 curve operations (Prague fork) |

## Running Fuzz Targets

List available targets:
```bash
cargo +nightly fuzz list
```

Run a specific target:
```bash
cargo +nightly fuzz run rlp_decode
```

Run with a time limit (5 minutes):
```bash
cargo +nightly fuzz run rlp_decode -- -max_total_time=300
```

Run with multiple jobs:
```bash
cargo +nightly fuzz run rlp_decode -- -jobs=4 -workers=4
```

## Coverage

Generate coverage report:
```bash
cargo +nightly fuzz coverage rlp_decode
```

## Corpus

Fuzzing corpus is stored in `fuzz/corpus/<target_name>/`. The fuzzer will automatically save interesting inputs.

## Crash Artifacts

When the fuzzer finds a crash, it saves the input to `fuzz/artifacts/<target_name>/`. To reproduce:
```bash
cargo +nightly fuzz run rlp_decode fuzz/artifacts/rlp_decode/crash-<hash>
```

## CI Integration

Fuzzing runs weekly in CI. For local extended fuzzing:
```bash
# Run for 24 hours
cargo +nightly fuzz run precompile_all -- -max_total_time=86400
```

## Adding New Fuzz Targets

1. Create a new file in `fuzz/fuzzers/`
2. Add the binary target to `fuzz/Cargo.toml`
3. Use the `fuzz_target!` macro from `libfuzzer-sys`

Example:
```rust
#![no_main]

use libfuzzer_sys::fuzz_target;
use ethrex_rlp::decode::RLPDecode;

fuzz_target!(|data: &[u8]| {
    // Your fuzzing logic here
    let _ = SomeType::decode(data);
});
```
