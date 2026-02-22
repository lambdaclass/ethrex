# Phase 1.1: Fork & Environment Setup

*Status: IN PROGRESS | Target: Week 1-2*

## Objective

Establish a verified build environment, CI pipeline, and Tokamak infrastructure foundations on the ethrex fork.

---

## 1. Build Verification

### 1-1. Workspace Build

```bash
cargo build --workspace                          # Full workspace compilation
cargo test --workspace                           # Test suite + baseline pass rate
cargo clippy --workspace -- -D warnings          # Lint compliance
cargo build --features perf_opcode_timings       # PoC feature (already verified)
```

**Expected results:**
- Build: PASS (verified in PoC phase, 3m 44s release build)
- Tests: Record baseline pass rate (some L2/prover tests may fail without backends)
- Clippy: PASS with existing codebase

### 1-2. Feature-Specific Builds

```bash
cargo build -p ethrex-levm                       # LEVM alone
cargo build -p ethrex-levm --features tokamak    # Tokamak feature (after 4-1)
cargo build -p ethrex --features tokamak         # Binary with Tokamak (after 4-1)
```

---

## 2. CI Pipeline Setup

### 2-1. Existing Workflows to Maintain

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| `pr-main_l1.yaml` | PR to main | L1 lint + test + Hive |
| `pr-main_levm.yaml` | PR to main | LEVM + EF tests |
| `pr_perf_levm.yaml` | PR to main | LEVM performance benchmarks |
| `pr-main_l1_ef_tests.yaml` | PR to main | Ethereum Foundation test suite |
| `pr_lint_gha.yaml` | PR (workflow changes) | GHA linting |
| `pr_lint_license.yaml` | PR (Cargo.toml changes) | License check |

### 2-2. New Workflow: `pr-tokamak.yaml`

```yaml
name: Tokamak
on:
  pull_request:
    branches: ["**"]
    paths:
      - "crates/vm/tokamak-jit/**"
      - "crates/tokamak-bench/**"
      - "crates/tokamak-debugger/**"
      - "crates/vm/levm/src/**"
      - "docs/tokamak/**"

jobs:
  quality-gate:
    # cargo build --workspace
    # cargo test -p tokamak-jit -p tokamak-bench -p tokamak-debugger
    # cargo clippy --workspace -- -D warnings

  safety-review:
    # cargo audit
    # Check no new unsafe code introduced

  diff-test:
    # (Phase 1.3+) Run differential tests against EF test vectors
```

### 2-3. Workflows to Skip

| Workflow | Reason | Enable At |
|----------|--------|-----------|
| `pr-main_l2.yaml` | L2 sequencer not relevant yet | Phase 4 |
| `pr-main_l2_prover.yaml` | Prover not relevant yet | Phase 4 |
| `pr_upgradeability.yaml` | L2 contracts not relevant yet | Phase 4 |
| `assertoor_*.yaml` (4) | Multi-client testing | Phase 5 |

---

## 3. Workspace Structure Initialization

### 3-1. Skeleton Crates

Three new crates, initially empty (build-passing skeletons):

**`crates/vm/tokamak-jit/`** (Phase 3 implementation)
```toml
[package]
name = "tokamak-jit"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
ethrex-levm.workspace = true

[lints]
workspace = true
```

**`crates/tokamak-bench/`** (Phase 1.3 implementation)
```toml
[package]
name = "tokamak-bench"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
ethrex-levm.workspace = true
ethrex-vm.workspace = true

[lints]
workspace = true
```

**`crates/tokamak-debugger/`** (Phase 2 implementation)
```toml
[package]
name = "tokamak-debugger"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
ethrex-levm.workspace = true

[lints]
workspace = true
```

### 3-2. Workspace Registration

Add to root `Cargo.toml` members:
```toml
members = [
    # ... existing members ...
    "crates/vm/tokamak-jit",
    "crates/tokamak-bench",
    "crates/tokamak-debugger",
]
```

---

## 4. Feature Flag Initialization

### 4-1. Declare `tokamak` Feature

**`crates/vm/levm/Cargo.toml`:**
```toml
[features]
tokamak = []  # Tokamak extensions (JIT, debugger, L2 hook)
```

**`crates/vm/Cargo.toml`:** (ethrex-vm)
```toml
[features]
tokamak = ["ethrex-levm/tokamak"]
```

**`cmd/ethrex/Cargo.toml`:**
```toml
[features]
tokamak = ["ethrex-vm/tokamak"]
```

### 4-2. No Code Changes Yet

The feature is declared but unused. No `#[cfg(feature = "tokamak")]` code is added in Phase 1.1. This establishes the propagation chain for later phases.

---

## 5. Documentation Updates

- Update `docs/tokamak/scaffold/HANDOFF.md` with Phase 1.1 status
- Record build results in this document (Section 7)

---

## 6. Success Criteria

| # | Criterion | Status |
|---|-----------|--------|
| 1 | `cargo build --workspace` PASS | |
| 2 | `cargo test --workspace` baseline recorded | |
| 3 | `cargo clippy --workspace -- -D warnings` PASS | |
| 4 | Skeleton crates (3) build successfully | |
| 5 | `tokamak` feature flag declared and propagating | |
| 6 | `cargo build --features tokamak` PASS | |
| 7 | CI workflow plan documented | |

---

## 7. Build Results

*To be filled after build verification*

| Command | Result | Duration | Notes |
|---------|--------|----------|-------|
| `cargo build --workspace` | | | |
| `cargo test --workspace` | | | |
| `cargo clippy --workspace -- -D warnings` | | | |
| `cargo build --features tokamak` | | | |
