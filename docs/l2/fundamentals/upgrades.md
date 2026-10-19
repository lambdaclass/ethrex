# Upgrades

## Sequencer and prover versions

Each committed batch stores the git commit hash of the sequencer build that produced it. The OnChainProposer uses that commit hash to look up the verifier key in its `verificationKeys` mapping. When the sequencer is upgraded, all batches committed before the upgrade must be proved with the prover matching the old version, and all batches committed after the upgrade must be proved with a prover built from the new version.

## Registering a new verification key

To allow proofs from a new sequencer/prover build, register its verification key against the commit hash:

1. Compute the commit hash as the Keccak-256 of the (reduced) git commit. For example, the commit `9219410` produces `b9105485bc4ba523201eaaf76478a47b259fa7399bbed795cf19294861b7fc57`.
2. From the OnChainProposer owner account, send the upgrade transaction. Example (replace addresses and keys with your values):
   ```
   rex send <ON_CHAIN_PROPOSER_ADDRESS> \
     "upgradeSP1VerificationKey(bytes32,bytes32)" \
     <KECCAK_GIT_COMMIT> \
     <VERIFICATION_KEY> \
     --private-key <ON_CHAIN_PROPOSER_OWNER_PK>
   ```
3. (Optional) Verify the mapping entry:
   ```
   rex call <ON_CHAIN_PROPOSER_ADDRESS> \
     "verificationKeys(bytes32,uint8)(bytes32)" \
     <KECCAK_GIT_COMMIT> \
     <PROVER_ID>
   ```
   `1` is the SP1 verifier ID, `2` is RISC0.

## Replacing the SP1 verifier

A new verification key is enough while the prover stays on the same SP1 version. A new SP1 version can also change the Groth16 circuit its proofs are wrapped in, and an `SP1VerifierGroth16` contract only accepts proofs from its own circuit: it checks the first four bytes of the proof against its `VERIFIER_HASH` and reverts with `WrongVerifierSelector` otherwise. Moving from SP1 5 to SP1 6 is such a change.

The OnChainProposer reads the verifier from `SP1_VERIFIER_ADDRESS`, the same for every batch. If that is Succinct's [`SP1VerifierGateway`](https://docs.succinct.xyz/docs/sp1/verification/contract-addresses), it routes each proof to the verifier of its SP1 version and there is nothing to replace. If it is a verifier of its own, which is what the deployer creates by default, replace it:

1. Let every batch committed by the old version be verified first, with the old prover: once the verifier is replaced, their proofs no longer verify. `upgradeSP1Verifier` enforces it, reverting with `UnverifiedBatchesPending` while `lastVerifiedBatch` is below `lastCommittedBatch`.
2. Deploy the new `SP1VerifierGroth16` from [sp1-contracts](https://github.com/succinctlabs/sp1-contracts), at the version that matches the prover's SP1 release (for example `v5.0.0` for SP1 5 and `v6.1.0` for SP1 6).
3. From the OnChainProposer owner, which is the Timelock on deployments since v9, call `upgradeSP1Verifier(address)` with the new verifier's address, and register the new version's verification key as described above.
4. Start the new sequencer and prover.

A deployment whose OnChainProposer predates `upgradeSP1Verifier` upgrades its implementation first, and can set the verifier in the same Timelock call: `upgradeToAndCall(<new implementation>, <calldata of upgradeSP1Verifier(<new verifier>)>)`. The same check applies, so the whole call reverts if a batch is still unverified.

### Verification key artifacts

The verification key that goes on-chain is obtained when you build the prover.

For SP1 it is stored at:

  - `crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-bn254`.

  - If proving with Aligned, use the `u32` form generated alongside it at `crates/l2/prover/src/guest_program/src/sp1/out/riscv32im-succinct-zkvm-vk-u32`.

For RISC0 it is stored at:
  - `crates/l2/prover/src/guest_program/src/risc0/out/riscv32im-risc0-vk`


## Upgrade sequencer with zero downtime

This is a **test/example** flow for local development. It demonstrates a zero-downtime handover by running two sequencers in parallel and coordinating the handoff using `--admin.start-at` and `POST /state-updater/stop-at/<N>`.

1. First, initialize L1:

```bash
cd crates/l2
make rm-db-l1 init-l1
```

2. Then deploy the contracts:

```bash
cd crates/l2
rm -rf ../../dev_ethrex_l*; make deploy-l1
```

3. Start the prover and point it at both proof coordinators:

```bash
./target/release/ethrex \
        l2 prover \
        --proof-coordinators tcp://127.0.0.1:3900 tcp://127.0.0.1:3901 \
        --backend exec
```

4. Start the first sequencer (from the repo root):

```bash
export $(cat cmd/.env | xargs); export COMPILE_CONTRACTS=true; target/release/ethrex \
    l2 --no-monitor \
    --network fixtures/genesis/l2.json \
    --datadir dev_ethrex_l2_a \
    --http.addr 127.0.0.1 --http.port 1729 --authrpc.port 8551 \
    --metrics --metrics.port 3702 \
    --p2p.addr 127.0.0.1 --p2p.port 30303 --discovery.port 30303 \
    --admin-server.addr 127.0.0.1 --admin-server.port 5555 \
    --proof-coordinator.addr 127.0.0.1 --proof-coordinator.port 3900 \
    --state-updater.sequencer-registry ${ETHREX_DEPLOYER_SEQUENCER_REGISTRY_ADDRESS} \
    --l1.bridge-address ${ETHREX_WATCHER_BRIDGE_ADDRESS} \
    --l1.on-chain-proposer-address ${ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS} \
    --eth.rpc-url http://localhost:8545 \
    --watcher.block-delay 0 \
    --osaka-activation-time 1761677592 \
    --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
    --block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
    --block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
    --block-producer.operator-fee-per-gas 1000000000 \
    --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
    --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d
```

5. In a second terminal, start the second sequencer and set `--admin.start-at` to the desired number. Keep it connected to the first sequencer via `--bootnodes` and point `--admin.l2-head-check-rpc-url` to the first sequencer’s HTTP endpoint:

```bash
export $(cat cmd/.env | xargs); export COMPILE_CONTRACTS=true;
  target/release/ethrex \
    l2 --no-monitor \
    --network fixtures/genesis/l2.json \
    --datadir dev_ethrex_l2_b \
    --http.addr 127.0.0.1 --http.port 1730 --authrpc.port 8552 \
    --metrics --metrics.port 3703 \
    --p2p.addr 127.0.0.1 --p2p.port 30304 --discovery.port 30304 \
    --admin-server.addr 127.0.0.1 --admin-server.port 5556 \
    --proof-coordinator.addr 127.0.0.1 --proof-coordinator.port 3901 \
    --state-updater.sequencer-registry ${ETHREX_DEPLOYER_SEQUENCER_REGISTRY_ADDRESS} \
    --l1.bridge-address ${ETHREX_WATCHER_BRIDGE_ADDRESS} \
    --l1.on-chain-proposer-address ${ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS} \
    --eth.rpc-url http://localhost:8545 \
    --watcher.block-delay 0 \
    --osaka-activation-time 1761677592 \
    --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
    --block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
    --block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
    --block-producer.operator-fee-per-gas 1000000000 \
    --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
    --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
    --admin.start-at 10 \
    --bootnodes enode://bbdc069e0513b13e92093e0b51d75c0fa7c5dd7c6aad40ee5055ed307c0516c8e78499696c77f1bab41aaf8ec827e7d319f393705c8f7d876f1bd9462e5b94ab@127.0.0.1:30303 \
    --admin.l2-head-check-rpc-url http://localhost:1729
```

6. Finally, stop the first sequencer at the same number:

```bash
curl -X POST http://localhost:5555/state-updater/stop-at/10
```

### Handing over to a new version

The flow above runs the same version on both sequencers. When the second sequencer runs a new release:

- Prover inputs are generated at commit time for the committing version, and a proof coordinator only hands a batch to a prover of that version. A sequencer that stops sequencing also stops sending proofs. So before the handover, stop the first sequencer's committer (`GET /committer/stop`), let its prover verify every batch it committed (`lastVerifiedBatch` equals `lastCommittedBatch`), and then stop that prover. The first sequencer keeps producing blocks meanwhile.
- Register the new release's verification key before the handover, since the second sequencer cannot commit without it. When the release changes the verifier contract, as SP1 6 does, switch it at the same point: after the old batches are verified and before the new release's first proof.
- Run a prover of the new release against the second sequencer's proof coordinator. It commits the blocks the first sequencer produced after its committer stopped.
- Take the first sequencer's enode from its startup log (`Local node initialized enode=...`); the L2 RPC only serves `admin_nodeInfo` when `admin` is in its `--http.api`.
- After the handover the first sequencer stays at block `N`, so move RPC traffic to the second one before stopping the first.
