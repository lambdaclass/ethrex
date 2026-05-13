## Syncing with Holesky

### Step 1: Set up a jwt secret for both clients

As an example, we put the secret in a `secrets` directory in the home folder.

```bash
mkdir -p ~/secrets
openssl rand -hex 32 | tr -d "\n" | tee ~/secrets/jwt.hex
```

We will pass this new file’s path as an argument for both clients.

### Step 2: Launch Ethrex

Pass holesky as a network and the jwt secret we set in the previous step.
This will launch the node in full sync mode, in order to test out snap sync you can add the flag `--syncmode snap`.

```bash
cargo run --release --bin ethrex -- --http.addr 0.0.0.0 --network holesky --authrpc.jwtsecret ~/secrets/jwt.hex
```

### Step 3: Set up a Consensus Node

For this quick tutorial we will be using lighthouse, but you can learn how to install and run any consensus node by reading their documentation.

You can choose your preferred installation method from [lighthouse's installation guide](https://lighthouse-book.sigmaprime.io/installation.html) and then run the following command to launch the node and sync it from a public endpoint

```bash
lighthouse bn --network holesky --execution-endpoint http://localhost:8551 --execution-jwt ~/secrets/jwt.hex --http --checkpoint-sync-url https://checkpoint-sync.holesky.ethpandaops.io
```

When using lighthouse directly from its repository, replace `lighthouse bn` with `cargo run --bin lighthouse -- bn`

Aside from holesky, these steps can also be used to connect to other supported networks by replacing the `--network` argument by another supported network and looking up a checkpoint sync endpoint for that network [in this community-maintained list](https://eth-clients.github.io/checkpoint-sync-endpoints/)

If you have a running execution node that you want to connect to your ethrex node you can do so by passing its enode as a bootnode using the `--bootnodes` flag

Once the node is up and running you will be able to see logs indicating the start of each sync cycle along with from which block hash to which block hash we are syncing. You will also get regular logs with the completion rate and estimated finish time for state sync and state rebuild processes during snap sync. This will look something like this:

```bash
INFO ethrex_p2p::sync: Syncing from current head 0xb5f7…bde4 to sync_head 0xce96…fa5e
INFO ethrex_p2p::sync::state_sync: Downloading state trie, completion rate: 68%, estimated time to finish: 1h20m14s
INFO ethrex_p2p::sync::trie_rebuild: State Trie Rebuild Progress: 68%, estimated time to finish: 1h5m45s
```

If you want to restart the sync from the very start you can do so by wiping the database using the following command:

```bash
cargo run --bin ethrex -- removedb
```

