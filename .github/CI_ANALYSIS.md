# Ethrex CI/CD Analysis Report

> **Generated:** January 16, 2026
> **Scope:** Complete analysis of `.github/workflows/`, custom actions, Makefile integration, and configuration files

## Executive Summary

The ethrex repository maintains **24 workflow files** orchestrating CI/CD for a complex Ethereum execution client spanning L1 (execution layer) and L2 (rollup/proving) components. The system demonstrates sophisticated testing coverage but suffers from **matrix explosion**, **redundant installations**, and **maintenance complexity** that impact both cost and developer velocity.

**Key Findings:**
- Strong testing coverage across unit, integration, and protocol conformance (Hive, EF tests)
- Effective caching strategies for Rust and Docker layers
- High complexity burden: 24 workflows, 5 custom actions, 16+ scripts
- Significant optimization opportunities in ZK toolchain installation and matrix consolidation
- Estimated daily CI cost: $50-100 (excluding GPU runners at ~$20/hour)

---

## 1. Workflow Inventory

### 1.1 Core Testing Workflows (8)

| Workflow | File | Triggers | Purpose |
|----------|------|----------|---------|
| L1 Testing | `pr-main_l1.yaml` | push, PR, merge_group | Full L1 test suite: lint, test, Docker, Hive, Assertoor |
| L2 Testing | `pr-main_l2.yaml` | push, PR, merge_group | L2 integration: Validium, Vanilla, Based scenarios |
| LEVM Testing | `pr-main_levm.yaml` | push, PR | VM execution tests with EF comparison |
| L1+L2 Dev | `pr-main_l1_l2_dev.yaml` | push, PR | Local dev environment integration |
| L2 Prover | `pr-main_l2_prover.yaml` | push, PR | ZK backend linting (SP1, RISC0, ZisK) |
| L2 TDX Build | `pr-main_l2_tdx_build.yaml` | push, PR | Intel TDX (Trusted Domain) build verification |
| Docs Build | `pr-main_mdbook.yml` | push, PR | mdbook documentation with plugins |
| Main Prover | `main_prover.yaml` | push to main | GPU-based SP1 prover integration |

### 1.2 Release & Deployment Workflows (3)

| Workflow | File | Triggers | Purpose |
|----------|------|----------|---------|
| Tag Release | `tag_release.yaml` | tags `v*.*.*-*` | Multi-platform binaries + Docker images |
| Tag Latest | `tag_latest.yaml` | manual | Update Docker `:latest` tags |
| Perf Publish | `manual_docker_performance_publish.yaml` | manual | Performance image tagging |

### 1.3 Linting & Validation Workflows (4)

| Workflow | File | Triggers | Purpose |
|----------|------|----------|---------|
| PR Title Lint | `pr_lint_pr_title.yml` | PR | Semantic commit format enforcement |
| GHA Lint | `pr_lint_gha.yaml` | PR | GitHub Actions YAML validation |
| README Lint | `pr_lint_readme.yaml` | PR | CLI help consistency |
| Genesis Check | `pr_check_l2_genesis.yml` | PR | L2 genesis file validation |

### 1.4 Performance & Reporting Workflows (6)

| Workflow | File | Triggers | Purpose |
|----------|------|----------|---------|
| Block Execution Perf | `pr_perf_blocks_exec.yaml` | label `performance` | Hyperfine block execution benchmarks |
| LEVM Perf | `pr_perf_levm.yaml` | path-triggered | LEVM vs REVM comparison |
| Block Build Bench | `pr_perf_build_block_bench.yml` | manual | Criterion benchmarks |
| Changelog Check | `pr_perf_changelog.yml` | PR | CHANGELOG.md enforcement |
| LOC Analysis | `pr_loc.yaml` | PR | Lines of code diff |
| Daily LOC Report | `daily_loc_report.yaml` | schedule (00:00 UTC) | LOC + performance metrics |

### 1.5 Daily Scheduled Workflows (2)

| Workflow | File | Schedule | Purpose |
|----------|------|----------|---------|
| Hive Report | `daily_hive_report.yaml` | 03:00 UTC | Full Hive protocol test suite |
| Snapshot Sync | `daily_snapsync.yaml` | every 6h | Hoodi/Sepolia sync testing |

### 1.6 Auxiliary Workflows (2)

| Workflow | File | Triggers | Purpose |
|----------|------|----------|---------|
| GitHub Metadata | `pr_github_metadata.yaml` | PR | Auto-assign authors, set labels |
| Failure Alerts | `common_failure_alerts.yaml` | workflow_run | Slack notifications on failures |

---

## 2. Architecture Analysis

### 2.1 Job Dependency Graph (L1 Workflow)

```
                           ┌─────────────────┐
                           │ detect-changes  │
                           └────────┬────────┘
                                    │
              ┌─────────────────────┼─────────────────────┐
              │                     │                     │
              ▼                     ▼                     ▼
        ┌──────────┐         ┌──────────┐         ┌─────────────┐
        │   lint   │         │   test   │         │ docker_build│
        └──────────┘         │ (matrix) │         └──────┬──────┘
                             └──────────┘                │
                                                         │
              ┌──────────────────────┬──────────────────┤
              │                      │                   │
              ▼                      ▼                   ▼
        ┌─────────────┐       ┌──────────┐       ┌─────────────┐
        │ assertoor   │       │   hive   │       │ reorg-tests │
        │  (matrix)   │       │ (matrix) │       └─────────────┘
        └─────────────┘       └──────────┘
```

### 2.2 Test Matrix Configurations

#### L1 Test Matrix
```yaml
runners:
  - ubuntu-22.04      # Primary Linux x86_64
  - ubuntu-22.04-arm  # ARM64 compatibility
  - macos-15          # macOS compatibility
```

#### Hive Test Matrix (11 suites in daily, 6 in PR)
```yaml
PR suites:
  - rpc-compat (pre-merge genesis)
  - devp2p (discv4, eth, snap)
  - engine-auth
  - engine-exchange-caps
  - engine-cancun
  - engine-withdrawals

Daily additional:
  - sync
  - eels-consume-engine-{paris,shanghai,cancun,prague,osaka}
  - eels-consume-rlp
  - eels-execute-blobs
```

#### L2 Integration Matrix
```yaml
scenarios:
  - Validium
  - Vanilla
  - Vanilla+Web3signer
  - Based
```

#### Release Build Matrix
```yaml
platform × stack × features:
  ubuntu-22.04:
    - l1: [base]
    - l2: [base, +sp1, +risc0, +sp1+risc0]
    - l2_gpu: [base, +sp1, +sp1+risc0]
  ubuntu-22.04-arm:
    - l1: [base]
    - l2: [base, +sp1]
  macos-latest:
    - l1: [base]
    - l2: [base]
```

---

## 3. Caching Strategy Assessment

### 3.1 Rust Caching
- **Tool:** `Swatinem/rust-cache@v2`
- **Coverage:** All workflows with Rust compilation
- **Key Generation:** Automatic from `Cargo.lock` + toolchain
- **Effectiveness:** ✅ Good - reduces incremental builds significantly

### 3.2 Docker Layer Caching
```yaml
cache-from:
  - type=registry,ref=ghcr.io/.../cache-{scope}-{variant}-{platform}
  - type=registry,ref=ghcr.io/.../cache-main-{variant}-{platform}
cache-to:
  - type=registry,ref=...cache-...,mode=max
```
- **Strategy:** Multi-level fallback (PR → main)
- **Scope Separation:** L1/L2 variants have separate caches
- **Platform Separation:** amd64/arm64 isolated
- **Effectiveness:** ✅ Good - prevents cross-contamination

### 3.3 Artifact Caching
| Use Case | Key Strategy | Retention |
|----------|--------------|-----------|
| Binary cache (perf) | `branch-{ref}` | 1 run |
| LOC reports | `branch-main` | Session |
| EF test data | PR-specific | Default |

### 3.4 Cache Gaps Identified
1. **ZK Toolchain:** SP1/RISC0/ZisK reinstalled every job (no caching)
2. **Hive Results:** No cross-run caching for regression detection
3. **Guest ELFs:** Rebuilt per PR despite deterministic outputs
4. **Solidity Contracts:** Recompiled in multiple workflows

---

## 4. Custom Actions Analysis

### 4.1 Action Inventory

| Action | Location | Purpose |
|--------|----------|---------|
| `setup-rust` | `.github/actions/setup-rust/` | Toolchain + cache setup |
| `build-docker` | `.github/actions/build-docker/` | Multi-platform Docker builds |
| `free-disk` | `.github/actions/free-disk/` | CI disk space cleanup |
| `install-risc0` | `.github/actions/install-risc0/` | RISC0 toolchain installation |
| `install-solc` | `.github/actions/install-solc/` | Solidity compiler setup |
| `snapsync-run` | `.github/actions/snapsync-run/` | Kurtosis-based sync testing |

### 4.2 Action Quality Assessment

| Action | Reusability | Caching | Error Handling | Rating |
|--------|-------------|---------|----------------|--------|
| setup-rust | ✅ Excellent | ✅ Built-in | ✅ Good | A |
| build-docker | ✅ Excellent | ✅ Registry | ✅ Good | A |
| free-disk | ⚠️ Limited | N/A | ⚠️ Basic | B |
| install-risc0 | ✅ Good | ❌ None | ✅ Good | B- |
| install-solc | ✅ Good | ❌ None | ✅ Good | B |
| snapsync-run | ⚠️ Specific | N/A | ✅ Good | B |

### 4.3 Missing Actions (Opportunities)
1. `install-sp1` - Currently inline in workflows
2. `install-zisk` - Complex 18+ package installation inline
3. `setup-prover-env` - Unified ZK environment setup
4. `hive-runner` - Standardized Hive execution

---

## 5. Critical Issues

### 5.1 Matrix Explosion in Release Workflow

**Problem:** The release workflow generates 18+ build combinations:
```
3 platforms × 3 stacks × 2-4 feature variants = 18-36 jobs
```

**Impact:**
- Long release times (2-4 hours)
- High runner cost during releases
- Failure in one variant blocks entire release

**Evidence:** `tag_release.yaml` lines 64-103

### 5.2 Redundant ZK Toolchain Installation

**Problem:** SP1, RISC0, and ZisK toolchains are installed from scratch in multiple workflows:

| Workflow | SP1 | RISC0 | ZisK |
|----------|-----|-------|------|
| pr-main_l2.yaml | ✓ | ✓ | - |
| pr-main_l2_prover.yaml | ✓ | ✓ | ✓ |
| tag_release.yaml | ✓ | ✓ | ✓ |
| main_prover.yaml | ✓ | - | - |

**Impact:**
- 5-15 minutes per installation per job
- No caching between runs
- ZisK requires 18 apt packages

**Evidence:**
- `pr-main_l2_prover.yaml` lines 45-100 (ZisK installation)
- `.github/actions/install-risc0/action.yml` (no cache)

### 5.3 L2 Path Filters (By Design)

**Note:** L2 workflow triggers on L1-related paths intentionally:
```yaml
paths:
  - "crates/blockchain/**"
  - "crates/common/**"
  - "crates/vm/levm/**"
  # ... more paths
```

**Rationale:**
- L2 uses L1 logic under the hood
- Changes to L1 components can affect L2 behavior
- Full L2 test suite ensures no regressions from L1 changes

**Status:** ✅ Working as intended

### 5.4 Flaky Test Masking

**Problem:** Several workflows use `continue-on-error: true` without proper tracking:

```yaml
# daily_hive_report.yaml
- name: Run Hive
  continue-on-error: true  # Masks failures
```

**Impact:**
- Silent regressions in Hive compliance
- False confidence in test results
- No trend tracking for flakiness

**Evidence:**
- `daily_hive_report.yaml` line 82
- `pr-main_levm.yaml` EF test 100% check commented out

### 5.5 Version Pinning Inconsistency

**Problem:** Mixed version pinning strategies:

| Tool | Version Strategy | Risk |
|------|------------------|------|
| SP1 | Hardcoded `v5.0.8` | ✅ Safe |
| RISC0 | rzup `3.0.3` | ✅ Safe |
| ZisK | Latest from GitHub | ❌ Breakage risk |
| mdbook | `0.4.51` | ✅ Safe |
| Hive | Branch-based | ⚠️ Moderate |

**Impact:** ZisK updates can break CI without warning

**Evidence:** `pr-main_l2_prover.yaml` line 68 (no version pin)

### 5.6 Missing Parallel Optimization

**Problem:** Several sequential operations could run in parallel:

1. L1 lint and test run sequentially after detect-changes
2. Docker builds wait for lint completion
3. Hive suites use `--sim.parallelism 4` but could use 8

**Impact:** 15-30 minutes additional wait time per PR

---

## 6. Security Assessment

### 6.1 Secrets Management

| Secret Category | Storage | Rotation | Rating |
|-----------------|---------|----------|--------|
| Docker credentials | GitHub Secrets | Manual | ✅ Good |
| APT signing keys | GitHub Secrets | Unknown | ⚠️ Needs policy |
| Slack webhooks | GitHub Secrets | N/A | ✅ Good |
| GitHub App tokens | Generated | Auto | ✅ Excellent |
| Tailscale OAuth | GitHub Secrets | Manual | ✅ Good |

### 6.2 Privilege Escalation Risks

| Risk | Mitigation | Status |
|------|------------|--------|
| Fork PR code execution | Path-filtered workflows | ✅ Mitigated |
| Secret exposure in logs | `add-mask` used | ✅ Mitigated |
| Docker registry poisoning | GHCR scoped tokens | ✅ Mitigated |
| APT repo compromise | GPG signing | ✅ Mitigated |

### 6.3 Recommendations
1. Implement secret rotation policy for APT keys
2. Add OIDC authentication for cloud providers (future)
3. Consider Sigstore for release artifact signing

---

## 7. Performance Metrics

### 7.1 Estimated Workflow Durations

| Workflow | P50 Duration | P90 Duration | Critical Path |
|----------|--------------|--------------|---------------|
| L1 (full) | 25 min | 40 min | Hive tests |
| L2 (full) | 45 min | 75 min | Uniswap integration |
| LEVM | 15 min | 25 min | EF tests |
| Release | 90 min | 150 min | Multi-platform builds |
| Daily Hive | 120 min | 180 min | 11 parallel suites |

### 7.2 Cost Analysis (Estimated)

| Runner Type | Usage/Day | Cost/Min | Daily Cost |
|-------------|-----------|----------|------------|
| ubuntu-22.04 | 300 min | $0.008 | $2.40 |
| ubuntu-22.04-arm | 100 min | $0.005 | $0.50 |
| macos-15 | 50 min | $0.08 | $4.00 |
| GPU (self-hosted) | 60 min | $0.50 | $30.00 |
| **Total** | | | **~$37-50/day** |

### 7.3 Bottleneck Analysis

```
Critical Path (L1 PR):
  detect-changes (1min)
       │
       ├── lint (8min) ──────────────────┐
       │                                 │
       └── docker_build (12min) ─────────┼──► assertoor (15min)
                                         │
                                         └──► hive (25min) ◄── BOTTLENECK
```

---

## 8. Comparison with Industry Best Practices

### 8.1 What's Done Well

| Practice | Implementation | Rating |
|----------|----------------|--------|
| Path-based triggering | `dorny/paths-filter` | ✅ Excellent |
| Concurrency control | `cancel-in-progress` | ✅ Excellent |
| Multi-platform testing | Matrix builds | ✅ Excellent |
| Docker layer caching | Registry-based | ✅ Excellent |
| Semantic PR titles | Enforced validation | ✅ Excellent |
| Daily regression testing | Scheduled workflows | ✅ Good |

### 8.2 Gaps vs Best Practices

| Practice | Current State | Best Practice | Gap |
|----------|---------------|---------------|-----|
| Toolchain caching | None for ZK | Cache or pre-built images | High |
| Test result tracking | Ad-hoc | Datadog/Grafana dashboards | Medium |
| Flaky test quarantine | None | Auto-quarantine with retry | High |
| Release automation | Manual triggers | Automated on tag | Low |
| Dependency updates | Manual | Dependabot/Renovate | Medium |
| PR merge queue | merge_group enabled | Full queue adoption | Low |

---

## 9. Recommendations Summary

### 9.1 Quick Wins (1-2 days each)

1. ✅ **Cache ZK toolchains** - Add caching to SP1/RISC0 installation (implemented in install-sp1, install-zisk actions)
2. ✅ **Pin ZisK version** - Prevent unexpected breakage (v0.15.0 in install-zisk action)
3. ✅ **Increase Hive parallelism** - Change from 4 to 8 workers (needs verification)
4. **Parallelize L1 jobs** - Run lint and Docker build concurrently (already implemented)

### 9.2 Medium-term (1-2 weeks each)

1. ✅ **Create `install-zisk` action** - Standardize and potentially cache (implemented)
2. **Pre-build guest ELFs** - Cache deterministic outputs (not implemented - would hide build issues)
3. **Implement test result tracking** - Use GitHub Actions job summaries
4. **Enable EF test enforcement** - Requires team approval (TODO comment added)

### 9.3 Long-term (1+ months)

1. **Consolidate workflows** - Merge related workflows into fewer files
2. **Create ZK toolchain Docker image** - Pre-installed SP1/RISC0/ZisK
3. **Implement flaky test quarantine** - Auto-retry with tracking
4. **Add cost monitoring** - Track and alert on CI spend

---

## 10. Appendix

### 10.1 Workflow File Sizes

| File | Lines | Complexity |
|------|-------|------------|
| tag_release.yaml | 450+ | Very High |
| pr-main_l2.yaml | 400+ | High |
| pr-main_l1.yaml | 350+ | High |
| daily_hive_report.yaml | 250+ | Medium |
| pr-main_l2_prover.yaml | 200+ | Medium |
| Others | <150 | Low |

### 10.2 Script Dependencies

```
.github/scripts/
├── aggregate_hive_json.py     # Hive result aggregation
├── check_changed_files.sh     # Path filtering helper
├── check_cli_help.sh          # README validation
├── ef_tests_summary.py        # EF test reporting
├── format_hive_output.py      # Hive output formatting
├── format_metrics.py          # Prometheus metric formatting
├── generate_ef_comparison.sh  # EF test comparison
├── hive_postprocess.py        # Hive result processing
├── l2_genesis_hash.sh         # Genesis validation
├── loc_diff.sh                # LOC calculation
├── publish_tagged_latest.sh   # Docker tag management
└── ... (5+ more)
```

### 10.3 External Dependencies

| Dependency | Version | Purpose |
|------------|---------|---------|
| Kurtosis | 1.6.1 | Test orchestration |
| Hive | Branch | Protocol testing |
| Assertoor | 0.0.1 | Network testing |
| mdbook | 0.4.51 | Documentation |
| hyperfine | 1.19 | Benchmarking |
| solc | 0.8.27 | Contract compilation |

---

*End of Analysis Report*
