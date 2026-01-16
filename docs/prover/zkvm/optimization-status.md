# ethrex zkVM Optimization Status

This document provides a detailed analysis of ethrex's current zkVM optimization state, including patch utilization, known gaps, and performance implications.

> **Last Updated:** January 2026
> **Analyzed Backends:** ZisK v0.15.0, SP1 v5.0.8, RISC0 v3.0.3

## Executive Summary

| Backend | Patch Utilization | Critical Gaps |
|---------|-------------------|---------------|
| **ZisK** | 100% (7/7) | P256 unpatched (no patch exists) |
| **SP1** | ~80% (7-8/9) | ECADD bug, no c-kzg |
| **RISC0** | ~90% active | Keccak/BLS12-381 **disabled** |

**Recommendation:** ZisK provides the best optimization coverage. For production, SP1 offers the best balance of stability and performance.

## Feature Flag Architecture

ethrex uses Cargo feature flags to select code paths for each zkVM backend:

```
Guest Cargo.toml
    └── ethrex-vm [features: zisk|sp1|risc0|openvm]
        └── ethrex-levm [enables: substrate-bn, ziskos, etc.]
        └── ethrex-common [enables: kzg backend selection]
```

Key feature flags:
- `zisk` — Enables ZisK-specific optimizations
- `sp1` — Enables SP1-specific optimizations
- `risc0` — Enables RISC0-specific optimizations
- `secp256k1` — Uses native secp256k1 crate (disabled in zkVMs)
- `c-kzg` — Uses c-kzg for KZG operations
- `kzg-rs` — Uses kzg-rs for KZG operations

## Crypto Operation Analysis

### Library Selection Matrix

This table shows which library handles each crypto operation for each backend:

| Operation | Native | SP1 | RISC0 | ZisK |
|-----------|--------|-----|-------|------|
| **ECRECOVER** | `secp256k1` | `k256` | `k256` | `k256` |
| **SHA2-256** | `sha2` | `sha2` | `sha2` | `sha2` |
| **Keccak-256** | ASM | `tiny-keccak` | `tiny-keccak` | `tiny-keccak` |
| **SHA3 (EIP-7702)** | `sha3` | `sha3` | `sha3` | `sha3` |
| **BN254 ECADD** | `ark_bn254` | `ark_bn254` | `ark_bn254` | `substrate_bn` |
| **BN254 ECMUL** | `ark_bn254` | `substrate_bn` | `ark_bn254` | `substrate_bn` |
| **BN254 PAIRING** | `ark_bn254` | `substrate_bn` | `substrate_bn` | `substrate_bn` |
| **BLS12-381** | `bls12_381` | `bls12_381` | `bls12_381` | `sp1_bls12_381` |
| **KZG** | `c-kzg`/`kzg-rs` | `kzg-rs` | `c-kzg` | `kzg-rs` |
| **P256VERIFY** | `p256` | `p256` | `p256` | `p256` |
| **MODEXP** | `malachite` | `malachite` | `malachite` | `ziskos::modexp` |

### Patch Status Legend

| Symbol | Meaning |
|--------|---------|
| Library name only | Uses native (unpatched) library |
| **Bold** | Critical performance impact |
| Italics in analysis | Partial optimization |

## Per-Backend Analysis

### ZisK — 100% Patch Utilization

ZisK achieves the highest optimization level in ethrex.

| Patch | Status | Code Location |
|-------|--------|---------------|
| `sha2` | Active | `precompiles.rs:sha2_256` |
| `sha3` | Active | `utils.rs`, `transaction.rs` (EIP-7702) |
| `k256` | Active | `precompiles.rs:ecrecover`, `transaction.rs:recover_address` |
| `substrate-bn` | Active | `precompiles.rs:bn254_g1_add`, `bn254_g1_mul`, `pairing_check` |
| `sp1_bls12_381` | Active | `precompiles.rs:bls12_*` functions |
| `tiny-keccak` | Active | `keccak/mod.rs` (RISC-V target) |
| `kzg-rs` | Active | `crypto/kzg.rs:verify_kzg_proof` |

**Known Gap:** P256VERIFY runs unpatched because ZisK/Polygon doesn't provide a P256 patch.

**Unique Optimization:** ZisK's `ziskos::modexp` provides a native MODEXP precompile, which other zkVMs lack.

---

### SP1 — ~80% Patch Utilization

SP1 has good coverage but with notable gaps.

| Patch | Status | Notes |
|-------|--------|-------|
| `sha2` | Active | SHA2-256 precompile |
| `sha3` | Active | Keccak/SHA3 operations |
| `crypto-bigint` | Active | Transitive via k256/p256 |
| `tiny-keccak` | Active | Keccak on RISC-V |
| `p256` | Active | P256VERIFY precompile |
| `secp256k1` | **Unused** | ethrex uses `k256` instead |
| `ecdsa` | Active | Transitive via k256/p256 |
| `k256` | Active | ECRECOVER |
| `substrate-bn` | **Partial** | ECMUL/Pairing only |

#### SP1 ECADD Bug

> [!WARNING]
> SP1's `substrate-bn` patch is **intentionally not used for ECADD** due to a bug.

From `precompiles.rs:814`:
```rust
// SP1 patches the substrate-bn crate too, but some Ethereum Mainnet
// blocks fail to execute with it with a GasMismatch error
// so for now we will only use it for ZisK.
```

**Impact:** ECADD falls back to native `ark_bn254`, increasing cycle count significantly.

#### Other SP1 Gaps

- **KZG:** Uses unpatched `kzg-rs` instead of `c-kzg` (no SP1 c-kzg patch in ethrex)
- **BLS12-381:** Uses lambdaclass `bls12_381` fork, not SP1's patch

---

### RISC0 — ~90% Active, Critical Gaps

RISC0 has good active patch utilization, but **two critical patches are disabled**.

| Patch | Status | Notes |
|-------|--------|-------|
| `sha2` | Active | SHA2-256 precompile |
| `k256` | Active | ECRECOVER |
| `p256` | Active | P256VERIFY precompile |
| `crypto-bigint` | Active | Transitive via k256/p256 |
| `c-kzg` | Active | KZG point evaluation |
| `substrate-bn` | **Partial** | Pairing only, not ECADD/ECMUL |
| `tiny-keccak` | **Disabled** | Requires "unstable" feature |
| `bls12_381` | **Disabled** | Requires "unstable" feature |

#### Critical: Keccak Runs Unpatched

> [!WARNING]
> RISC0's `tiny-keccak` patch requires the "unstable" feature flag, which is not suitable for production. Keccak-256 operations run **completely unpatched**.

From `risc0/Cargo.toml`:
```toml
# These precompiles require the "unstable" risc0 feature which is not suited
# for production environments.
# tiny-keccak = { git = "https://github.com/risc0/tiny-keccak", tag = "..." }
```

**Impact:** Every `keccak256` call (used extensively in Ethereum) runs in software, dramatically increasing cycle counts.

#### Critical: BLS12-381 Runs Unpatched

The same "unstable" requirement affects BLS12-381, impacting:
- EIP-4844 blob operations
- Prague precompiles (BLS12_G1ADD, BLS12_G1MSM, etc.)

#### BN254 Inconsistency

RISC0's `substrate-bn` patch is only used for **pairing**, not ECADD or ECMUL:

| Operation | RISC0 Library | Patched? |
|-----------|---------------|----------|
| ECADD | `ark_bn254` | No |
| ECMUL | `ark_bn254` | No |
| PAIRING | `substrate_bn` | Yes |

---

## Native Library Fallbacks

When patches are missing or disabled, these native libraries are used:

| Library | Used For | Performance Impact |
|---------|----------|-------------------|
| `ark_bn254` | BN254 ECADD/ECMUL | High cycle count |
| `malachite` | MODEXP | High cycle count |
| `bls12_381` (lambdaclass) | BLS12-381 | High cycle count |
| `tiny-keccak` (unpatched) | Keccak-256 | **Very high** cycle count |
| `sha3` (unpatched) | SHA3 | **Very high** cycle count |
| `kzg-rs` (unpatched) | KZG | Moderate impact |
| `p256` (unpatched) | P256VERIFY | High cycle count |

## Recommendations

### Short-term

1. **RISC0 users:** Be aware of significant Keccak overhead. Avoid blocks with heavy hashing.
2. **SP1 users:** ECADD-heavy blocks will be slower than expected.
3. **Performance-critical:** Use ZisK backend.

### Long-term

1. **Enable RISC0 unstable patches** when they stabilize
2. **Investigate SP1 ECADD bug** — may be fixable in newer substrate-bn versions
3. **Add ZisK P256 support** when patch becomes available
4. **Unify BN254 handling** across backends

## Code References

Key files for understanding the conditional compilation:

| File | Purpose |
|------|---------|
| `crates/vm/levm/src/precompiles.rs` | Precompile implementations with `#[cfg]` |
| `crates/common/types/transaction.rs:1423-1480` | ECRECOVER dual implementations |
| `crates/common/crypto/keccak/mod.rs` | ASM vs tiny-keccak selection |
| `crates/common/crypto/kzg.rs` | KZG backend selection |
| `crates/l2/prover/src/guest_program/src/*/Cargo.toml` | Patch declarations |

## Further Reading

- [Backend Comparison](./backends.md) — Detailed backend comparison
- [Patches & Precompiles](./patches.md) — How patches work
- [Guest Program](../guest_program.md) — Guest program architecture
