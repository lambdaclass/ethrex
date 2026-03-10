## 1. Snap Sync Test Action

- [ ] 1.1 Create `.github/actions/snap-sync-test/action.yml` composite action with inputs: `mode` (binary/docker), `binary_path`, `image_tag`, `network`, `timeout`, `lighthouse_version`, `poll_interval`
- [ ] 1.2 Implement the sync polling script (`.github/actions/snap-sync-test/poll-sync.sh`): two-phase detection (wait for `highestBlock > 0`, then wait for `eth_syncing == false`), `eth_blockNumber` validation, timeout handling, log collection on failure
- [ ] 1.3 Implement binary mode: download Lighthouse binary for the runner's platform, generate JWT, start Lighthouse + ethrex binary, invoke poll script, cleanup on exit
- [ ] 1.4 Implement docker mode: set `ETHREX_TAG` env var, `docker compose up -d`, invoke poll script against `localhost:8545`, `docker compose down` on exit
- [ ] 1.5 Parameterize root `docker-compose.yaml`: change `image: "ghcr.io/lambdaclass/ethrex:main"` to `image: "ghcr.io/lambdaclass/ethrex:${ETHREX_TAG:-main}"`

## 2. Prepare Release Workflow

- [ ] 2.1 Create `.github/workflows/prepare-release.yaml` with `workflow_dispatch` trigger accepting `version` (string) and `commit_sha` (string) inputs
- [ ] 2.2 Implement input validation: version format (`X.Y.Z` regex), commit SHA existence (`git cat-file -t`), branch non-existence check
- [ ] 2.3 Implement branch creation: checkout `commit_sha`, create and push `release/vX.Y.Z`
- [ ] 2.4 Implement version bump script: `sed` updates across 6 Cargo.toml files + `docs/CLI.md` (`"ethrex OLD"` → `"ethrex X.Y.Z"`). Detect the current version from workspace `Cargo.toml` to know what to replace.
- [ ] 2.5 Implement lockfile update: run `make update-cargo-lock` (requires Rust toolchain setup)
- [ ] 2.6 Implement commit + tag + push: commit all changes, create tag `vX.Y.Z-rc.1`, push branch and tag

## 3. Release Test Workflow

- [ ] 3.1 Create `.github/workflows/release-test.yaml` with `workflow_run` trigger on `tag_release.yaml` completion, filtered to rc tags only
- [ ] 3.2 Implement tag extraction: parse the rc tag name from the triggering workflow run context
- [ ] 3.3 Implement snap sync test job with matrix: all binary artifacts (download from GitHub pre-release via `gh release download`) + all Docker image artifacts (pull from GHCR). Use `fail-fast: false`. Assign runner labels per platform (`ethrex-sync` for x86_64, future labels for arm64/macOS/GPU).
- [ ] 3.4 For binary matrix entries: download binary from pre-release, `chmod +x`, invoke `snap-sync-test` action in binary mode
- [ ] 3.5 For Docker matrix entries: invoke `snap-sync-test` action in docker mode with the rc image tag
- [ ] 3.6 Implement L2 integration test job with matrix: L2 Docker image × {Validium, Vanilla, Web3signer, Based}. Runs on `ubuntu-latest`. Pull release Docker images from GHCR, tag them as expected by `crates/l2/docker-compose.yaml` (`ethrex:main` and `ethrex:main-l2`), follow existing L2 integration test pattern from `pr-main_l2.yaml`.
- [ ] 3.7 Implement aggregation job (`if: always()`, `needs: [snap-sync, l2-integration]`): collect all job results, build a markdown summary table, post as comment on the GitHub pre-release via `gh release edit --notes`, set `all_passed` output
- [ ] 3.8 Implement Slack failure notification in aggregation job: on any failure, send message to `ETHREX_L1_SLACK_WEBHOOK` with failed test list and workflow run link
- [ ] 3.9 Set concurrency groups for snap sync jobs: `group: snap-sync-${{ matrix.runner-label }}`, `cancel-in-progress: false`

## 4. Promote Release Workflow

- [ ] 4.1 Create `.github/workflows/promote-release.yaml` with `workflow_run` trigger on `release-test.yaml` completion, filtered to success + `all_passed == true`
- [ ] 4.2 Implement tag extraction: derive `vX.Y.Z` from the rc tag (strip `-rc.N` suffix)
- [ ] 4.3 Implement final tag creation: `git tag vX.Y.Z <commit>` and push
- [ ] 4.4 Implement release edit: use `gh release edit` to update tag to `vX.Y.Z`, update title, set `--latest`, remove `--prerelease` flag
- [ ] 4.5 Implement merge PR creation: `gh pr create` from `release/vX.Y.Z` to `main` with title and description

## 5. Homebrew Automation

- [ ] 5.1 Extend `tag_latest.yaml`: add `update-homebrew` job after `publish-apt` that sends `repository_dispatch` to `lambdaclass/homebrew-tap` with version payload. Requires `HOMEBREW_DISPATCH_TOKEN` secret.
- [ ] 5.2 Create `update-formula.yaml` in `lambdaclass/homebrew-tap`: triggered by `repository_dispatch` with `event_type: update-formula`
- [ ] 5.3 Implement formula update: download source tarball, compute sha256, update `Formula/ethrex.rb` URL and sha256
- [ ] 5.4 Implement bottle build: install ethrex from source, create bottle, compute bottle sha256, update formula bottle block
- [ ] 5.5 Commit formula changes and create release with bottle attached

## 6. Documentation

- [ ] 6.1 Update `docs/developers/release-process/release-process.md` to document the automated pipeline: new workflow_dispatch trigger, what happens automatically, what requires human action (merge PR)
- [ ] 6.2 Update `docs/developers/release-process/pre-release-checklist.md` to note that pre-release testing is now automated and reference the `release-test.yaml` workflow
- [ ] 6.3 Update `docs/developers/release-process/post-release-checklist.md` to note that Homebrew and apt updates are automated
- [ ] 6.4 Add inline documentation in each new workflow file: purpose, trigger chain, inputs/outputs, and how to debug failures
