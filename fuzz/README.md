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
| `fuzz_rlp_decode` | Tests RLP decoding never panics on arbitrary input |
| `fuzz_rlp_roundtrip` | Tests RLP encode/decode roundtrip consistency |
| `fuzz_trie_operations` | Tests trie insert/get/remove consistency |
| `fuzz_transaction_decode` | Tests transaction parsing never panics |
| `fuzz_block_decode` | Tests block parsing never panics |
| `fuzz_block_header_decode` | Tests block header parsing never panics |

## Running Fuzz Targets

List available targets:
```bash
cargo +nightly fuzz list
```

Run a specific target:
```bash
cargo +nightly fuzz run fuzz_rlp_decode
```

Run with a time limit (5 minutes):
```bash
cargo +nightly fuzz run fuzz_rlp_decode -- -max_total_time=300
```

Run with multiple jobs:
```bash
cargo +nightly fuzz run fuzz_rlp_decode -- -jobs=4 -workers=4
```

## Coverage

Generate coverage report:
```bash
cargo +nightly fuzz coverage fuzz_rlp_decode
```

## Corpus

Fuzzing corpus is stored in `fuzz/corpus/<target_name>/`. The fuzzer will automatically save interesting inputs.

## Crash Artifacts

When the fuzzer finds a crash, it saves the input to `fuzz/artifacts/<target_name>/`. To reproduce:
```bash
cargo +nightly fuzz run fuzz_rlp_decode fuzz/artifacts/fuzz_rlp_decode/crash-<hash>
```

## CI Integration

For continuous fuzzing, consider running for extended periods:
```bash
# Run for 24 hours
cargo +nightly fuzz run fuzz_rlp_decode -- -max_total_time=86400
```

## Adding New Fuzz Targets

1. Create a new file in `fuzz/fuzz_targets/`
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
