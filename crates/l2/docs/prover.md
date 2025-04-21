# ethrex L2 Prover

## ToC

- [ethrex L2 Prover](#ethrex-l2-prover)
  - [ToC](#toc)
  - [What](#what)
  - [Workflow](#workflow)
  - [How](#how)
    - [Quick Test](#quick-test)
    - [Dev Mode](#dev-mode)
      - [Run the whole system with the prover - In one Machine](#run-the-whole-system-with-the-prover---in-one-machine)
    - [GPU mode](#gpu-mode)
      - [Proving Process Test](#proving-process-test)
      - [Run the whole system with a GPU Prover](#run-the-whole-system-with-a-gpu-prover)
  - [Configuration](#configuration)

> [!NOTE]
> The shipping/deploying process and the `Prover` itself are under development.

## Usage

### Dependencies

- [RISC0](https://dev.risczero.com/api/zkvm/install)
  1. `curl -L https://risczero.com/install | bash`
  2. `rzup install cargo-risczero 1.2.0`
- [SP1](https://docs.succinct.xyz/docs/sp1/introduction)
  1. `curl -L https://sp1up.succinct.xyz | bash`
  2. `sp1up --version 4.1.0`
- [Pico](https://docs.brevis.network/)
  1. `cargo +nightly install --git https://github.com/brevis-network/pico pico-cli`
  2. `rustup install nightly-2024-11-27`
  3. `rustup component add rust-src --toolchain nightly-2024-11-27`
- [SOLC](https://docs.soliditylang.org/en/latest/installing-solidity.html)

After installing the toolchains, a quick test can be performed to check if we have everything installed correctly.

### Test

To test the `zkvm` execution quickly, the following test can be run:

```sh
cd crates/l2/prover
```

Then run any of the targets:

- `make perf-pico`
- `make perf-risc0`
- `make perf-sp1`

### Dev Mode

To run the blockchain (`proposer`) and prover in conjunction, start the `prover_client`, use the following command:

```sh
make init-prover T="prover_type (pico,risc0,sp1) G=true"
```

#### Run the whole system with the prover - In one Machine

> [!NOTE]
> Used for development purposes.

1. `cd crates/l2`
2. `make rm-db-l2 && make down`
   - It will remove any old database, if present, stored in your computer. The absolute path of libmdbx is defined by [data_dir](https://docs.rs/dirs/latest/dirs/fn.data_dir.html).
3. `cp configs/sequencer_config_example.toml configs/sequencer_config.toml` &rarr; check if you want to change any config.
4. `cp configs/prover_client_config_example.toml configs/prover_client_config.toml` &rarr; check if you want to change any config.
5. `make init`
   - Make sure you have the `solc` compiler installed in your system.
   - Init the L1 in a docker container on port `8545`.
   - Deploy the needed contracts for the L2 on the L1.
   - Start the L2 locally on port `1729`.
6. In a new terminal &rarr; `make init-prover T=(sp1,risc0,pico)`.

After this initialization we should have the prover running in `dev_mode` &rarr; No real proofs.

### GPU mode

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

#### Proving Process Test

To test the `zkvm` proving process using a `gpu` quickly, the following test can be run:

```sh
cd crates/l2/prover
```

Then run any of the targets:

- `make perf-pico-gpu`
- `make perf-risc0-gpu`
- `make perf-sp1-gpu`

#### Run the whole system with a GPU Prover

Two servers are required: one for the `prover` and another for the `proposer`. If you run both components on the same machine, the `prover` may consume all available resources, leading to potential stuttering or performance issues for the `proposer`/`node`.

- The number 1 simbolizes a machine with GPU for the `prover_client`.
- The number 2 simbolizes a machine for the `sequencer`/L2 node itself.

1. `prover_client`/`zkvm` &rarr; prover with gpu, make sure to have all the required dependencies described at the beginning of [Gpu Mode](#gpu-mode) section.
   1. `cd ethrex/crates/l2`
   2. `cp configs/prover_client_config_example.toml configs/prover_client_config.toml` and change the `prover_server_endpoint` with machine's `2` ip and make sure the port matches the one defined in machine 2.

The important variables are:

```sh
[prover_client]
prover_server_endpoint=<ip-address>:3900
```

- `Finally`, to start the `prover_client`/`zkvm`, run:
  - `make init-prover T=(sp1,risc0,pico) G=true`

2. `prover_server`/`proposer` &rarr; this server just needs rust installed.
   1. `cd ethrex/crates/l2`
   2. `cp configs/sequencer_config_example.toml configs/sequencer_config.toml` and change the addresses and the following fields:
      - [prover_server]
        - `listen_ip=0.0.0.0` &rarr; Used to handle TCP communication with other servers from any network interface.
      - The `COMMITTER` and `PROVER_SERVER_VERIFIER` must be different accounts, the `DEPLOYER_ADDRESS` as well as the `L1_WATCHER` may be the same account used by the `COMMITTER`.
      - [deployer]
        - `salt_is_zero=false` &rarr; set to false to randomize the salt.
      - `sp1_deploy_verifier = true` overwrites `sp1_contract_verifier`. Check if the contract is deployed in your preferred network or set to `true` to deploy it.
      - `risc0_contract_verifier`
        - Check the if the contract is present on your preferred network.
      - `sp1_contract_verifier`
        - It can be deployed.
        - Check the if the contract is present on your preferred network.
      - `pico_contract_verifier`
        - It can be deployed.
        - Check the if the contract is present on your preferred network.
      - Set the [eth] `rpc_url` to any L1 endpoint.

> [!NOTE]
> Make sure to have funds, if you want to perform a quick test `0.2[ether]` on each account should be enough.

- `Finally`, to start the `proposer`/`l2 node`, run:
  - `make rm-db-l2 && make down`
  - `make deploy-l1 && make init-l2`

## Configuration

Configuration is done through environment variables. The easiest way to configure the ProverClient is by creating a `prover_client_config.toml` file and setting the variables there. Then, at start, it will read the file and set the variables.

The following environment variables are available to configure the Proposer consider looking at the provided [prover_client_config_example.toml](../configs/prover_client_config_example.toml):

The following environment variables are used by the ProverClient:

- `CONFIGS_PATH`: The path where the `PROVER_CLIENT_CONFIG_FILE` is located at.
- `PROVER_CLIENT_CONFIG_FILE`: The `.toml` that contains the config for the `prover_client`.
- `PROVER_ENV_FILE`: The name of the `.env` that has the parsed `.toml` configuration.
- `PROVER_CLIENT_PROVER_SERVER_ENDPOINT`: Prover Server's Endpoint used to connect the Client to the Server.

The following environment variables are used by the ProverServer:

- `PROVER_SERVER_LISTEN_IP`: IP used to start the Server.
- `PROVER_SERVER_LISTEN_PORT`: Port used to start the Server.
- `PROVER_SERVER_VERIFIER_ADDRESS`: The address of the account that sends the zkProofs on-chain and interacts with the `OnChainProposer` `verify()` function.
- `PROVER_SERVER_VERIFIER_PRIVATE_KEY`: The private key of the account that sends the zkProofs on-chain and interacts with the `OnChainProposer` `verify()` function.

> [!NOTE]
> The `PROVER_SERVER_VERIFIER` account must differ from the `COMMITTER_L1` account.

## How it works

The prover's sole purpose is to generate a block execution proof. For this, ethrex-prover implements a block execution program and generates a proof of it with different RISC-V zkVMs. 

### Program inputs

The inputs for the block execution program (also called program inputs or prover inputs) are:
- the block to prove (header and body)
- the block's parent header
- an execution witness
and the L2 specific ones are:
- the block's deposits hash
- the block's withdrawals Merkle root
- the block's state diff hash

#### Execution witness
the purpose of the execution witness is to allow executing a block without having access to the whole Ethereum state, as it can't fit in a zkVM program. So it contains only each state value needed during the execution.

an execution witness (represented by the `ExecutionDB` type) contains:
1. all the initial state values that will be read or written to during the block's execution (accounts, code, storage and block hashes).
2. Merkle Patricia Trie (MPT) proofs that prove the inclusion or exclusion of each initial value in the initial world trie.

an execution witness is created from a past execution of the block, meaning that before proving we need to:
1. execute the block (also called "pre-execution").
2. log every initial state value that is accessed or updated during this execution.
3. store each logged value in an in-memory kv database (we just use hash maps as fields for the `ExecutionDB`).
4. retrieve a proof for each value, each proof links a value (or it's non-existence) into a trie root hash.

the first three steps are straightforward, step 4 contains more complex logic given the problem introduced in section [Final state validation](#Final state validation).

if during block execution a value is removed (which means that it existed in the initial state but doesn't in the final) then the leaf node containing this value will be removed, and there is two pathological cases where we don't have sufficient information to complete the removal:

**Case 1**
![](img/execw_case1.png)

Here only the value contained in **leaf 1** is part of the execution witness, and so we don't have a proof for **leaf 2**, therefore we don't have that node. After removing **leaf 1** then there's no use to the **branch 1** node, so during the reestructuring step it's removed and replaced by **leaf 3**, which contains the value and path of **leaf 2**, but adding a nibble prefix to the path, which will  be the index of the choice of **branch 1**.

```
branch1 = {c_1, c_2, c_3, .., c_k, .., c_16} 
  where c_i is empty for i != k
  and c_k = hash(leaf2)
leaf2 = {value, path}
leaf3 = {value, concat(k, path)}
```

Because we don't have **leaf 2** we can't know how to construct **leaf 3** to complete the reestructure. The solution here is simple: we will fetch the *final* state proof for the key of **leaf 2**, which will yield an exclusion proof containing **leaf 3**. Then we can just remove the prefix `k` and voilá, we have **leaf 2**.

This situation might be repeated in an upper level of the trie, in which case the final proof leaf has many choice nibbles appended as prefix, we can't be sure how many nibbles there are so we can store each possible leaf variant by removing one nibble at a time, until `path = empty`

**Case 2**
![](img/execw_case2.png)
In this case we need **branch/ext 2** which can be both of type branch or extension. Here one could say that by checking the final **extension** node we can deduce **branch/ext 2**, which is correct in the simple case presented, but doesn't work if we have the same situation in an upper level of the trie and more removals.

The solution is to fetch the missing node using a `debug` RPC-API method, like `debug_dbGet` or `debug_accountRange` and `debug_storageRangeAt` if using a geth node.

### Block execution program
The program leverages ethrex-common primitives and ethrex-vm methods. ethrex-prover just takes the already existing execution logic and creates a proof of it via a zkVM. 

Some L2 specific logic is added on top of basic block execution. 

#### State trie basics
<TODO add link to a MPT resource>
for each executed block there is an initial and final state (the Ethereum state before and after execution respectively). State values are stored in MPTs, specifically:
1. for each account there's a Storage Trie that contains all its storage values.
2. the World State Trie contains all accounts info, which include their storage root hash (so we can link a storage trie for each account in the world trie).

the way to identify a particular Ethereum state is to hash the root node of the world state trie. This hash is known as "state hash" as it summarizes a state. 

There are two kinds of MPT proofs:
1. inclusion proofs prove that `k: v` is a valid entry in the MPT with root hash `h`.
2. exclusion proofs proof that `k` is not a valid key in the MPT with root hash `h`.  
this way we can verify that some value is a valid value in a state, or that a key doesn't exists in a state.

#### Initial state validation
We must validate that an `ExecutionDB` contains valid data by iterating over each state value and verifying each proof, knowing beforehand the initial state hash. We also must validate that, after execution, the final state values are all correct. This is done in a different way than for the initial state.

Having the initial state proofs (paths from the root to each relevant leaf) is equivalent to having a subset of the world state trie and storage tries: the set of "pruned tries" that are relevant to this particular execution. This means we can operate over these pruned tries (add, remove, modify values).

#### Final state validation
During execution, state values are updated (modified, created or removed). After executing a block we can obtain a list of all state updates. Then the final state is calculated by applying state updates to the initial state.

We can apply state updates to the relevant pruned tries, which will result in a new world state root node. By hashing it we retrieve the final state hash. After this we can check that this calculated hash is the one we expected, thus validating the final state.

There is a problem in this process: in the case of removed values we may not have the necessary information for updating the pruned tries, because the removal may invoke a subtrie reestructure for which we need nodes beyond the ones included in our set.