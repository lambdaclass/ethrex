# Pre-release Checklist

> **Note**: Pre-release testing is now automated by the `release-test.yaml` workflow. This checklist documents what the automation covers. You do not need to perform these steps manually.

## Automated by CI

The `release-test.yaml` workflow runs automatically after `tag_release.yaml` builds the release candidate. It covers:

### Snap sync tests

Each artifact is tested against the Hoodi testnet with a 3-hour timeout. The test runs Lighthouse alongside ethrex, polls `eth_syncing` for completion, and validates `eth_blockNumber` is within range of the network head.

| Artifact | Mode | Runner |
|----------|------|--------|
| `ethrex-linux-x86_64` | Binary | Self-hosted (`ethrex-sync`, x86_64) |
| `ethrex-linux-aarch64` | Binary | Self-hosted (arm64, when available) |
| `ethrex-macos-aarch64` | Binary | Self-hosted (macOS, when available) |
| `ethrex-l2-linux-x86_64` | Binary | Self-hosted (`ethrex-sync`, x86_64) |
| `ethrex-l2-linux-x86_64-gpu` | Binary | Self-hosted (x86_64 + CUDA, when available) |
| `ethrex-l2-linux-aarch64` | Binary | Self-hosted (arm64, when available) |
| `ethrex-l2-linux-aarch64-gpu` | Binary | Self-hosted (arm64 + CUDA, when available) |
| `ethrex-l2-macos-aarch64` | Binary | Self-hosted (macOS, when available) |
| `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N` | Docker | Self-hosted (`ethrex-sync`, x86_64) |
| `ghcr.io/lambdaclass/ethrex:X.Y.Z-rc.N-l2` | Docker | Self-hosted (`ethrex-sync`, x86_64) |

Snap sync tests run serially per runner (one at a time) using GitHub Actions concurrency groups.

### L2 integration tests

L2 artifacts are tested with the following variants on GitHub-hosted `ubuntu-latest` runners (fully parallel):

- Validium
- Vanilla
- Web3signer
- Based

### Test behavior

- All tests use `fail-fast: false` -- every test runs to completion even if others fail.
- An aggregation job collects results and posts a summary table as a comment on the GitHub pre-release.
- Failed tests trigger a Slack notification with the failure list and a link to the workflow run.
- If all tests pass, promotion proceeds automatically.
