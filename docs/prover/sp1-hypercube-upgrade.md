# SP1 Hypercube (v6.0.0) Upgrade

## Overview

SP1 Hypercube is Succinct's new proof system built on multilinear polynomials, replacing SP1 Turbo. It claims up to 5x improvement for compute-heavy workloads and ~2x for precompile-heavy workloads like Ethereum proving.

This document tracks the upgrade from SP1 v5.0.8 (Turbo) to SP1 v6.0.0 (Hypercube).

## What's Done

### Dependency Upgrades

All SP1 dependencies bumped from `=5.0.8` to `=6.0.0`:

| File | Dependencies |
|------|-------------|
| `crates/guest-program/Cargo.toml` | `sp1-build =6.0.0`, `sp1-sdk =6.0.0` |
| `crates/guest-program/bin/sp1/Cargo.toml` | `sp1-zkvm =6.0.0` |
| `crates/l2/Cargo.toml` | `sp1-sdk =6.0.0` |
| `crates/l2/prover/Cargo.toml` | `sp1-sdk =6.0.0`, `sp1-prover =6.0.0`, `sp1-recursion-gnark-ffi =6.0.0` |

### Patch Updates

All accelerated cryptography patches in `crates/guest-program/bin/sp1/Cargo.toml` updated to v6 stable tags (`-sp1-6.0.0`).

| Patch | Old Tag | New Tag |
|-------|---------|---------|
| sha2 | `patch-sha2-0.10.9-sp1-4.0.0` | `patch-sha2-0.10.9-sp1-6.0.0` |
| sha3 | `patch-sha3-0.10.8-sp1-4.0.0` | `patch-sha3-0.10.8-sp1-6.0.0` |
| crypto-bigint | `patch-0.5.5-sp1-4.0.0` | `patch-0.5.5-sp1-6.0.0` |
| tiny-keccak | `patch-2.0.2-sp1-4.0.0` | `patch-2.0.2-sp1-6.0.0` |
| p256 | `patch-p256-13.2-sp1-5.0.0` | `patch-p256-13.2-sp1-6.0.0` |
| secp256k1 | `patch-0.30.0-sp1-5.0.0` | `patch-0.30.0-sp1-6.0.0` |
| k256 | `patch-k256-13.4-sp1-5.0.0` | `patch-k256-13.4-sp1-6.0.0` |
| substrate-bn | `patch-0.6.0-sp1-5.0.0` | `patch-0.6.0-sp1-6.0.0` |
| ecdsa | `patch-16.9-sp1-4.1.0` | **Removed** (see below) |
| sp1-lib | (RC workaround) | **Removed** (no longer needed) |

**ecdsa patch removed:** The k256 v6 patch brings its own ecdsa via git (`sp1-skip-verify-on-recovery` tag). The old v4.1.0 ecdsa patch adds `HIGH_S_ALLOWED` to the `VerifyPrimitive` trait, which breaks the p256 v6 patch that has an empty impl. The vanilla crates.io ecdsa works fine for p256.

### API Migration (v5 → v6)

SP1 v6 made all prover methods async. We use the `blocking` module (`sp1_sdk::blocking`) for synchronous wrappers.

Key API changes:

| v5 | v6 |
|----|-----|
| `CpuProver::new()` | `blocking::ProverClient::from_env()` |
| `client.setup(&elf_bytes)` → `(pk, vk)` | `client.setup(Elf::from(bytes))` → `pk`, then `pk.verifying_key()` |
| `client.prove(&pk, stdin, mode)` | `client.prove(&pk, stdin).mode(mode).run()` |
| `client.verify(&proof, &vk)` | `client.verify(&proof, &vk, None)` |
| `client.execute(elf_bytes, stdin)` | `client.execute(Elf::from(bytes), stdin).run()` |
| `CpuProver` / `CudaProverBuilder` | `blocking::EnvProver` (unified, reads `SP1_PROVER` env var) |
| `SP1ProvingKey` (concrete) | `<EnvProver as Prover>::ProvingKey` (associated type) |

Files modified:
- `crates/l2/prover/src/backend/sp1.rs` — Full rewrite of prover backend
- `crates/l2/sequencer/l1_proof_sender.rs` — Updated `init_sp1_vk()` method
- `crates/guest-program/build.rs` — Updated VK generation to use blocking API

### Build Configuration

- Docker tag updated from `v5.0.8` to `v6.0.0` in `build.rs`
- `sp1-zkvm/embedded` feature removed (no longer exists in v6)
- Added early return in `build.rs` when `SP1_SKIP_PROGRAM_BUILD=true` is set (VK generation requires the ELF)

### On-Chain Verifier Contract

- `cmd/ethrex/build_l2.rs`: Clone branch changed to `main` (sp1-contracts merged to main)
- Contract path changed from `v5.0.0/SP1VerifierGroth16.sol` to `v6.0.0/SP1VerifierGroth16.sol`

### Aligned Integration

Temporarily disabled. The `aligned-sdk` depends on `sp1-sdk` v5, causing type mismatches between v5 and v6 `SP1ProofWithPublicValues` / `SP1VerifyingKey`. The `submit_sp1_proof_to_aligned` method returns an error until aligned updates their SDK.

### Compilation & Build Verification

- `cargo check -p ethrex-prover --features sp1` — passes
- `cargo check -p ethrex-l2 --features sp1` — passes (1 warning: unused `sp1_vk` field)
- `cargo check --release -p ethrex --features sp1` — passes
- Guest program (ELF) built successfully with v6 toolchain (4.1MB)

## Known Workarounds

All RC-era workarounds have been resolved with the SP1 v6.0.0 stable release:

- **substrate-bn manual cache fix** — Resolved. The stable `patch-0.6.0-sp1-6.0.0` tag now resolves correctly without any manual Cargo cache editing.
- **sp1-lib patch** — Removed. No longer needed since `sp1-lib = "6.0.0"` resolves correctly against the stable release.
- **Makefile workaround block** — Removed. The `check-cargo-lock` target no longer needs the `cargo fetch` + `perl` substitution steps.

## Next Steps

### Benchmarking

1. **Deploy on benchmark server** — Push this branch, deploy on the remote server with `--sp1 true`
2. **Run SP1 Hypercube benchmarks** — Same workload as the SP1 Turbo baseline (15 batches, same tx count/type)
3. **Compare results** — Compare against the baseline in `sp1_bench_results.md`
4. **Record** — Document benchmark results alongside the Turbo baseline

### Post-Benchmark

5. **Update aligned-sdk** — Once aligned updates to sp1-sdk v6, restore the `submit_sp1_proof_to_aligned` implementation
6. **Re-enable ecdsa patch** — When `sp1-patches/signatures` publishes a v6 tag, add it back for potential precompile acceleration
7. **Merge or rebase** — If benchmarks show improvement, rebase onto `main` and prepare for merge

### References

- [SP1 Hypercube announcement](https://docs.succinct.xyz/docs/sp1/introduction)
- [SP1 v6.0.0 release](https://github.com/succinctlabs/sp1/releases/tag/v6.0.0)
- [SP1 v6 contracts PR](https://github.com/succinctlabs/sp1-contracts/pull/65)
- SP1 Turbo baseline: `sp1_bench_results.md` (on this branch)
