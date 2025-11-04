# Comparative Analysis: Transaction Volume in Blobs Using State Diffs and Transaction Lists

The following are results from measurements conducted to understand the efficiency of blob utilization in an ethrex L2 network through the simulation of different scenarios with varying transaction complexities (e.g., ETH transfers, ERC20 transfers, and other complex smart contract interactions) and data encoding strategies, with the final goal of estimating the approximate number of transactions that can be packed into a single blob using state diffs versus full transaction lists, thereby optimizing calldata costs and achieving greater scalability.

## Measurements (Amount of transactions per batch)

### ETH Transfers

| Blob Payload | Batch 2 | Batch 3 | Batch 4 | Batch 5 | Batch 6 | Batch 7 | Batch 8 | Batch 9 | Batch 10 | Batch 11 |
| ------------ | ------- | ------- | ------- | ------- | ------- | ------- | ------- | ------- | -------- | -------- |
| State Diff   | 2030    | 2335    | 2361    | 2267    | 2204    | 2135    | 2215    | 2172    | 2321     | 2352     |
| Block List   | 741     | 936     | 893     | 891     | 896     | 896     | 901     | 893     | 1015     | 1019     |

### ERC20 Transfers

| Blob Payload | Batch 2 | Batch 3 | Batch 4 | Batch 5 | Batch 6 | Batch 7 | Batch 8 | Batch 9 | Batch 10 | Batch 11 |
| ------------ | ------- | ------- | ------- | ------- | ------- | ------- | ------- | ------- | -------- | -------- |
| State Diff   | 1846    | 1835    | 1869    | 1905    | 1910    | 1819    | 1897    | 1895    | 1908     | 1758     |
| Block List   | 636     | 649     | 611     | 611     | 644     | 540     | 503     | 508     | 504      | 505      |

### Summary

| Blob Payload | Avg. ETH Transfers per Batch | Avg. ERC20 Transfers per Batch |
| ------------ | ---------------------------- | ------------------------------ |
| State Diff   | 2239                         | 1864                           |
| Block List   | 908                          | 571                            |

## Conclusion

Sending block lists in blobs instead of state diffs decreases the number of transactions that can fit in a single blob by approximately 2.47x for ETH transfers and 3.26x for ERC20 transfers.

## How this measurements were done

### Prerequisites

- Fresh cloned ethrex repository
- The spammer and measurer code provided in the appendix set up for running (you can create a new cargo project and copy the code there)

### Steps

#### 1. Run an L2 ethrex:

For running the measurements, we need to run an ethrex L2 node. For doing that, change your current directory to `ethrex/crates/l2` in your fresh-cloned ethrex and run the following in a terminal:

```shell
ETHREX_COMMITTER_COMMIT_TIME=120000 MEMPOOL_MAX_SIZE=1000000 make init-l2-dev
```

This will set up and run an ethrex L2 node in dev mode with a mempool size big-enough to be able to handle the spammer transactions. And after this you should see the ethrex L2 monitor running.

#### 2. Run the desired transactions spammer

> [!IMPORTANT]
> Wait a few seconds after running the L2 node to make sure it's fully up and running before starting the spammer, and to ensure that the rich account used by the spammer has funds.

In another terminal, change your current directory to the spammer code you want to run (either ETH or ERC20) and run:

```shell
cargo run
```

It's ok not to see any logs or prints as output, since the spammer code doesn't print anything.

If you go back to the terminal where the L2 node is running, you should start seeing the following:

1. The mempool table growing in size as transactions are being sent to the L2 node.
2. In the L2 Blocks table, new blocks with `#Txs` greater than 0 being created as the spammer transactions are included in blocks.
3. Every 2 minutes (or the time you set in `ETHREX_COMMITTER_COMMIT_TIME`), new batches being created in the L2 Batches table.

#### 3. Run the measurer

> [!IMPORTANT]
>
> - Wait until enough batches are created before running the measurer.
> - Ignore the results of the first 2/3 batches, since they contain other transactions created during the L2 node initialization.

In another terminal, change your current directory to the measurer code and run:

```shell
cargo run
```

This will start printing the total number of transactions included in each batch until the last committed one.

> [!NOTE]
>
> - The measurer will query batches starting from batch 1 and will continue indefinitely until it fails to find a batch (e.g. because the L2 node hasn't created it yet), so it is ok to see an error at the end of the output once the measurer reaches a batch that hasn't been created yet.

## Appendix

- [ETH Transactions Spammer](#eth-transactions-spammer)
  - [`main.rs`](#mainrs)
  - [`Cargo.toml`](#cargotoml)
- [Measurer](#measurer)
  - [`main.rs`](#mainrs-1)
  - [`Cargo.toml`](#cargotoml-1)
- [ERC20 Transactions Spammer](#erc20-transactions-spammer)
  - [`main.rs`](#mainrs-2)
  - [`Cargo.toml`](#cargotoml-2)

### ETH Transactions Spammer

> [!NOTE]
> This is using ethrex v6.0.0

#### `main.rs`

```rs
use ethrex_common::{Address, U256, types::{EIP1559Transaction, Transaction, TxKind}};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_l2_sdk::send_generic_transaction;
use ethrex_rpc::EthClient;
use tokio::time::sleep;
use url::Url;

#[tokio::main]
async fn main() {
    let chain_id = 65536999;
    let signer = Signer::Local(LocalSigner::new(
        "39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d"
            .parse()
            .expect("invalid private key"),
    ));
    let eth_client: EthClient = EthClient::new(Url::parse("http://localhost:1729").expect("Invalid URL")).expect("Failed to create EthClient");
    let mut nonce = 0;
    loop {
        let signed_tx = generate_signed_transaction(nonce, chain_id, &signer).await;
        send_generic_transaction(&eth_client, signed_tx.into(), &signer).await.expect("Failed to send transaction");
        nonce += 1;
        sleep(std::time::Duration::from_millis(10)).await;
    }

}

async fn generate_signed_transaction(nonce: u64, chain_id: u64, signer: &Signer) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        value: U256::one(),
        gas_limit: 250000,
        max_fee_per_gas: u64::MAX,
        max_priority_fee_per_gas: 10,
        chain_id,
        to: TxKind::Call(Address::random()),
        ..Default::default()
    })
    .sign(&signer)
    .await
    .expect("failed to sign transaction")
}
```

#### `Cargo.toml`

```toml
[package]
name = "tx_spammer"
version = "0.1.0"
edition = "2024"

[dependencies]
ethrex-sdk = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
ethrex-common = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
ethrex-l2-rpc = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
ethrex-rpc = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }

tokio = { version = "1", features = ["full"] }
url = "2"
hex = "0.4"
```

### Measurer

A simple program that queries the L2 node for batches and blocks, counting the number of transactions in each block, and summing them up per batch.

#### `main.rs`

```rs
use reqwest::Client;
use serde_json::{Value, json};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let mut batch = 1;

    loop {
        let (first,last) = fetch_batch(batch).await;
        let mut txs = 0u64;
        for number in first as u64 ..= last as u64 {
            txs += fetch_block(number).await;
        }
        println!("Total transactions in batch {}: {}", batch, txs);

        batch += 1;
    }
}

async fn fetch_batch(number: u64) -> (i64, i64) {
    // Create the JSON body equivalent to the --data in curl
    let body = json!({
        "method": "ethrex_getBatchByNumber",
        "params": [format!("0x{:x}", number), false],
        "id": 1,
        "jsonrpc": "2.0"
    });

    // Create a blocking HTTP client
    let client = Client::new();

    // Send the POST request
    let response = client
        .post("http://localhost:1729")
        .header("Content-Type", "application/json")
        .json(&body)
        .send().await.expect("Failed to send request")
        .json::<Value>().await.unwrap();

    let result = &response["result"];
    let first_block = &result["first_block"].as_i64().unwrap();
    let last_block = &result["last_block"].as_i64().unwrap();
    (*first_block, *last_block)
}

async fn fetch_block(number: u64) -> u64 {
    // Create the JSON body equivalent to the --data in curl
    let body = json!({
        "method": "eth_getBlockByNumber",
        "params": [format!("0x{:x}", number), false],
        "id": 1,
        "jsonrpc": "2.0"
    });

    // Create a blocking HTTP client
    let client = Client::new();

    // Send the POST request
    let response = client
        .post("http://localhost:1729")
        .header("Content-Type", "application/json")
        .json(&body)
        .send().await.expect("Failed to send request")
        .json::<Value>().await.unwrap();

    let result = &response["result"];
    let transactions = &result["transactions"];
    transactions.as_array().unwrap().len() as u64
}
```

#### `Cargo.toml`

```toml
[package]
name = "measurer"
version = "0.1.0"
edition = "2024"

[dependencies]
reqwest = { version = "0.11", features = ["json"] }
serde_json = "1.0"
tokio = { version = "1", features = ["full"] }
```

### ERC20 Transactions Spammer

#### `main.rs`

```rs
use ethrex_blockchain::constants::TX_GAS_COST;
use ethrex_common::{Address, U256, types::{EIP1559Transaction, GenericTransaction, Transaction, TxKind, TxType}};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_l2_sdk::{build_generic_tx, calldata::encode_calldata, create_deploy, send_generic_transaction, wait_for_transaction_receipt};
use ethrex_rpc::{EthClient, clients::Overrides};
use tokio::time::sleep;
use url::Url;

// ERC20 compiled artifact generated from this tutorial:
// https://medium.com/@kaishinaw/erc20-using-hardhat-a-comprehensive-guide-3211efba98d4
// If you want to modify the behaviour of the contract, edit the ERC20.sol file,
// and compile it with solc.
const ERC20: &str =
    include_str!("./TestToken.bin").trim_ascii();

#[tokio::main]
async fn main() {
    let chain_id = 65536999;
    let signer = Signer::Local(LocalSigner::new(
        "39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d"
            .parse()
            .expect("invalid private key"),
    ));
    let eth_client: EthClient = EthClient::new(Url::parse("http://localhost:1729").expect("Invalid URL")).expect("Failed to create EthClient");
    let mut nonce = 2;
    let contract_address = erc20_deploy(eth_client.clone(), &signer)
        .await
        .expect("Failed to deploy ERC20 contract");
    claim_erc20_balances(contract_address, eth_client.clone(), &signer)
        .await
        .expect("Failed to claim ERC20 balances");
    loop {
        let signed_tx = generate_erc20_transaction(nonce, chain_id, &signer, &eth_client, contract_address).await;
        send_generic_transaction(&eth_client, signed_tx.into(), &signer).await.expect("Failed to send transaction");
        nonce += 1;
        sleep(std::time::Duration::from_millis(10)).await;
    }

}

// Given an account vector and the erc20 contract address, claim balance for all accounts.
async fn claim_erc20_balances(
    contract_address: Address,
    client: EthClient,
    account: &Signer,
) -> eyre::Result<()> {

    let claim_balance_calldata = encode_calldata("freeMint()", &[]).unwrap();

    let claim_tx = build_generic_tx(
        &client,
        TxType::EIP1559,
        contract_address,
        account.address(),
        claim_balance_calldata.into(),
        Default::default(),
    )
    .await
    .unwrap();
    let tx_hash = send_generic_transaction(&client, claim_tx, &account)
        .await
        .unwrap();
    wait_for_transaction_receipt(tx_hash, &client, 1000).await.unwrap();
    Ok(())
}

async fn deploy_contract(
    client: EthClient,
    deployer: &Signer,
    contract: Vec<u8>,
) -> eyre::Result<Address> {
    let (_, contract_address) =
        create_deploy(&client, deployer, contract.into(), Overrides::default()).await?;

    eyre::Ok(contract_address)
}

async fn erc20_deploy(client: EthClient, deployer: &Signer) -> eyre::Result<Address> {
    let erc20_bytecode = hex::decode(ERC20).expect("Failed to decode ERC20 bytecode");
    deploy_contract(client, deployer, erc20_bytecode).await
}

async fn generate_erc20_transaction(nonce: u64, chain_id: u64, signer: &Signer, client: &EthClient,contract_address: Address) -> GenericTransaction {
    let send_calldata = encode_calldata(
                    "transfer(address,uint256)",
                    &[ethrex_l2_common::calldata::Value::Address(Address::random()), ethrex_l2_common::calldata::Value::Uint(U256::one())],
                )
                .unwrap();

    let tx = build_generic_tx(
                    client,
                    TxType::EIP1559,
                    contract_address,
                    signer.address(),
                    send_calldata.into(),
                    Overrides {
                        chain_id: Some(chain_id),
                        value: None,
                        nonce: Some(nonce),
                        max_fee_per_gas: Some(i64::MAX as u64),
                        max_priority_fee_per_gas: Some(10_u64),
                        gas_limit: Some(TX_GAS_COST * 100),
                        ..Default::default()
                    },
                )
                .await.unwrap();

    tx
}

async fn generate_signed_transaction(nonce: u64, chain_id: u64, signer: &Signer) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        value: U256::one(),
        gas_limit: 250000,
        max_fee_per_gas: u64::MAX,
        max_priority_fee_per_gas: 10,
        chain_id,
        to: TxKind::Call(Address::random()),
        ..Default::default()
    })
    .sign(&signer)
    .await
    .expect("failed to sign transaction")
}
```

##### `Cargo.toml`

```toml
[package]
name = "tx_spammer"
version = "0.1.0"
edition = "2024"

[dependencies]
ethrex-sdk = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
ethrex-common = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
ethrex-l2-rpc = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
ethrex-rpc = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
tokio = { version = "1", features = ["full"] }
ethrex-l2-common = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
ethrex-blockchain = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
url = "2"
hex = "0.4"
eyre = "0.6"
```
