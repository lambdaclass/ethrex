# Running the Prover

This guide provides instructions for setting up and running the Ethrex L2 prover for development and testing purposes.

## Dependencies

Before you begin, ensure you have the following dependencies installed:

- [RISC0](https://dev.risczero.com/api/zkvm/install)
  1. `curl -L https://risczero.com/install | bash`
  2. `rzup install`
- [SP1](https://docs.succinct.xyz/docs/sp1/introduction)
  1. `curl -L https://sp1up.succinct.xyz | bash`
  2. `sp1up --version 5.0.8`
- [SOLC (v0.8.31)](https://docs.soliditylang.org/en/latest/installing-solidity.html)

After installing the toolchains, a quick test can be performed to check if we have everything installed correctly.

### L1 block proving

ethrex-prover is able to generate execution proofs of Ethereum Mainnet/Testnet blocks. An example binary was created for this purpose in `crates/l2/prover/bench`. Refer to its README for usage.

## Dev Mode

To run the blockchain (`proposer`) and prover in conjunction, start the `Prover`, use the following command:

```sh
make init-prover-<sp1|risc0|exec> # optional: GPU=true
```

### Run the whole system with the prover - In one Machine

> [!NOTE]
> Used for development purposes.

1. `cd crates/l2`
2. `make rm-db-l2 && make down`
   - It will remove any old database, if present, stored in your computer. The absolute path of SQL is defined by [datadir](https://docs.rs/dirs/latest/dirs/fn.datadir.html).
3. `make init`
   - Make sure you have the `solc` compiler installed in your system.
   - Init the L1 in a docker container on port `8545`.
   - Deploy the needed contracts for the L2 on the L1.
   - Start the L2 locally on port `1729`.
4. In a new terminal &rarr; `make init-prover-<sp1|risc0|exec> # GPU=true`.

After this initialization we should have the prover running in `dev_mode` &rarr; No real proofs.

## GPU mode

**Steps for Ubuntu 22.04 with Nvidia A4000:**

1. Install `docker` &rarr; using the [Ubuntu apt repository](https://docs.docker.com/engine/install/ubuntu/#install-using-the-repository)
   - Add the `user` you are using to the `docker` group &rarr; command: `sudo usermod -aG docker $USER`. (needs reboot, doing it after CUDA installation)
   - `id -nG` after reboot to check if the user is in the group.
2. Install [Rust](https://www.rust-lang.org/tools/install)
3. Install [RISC0](https://dev.risczero.com/api/zkvm/install)
4. Install [CUDA for Ubuntu](https://developer.nvidia.com/cuda-downloads?target_os=Linux&target_arch=x86_64&Distribution=Ubuntu&target_version=22.04&target_type=deb_local)
   - Install `CUDA Toolkit Installer` first. Then the `nvidia-open` drivers.
5. Reboot
6. Run the following commands:

```sh
sudo apt-get install libssl-dev pkg-config libclang-dev clang
echo 'export PATH=/usr/local/cuda/bin:$PATH' >> ~/.bashrc
echo 'export LD_LIBRARY_PATH=/usr/local/cuda/lib64:$LD_LIBRARY_PATH' >> ~/.bashrc
```

### Run the whole system with a GPU Prover

Two separate machines are recommended for running the `Prover` and the `sequencer` to avoid resource contention. However, for development, you can run them in two separate terminals on the same machine.

- **Machine 1 (or Terminal 1)**: For the `Prover` (GPU is recommended).
- **Machine 2 (or Terminal 2)**: For the `sequencer`/L2 node.

1. **`Prover`/`zkvm` Setup**
   1. `cd ethrex/crates/l2`
   2. You can set the following environment variables to configure the prover:
      - `PROVER_CLIENT_PROVER_SERVER_ENDPOINT`: The address of the server where the client will request the proofs from.
      - `PROVER_CLIENT_PROVING_TIME_MS`: The amount of time to wait before requesting new data to prove.
   3. To start the `Prover`/`zkvm`, run:
      ```sh
      make init-prover-<sp1|risc0|exec> # optional: GPU=true
      ```

2. **`ProofCoordinator`/`sequencer` Setup**
   1. `cd ethrex/crates/l2`
   2. Create a `.env` file with the following content:
      ```env
      # Should be the same as ETHREX_COMMITTER_L1_PRIVATE_KEY and ETHREX_WATCHER_L2_PROPOSER_PRIVATE_KEY
      ETHREX_DEPLOYER_L1_PRIVATE_KEY=<private_key>
      # Should be the same as ETHREX_COMMITTER_L1_PRIVATE_KEY and ETHREX_DEPLOYER_L1_PRIVATE_KEY
      ETHREX_WATCHER_L2_PROPOSER_PRIVATE_KEY=<private_key>
      # Should be the same as ETHREX_WATCHER_L2_PROPOSER_PRIVATE_KEY and ETHREX_DEPLOYER_L1_PRIVATE_KEY
      ETHREX_COMMITTER_L1_PRIVATE_KEY=<private_key>
      # Should be different from ETHREX_COMMITTER_L1_PRIVATE_KEY and ETHREX_WATCHER_L2_PROPOSER_PRIVATE_KEY
      ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=<private_key>
      # Used to handle TCP communication with other servers from any network interface.
      ETHREX_PROOF_COORDINATOR_LISTEN_ADDRESS=0.0.0.0
      # Set to true to randomize the salt.
      ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true
      # Set to true if you want SP1 proofs to be required
      ETHREX_L2_SP1=true
      # Check the if the verification contract is present on your preferred network. Don't define this if you want it to be deployed automatically.
      ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS=<address>
      # Set to true if you want proofs to be required
      ETHREX_L2_RISC0=true
      # Check the if the contract is present on your preferred network. You shall deploy it manually if not.
      ETHREX_DEPLOYER_RISC0_VERIFIER_ADDRESS=<address>
      # Set to any L1 endpoint.
      ETHREX_ETH_RPC_URL=<url>
      ```
   3. `source .env`

> [!NOTE]
> Make sure to have funds, if you want to perform a quick test `0.2[ether]` on each account should be enough.

- `Finally`, to start the `proposer`/`l2 node`, run:
  - `make rm-db-l2 && make down`
  - `make deploy-l1 && make init-l2` (if running a risc0 prover, see the next step before invoking the L1 contract deployer)

- If running with a local L1 (for development), you will need to manually deploy the risc0 contracts by following the instructions [here](https://github.com/risc0/risc0-ethereum/tree/main/contracts/script).
- For a local L1 running with ethrex, we do the following:

  1.  clone the risc0-ethereum repo
  1.  edit the `risc0-ethereum/contracts/deployment.toml` file by adding
      ```toml
      [chains.ethrex]
      name = "Ethrex local devnet"
      id = 9
      ```
  1.  export env. variables (we are using an ethrex's rich L1 account)
      ```bash
      export VERIFIER_ESTOP_OWNER="0x4417092b70a3e5f10dc504d0947dd256b965fc62"
      export DEPLOYER_PRIVATE_KEY="0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e"
      export DEPLOYER_ADDRESS="0x4417092b70a3e5f10dc504d0947dd256b965fc62"
      export CHAIN_KEY="ethrex"
      export RPC_URL="http://localhost:8545"

      export ETHERSCAN_URL="dummy"
      export ETHERSCAN_API_KEY="dummy"
      ```
      the last two variables need to be defined with some value even if not used, else the deployment script fails.
  1.  cd into `risc0-ethereum/`
  1.  run the deployment script
      ```bash
      bash contracts/script/manage DeployEstopGroth16Verifier --broadcast
      ```
  1.  if the deployment was successful you should see the contract address in the output of the command, you will need to pass this as an argument to the L2 contract deployer, or via the `ETHREX_DEPLOYER_RISC0_VERIFIER_ADDRESS=<address>` env. variable.
      if you get an error like `risc0-ethereum/contracts/../lib/forge-std/src/Script.sol": No such file or directory (os error 2)`, try to update the git submodules (foundry dependencies) with `git submodule update --init --recursive`.

## Configuration

Configuration is done through environment variables or CLI flags.
You can see a list of available flags by passing `--help` to the CLI, or checkout [CLI](../../CLI.md).
