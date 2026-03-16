# Deploying an ethrex L2 with shared bridge enabled

In this section, we'll cover how to deploy two ethrex L2 with shared bridge enabled on a devnet.

## Prerequisites

This guide assumes that you have the ethrex repository cloned.

## Steps

### Change directory

Every command should be run under `crates/l2`

```bash
cd crates/l2
```

### Start an L1

```bash
make init-l1
```

### Deploy the first L2

On another terminal

```bash
ETHREX_SHARED_BRIDGE_DEPLOY_ROUTER=true make deploy-l1
```

### Start the first L2

Replace `L1_BRIDGE_ADDRESS`, `L1_ON_CHAIN_PROPOSER_ADDRESS` and `ROUTER_ADDRESS` with the outputs of the previous command, you can also check it under `cmd/.env`.

```bash
../../target/release/ethrex \
	l2 \
	--watcher.block-delay 0 \
	--network ../../fixtures/genesis/l2.json \
	--http.port 1729 \
	--http.addr 0.0.0.0 \
	--metrics \
	--metrics.port 3702 \
	--datadir dev_ethrex_l2 \
	--l1.bridge-address <L1_BRIDGE_ADDRESS> \
	--l1.on-chain-proposer-address <L1_ON_CHAIN_PROPOSER_ADDRESS> \
	--eth.rpc-url http://localhost:8545 \
	--osaka-activation-time 1761677592 \
	--block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
	--block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
	--block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
	--block-producer.operator-fee-per-gas 1000000000 \
	--committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
	--proof-coordinator.addr 127.0.0.1 \
    --l1.router-address <ROUTER_ADDRESS> \
    --watcher.l2-rpcs http://localhost:1730 \
    --watcher.l2-chain-ids 1730
```

### Deploy the second L2

On another terminal

Copy the `../../fixtures/genesis/l2.json` file to `../../fixtures/genesis/l2_2.json` and modify chain id to 1730

Replace `ROUTER_ADDRESS` with the outputs of the first deploy

```bash
../../target/release/ethrex l2 deploy \
	--eth-rpc-url http://localhost:8545 \
	--private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--on-chain-proposer-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
	--bridge-owner 0x4417092b70a3e5f10dc504d0947dd256b965fc62 \
	--deposit-rich \
	--private-keys-file-path ../../fixtures/keys/private_keys_l1.txt \
	--genesis-l1-path ../../fixtures/genesis/l1.json \
	--genesis-l2-path ../../fixtures/genesis/l2_2.json \
    --randomize-contract-deployment \
    --router.address <ROUTER_ADDRESS>
```


### Start the second L2

Replace `L1_BRIDGE_ADDRESS` and `L1_ON_CHAIN_PROPOSER_ADDRESS` with the outputs of the previous command, you can also check it under `cmd/.env`.
And `ROUTER_ADDRESS` with the outputs of the first deploy


```bash
../../target/release/ethrex \
	l2 \
	--watcher.block-delay 0 \
	--network ../../fixtures/genesis/l2_2.json \
	--http.port 1730 \
	--http.addr 0.0.0.0 \
	--metrics \
	--metrics.port 3703 \
	--datadir dev_ethrex_l2_2 \
	--l1.bridge-address <L1_BRIDGE_ADDRESS> \
	--l1.on-chain-proposer-address <L1_ON_CHAIN_PROPOSER_ADDRESS> \
	--eth.rpc-url http://localhost:8545 \
	--osaka-activation-time 1761677592 \
	--block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d \
	--block-producer.base-fee-vault-address 0x000c0d6b7c4516a5b274c51ea331a9410fe69127 \
	--block-producer.operator-fee-vault-address 0xd5d2a85751b6F158e5b9B8cD509206A865672362 \
	--block-producer.operator-fee-per-gas 1000000000 \
	--committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924 \
	--proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d \
	--proof-coordinator.addr 127.0.0.1 \
    --proof-coordinator.port 3901 \
    --l1.router-address <ROUTER_ADDRESS> \
    --watcher.l2-rpcs http://localhost:1729 \
    --watcher.l2-chain-ids 65536999
```


### Start the prover

On another terminal 

```bash
../../target/release/ethrex \
	l2 prover \
	--proof-coordinators tcp://127.0.0.1:3900 tcp://127.0.0.1:3901 \
	--backend exec
```
