# Ethrex L1

Ethrex is a minimalist Ethereum Rust execution client.

To quickly get started, visit [our quick-start page](./quick-start-l1.md).

If you're looking to experiment with the codebase, ["L1 dev setup" will be of help](./dev-setup-l1.md).

## Sync (i.e. snap sync) and run the node to produce blocks with Hoodi network

### Prerequisite

* lighthouse. ([lighthouse's installation guide](https://lighthouse-book.sigmaprime.io/installation.html)).
* openssl

### Steps

* **Setup secrets:** Put the secret in a `secrets` directory in the home folder:

```bash
mkdir -p ~/secrets
openssl rand -hex 32 | tr -d "\n" | tee ~/secrets/jwt.hex
```

We will pass this new file’s path as an argument for both clients.

* **Run lighthouse:** In a console, run `lighthouse` to sync with Hoodi:

```bash
./lighthouse bn --network hoodi --execution-endpoint http://localhost:8551 --execution-jwt ~/secrets/jwt.hex --http --checkpoint-sync-url https://hoodi-checkpoint-sync.attestant.io/ --purge-db
```

* **Run ethrex:** In a different console, having the binary executable in the same directory, run the following command:

In linux:

```bash
ulimit -n 65000 && rm -rf ~/.local/share/ethrex && RUST_LOG=ethrex_p2p::rlpx::eth::blocks=off,ethrex_p2p::sync=debug,ethrex_p2p::network=info,ethrex_p2p::discv4=off,spawned_concurrency::tasks::gen_server=off ./ethrex --http.addr 0.0.0.0 --network hoodi --authrpc.jwtsecret ~/secrets/jwt.hex --syncmode snap 2>&1 | tee output.log
```

In macOS:

```sh
ulimit -n 65000 && rm -rf ~/Library/Application\ Support/ethrex && RUST_LOG=ethrex_p2p::rlpx::eth::blocks=off,ethrex_p2p::sync=debug,ethrex_p2p::network=info,ethrex_p2p::discv4=off,spawned_concurrency::tasks::gen_server=off ./ethrex --http.addr 0.0.0.0 --network hoodi --authrpc.jwtsecret ~/secrets/jwt.hex --syncmode snap 2>&1 | tee output.log
```
