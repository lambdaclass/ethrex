## Context

The ethrex release process is documented at `docs/developers/release-process/release-process.md` with manual checklists in PR #5537. The current CI has two release-related workflows:

- `tag_release.yaml`: Triggered by tag pushes matching `v*.*.*-*`. Builds 8 binary variants, 3 guest programs, Docker images (L1+L2, amd64+arm64), multi-arch manifests, and creates a GitHub pre-release. Also triggers on pushes to `main` for `main`-tagged Docker images.
- `tag_latest.yaml`: Triggered by release edits. Retags Docker images to `:latest`/`:l2`, publishes apt packages via `lambdaclass/ethrex-apt`.

The gap is everything between build and publish: version bumping (7 files), artifact testing (snap sync + L2 integration for 14 artifacts), promotion from pre-release to final, and Homebrew updates.

Infrastructure available:
- Self-hosted runner `ethrex-sync` (x86_64 Linux) for snap sync testing
- Root `docker-compose.yaml` already runs Lighthouse + ethrex for any network
- L2 integration tests are Docker-based, run on GitHub-hosted `ubuntu-latest`
- Snap sync Hoodi takes ~1h, L2 integration ~30min per variant
- Slack notifications via `ETHREX_L1_SLACK_WEBHOOK` secret

## Goals / Non-Goals

**Goals:**
- One-button release initiation (version + commit SHA → everything cascades)
- Automated testing of every release artifact (all binaries and Docker images)
- Zero-touch promotion when all tests pass
- Comprehensive failure visibility (all tests run even if some fail)
- Automated Homebrew formula updates
- Complete documentation of the automated pipeline
- Extensible to new platforms as runners are added

**Non-Goals:**
- Automated rc bump on failure (manual: fix → push new tag)
- Automated merge of release PR to main (create PR only, human merges)
- OpenVM guest program Cargo.toml bump (not listed in current release process, only sp1/risc0/zisk/openvm are bumped)
- Replacing the existing `tag_release.yaml` or `tag_latest.yaml` workflows
- Provisioning new self-hosted runners (arm64, macOS, GPU) — the system is designed to scale to them but they're out of scope

## Decisions

### 1. Workflow chain via `workflow_run`

The pipeline uses `workflow_run` triggers to chain workflows:

```
prepare-release → (tag push) → tag_release → (workflow_run) → release-test → (workflow_run) → promote-release → (release edit) → tag_latest
```

**Why not `workflow_call`?** `workflow_call` creates tight coupling — the caller must know the callee's inputs/outputs. `workflow_run` keeps workflows independently triggerable and testable. The trade-off is that all workflow files must exist on `main` before the first automated release.

**Why not `repository_dispatch`?** `workflow_run` is simpler — no extra step needed in the upstream workflow. It fires automatically when the referenced workflow completes.

### 2. Lightweight snap sync testing (no Kurtosis)

Snap sync tests run Lighthouse and ethrex directly — no Kurtosis, no assertoor, no ethereum-package.

```
Binary test:     download binary → run natively alongside Lighthouse
Docker test:     docker compose up with overridden image tag
Both:            poll eth_syncing → pass when synced, fail on 3h timeout
```

**Why not Kurtosis?** Kurtosis adds complexity without value for single EL+CL pair testing. The existing `docker-compose.yaml` already does exactly what we need. For binary tests, running natively is simpler and tests the actual release artifact without Docker wrapping.

**Alternative considered**: Wrapping binaries in minimal Docker images to reuse Kurtosis infrastructure. Rejected because it adds a layer between the test and the actual artifact, and the binary-in-Docker is not what users download.

### 3. Serial snap sync per runner, parallel L2 integration

Snap sync tests run one at a time per self-hosted runner (via GitHub Actions concurrency groups). L2 integration tests run fully parallel on GitHub-hosted runners.

**Why serial snap sync?** Each snap sync is I/O, CPU, and network intensive. Running multiple concurrent syncs on one machine would increase wall time for each and risk resource exhaustion. GitHub Actions naturally queues matrix jobs when runners are busy.

**Why parallel L2 integration?** L2 integration uses local Docker devnets (no real network access), runs on GitHub-hosted runners (unlimited parallelism), and takes ~30min each.

### 4. `fail-fast: false` with aggregation job

All test matrix entries run to completion regardless of failures. An aggregation job (with `if: always()`) collects all results, posts a summary table as a release comment, sends Slack notifications for failures, and gates promotion.

**Why?** The user explicitly needs to see all failures at once — fixing one failure and discovering another on the next run wastes hours given the 4h+ test cycle.

### 5. Fully automatic promotion

When all tests pass, `promote-release.yaml` fires automatically — no human gate. It creates the final `vX.Y.Z` tag, edits the GitHub release (marks as latest, removes pre-release flag), and creates a PR `release/vX.Y.Z → main`.

**Why fully automatic?** The testing is comprehensive (snap sync + L2 integration for every artifact). If all 14 artifacts pass snap sync on Hoodi and all L2 integration variants pass, there's no additional signal a human check would provide. The merge PR still requires human approval.

**Risk**: A bug that passes all automated tests but manifests on mainnet. Mitigated by: the tests are the same ones done manually today, and the merge PR provides a final human checkpoint.

### 6. Parameterized docker-compose.yaml

The root `docker-compose.yaml` changes from:
```yaml
image: "ghcr.io/lambdaclass/ethrex:main"
```
to:
```yaml
image: "ghcr.io/lambdaclass/ethrex:${ETHREX_TAG:-main}"
```

This lets release tests override the image tag without modifying the file. Existing usage (no env var set) behaves identically.

### 7. Homebrew automation via repository_dispatch

`tag_latest.yaml` dispatches to `lambdaclass/homebrew-tap` with the version number. A new workflow there updates the formula, computes checksums, builds the bottle, and creates a release.

**Why dispatch, not direct push?** Cross-repo workflow triggers keep the Homebrew tap's CI self-contained. The ethrex repo doesn't need write access to the tap repo — just dispatch permission.

### 8. Sync completion detection: two-phase polling

The snap sync test polls `eth_syncing` in two phases:
1. **Wait for sync start**: poll until `eth_syncing` returns a syncing object with `highestBlock > 0` (distinguishes "not started yet" from "completed")
2. **Wait for sync complete**: poll until `eth_syncing` returns `false`

**Why two phases?** `eth_syncing` returns `false` both when "not syncing yet" and "sync complete." Without phase 1, the test would pass immediately before sync begins.

Validation after sync: check `eth_blockNumber` is within a reasonable range of the network head.

## Risks / Trade-offs

- **[workflow_run bootstrap]** → All workflow files must be on `main` before first use. Mitigation: merge the implementation PR first, then cut the first automated release.

- **[4h wall time with 1 runner]** → Serial snap syncs on one x86_64 runner take ~4 hours. Mitigation: L2 integration runs fully parallel on GitHub-hosted runners; adding more self-hosted runners reduces wall time linearly. The system scales without structural changes.

- **[Platform coverage gaps]** → Only x86_64 Linux artifacts are testable on the current runner. arm64, macOS, and GPU tests queue indefinitely until runners exist. Mitigation: matrix entries for unavailable runners can be commented out or conditioned on runner availability labels. Coverage expands incrementally.

- **[Hoodi network instability]** → Snap sync depends on the Hoodi testnet being healthy. If Hoodi is down, all sync tests fail. Mitigation: 3h timeout provides buffer; if persistent, the team manually investigates and retags an rc.

- **[Cross-repo secret for Homebrew dispatch]** → `repository_dispatch` to `lambdaclass/homebrew-tap` requires a PAT with `repo` scope. Mitigation: use a fine-grained PAT scoped to only `homebrew-tap` and `actions:write`.

- **[Automatic promotion risk]** → A subtle bug could pass all tests and get promoted automatically. Mitigation: the merge PR (`release/vX.Y.Z → main`) still requires human review and approval. The promotion creates the release but doesn't merge the code.

## Open Questions

- **Lighthouse version pinning**: Should the snap sync test pin a specific Lighthouse version (like `daily_snapsync.yaml` does with `v8.0.1`), or use `latest`? Pinning is more reproducible; `latest` catches CL compatibility issues early.
- **Hoodi checkpoint sync URL**: The `docker-compose.yaml` uses `https://${ETHREX_NETWORK:-mainnet}-checkpoint-sync.attestant.io`. Need to confirm this works for Hoodi (`hoodi-checkpoint-sync.attestant.io`).
