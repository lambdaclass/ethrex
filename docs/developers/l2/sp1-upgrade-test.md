# SP1 version upgrade test

This test checks that an L2 running a release with one SP1 version can move to a release with a newer one, such as SP1 5 to SP1 6, without redeploying. It runs on the GPU host (`l2-gpu`) with real proofs: the verification key and the verifier contract are what an SP1 upgrade changes, and the `exec` backend never consults either.

How the upgrade goes depends on what the OnChainProposer's `SP1_VERIFIER_ADDRESS` points at:

| Case | `SP1_VERIFIER_ADDRESS` | What changes on L1 |
|---|---|---|
| 1. Own verifier | The `SP1VerifierGroth16` the deployer created, the default | A new OnChainProposer implementation and verifier, and the new verification key |
| 2. Gateway, with downtime | Succinct's [`SP1VerifierGateway`](https://docs.succinct.xyz/docs/sp1/verification/contract-addresses) | The new verification key, after reverting the batches the gateway can no longer verify |
| 3. Gateway, without downtime | Succinct's `SP1VerifierGateway` | As in case 2, while the old sequencer keeps producing blocks |

Case 1 can also run without downtime; the steps are the same as case 3's handover, without the revert.

The gateway routes each proof by its first four bytes to the verifier of its SP1 version, so proofs of a new SP1 version need no change on L1. It also freezes old routes: Succinct froze the `v5.0.0` route (`0xa4594c59`) on Ethereum, Sepolia and Hoodi. A batch committed with SP1 5 and not verified before the freeze can no longer be verified with its SP1 5 proof, so it is reverted and committed again by the new release.

## Setup

```bash
export OLD=<previous release tag, e.g. v25.0.0>
export WORK=$HOME/sp1-upgrade-test
export L1_RPC=http://localhost:8545
export DEPLOYER_PK=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
# The --on-chain-proposer-owner and --bridge-owner account, which is also the Timelock's Security Council
export SC_PK=0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e
export SC_ADDR=0x4417092b70a3e5f10dc504d0947dd256b965fc62
mkdir -p $WORK && cd $WORK
```

- The old release: its `ethrex-l2-linux-x86_64-gpu` binary, its `ethrex-contracts.tar.gz` (for `ethrex-riscv32im-succinct-zkvm-vk-bn254`) and a checkout of its tag (for `fixtures/`).
- The new release, built from source with its contracts:

  ```bash
  COMPILE_CONTRACTS=true cargo build --release --features l2,l2-sql,sp1,gpu --bin ethrex
  export NEW=<path to the new checkout>
  export OUT=$(ls -d $NEW/target/release/build/ethrex-*/out/contracts/solc_out | head -1)
  # $OUT/OnChainProposer.bytecode and $OUT/SP1Verifier.bytecode (raw bytes)
  # The new release's key, under the keccak of the full commit hash its binary reports
  export COMMIT_HASH=$(cast keccak $($NEW/target/release/ethrex --version | grep -oE '[0-9a-f]{40}' | head -1))
  export VK=0x$(tr -d '\n' < $NEW/crates/guest-program/bin/sp1/out/riscv32im-succinct-zkvm-vk-bn254 | sed 's/^0x//')
  ```

- `cast`, `jq`, `xxd`, and `solc` for the gateway cases.
- GPU persistence mode on (`sudo nvidia-smi -pm 1`), and the reduced `SP1_WORKER_*` settings from the [SP1 prover guide](../../l2/deployment/prover/sp1.md#troubleshooting) on hosts with less than about 48 GB of memory.

Helpers used below. `cast` reads every option placed after `--create` as a constructor argument, so the options go first.

```bash
deploy_bytes() { cast send --private-key $SC_PK --rpc-url $L1_RPC --json --create 0x$(xxd -p "$1" | tr -d '\n') | jq -r .contractAddress; }
timelock() { cast send $TIMELOCK 'emergencyExecute(address,uint256,bytes)' $OCP 0 "$1" --private-key $SC_PK --rpc-url $L1_RPC --json | jq -r .status; }
```

## Run the old release

Start the L1 and deploy the old release with SP1 proofs required, following the [upgrade test](./upgrade-test.md) (Step 1), with one change for the gateway cases: deploy the gateway first (see below) and pass `--sp1.verifier-address $GW` to `ethrex l2 deploy`. Run the deployer from `$WORK`, so that it writes `$WORK/cmd/.env`, which the steps below read. Then:

```bash
set -a; source $WORK/cmd/.env; set +a
export OCP=$ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS TIMELOCK=$ETHREX_TIMELOCK_ADDRESS BRIDGE=$ETHREX_WATCHER_BRIDGE_ADDRESS
```

For the zero-downtime cases, start the old sequencer with p2p and its own admin server and proof coordinator, for example `--p2p.addr 127.0.0.1 --p2p.port 30303 --discovery.port 30303 --admin-server.port 5555 --proof-coordinator.port 3900`, and give the L1 node a different p2p port. Start the old prover against the old proof coordinator and wait until `lastVerifiedBatch()` is at least 1.

## Case 1: own verifier

1. Stop the old committer and wait until every committed batch is verified. With downtime, stop the old sequencer too; without downtime it keeps producing blocks.

   ```bash
   curl -sf http://localhost:5555/committer/stop
   cast call $OCP 'lastCommittedBatch()(uint256)' --rpc-url $L1_RPC
   cast call $OCP 'lastVerifiedBatch()(uint256)' --rpc-url $L1_RPC
   ```

2. Stop the old prover.
3. Deploy the new verifier and OnChainProposer implementation, upgrade the OnChainProposer and set the verifier in one Timelock call, and register the new key.

   ```bash
   NEW_VERIFIER=$(deploy_bytes $OUT/SP1Verifier.bytecode)
   NEW_IMPL=$(deploy_bytes $OUT/OnChainProposer.bytecode)
   timelock $(cast calldata 'upgradeToAndCall(address,bytes)' $NEW_IMPL $(cast calldata 'upgradeSP1Verifier(address)' $NEW_VERIFIER))
   timelock $(cast calldata 'upgradeSP1VerificationKey(bytes32,bytes32)' $COMMIT_HASH $VK)
   ```

4. Move to the new release: with downtime, restart the sequencer and prover with the new binary on the old data directories; without downtime, follow [the handover](#handover-without-downtime).

Expected: `SP1_VERIFIER_ADDRESS()` is `$NEW_VERIFIER` and `VERSION()` on it is `"v6.1.0"`, the first batch committed by the new release is verified, and the [checks](#checks) pass.

## Cases 2 and 3: Succinct's gateway

### Deploy a gateway on the test L1

The gateway is not on the local L1, so the test deploys one from sp1-contracts, which the build already fetched, with routes for both SP1 versions. Its owner stands in for Succinct.

```bash
SRC=$(ls -d $NEW/target/release/build/ethrex-*/out/contracts/lib/sp1-contracts/contracts | head -1)
mkdir -p gw/lib && cp -r $SRC/src gw/src
ln -s $(dirname $SRC)/../openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts gw/lib/openzeppelin-contracts
(cd gw && solc --base-path . --allow-paths .,$(readlink -f lib/openzeppelin-contracts) --optimize --bin -o out \
  src/SP1VerifierGateway.sol src/v5.0.0/SP1VerifierGroth16.sol)

V5=$(cast send --private-key $SC_PK --rpc-url $L1_RPC --json --create 0x$(cat gw/out/SP1Verifier.bin) | jq -r .contractAddress)
V6=$(deploy_bytes $OUT/SP1Verifier.bytecode)
GW=$(cast send --private-key $SC_PK --rpc-url $L1_RPC --json --create 0x$(cat gw/out/SP1VerifierGateway.bin) 'constructor(address)' $SC_ADDR | jq -r .contractAddress)
cast send $GW 'addRoute(address)' $V5 --private-key $SC_PK --rpc-url $L1_RPC
cast send $GW 'addRoute(address)' $V6 --private-key $SC_PK --rpc-url $L1_RPC
```

Deploy the old release with `--sp1.verifier-address $GW` and start it as above.

### Freeze the old route

Once `lastCommittedBatch` is at least two ahead of `lastVerifiedBatch`, freeze the `v5.0.0` route, as Succinct did:

```bash
cast send $GW 'freezeRoute(bytes4)' 0xa4594c59 --private-key $SC_PK --rpc-url $L1_RPC
cast call $GW 'routes(bytes4)(address,bool)' 0xa4594c59 --rpc-url $L1_RPC   # $V5 true
```

From here the old sequencer keeps committing batches, but `lastVerifiedBatch` stays where it was: every SP1 5 proof is rejected.

### Recover

1. Stop the old committer and the old prover. Record the last verified and last committed batches.

   ```bash
   curl -sf http://localhost:5555/committer/stop
   V=$(cast call $OCP 'lastVerifiedBatch()(uint256)' --rpc-url $L1_RPC)
   C=$(cast call $OCP 'lastCommittedBatch()(uint256)' --rpc-url $L1_RPC)
   ```

2. Check that none of the batches to revert published withdrawals. `CommonBridge` refuses to publish withdrawals twice for one batch number and `revertBatch` does not clear them, so a batch with a non-zero root cannot be committed again. If any root is non-zero, stop here: there is no way to recover that batch until the contracts clear the roots on revert (#7414).

   ```bash
   for n in $(seq $((V+1)) $C); do cast call $BRIDGE 'batchWithdrawalLogsMerkleRoots(uint256)(bytes32)' $n --rpc-url $L1_RPC; done
   ```

3. Revert them on L1 and register the new key. `revertBatch` needs the OnChainProposer paused, and all three calls go through the Timelock.

   ```bash
   timelock $(cast calldata 'pause()')
   timelock $(cast calldata 'revertBatch(uint256)' $((V+1)))
   timelock $(cast calldata 'unpause()')
   timelock $(cast calldata 'upgradeSP1VerificationKey(bytes32,bytes32)' $COMMIT_HASH $VK)
   ```

4. Case 2, with downtime: stop the old sequencer, revert its rollup store, and restart on the new release with the same data directory. `ethrex l2 revert-batch` keeps the batch it is given and deletes the later ones, so pass the last verified batch. It also leaves behind the committer's state snapshots of the reverted batches, which the new committer would take as its own, so delete them too.

   ```bash
   ethrex l2 revert-batch $V $OCP --datadir <L2 datadir>
   find <L2 datadir> -maxdepth 1 -type d \( -name 'checkpoint_batch_*' -o -name 'temp_checkpoint_batch_*' \) | while read -r d; do
     n=$(basename "$d" | sed -E 's/^(temp_)?checkpoint_batch_([0-9]+).*/\2/'); [ "$n" -gt "$V" ] && rm -rf "$d"
   done
   ```

5. Case 3, without downtime: leave the old sequencer running and follow [the handover](#handover-without-downtime). The new sequencer receives the batches the old one sealed and commits them again, generating its own prover input for each.

Expected: the first re-committed batch is proven with SP1 6 and verified through the gateway's `v6.1.0` route, `SP1_VERIFIER_ADDRESS()` still being `$GW`, and the [checks](#checks) pass.

## Handover without downtime

1. Start the new sequencer next to the old one, with its own data directory, ports and proof coordinator, following the old one over p2p. The L2 RPC only serves `admin_nodeInfo` when `admin` is in its `--http.api`, so take the old sequencer's enode from its startup log. `N` is a block far enough ahead for the new sequencer to catch up.

   ```bash
   ENODE=$(grep -m1 -oE 'enode://[0-9a-f]+@[0-9.]+:[0-9]+' <old sequencer log>)
   N=$(( $(cast block-number --rpc-url http://localhost:1729) + 30 ))
   $NEW/target/release/ethrex l2 <the old sequencer's flags> --datadir <new datadir> \
     --http.port 1730 --authrpc.port 8562 --metrics.port 3703 --p2p.port 30304 --discovery.port 30304 \
     --admin-server.port 5556 --proof-coordinator.port 3901 \
     --bootnodes $ENODE --admin.start-at $N --admin.l2-head-check-rpc-url http://localhost:1729
   $NEW/target/release/ethrex l2 prover --backend sp1 --proof-coordinators tcp://127.0.0.1:3901
   ```

2. Tell the old sequencer to stop at `N`. It produces block `N` and switches to following; the new sequencer logs `The sequencer is now Sequencing` and produces the next block.

   ```bash
   curl -sf -X POST http://localhost:5555/state-updater/stop-at/$N
   ```

3. The old sequencer stays at block `N`, so point clients at the new one (`http://localhost:1730`) and stop the old one.

## Checks

- The block timestamps around `N` are as far apart as before: the handover leaves no gap.

  ```bash
  for b in $(seq $((N-3)) $((N+3))); do cast block $b timestamp --rpc-url http://localhost:1730; done
  ```

- `lastVerifiedBatch()` passes the batches the old release committed, then keeps up with `lastCommittedBatch()`.
- The L2 integration tests pass against the new sequencer:

  ```bash
  cd $NEW && cp $WORK/cmd/.env cmd/.env
  INTEGRATION_TEST_L1_RPC=$L1_RPC INTEGRATION_TEST_L2_RPC=<new sequencer RPC> \
  ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH=<old checkout>/fixtures/keys/private_keys_l1.txt \
  INTEGRATION_TEST_PRIVATE_KEYS_FILE_PATH=<old checkout>/fixtures/keys/private_keys_l1.txt \
  INTEGRATION_TEST_BRIDGE_OWNER_PRIVATE_KEY=$SC_PK \
  cargo test -p ethrex-test l2::integration_tests --release --features l2 -- --nocapture --test-threads=1
  ```

## Last run

From v25.0.0 to the first build with SP1 6.9.0, on `l2-gpu` (NVIDIA RTX 4000 Ada, 20 GB), in October 2026:

| Case | Old release when the upgrade started | Gap at the handover | First new batch verified | Integration tests | Batches verified at the end |
|---|---|---|---|---|---|
| 1. Own verifier, with downtime | 13 committed, 13 verified | the sequencer stops | 2 minutes after the restart | passed | 35 of 36 |
| 1. Own verifier, without downtime | 8 committed, 8 verified | none: blocks 5 s apart | 2.5 minutes after the handover | passed | 30 of 30 |
| 2. Gateway, with downtime | 6 committed, 1 verified, 5 reverted | the sequencer stops | 2 minutes after the restart | passed | 21 of 21 |
| 3. Gateway, without downtime | 6 committed, 1 verified, 5 reverted | none: blocks 5 s apart | 2 minutes after the handover | passed | 21 of 22 |

In case 3 the new sequencer logged `Batch 2 was sealed without a prover input for version ..., generating it` for each batch the old one had sealed, and the old release's database was migrated at startup in case 2.
