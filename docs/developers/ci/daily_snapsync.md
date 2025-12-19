# Daily Snapsync Check

Inside [GitHub actions tab](https://github.com/lambdaclass/ethrex/actions) there is a pinned workflow called [Daily Snapsync Check](https://github.com/lambdaclass/ethrex/actions/workflows/daily_snapsync.yaml). As the name suggests, it runs an Ethrex node and attemps to complete a snap sync and catch the head of the chain. If it is not able to finish snapsync before the timeout, the job fails and an [alert](https://github.com/lambdaclass/ethrex/blob/9feefd2e3fd2e8bb2097e5e39e0d20f7315c5880/.github/workflows/common_failure_alerts.yaml#L8) is sent to Slack. The way it works is through an [Assertoor playbook](https://github.com/lambdaclass/ethrex/blob/9feefd2e3fd2e8bb2097e5e39e0d20f7315c5880/.github/config/assertoor/syncing-check.yaml#L1-L17) that checks that the node `eth_syncing` returns `false`.

Currently it runs this check on Sepolia and Hoodi.

Apart from being a useful job to catch regressions, it is a good log to see if there were any speedups of slowdowns in terms of time to complete a snap sync.

Nice to haves:
- Currently the job runs on the `main` docker image of ethrex. It would be nice to be able to trigger this from a branch so that the workflow is executed by trying to sync the branch ethrex code.
