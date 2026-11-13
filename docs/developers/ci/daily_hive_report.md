# Daily Hive Report
This workflow runs several of the simulations that are displayed on the [Ethereum Hive](https://hive.ethpandaops.io/#/group/generic?groupBy=client&clients=ethrex_default) page.

Notice that since Ethrex is a client that only supports post-merge forks, some simulations that appear on the official Hive page are not applicable and thus not included in this workflow.

At the time of this writing, these simulations should be running:

Supported:
```
consensus
discv4
eels/consume-engine
eels/consume-rlp
eels/consume-wirex
engine-api
engine-auth
engine-cancun
engine-exchange-capabilities
engine-withdrawals
eth
rpc-compat*
```

Not supported:
```
legacy
legacy-cancun

```

`rpc-compat` tests are a special case. We were passing all tests but this [PR](https://github.com/ethereum/execution-apis/pull/627) changed the genesis file from post-merge to pre-merge, so now we are not compatible. We should start a discussion with the STEEL team to use a post-merge genesis file. Note: on PRs, we pin the version of `execution-apis` to be one commit before the change, see [this link](https://github.com/lambdaclass/ethrex/blob/9feefd2e3fd2e8bb2097e5e39e0d20f7315c5880/.github/workflows/pr-main_l1.yaml#L186). Also see [this conversation](https://discord.com/channels/1359927674746835211/1428002540661899274/1447992228545953943). For now we are pinning the version of `execution-apis` to be one commit before the change.

# Daily report vs Official Hive Page
The tests that are run are almost the same, with some small discrepancies:
- We pin the fixture release the EELS simulators run, the hive commit, and the pins of the simulators that need their own, all in `.github/config/hive/pins.yaml`.
- We run the Fusaka tests. When we created these tests, Fusaka was not activated, but we wanted to include them. Now that we are in post-Fusaka, we might want to just run the same as the Hive page, see [this](https://github.com/lambdaclass/ethrex/blob/e9e0c3389b09c658295f522ac13f2d5f02645d90/.github/workflows/daily_hive_report.yaml#L115).

## Daily report vs CI run
We run some of the same simulations in the CI workflow, with a couple of differences:
- Both run the hive commit and the fixture release pinned in `.github/config/hive/pins.yaml`, and build their flags with `.github/scripts/hive-flags.sh`, so a simulation runs the same way in both.
- Of the Hive EELS simulators, the CI runs `consume-enginex` on the whole fixture release, on every push to a pull request. `consume-engine`, `consume-rlp`, `consume-wirex` and `execute-blobs` only run in the daily report, since they take too long for the CI. The CI also runs the lower level "blockchain tests" on the same fixtures.
