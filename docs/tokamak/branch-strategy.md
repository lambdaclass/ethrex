# Tokamak ethrex Branch Strategy

## Overview

lambdaclass/ethrex fork (tokamak-network/ethrex) branch management strategy.
Maintain upstream sync while developing Tokamak-specific features across L1, L2, ZK, and new modules.

**Core principle: `main` stays as a clean upstream mirror. Tokamak code lives in the `tokamak` branch family.**

Benefits:
- Clear separation between upstream and Tokamak code
- Simple upstream sync (`main` fast-forward pull)
- Easy full independence later (`tokamak` -> new `main`)
- Clean upstream contributions (PR from `main` base)

## Branch Structure

```
upstream/main (lambdaclass)
    |
    v (periodic fast-forward)
main ----------------------------------------- upstream mirror (pure upstream code)
    |
    v (periodic merge)
tokamak -------------------------------------- Tokamak stable branch (deployable state)
    |
    +-- tokamak-dev -------------------------- Tokamak integration dev branch
    |       |
    |       +-- feat/l1/xxx ----------------- L1 feature development
    |       +-- feat/l2/xxx ----------------- L2 feature development
    |       +-- feat/zk/xxx ----------------- ZK related development
    |       +-- feat/mod/xxx ---------------- New module development
    |       |
    |       +-- fix/l1/xxx ------------------ L1 bug fixes
    |       +-- fix/l2/xxx ------------------ L2 bug fixes
    |       |
    |       +-- refactor/xxx ---------------- Refactoring
    |       +-- test/xxx -------------------- Test additions/changes
    |       +-- docs/xxx -------------------- Documentation
    |
    +-- release/vX.Y.Z ---------------------- Release preparation
    +-- hotfix/xxx --------------------------- Emergency fixes (branch from tokamak)
```

## Branch Details

### Permanent Branches

| Branch | Purpose | Protection Rules |
|--------|---------|------------------|
| `main` | upstream mirror. Pure lambdaclass code only | No direct push, upstream sync only |
| `tokamak` | Tokamak stable version, deployable state | PR required, 2+ reviewers, CI must pass |
| `tokamak-dev` | Integration dev branch, feature branches merge here | PR required, 1+ reviewer, CI must pass |

### main Branch Rules

`main` is **upstream-only**:
- No direct Tokamak code commits
- Only upstream sync changes
- `git diff main..tokamak` shows **all Tokamak changes** at a glance

### Work Branch Naming

```
<type>/<scope>/<short-description>
```

**type:**
- `feat` : New feature
- `fix` : Bug fix
- `refactor` : Refactoring
- `test` : Tests
- `docs` : Documentation
- `chore` : Build, CI, config, etc.

**scope:**
- `l1` : L1 (execution client) related
- `l2` : L2 (rollup, sequencer, proposer, etc.) related
- `zk` : ZK prover/verifier related
- `mod` : New module/crate additions
- `infra` : CI/CD, Docker, infrastructure
- `common` : Shared libraries, utilities
- Scope can be omitted if not clear

**Examples:**
```
feat/l2/custom-sequencer-logic
fix/zk/prover-memory-leak
feat/mod/tokamak-bridge
refactor/l1/storage-optimization
chore/infra/ci-docker-cache
```

### Special Branches

| Branch | Branch From | Merge To | Purpose |
|--------|-------------|----------|---------|
| `release/vX.Y.Z` | `tokamak-dev` | `tokamak` + `tokamak-dev` | Release prep, QA, version tagging |
| `hotfix/xxx` | `tokamak` | `tokamak` + `tokamak-dev` | Production emergency fixes |
| `upstream-contrib/xxx` | `main` | upstream PR only | Contributing back to upstream |

## Upstream Sync Strategy

### Sync Flow

```
upstream/main
    |
    v fast-forward
main (always identical to upstream)
    |
    v merge into tokamak-dev (resolve conflicts)
tokamak-dev
    |
    v after stability check
tokamak
```

### Sync Procedure

```bash
# 1. Update upstream -> reflect in main
git fetch upstream
git checkout main
git merge upstream/main        # fast-forward (no conflicts expected)
git push origin main

# 2. Merge main into tokamak-dev
git checkout tokamak-dev
git merge main                 # resolve conflicts here
# Commit after conflict resolution

# 3. Create PR: tokamak-dev -> tokamak (after stability check)
```

### Sync Frequency
- **Recommended**: Every 2 weeks (or when upstream has significant changes)
- **Owner**: Rotation or designated person
- **Note**: `main` always fast-forward only. Conflict resolution happens in `tokamak-dev`.

### Contributing to Upstream

```bash
# Create branch from main (= pure upstream)
git checkout main
git checkout -b upstream-contrib/fix-block-validation

# Work then create PR to upstream
# No Tokamak code mixed in, clean PR possible
```

## Full Independence Later

```bash
# tokamak branch becomes the new main
git branch -m main upstream-archive   # archive old main
git branch -m tokamak main            # promote tokamak -> main
git remote remove upstream            # disconnect upstream
```

`git diff upstream-archive..main` shows all Tokamak changes.

## Workflows

### Regular Feature Development

```
1. Create feature branch from tokamak-dev
   git checkout tokamak-dev
   git checkout -b feat/l2/custom-sequencer

2. Commit with Conventional Commits
   git commit -m "feat(l2): add custom sequencer logic"

3. Create PR -> tokamak-dev
   - Assign reviewers (area owners)
   - Verify CI passes

4. Squash Merge after approval
```

### Release

```
1. Create release branch from tokamak-dev
   git checkout tokamak-dev
   git checkout -b release/v0.1.0

2. Update version numbers, final QA

3. PR -> tokamak (2 reviewers)
4. Tag on tokamak: v0.1.0
5. Merge release branch into tokamak-dev (reflect version changes)
```

### Emergency Fix

```
1. Create hotfix branch from tokamak
   git checkout tokamak
   git checkout -b hotfix/critical-crash-fix

2. Fix then PR -> tokamak + tokamak-dev
```

## Commit Message Convention

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Examples:**
```
feat(l2): add Tokamak custom deposit handling
fix(zk): resolve prover OOM on large batches
refactor(l1): simplify block validation pipeline
docs(common): update API documentation for bridge module
chore(infra): add prover benchmark CI job
```

## PR Rules

- **PR to tokamak-dev**: Min 1 reviewer, CI must pass
- **PR to tokamak**: Min 2 reviewers, CI must pass, only from tokamak-dev
- **main**: No direct PRs. Upstream sync only
- **PR title**: Same format as commit convention
- **PR body**: Change summary, related issue links, test plan

## Code Ownership (Reference)

| Area | Directory (expected) | Owner |
|------|---------------------|-------|
| L1 Execution Client | `crates/blockchain/`, `crates/networking/` | TBD |
| L2 Rollup | `crates/l2/` | TBD |
| ZK Prover | `crates/l2/prover/` | TBD |
| New Modules | `crates/tokamak-*` (new) | TBD |
| Infra/CI | `.github/`, `docker/`, `scripts/` | TBD |

> Setting up CODEOWNERS auto-assigns reviewers on PRs.

## Branch Lifecycle

- **feature/fix branches**: Delete after merge
- **release branches**: Delete after release complete
- **hotfix branches**: Delete after merge
- **upstream-contrib branches**: Delete after upstream PR complete
- **main, tokamak, tokamak-dev**: Permanent
