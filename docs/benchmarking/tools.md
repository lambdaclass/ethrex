# Server Setup

To run the benchmarking tools and the node, you need to set up a Linux machine with the following prerequisites.

## Dependencies

- **Rust:**
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  # Proceed with standard installation (1)
  ```

- **General Dependencies and Tools:**
  ```bash
  sudo apt-get update && sudo apt-get install libclang-dev clang tmux rsync linux-perf git pkg-config
  ```

- **Clone the Repository:**
  ```bash
  git clone https://github.com/lambdaclass/ethrex.git
  ```

### Optional Dependencies

#### Kurtosis & ethereum-package for localnets
Kurtosis + ethereum-package are used for running local Ethereum networks and testing environments, this is usefull to set up localnets on a single server both for benchmarking and development purposes.

- **Docker:**
  follow the [official Docker installation guide](https://docs.docker.com/engine/install/).
  After finishing the installation make sure to add the current user to the docker group to avoid sudo issues:
  ```bash
  sudo usermod -aG docker $USER
  newgrp docker
  id # should show docker group
  docker help # to verify installation
  ```

- **Kurtosis:**
  follow the [official Kurtosis installation guide](https://docs.kurtosis.com/install/).
  After finishing the installation, you can verify it by running:  
  ```bash
  kurtosis help
  ```

- **ethereum-package:**
  We have already a make command to checkout our own fork of the ethereum-package, so you don't need to install it manually.
  ```bash
  make checkout-ethereum-package
  ```


## Benchmarking Tools

Here are the primary tools we use for benchmarking.

### Gas Benchmarks

A collection of scripts to run benchmarks across multiple Ethereum clients. We have an internal and, for now, [private fork](https://github.com/lambdaclass/gas-benchmarks-private/) of the original [Gas Benchmarks Repo](https://github.com/NethermindEth/gas-benchmarks), but with Ethrex added and some convenient make tasks and usefull changes.

#### Prerequisites:
- **asdf:**
    Install [asdf](https://asdf-vm.com/guide/getting-started.html) and don't forget to [configure it](https://asdf-vm.com/guide/getting-started.html#_2-configure-asdf).
    Install plugins:
        - python: Install the [asdf python plugin](https://github.com/asdf-community/asdf-python?tab=readme-ov-file#install), and be aware of the [build dependencies](https://github.com/pyenv/pyenv/wiki#suggested-build-environment).
        - dotnet: Install the [asdf dotnet plugin](https://github.com/hensou/asdf-dotnet?tab=readme-ov-file#install), and [configure it](https://github.com/hensou/asdf-dotnet?tab=readme-ov-file#-manutally-updating-global-environment-variables).

- **Clone the repo:**
    Clone the repo and install the prerequisites:
    ```bash
    # You'll need an authorized token to access the private repo
    git clone https://github.com/lambdaclass/gas-benchmarks-private
    cd gas-benchmarks-private
    git checkout refactor/improve-report
    asdf install
    ```

- **Setup:**
    follow the setup instructions in the [README](https://github.com/lambdaclass/gas-benchmarks-private?tab=readme-ov-file#setup).
    Check it by running a quick single benchmark for ethrex:
    ```bash
    make run_single_benchmark
    ```

**Usage:**

To run the full benchmark pipeline, use the `run.sh` script or one of the new convenient make tasks:
```bash
bash run.sh -t "tests/" -w "warmup/warmup-1000bl-16wi-24tx.txt" -c "ethrex, nethermind,geth,reth" -r 8
# or for running all test against ethrex
make run_ethrex_benchmarks
# or for running all tests against all clients
make run_all_benchmarks
```

## Flood

Flood is a load testing tool for benchmarking EVM nodes over RPC. It allows you to measure performance metrics like latency and error rate under various loads.

### Prerequisites:
- **asdf:**
    Install [asdf](https://asdf-vm.com/guide/getting-started.html) and don't forget to [configure it](https://asdf-vm.com/guide/getting-started.html#_2-configure-asdf).
    Create a new .tool-versions file or modify the existing one to include the required plugins and versions (python and go are required):
    ```plaintext
    [...] # Other versions
    python 3.11.13
    go 1.23.2 # If you are using linux
    ```
    Install plugins:
        - python: Install the [asdf python plugin](https://github.com/asdf-community/asdf-python?tab=readme-ov-file#install), and be aware of the [build dependencies](https://github.com/pyenv/pyenv/wiki#suggested-build-environment).
        - dotnet: Install the [asdf go plugin](https://github.com/asdf-community/asdf-golang?tab=readme-ov-file#install), and [configure the shell](https://github.com/asdf-community/asdf-golang?tab=readme-ov-file#use).
    Then install the required versions:
    ```bash
    asdf install
    ```
- **vegeta:**
    Install vegeta following the instructions in the [Flood README](https://github.com/paradigmxyz/flood?tab=readme-ov-file#prerequisites). for linux:
    ```bash
    go install github.com/tsenart/vegeta/v12@v12.8.4
    ```

- **flood:**
    Flood is a Python package, so you can install it using pip. Make sure you have Python and pip installed:
    ```bash
    pip install paradigm-flood
    ```
    There are some issues with the python dependencies (we need to use python versions before 3.12), and we need to upgrade/downgrade some of them:
    ```bash
    pip install --upgrade pip setuptools wheel lxml_html_clean
    pip install "toolstr==0.9.7"
    pip cache purge
    ```

### Usage:

Here’s an example of a basic load test:
```bash
flood eth_getBlockByNumber node1=http://localhost:$PORT --rates 100 500 1000 5000 10000 20000 30000 40000 50000 --duration 30 --output  <TEST_DIR>
# or running inside docker
docker run --rm -it -e PORT=8545 -p 8545:8545 paradigmxyz/flood:latest flood eth_getBlockByNumber node1=http://localhost:$PORT --rates 100 500 1000 5000 10000 20000 30000 40000 50000 --duration 30 --output  <TEST_DIR>
```

To generate a report after running a test:
```bash
flood report <TEST_DIR>
```

### Spamoor

Spamoor is a powerful tool for generating various types of random transactions on Ethereum testnets. It's ideal for stress testing, network validation, or continuous transaction testing.

It can be run as a standalone command-line tool or in a daemon mode with a web UI for managing multiple spammers.

**Features:**

- **Multiple Scenarios:** Supports various transaction types, including EOA to EOA transfers, ERC20 transfers, contract deployments, and more.
- **Daemon Mode:** Provides a web interface and a REST API to create, monitor, and control spammers.
- **Metrics:** Exposes Prometheus metrics for real-time monitoring of running scenarios.

**Usage:**

To run spamoor, you can use the command-line interface:

```bash
spamoor <scenario> [flags]
```

For example, to send EOA transactions:

```bash
spamoor eoatx --privkey <PRIVATE_KEY> --rpchost <RPC_HOST>
```

For more advanced usage, you can run the spamoor daemon:

```bash
spamoor-daemon [flags]
```

This will start the web interface on `http://localhost:8080` by default.



### Ethrex

Ethrex is a tool for replaying Ethereum transactions from a given network.

#### Dependencies

- **Clone the Repo:**
  ```bash
  git clone https://github.com/lambdaclass/ethrex.git
  ```

#### Run

##### L1

###### Prereqs
- Install lighthouse as outlined [here](https://lighthouse-book.sigmaprime.io/installation.html)
- Install the `ethereum-metrics-exporter` as outlined [here](https://github.com/ethpandaops/ethereum-metrics-exporter?tab=readme-ov-file#standalone)

###### Run the node in Hoodi
- Move to tooling/sync folder and execute the appropiate make target
    ```bash
    cd ./tooling/sync && make start_hoodi_metrics
    ```
