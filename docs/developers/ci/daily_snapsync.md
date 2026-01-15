# Daily Snapsync Check

Inside [GitHub actions tab](https://github.com/lambdaclass/ethrex/actions) there is a pinned workflow called [Daily Snapsync Check](https://github.com/lambdaclass/ethrex/actions/workflows/daily_snapsync.yaml). As the name suggests, it runs an Ethrex node and attempts to complete a snap sync and catch the head of the chain. If it is not able to finish snapsync before the timeout, the job fails and an [alert](https://github.com/lambdaclass/ethrex/blob/9feefd2e3fd2e8bb2097e5e39e0d20f7315c5880/.github/workflows/common_failure_alerts.yaml#L8) is sent to Slack. The way it works is through an [Assertoor playbook](https://github.com/lambdaclass/ethrex/blob/9feefd2e3fd2e8bb2097e5e39e0d20f7315c5880/.github/config/assertoor/syncing-check.yaml#L1-L17) that checks that the node `eth_syncing` returns `false`.

Currently it runs this check on Sepolia and Hoodi with both Lighthouse and Prysm consensus clients.

## Debug Assertions

By default, the workflow builds ethrex with debug assertions enabled (`release-with-debug-assertions` profile). This helps catch potential bugs and invariant violations during the sync process.

The `release-with-debug-assertions` profile:
- Inherits all release optimizations
- Enables `debug_assert!()` macros to run

## Triggering the Workflow

The workflow can be triggered in three ways:

1. **Scheduled**: Runs every 6 hours automatically on both Hoodi and Sepolia networks
2. **Pull Request**: Runs on PRs that modify the workflow file (Hoodi only)
3. **Manual Dispatch**: Can be triggered manually via GitHub Actions UI with options:
   - `network`: Choose between `hoodi` or `sepolia`
   - `build_profile`: Choose between `release` or `release-with-debug-assertions` (default: `release-with-debug-assertions`)

## Use Cases

Apart from being a useful job to catch regressions, it is a good log to see if there were any speedups or slowdowns in terms of time to complete a snap sync. With debug assertions enabled, it also serves as an additional validation layer to catch bugs that might not manifest as crashes in release builds.
