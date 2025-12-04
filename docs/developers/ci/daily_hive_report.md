# Daily Hive Report
This workflow runs several of the simulations that are displayed on the [Ethereum Hive](https://hive.ethpandaops.io/#/group/generic?groupBy=client&clients=ethrex_default) page.

Notice that since Ethrex is a client that only supports post-merge forks, some simulations that appear on the official Hive page are not applicable and thus not included in this workflow.

At the time of this writing, these simulations should be running:

Supported:
```
consensus
discv4
eels/consmume-engine
eels/consume-rlp
engine-api
engine-auth
engine-cancun
engine-exchange-capabilities
engine-withdrawals
eth
```

Not supported:
```
legacy
legacy-cancun
rpc-compat
```

`rpc-compat` tests are a special case. We were passing all tests but this [PR](https://github.com/ethereum/execution-apis/pull/627) changed the genesis file from post-merge to pre-merge, so now we are not compatible. We should start a discussion with the STEEL team to use a post-merge genesis file. Note: on PRs, we pin the version of `execution-apis` to be one commit before the change, see [this link](https://github.com/lambdaclass/ethrex/blob/9feefd2e3fd2e8bb2097e5e39e0d20f7315c5880/.github/workflows/pr-main_l1.yaml#L186).

## Daily report vs CI run
We run some of the same simulations in the CI workflow, with a couple of differences:
- In the CI we run a pinned version of hive (see this [link](https://github.com/lambdaclass/ethrex/blob/9feefd2e3fd2e8bb2097e5e39e0d20f7315c5880/.github/workflows/pr-main_l1.yaml#L259)) whereas in the daily report we run the latest master branch. This is because we prioritize stability on the CI workflow, while the daily report aims to provide a more up-to-date simulation results.
- We only run Hive EELS tests in the daily run, since they take too long to be run on the CI. That being said, we run the equivalent more low level "blockchain tests", which should provide the same coverage (see this [link](https://github.com/lambdaclass/ethrex/blob/9feefd2e3fd2e8bb2097e5e39e0d20f7315c5880/.github/workflows/pr-main_l1.yaml#L107)).
