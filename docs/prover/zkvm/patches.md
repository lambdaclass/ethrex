# Patches & Precompiles

This document explains how crypto acceleration works in zkVMs through patches and precompiles.

## The Problem: Crypto in zkVMs is Expensive

When proving program execution in a zkVM, every operation costs **cycles**. Cryptographic operations like hashing and elliptic curve arithmetic are computationally intensive and dominate cycle counts.

Consider a single `keccak256` hash:
- **Native Rust**: ~1,000 cycles
- **Unpatched in zkVM**: ~100,000+ cycles (every step must be proven)
- **Patched with precompile**: ~1,000 cycles (precompile is a trusted primitive)

For Ethereum block execution, which involves thousands of hashes and signature verifications, the difference between patched and unpatched can be **100x or more** in proving time.

## What are Precompiles?

Precompiles are accelerated operations built into the zkVM itself. They're "trusted" in the sense that the zkVM's proof system directly proves their correctness without tracing every CPU instruction.

Common zkVM precompiles:
- **SHA-256** — Used in Merkle trees, KZG
- **Keccak-256** — Ethereum's primary hash function
- **secp256k1** — ECDSA signature verification (ECRECOVER)
- **BN254** — Pairing operations for certain precompiles
- **BLS12-381** — Used in EIP-4844 and Ethereum consensus

## What are Patches?

Patches are **modified versions of standard crates** that replace native crypto implementations with calls to zkVM precompiles.

### How Patches Work

```
[Standard Code Path]
sha2::Sha256::digest(data)
    └── Executes SHA-256 round function in software
    └── ~100,000 zkVM cycles

[Patched Code Path]
sha2::Sha256::digest(data)  [patched crate]
    └── Calls zkVM SHA-256 precompile
    └── ~1,000 zkVM cycles
```

The patch replaces the crate **at compile time** via Cargo's `[patch.crates-io]` mechanism. Your code doesn't change — the same `sha2::Sha256::digest()` call just uses a faster implementation.

### Patch Configuration

Patches are declared in the guest program's `Cargo.toml`:

```toml
[patch.crates-io]
sha2 = { git = "https://github.com/sp1-patches/RustCrypto-hashes", tag = "patch-sha2-0.10.9-sp1-4.0.0" }
k256 = { git = "https://github.com/sp1-patches/elliptic-curves", tag = "patch-k256-13.4-sp1-5.0.0" }
```

## Available Patches by Provider

### ZisK (Polygon)

Repository pattern: `github.com/0xPolygonHermez/zisk-patch-*`

| Crate | Latest Tag | Operation |
|-------|------------|-----------|
| sha2 | `patch-sha2-0.10.9-zisk-0.15.0` | SHA-256 |
| sha3 | `patch-sha3-0.10.8-zisk-0.15.0` | SHA-3/Keccak |
| k256 | `patch-k256-0.13.4-zisk-0.15.0` | secp256k1 |
| substrate-bn | `patch-0.6.0-zisk-0.15.0` | BN254 pairing |
| sp1_bls12_381 | `patch-0.8.0-zisk-0.15.0` | BLS12-381 |
| tiny-keccak | `patch-2.0.2-zisk-0.15.0` | Keccak |
| kzg-rs | `patch-0.2.7-zisk-0.15.0` | KZG commitments |
| ark-algebra | `patch-0.5.0-zisk-0.15.0` | Field arithmetic |

### SP1 (Succinct)

Repository pattern: `github.com/sp1-patches/*`

| Crate | Latest Tag | Operation |
|-------|------------|-----------|
| sha2 | `patch-sha2-0.10.9-sp1-4.0.0` | SHA-256 |
| sha3 | `patch-sha3-0.10.8-sp1-4.0.0` | SHA-3 |
| tiny-keccak | `patch-2.0.2-sp1-6.0.0` | Keccak |
| k256 | `patch-k256-13.4-sp1-5.0.0` | secp256k1 |
| p256 | `patch-p256-13.2-sp1-5.0.0` | P-256 |
| secp256k1 | `patch-0.30.0-sp1-6.0.0` | secp256k1 bindings |
| substrate-bn | `patch-0.6.0-sp1-6.0.0` | BN254 |
| crypto-bigint | `patch-0.5.5-sp1-6.0.0` | Big integers |

### RISC0

Repository pattern: `github.com/risc0/*`

| Crate | Latest Tag | Operation |
|-------|------------|-----------|
| sha2 | `sha2-v0.10.9-risczero.0` | SHA-256 |
| k256 | `k256/v0.13.4-risczero.1` | secp256k1 |
| p256 | `p256/v0.13.2-risczero.1` | P-256 |
| crypto-bigint | `v0.5.5-risczero.0` | Big integers |
| c-kzg | `c-kzg/v2.1.1-risczero.0` | KZG (EIP-4844) |
| substrate-bn | `v0.6.0-risczero.0` | BN254 |
| tiny-keccak | `tiny-keccak/v2.0.2-risczero.0` | Keccak (requires "unstable") |
| bls12_381 | `bls12_381/v0.8.0-risczero.1` | BLS12-381 (requires "unstable") |

## Patch Compatibility

### Version Matching

Patches are version-specific. The patch tag encodes compatibility:

```
patch-<crate>-<crate_version>-<zkvm>-<zkvm_sdk_version>
```

Example: `patch-sha2-0.10.9-sp1-4.0.0` means:
- Patches the `sha2` crate version `0.10.9`
- For SP1 SDK version `4.0.0`

> [!WARNING]
> Using a patch with the wrong crate version or SDK version may cause compilation errors or incorrect behavior.

### Transitive Dependencies

Some patches affect transitive dependencies. For example:
- `crypto-bigint` is used internally by `k256` and `p256`
- `ecdsa` is used internally by signature verification crates

Even if you don't directly use these crates, patching them accelerates the libraries that do.

## When Patches Can't Be Used

Sometimes patches aren't available or can't be used:

### Missing Patches

Not every crate has a patch for every zkVM:
- ZisK doesn't have a `p256` patch
- RISC0's `tiny-keccak` and `bls12_381` patches require the "unstable" feature

### Compatibility Issues

Patches may have bugs or incompatibilities:
- SP1's `substrate-bn` patch causes GasMismatch errors on certain mainnet blocks for ECADD
- ethrex uses native `ark_bn254` for SP1 ECADD as a workaround

### Feature Flags

In ethrex, we use Cargo feature flags to select between patched and native implementations:

```rust
#[cfg(feature = "zisk")]
pub fn bn254_g1_add(...) {
    // Uses patched substrate-bn
}

#[cfg(not(feature = "zisk"))]
pub fn bn254_g1_add(...) {
    // Uses native ark_bn254
}
```

## Debugging Patch Issues

### Identifying Unpatched Code

Use `ziskemu` (for ZisK) or similar profiling tools to identify expensive operations:

```bash
ziskemu -e <ELF> -i <INPUT> -D -X -S
```

Look for:
- High cycle counts in crypto functions
- Functions that should use precompiles but show software implementation patterns

### Common Issues

| Symptom | Likely Cause |
|---------|--------------|
| Compilation error mentioning patch | Version mismatch |
| Unexpectedly high cycle counts | Patch not applied or wrong code path |
| Runtime errors in crypto ops | Patch incompatibility |

## Performance Impact

Approximate cycle reduction with patches (varies by zkVM):

| Operation | Unpatched | Patched | Speedup |
|-----------|-----------|---------|---------|
| SHA-256 (32 bytes) | ~100,000 | ~1,000 | 100x |
| Keccak-256 (32 bytes) | ~150,000 | ~1,500 | 100x |
| secp256k1 verify | ~500,000 | ~10,000 | 50x |
| BN254 pairing | ~2,000,000 | ~50,000 | 40x |

> [!NOTE]
> These numbers are illustrative. Actual performance depends on the specific zkVM, input sizes, and hardware.

## Further Reading

- [Backend Comparison](./backends.md) — Which patches each backend supports
- [Optimization Status](./optimization-status.md) — Current ethrex patch utilization
- [zkVM Overview](./README.md) — Introduction to zkVMs
