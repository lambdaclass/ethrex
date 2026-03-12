# Post-release Checklist

> **Note**: Most post-release tasks are now automated. This documents what happens automatically and what to verify manually.

## Automated by CI

The following steps are handled by `tag_latest.yaml` when a release is promoted:

- **Docker image retagging**: RC images are retagged to `ghcr.io/lambdaclass/ethrex:X.Y.Z`, `ghcr.io/lambdaclass/ethrex:latest`, and corresponding `-l2` / `l2` variants.
- **Apt package publishing**: The apt package is published via `lambdaclass/ethrex-apt`.
- **Homebrew formula update**: A `repository_dispatch` event is sent to `lambdaclass/homebrew-tap`, which updates the formula, computes checksums, builds the bottle, and creates a release.

## Manual verification

After a release is promoted, verify:

- [ ] The [release page](https://github.com/lambdaclass/ethrex/releases) shows the release as "Latest" with the correct `vX.Y.Z` tag.
- [ ] Docker images are available: `docker pull ghcr.io/lambdaclass/ethrex:latest`
- [ ] The merge PR from `release/vX.Y.Z` to `main` has been created. **Review and merge it.**
- [ ] If `tag_latest.yaml` failed, follow the [troubleshooting steps](./release-process.md#failure-on-latest-release-workflow) to manually retag Docker images.
