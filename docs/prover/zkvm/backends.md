# zkVM Backend Comparison

ethrex supports multiple zkVM backends. This document compares their characteristics, trade-offs, and production readiness.

## Overview

| Backend | Architecture | Proof System | GPU Support | Production Status |
|---------|--------------|--------------|-------------|-------------------|
| SP1 | RISC-V | STARK + Groth16 | Yes | Production |
| RISC0 | RISC-V | STARK + Groth16 | Yes | Production |
| ZisK | RISC-V | STARK | Yes | Active Development |
| OpenVM | Custom ISA | STARK | No | Experimental |

## SP1

**Organization:** [Succinct Labs](https://succinct.xyz)
**Documentation:** [docs.succinct.xyz](https://docs.succinct.xyz/docs/sp1/introduction)

### Characteristics

- Mature ecosystem with extensive documentation
- Strong patch coverage for crypto operations
- Supports both CPU and GPU proving
- Groth16 wrapper for on-chain verification

### ethrex Integration

```
crates/l2/prover/src/guest_program/src/sp1/
```

**Patches used:**
- `sha2`, `sha3`, `k256`, `p256`, `substrate-bn`, `tiny-keccak`, `crypto-bigint`, `ecdsa`

**Known limitations:**
- ECADD uses native `ark_bn254` due to a GasMismatch bug with the `substrate-bn` patch on certain mainnet blocks
- KZG uses `kzg-rs` (unpatched) instead of `c-kzg`
- BLS12-381 uses lambdaclass fork (not SP1's patch)

### When to Use

- General production deployments
- When you need stable, well-documented tooling
- When GPU acceleration is required

---

## RISC0

**Organization:** [RISC Zero](https://risczero.com)
**Documentation:** [dev.risczero.com](https://dev.risczero.com/api)

### Characteristics

- First major zkVM, pioneered the space
- Strong focus on security and formal verification
- Bonsai network for distributed proving
- Native Groth16 support for on-chain verification

### ethrex Integration

```
crates/l2/prover/src/guest_program/src/risc0/
```

**Patches used:**
- `sha2`, `k256`, `p256`, `crypto-bigint`, `c-kzg`, `substrate-bn`

**Known limitations:**
- `tiny-keccak` patch **commented out** (requires "unstable" feature)
- `bls12_381` patch **commented out** (requires "unstable" feature)
- ECADD and ECMUL use native `ark_bn254` (only Pairing uses `substrate-bn`)

> [!WARNING]
> RISC0's Keccak-256 and BLS12-381 operations run **unpatched** in ethrex. This significantly impacts performance for blocks heavy in these operations.

### When to Use

- When Groth16 on-chain verification is required
- When using Bonsai distributed proving network
- When formal verification guarantees are important

---

## ZisK

**Organization:** [Polygon (0xPolygonHermez)](https://github.com/0xPolygonHermez/zisk)
**Documentation:** [0xpolygonhermez.github.io/zisk](https://0xpolygonhermez.github.io/zisk/)

### Characteristics

- Newest zkVM, actively developed by Polygon
- Designed for Ethereum execution proving
- Best patch coverage in ethrex
- Native MODEXP precompile (not available in other zkVMs)

### ethrex Integration

```
crates/l2/prover/src/guest_program/src/zisk/
```

**Patches used:**
- `sha2`, `sha3`, `k256`, `substrate-bn`, `sp1_bls12_381`, `tiny-keccak`, `kzg-rs`

**All 7 declared patches are utilized**, making ZisK the most optimized backend in ethrex.

**Known limitations:**
- No P256 patch available (P256VERIFY runs unpatched)
- Still in active development, API may change

### When to Use

- Performance-critical deployments
- When proving Ethereum mainnet blocks
- When EIP-4844 (blobs) proving is needed

---

## OpenVM

**Organization:** [Axiom](https://github.com/openvm-org/openvm)
**Documentation:** [docs.openvm.dev](https://docs.openvm.dev/book/getting-started/introduction/)

### Characteristics

- Modular architecture with custom ISA
- Extensions instead of patches (built-in acceleration)
- Designed for composability
- Experimental status in ethrex

### ethrex Integration

```
crates/l2/prover/src/guest_program/src/openvm/
```

**Extensions used:**
- `openvm-kzg` for KZG operations
- Standard OpenVM crypto extensions

### When to Use

- Experimental deployments
- When modularity and composability are priorities
- Research and development

---

## Comparison Matrix

### Crypto Operation Support

| Operation | SP1 | RISC0 | ZisK | OpenVM |
|-----------|-----|-------|------|--------|
| SHA2-256 | Patched | Patched | Patched | Extension |
| Keccak-256 | Patched | **Unpatched** | Patched | Extension |
| secp256k1 (ECRECOVER) | Patched | Patched | Patched | Extension |
| BN254 ECADD | **Unpatched** | **Unpatched** | Patched | Extension |
| BN254 ECMUL | Patched | **Unpatched** | Patched | Extension |
| BN254 Pairing | Patched | Patched | Patched | Extension |
| BLS12-381 | Unpatched | **Unpatched** | Patched | Extension |
| KZG | Unpatched | Patched | Patched | Extension |
| P256 | Patched | Patched | **Unpatched** | Extension |
| MODEXP | Unpatched | Unpatched | **Precompile** | Unpatched |

### Trade-off Summary

| Criterion | Best Choice | Notes |
|-----------|-------------|-------|
| **Overall optimization** | ZisK | 100% patch utilization |
| **Production stability** | SP1 or RISC0 | Mature ecosystems |
| **On-chain verification** | RISC0 | Native Groth16 |
| **GPU proving** | SP1 | Best GPU support |
| **BLS12-381 heavy blocks** | ZisK | Only backend with BLS patch active |
| **Keccak heavy blocks** | ZisK or SP1 | RISC0 patch is disabled |

## Selecting a Backend

Choose based on your requirements:

1. **Maximizing performance?** → ZisK
2. **Need production stability?** → SP1
3. **Need on-chain verification?** → RISC0
4. **Research/experimental?** → OpenVM

For most deployments, we recommend starting with **SP1** for its balance of stability and performance, then evaluating **ZisK** as it matures.

## Further Reading

- [Patches & Precompiles](./patches.md) — How crypto acceleration works
- [Optimization Status](./optimization-status.md) — Detailed analysis of ethrex's current state
- [Guest Program](../guest_program.md) — ethrex guest program architecture
