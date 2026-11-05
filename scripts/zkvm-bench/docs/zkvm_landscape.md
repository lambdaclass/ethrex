# zkVM Landscape Context

Knowledge base for zero-knowledge virtual machines, proving workflows, and ethrex integration.

---

## Overview

### What is a zkVM?

A zero-knowledge virtual machine (zkVM) proves correct execution of programs without revealing inputs. Write code in Rust/C++, compile to RISC-V (or custom ISA), generate cryptographic proof of execution.

### Key Players

| zkVM | Organization | Architecture | Language |
|------|--------------|--------------|----------|
| ZisK | Polygon (0xPolygonHermez) | RISC-V | Rust |
| SP1 | Succinct (succinctlabs) | RISC-V | Rust, C++, C |
| RISC Zero | RISC Zero | RISC-V | Rust, C++ |
| OpenVM | openvm-org | Custom ISA (modular) | Rust |
| Pico | Brevis | Glue-and-coprocessor | Rust |

### Common Terminology

| Term | Meaning |
|------|---------|
| Guest program | Code that runs inside the zkVM and gets proven |
| Host | Code that runs outside zkVM, prepares inputs, verifies proofs |
| ELF | Compiled guest program (RISC-V executable) |
| Proof | Cryptographic evidence of correct execution |
| Receipt | Proof + public outputs (RISC Zero term) |
| Precompile | Accelerated operation built into the zkVM (crypto, hashing) |
| Patch | Modified crate that uses precompiles instead of native code |

---

## ZisK (Primary Focus)

### Installation

Assume `cargo-zisk` and `ziskemu` are installed. Installation process documented separately.

### ELF Naming Convention (EthProofs Spec)

```
<EL_NAME>-<EL_VERSION>-<ZKVM_NAME>-<ZKVM_SDK_VERSION>
```

Example: `ethrex-v0_1_0-zisk-v0_15_0`

Source: https://hackmd.io/@kevaundray/BJM9wZKbbl

### Commands

#### Build Guest Program
```bash
cd crates/l2/prover/src/guest_program/src/zisk
cargo-zisk build --release
# Output: target/riscv64ima-zisk-zkvm-elf/release/zkvm-zisk-program
```

#### Rom Setup (once per ELF)
```bash
cargo-zisk rom-setup -e <ELF_PATH> -k ~/.zisk/provingKey
```

#### Check Setup
```bash
cargo-zisk check-setup -k ~/.zisk/provingKey -a
```

#### Prove
```bash
cargo-zisk prove -e <ELF> -i <INPUT> -o /tmp/proof-output -a -u -y
```

Flags:
- `-e` ELF file (required)
- `-i` input file (required)
- `-o` output dir (use temp, process immediately)
- `-a` aggregation (recommended by ZisK team)
- `-u` unlock mapped memory (recommended by ZisK team)
- `-y` verify proof after generation
- `-k` proving key path (default: ~/.zisk/provingKey)

#### Execute (no proof, for testing)
```bash
cargo-zisk execute -e <ELF> -i <INPUT>
```

#### Debug with ziskemu
When prove/execute fails:
```bash
ziskemu -e <ELF_PATH> -i <INPUT_PATH> -D -X -S
```

Flags:
- `-D` detailed analysis for top callers of each ROI
- `-X` stats (opcodes, memory usage)
- `-S` load symbols from ELF (function names for readable output)

Use ziskemu to:
1. Debug failures
2. Find performance bottlenecks
3. Identify missing patches (high cycle counts in crypto functions)

---

## ethrex Integration

### Guest Program Location

```
ethrex/crates/l2/prover/src/guest_program/src/
├── zisk/      # ZisK guest
├── sp1/       # SP1 guest
├── risc0/     # RISC0 guest
└── openvm/    # OpenVM guest
```

### Input Generation (ethrex-replay)

**Repository:** https://github.com/lambdaclass/ethrex-replay

```bash
# Single block
ethrex-replay generate-input --block <BLOCK_NUMBER> --rpc-url <RPC_URL> --output-dir <DIR>

# Range of blocks
ethrex-replay generate-input --from <START> --to <END> --rpc-url <RPC_URL> --output-dir <DIR>

# Multiple specific blocks
ethrex-replay generate-input --blocks <B1>,<B2>,<B3> --rpc-url <RPC_URL>
```

Output format: `ethrex_<network>_<block>_input.bin` (rkyv serialized)

### Full Proving Workflow

```bash
# 1. Build guest program
cd crates/l2/prover/src/guest_program/src/zisk
cargo-zisk build --release
# Rename ELF per naming convention

# 2. Rom setup (once per ELF)
cargo-zisk rom-setup -e <ELF_PATH> -k ~/.zisk/provingKey

# 3. Check setup is complete
cargo-zisk check-setup -k ~/.zisk/provingKey -a

# 4. Generate input
ethrex-replay generate-input --block <N> --rpc-url <URL> --output-dir ./inputs

# 5. Prove
cargo-zisk prove -e <ELF> -i ./inputs/<block>.bin -o /tmp/proof-output -a -u -y

# 6. Process results from output dir
```

---

## Patches

### What are Patches?

Patches replace standard crypto crates with zkVM-optimized versions that call precompiles instead of executing native code. This dramatically reduces proving time for crypto operations.

Defined in `[patch.crates-io]` section of guest Cargo.toml.

### How Patches Work

```
Standard code: sha2::Sha256::digest(data)
        ↓ patch replaces crate at compile time
Patched code: sha2::Sha256::digest(data) → calls zkVM precompile
        ↓
Proving is 100-1000x faster for crypto ops
```

### Patch Naming Convention

```
patch-<crate>-<version>-<zkvm>-<zkvm_version>
```

Example: `patch-sha2-0.10.9-zisk-0.15.0`

---

## Available Patches by zkVM

### ZisK Patches (github.com/0xPolygonHermez/zisk-patch-*)

| Crate | Latest Tag | Operation |
|-------|------------|-----------|
| sha2 | patch-sha2-0.10.9-zisk-0.15.0 | SHA-256 |
| sha3 | patch-sha3-0.10.8-zisk-0.15.0 | SHA-3/Keccak |
| k256 | patch-k256-0.13.4-zisk-0.15.0 | secp256k1 |
| substrate-bn | patch-0.6.0-zisk-0.15.0 | BN254 pairing |
| bls12_381 | patch-0.8.0-zisk-0.15.0 | BLS12-381 |
| tiny-keccak | patch-2.0.2-zisk-0.15.0 | Keccak |
| kzg-rs | patch-0.2.7-zisk-0.15.0 | KZG commitments |
| ark-algebra | patch-0.5.0-zisk-0.15.0 | Field arithmetic |
| ruint | patch-1.17.0-zisk-0.15.0 | Big integers |
| blst | patch-0.3.15-zisk-0.15.0 | BLS signatures |
| modexp | patch-1.2.0-zisk-0.15.0 | Modular exponentiation |

### SP1 Patches (github.com/sp1-patches/*)

| Crate | Latest Tag | Operation |
|-------|------------|-----------|
| sha2 | patch-sha2-0.10.9-sp1-4.0.0 | SHA-256 |
| sha3 | patch-sha3-0.10.8-sp1-4.0.0 | SHA-3 |
| tiny-keccak | patch-2.0.2-sp1-6.0.0 | Keccak |
| k256 | patch-k256-13.4-sp1-5.0.0 | secp256k1 |
| p256 | patch-p256-13.2-sp1-5.0.0 | P-256 |
| secp256k1 | patch-0.30.0-sp1-6.0.0 | secp256k1 bindings |
| ecdsa | patch-16.9-sp1-4.1.0 | ECDSA signatures |
| substrate-bn | patch-0.6.0-sp1-6.0.0 | BN254 |
| bls12_381 | patch-0.8.0-sp1-6.0.0 | BLS12-381 |
| crypto-bigint | patch-0.5.5-sp1-6.0.0 | Big integers |
| curve25519-dalek | patch-4.1.3-sp1-6.0.0 | Curve25519 |
| rsa | patch-0.9.6-sp1-6.0.0 | RSA |
| c-kzg-4844 | (available) | KZG (EIP-4844) |

### RISC0 Patches (github.com/risc0/*)

| Crate | Latest Tag | Operation |
|-------|------------|-----------|
| sha2 | sha2-v0.10.9-risczero.0 | SHA-256 |
| k256 | k256/v0.13.4-risczero.1 | secp256k1 |
| p256 | p256/v0.13.2-risczero.1 | P-256 |
| crypto-bigint | v0.5.5-risczero.0 | Big integers |
| c-kzg | c-kzg/v2.1.1-risczero.0 | KZG (EIP-4844) |
| substrate-bn | v0.6.0-risczero.0 | BN254 |
| blst | v0.3.15-risczero.1 | BLS signatures |
| bls12_381 | bls12_381/v0.8.0-risczero.1 | BLS12-381 |
| curve25519-dalek | curve25519-4.1.3-risczero.0 | Curve25519 |
| tiny-keccak | tiny-keccak/v2.0.2-risczero.0 | Keccak |

### OpenVM (Extensions, not patches)

OpenVM uses built-in modular extensions instead of patches:
- **openvm-keccak256** — Keccak-256
- **openvm-sha256** — SHA-256
- **openvm-ecc** — Elliptic curves (secp256k1, P-256)
- **openvm-pairing** — BN254, BLS12-381 pairings
- **openvm-algebra** — Field arithmetic
- **openvm-bigint** — Big integers

Extensions are part of the main `openvm` repo (v1.4.3).

---

## ethrex Patch Usage Analysis (Deep Dive)

### Feature Flag Propagation

```
Guest Cargo.toml → ethrex-vm → ethrex-levm → #[cfg(feature = "...")]
                            → ethrex-common
```

Features: `zisk`, `sp1`, `risc0`, `openvm`, `secp256k1`, `c-kzg`, `kzg-rs`

### Crypto Operation → Library Matrix

| Operation | Native | SP1 | RISC0 | ZisK |
|-----------|--------|-----|-------|------|
| **ECRECOVER** | `secp256k1` | `k256` ✅ | `k256` ✅ | `k256` ✅ |
| **SHA2-256** | `sha2` | `sha2` ✅ | `sha2` ✅ | `sha2` ✅ |
| **Keccak-256** | ASM | `tiny-keccak` ✅ | `tiny-keccak` ❌ | `tiny-keccak` ✅ |
| **SHA3 (EIP-7702)** | `sha3` | `sha3` ✅ | `sha3` ❌ | `sha3` ✅ |
| **BN254 ECADD** | `ark_bn254` | `ark_bn254` ❌ | `ark_bn254` ❌ | `substrate_bn` ✅ |
| **BN254 ECMUL** | `ark_bn254` | `substrate_bn` ✅ | `ark_bn254` ❌ | `substrate_bn` ✅ |
| **BN254 PAIRING** | `ark_bn254` | `substrate_bn` ✅ | `substrate_bn` ✅ | `substrate_bn` ✅ |
| **BLS12-381** | `bls12_381` (λ) | `bls12_381` (λ) ❌ | `bls12_381` (λ) ❌ | `sp1_bls12_381` ✅ |
| **KZG** | `c-kzg`/`kzg-rs` | `kzg-rs` ❌ | `c-kzg` ✅ | `kzg-rs` ✅ |
| **P256VERIFY** | `p256` | `p256` ✅ | `p256` ✅ | `p256` ❌ |
| **MODEXP** | `malachite` | `malachite` ❌ | `malachite` ❌ | `ziskos::modexp` ✅ |

Legend: ✅ = patched/accelerated, ❌ = native/unpatched, (λ) = lambdaclass fork

### ZisK — 7/7 patches (100%)

| Patch | Used? | Code Location |
|-------|-------|---------------|
| sha2 | ✅ | precompiles.rs:sha2_256 |
| sha3 | ✅ | utils.rs, transaction.rs (EIP-7702) |
| k256 | ✅ | precompiles.rs:ecrecover, transaction.rs:recover_address |
| substrate-bn | ✅ | precompiles.rs:bn254_g1_add/mul, pairing_check |
| sp1_bls12_381 | ✅ | precompiles.rs:bls12_* functions |
| tiny-keccak | ✅ | keccak/mod.rs (RISC-V arch) |
| kzg-rs | ✅ | crypto/kzg.rs:verify_kzg_proof |

**ZisK is the most optimized backend.**

**Missing but available:** p256 patch (ZisK doesn't have one for P256VERIFY)

### SP1 — 7-8/9 patches (~80%)

| Patch | Used? | Notes |
|-------|-------|-------|
| sha2 | ✅ | SHA2-256 precompile |
| sha3 | ✅ | Keccak/SHA3 operations |
| crypto-bigint | ✅ | Transitive via k256/p256 |
| tiny-keccak | ✅ | Keccak on RISC-V |
| p256 | ✅ | P256VERIFY precompile |
| secp256k1 | ❌ | Not used; uses k256 instead |
| ecdsa | ✅ | Transitive via k256/p256 |
| k256 | ✅ | ECRECOVER |
| substrate-bn | ⚠️ | ECMUL/Pairing yes, **ECADD no** |

**Critical:** SP1 doesn't use substrate-bn for ECADD due to bug causing GasMismatch errors on mainnet blocks (see comment in precompiles.rs:814).

**Missing:**
- ECADD uses unpatched `ark_bn254` (high cost)
- No c-kzg patch, uses unpatched `kzg-rs`
- BLS12-381 uses lambdaclass fork (unpatched)

### RISC0 — 5-6/6 active patches (~90%)

| Patch | Used? | Notes |
|-------|-------|-------|
| sha2 | ✅ | SHA2-256 precompile |
| k256 | ✅ | ECRECOVER |
| p256 | ✅ | P256VERIFY precompile |
| crypto-bigint | ✅ | Transitive via k256/p256 |
| c-kzg | ✅ | KZG point evaluation |
| substrate-bn | ⚠️ | **Pairing only!** ECADD/ECMUL use ark_bn254 |
| tiny-keccak | ❌ | **Commented out** (requires "unstable" feature) |
| bls12_381 | ❌ | **Commented out** (requires "unstable" feature) |

**Critical Issues:**
1. **Keccak/SHA3 runs UNPATCHED** — very expensive
2. **BN254 ECADD/ECMUL run UNPATCHED** — use ark_bn254
3. **BLS12-381 runs UNPATCHED** — uses lambdaclass fork

### Native Libraries Used When Patches Missing

| zkVM | Operation | Native Library | Cycle Impact |
|------|-----------|----------------|--------------|
| SP1 | ECADD | `ark_bn254` | High |
| RISC0 | ECADD | `ark_bn254` | High |
| RISC0 | ECMUL | `ark_bn254` | High |
| RISC0 | Keccak | `tiny-keccak` (unpatched) | **Very High** |
| RISC0 | SHA3 | `sha3` (unpatched) | **Very High** |
| RISC0 | BLS12-381 | `bls12_381` (lambdaclass) | High |
| SP1 | KZG | `kzg-rs` (unpatched) | Moderate |
| ZisK | P256 | `p256` (unpatched) | High |

### Key Findings

1. **ZisK is most optimized** — uses all 7 declared patches, has MODEXP precompile
2. **RISC0 has biggest gaps** — commented patches for Keccak/BLS12-381 are high-impact
3. **SP1 ECADD bug** — explicitly skipped substrate-bn due to mainnet failures
4. **BN254 inconsistency** — ECADD, ECMUL, Pairing use different libs per zkVM
5. **P256 missing for ZisK** — ZisK Polygon doesn't provide p256 patch

---

## Updating Patch Registry

```bash
# ZisK patches
gh repo list 0xPolygonHermez --limit 100 | grep zisk-patch
# Then for each: gh api repos/0xPolygonHermez/<repo>/tags --jq '.[0].name'

# SP1 patches
gh repo list sp1-patches --limit 100
# Then for each: gh api repos/sp1-patches/<repo>/tags --jq '.[].name' | grep sp1 | head -1

# RISC0 patches
gh repo list risc0 --limit 100 | grep -E "RustCrypto|kzg|bn|bls|keccak|dalek"
# Then for each: gh api repos/risc0/<repo>/tags --jq '.[].name' | grep risczero | head -1

# OpenVM (check main repo version)
gh api repos/openvm-org/openvm/tags --jq '.[0].name'
```

---

## Documentation Links

| Resource | URL |
|----------|-----|
| ZisK Docs | https://0xpolygonhermez.github.io/zisk/ |
| ZisK Repo | https://github.com/0xPolygonHermez/zisk |
| SP1 Docs | https://docs.succinct.xyz/docs/sp1/introduction |
| RISC Zero Docs | https://dev.risczero.com/api |
| OpenVM Docs | https://docs.openvm.dev/book/getting-started/introduction/ |
| Pico Docs | https://pico-docs.brevis.network |
| ethrex | https://github.com/lambdaclass/ethrex |
| ethrex-replay | https://github.com/lambdaclass/ethrex-replay |
| EthProofs Spec | https://hackmd.io/@kevaundray/BJM9wZKbbl |

---

## Learnings & Tips

### Finding Missing Patches with ziskemu

Run `ziskemu -e <ELF> -i <INPUT> -D -X -S` and look for:
- High cycle counts in crypto functions (sha256, keccak, secp256k1)
- Functions that should use precompiles but are running native code
- Top ROI (Regions of Interest) showing crypto operations

### Patch Debugging

If a patch isn't working:
1. Check tag exists: `gh api repos/<org>/<repo>/tags`
2. Verify crate version matches patch version
3. Check zkVM SDK version compatibility
4. Look for `[patch."https://..."]` for non-crates.io sources

### Performance Hierarchy

Fastest to slowest for crypto operations:
1. Precompile (patched crate)
2. Native optimized (SIMD, asm)
3. Pure Rust implementation

---

## Session Log

### 2026-01-16 — Deep Patch Analysis

- **Deep dive into feature flag propagation** through ethrex crates
- **Corrected patch usage analysis** by tracing actual code paths:
  - ZisK: 100% (7/7) — most optimized
  - SP1: ~80% (7-8/9) — ECADD bug, no c-kzg
  - RISC0: ~90% active, but critical gaps (Keccak/BLS12-381 commented)
- **Discovered critical finding:** SP1 skips substrate-bn for ECADD due to GasMismatch bug on mainnet
- **Mapped crypto operation → library matrix** showing exact lib used per zkVM
- **Identified native fallback libs** when patches missing (ark_bn254, malachite, etc.)

### 2026-01-16 — Initial Creation

- Documented ZisK proving workflow for ethrex
- Catalogued all available patches across ZisK, SP1, RISC0, OpenVM
- Initial patch usage analysis
- Added ethrex-replay input generation workflow
- Documented ziskemu debugging approach
