# How to Release an ethrex version

Releases are initiated with a single GitHub Actions dispatch and flow through an automated pipeline that builds, tests, promotes, and publishes artifacts.

## Overview

The release pipeline is a chain of GitHub Actions workflows:

```
prepare-release → tag_release → release-test → promote-release → tag_latest
```

| Step | Workflow | Trigger | What it does |
|------|----------|---------|--------------|
| 1 | `prepare-release.yaml` | `workflow_dispatch` (manual) | Creates release branch, bumps version, tags `vX.Y.Z-rc.1` |
| 2 | `tag_release.yaml` | Tag push (`v*.*.*-*`) | Builds binaries, Docker images, creates GitHub pre-release |
| 3 | `release-test.yaml` | `workflow_run` on `tag_release` | Snap sync + L2 integration tests on all artifacts |
| 4 | `promote-release.yaml` | `workflow_run` on `release-test` | Creates final tag, promotes release, opens merge PR |
| 5 | `tag_latest.yaml` | Release edit | Retags Docker images, publishes apt package, updates Homebrew |

## Triggering a Release

1. Go to the [Actions tab](https://github.com/lambdaclass/ethrex/actions) and select the **Prepare Release** workflow.
2. Click **Run workflow** and provide:
   - **version**: The release version in `X.Y.Z` format (e.g., `9.1.0`)
   - **commit_sha**: The commit to release from (must exist in the repository)
3. Click **Run workflow**.

Everything else is automatic. The workflow validates inputs, creates the `release/vX.Y.Z` branch, bumps the version in all required files, runs `make update-cargo-lock`, commits, tags `vX.Y.Z-rc.1`, and pushes.

## What Happens Automatically

### Version bump

The `prepare-release` workflow updates the version string in:

| File | Field |
|------|-------|
| `Cargo.toml` (workspace root) | `workspace.package.version` |
| `crates/guest-program/bin/sp1/Cargo.toml` | `package.version` |
| `crates/guest-program/bin/risc0/Cargo.toml` | `package.version` |
| `crates/guest-program/bin/zisk/Cargo.toml` | `package.version` |
| `crates/guest-program/bin/openvm/Cargo.toml` | `package.version` |
| `crates/l2/tee/quote-gen/Cargo.toml` | `package.version` |
| `docs/CLI.md` | `"ethrex X.Y.Z"` default value references |

After the version bump, `make update-cargo-lock` synchronizes all lockfiles.

### Build

Pushing the `vX.Y.Z-rc.1` tag triggers `tag_release.yaml`, which builds:

| Artifact | L1 | L2 | CUDA |
|----------|----|----|------|
| `ethrex-linux-x86-64` | yes | - | - |
| `ethrex-linux-aarch64` | yes | - | - |
| `ethrex-macos-aarch64` | yes | - | - |
| `ethrex-l2-linux-x86-64` | yes | yes | no |
| `ethrex-l2-linux-x86-64-gpu` | yes | yes | yes |
| `ethrex-l2-linux-aarch64` | yes | yes | no |
| `ethrex-l2-linux-aarch64-gpu` | yes | yes | yes |
| `ethrex-l2-macos-aarch64` | yes | yes | no |

Docker images are also built and pushed:
- `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N`
- `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N-l2`

A changelog is generated from commit messages since the last stable tag.

### Testing

`release-test.yaml` runs automatically when the build completes. It tests every artifact:

- **Snap sync tests**: Each binary and Docker image is tested against the Hoodi testnet with a 3-hour timeout. Snap sync tests run serially per self-hosted runner.
- **L2 integration tests**: L2 artifacts are tested with Validium, Vanilla, Web3signer, and Based variants. These run in parallel on GitHub-hosted `ubuntu-latest` runners.

All tests use `fail-fast: false` -- every test runs to completion regardless of other failures. An aggregation job posts a results summary as a comment on the GitHub pre-release and sends Slack notifications for any failures.

### Promotion

When all tests pass, `promote-release.yaml` triggers automatically and:

1. Creates the final `vX.Y.Z` tag on the same commit as the rc tag
2. Edits the GitHub release: changes the tag, updates the title, marks as latest, removes the pre-release flag
3. Creates a PR from `release/vX.Y.Z` to `main`

### Publishing

The release edit triggers `tag_latest.yaml`, which:

1. Retags Docker images to `ghcr.io/lambdaclass/ethrex:X.Y.Z`, `ghcr.io/lambdaclass/ethrex:latest`, and the corresponding `-l2` / `l2` tags
2. Publishes the apt package via `lambdaclass/ethrex-apt`
3. Dispatches to `lambdaclass/homebrew-tap` to update the Homebrew formula

## What Requires Human Action

1. **Merging the release PR**: The promotion step creates a PR from `release/vX.Y.Z` to `main`, but it does **not** auto-merge. A maintainer must review and merge it.
2. **Handling test failures**: If any test fails, the pipeline stops at the testing step. See [Handling Test Failures](#handling-test-failures) below.

## Handling Test Failures

If `release-test.yaml` reports failures:

1. Check the [Actions tab](https://github.com/lambdaclass/ethrex/actions) for the failed workflow run. The aggregation job posts a summary table showing which tests passed and which failed.
2. Fix the issue on the `release/vX.Y.Z` branch.
3. Create and push a new rc tag:

```bash
git tag vX.Y.Z-rc.2
git push origin vX.Y.Z-rc.2
```

This re-triggers the build and test pipeline from step 2.

## Monitoring

All pipeline progress is visible in the [GitHub Actions tab](https://github.com/lambdaclass/ethrex/actions). Each workflow in the chain shows its status. Test result summaries are posted as comments on the GitHub pre-release.

## Dealing with Hotfixes

If hotfixes are needed before the final release, commit them to `release/vX.Y.Z`, push, and create a new rc tag (e.g., `vX.Y.Z-rc.2`). The final `vX.Y.Z` tag is created automatically by the promotion workflow and points to the exact commit that passed all tests.

## Troubleshooting

### Failure on "latest release" workflow

If `tag_latest.yaml` fails, Docker tags `latest` and `l2` may not be updated. To manually push those changes:

- Create a new Github Personal Access Token (PAT) from the [settings](https://github.com/settings/tokens/new).
- Check `write:packages` permission (this will auto-check `repo` permissions too), give a name and a short expiration time.
- Save the token securely.
- Click on `Configure SSO` button and authorize LambdaClass organization.
- Log in to Github Container Registry: `docker login ghcr.io`. Put your Github username and use the token as your password.
- Pull RC images:

```bash
docker pull --platform linux/amd64 ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W
docker pull --platform linux/amd64 ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W-l2
```

- Retag them:

```bash
docker tag ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W ghcr.io/lambdaclass/ethrex:X.Y.Z
docker tag ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W-l2 ghcr.io/lambdaclass/ethrex:X.Y.Z-l2
docker tag ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W ghcr.io/lambdaclass/ethrex:latest
docker tag ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.W-l2 ghcr.io/lambdaclass/ethrex:l2
```

- Push them:

```bash
docker push ghcr.io/lambdaclass/ethrex:X.Y.Z
docker push ghcr.io/lambdaclass/ethrex:X.Y.Z-l2
docker push ghcr.io/lambdaclass/ethrex:latest
docker push ghcr.io/lambdaclass/ethrex:l2
```

- Delete the PAT for security ([here](https://github.com/settings/tokens))

### Bootstrap requirement

All workflow files must exist on `main` before the first automated release. The `workflow_run` trigger only works for workflows that exist on the default branch. Merge the pipeline implementation PR before attempting the first automated release.
