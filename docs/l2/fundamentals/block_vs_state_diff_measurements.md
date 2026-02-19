# Comparative Analysis: Transaction Volume in Blobs Using State Diffs and Transaction Lists

The following are results from measurements conducted to understand the efficiency of blob utilization in an ethrex L2 network through the simulation of different scenarios with varying transaction complexities (e.g., ETH transfers, ERC20 transfers, and other complex smart contract interactions) and data encoding strategies, with the final goal of estimating the approximate number of transactions that can be packed into a single blob using state diffs versus full transaction lists, thereby optimizing calldata costs and achieving greater scalability.

## Measurements (Amount of transactions per batch)

### ETH Transfers

| Blob Payload | Batch 2 | Batch 3 | Batch 4 | Batch 5 | Batch 6 | Batch 7 | Batch 8 | Batch 9 | Batch 10 | Batch 11 |
| ------------ | ------- | ------- | ------- | ------- | ------- | ------- | ------- | ------- | -------- | -------- |
| State Diff   |   2373  |   2134  |  2367   |   2141  |  2191   |   2370  |  2309   |  2361   |  2375    |   2367   |
| Block List   |   913   |   871   |  886    |   935   |  1019   |   994   |  1002   |  1011   |  1012    |   1015   |

### ERC20 Transfers

| Blob Payload | Batch 2 | Batch 3 | Batch 4 | Batch 5 | Batch 6 | Batch 7 | Batch 8 | Batch 9 | Batch 10 | Batch 11 |
| ------------ | ------- | ------- | ------- | ------- | ------- | ------- | ------- | ------- | -------- | -------- |
| State Diff   |  1942   |   1897  |   1890  |  1900   |   1915  |   1873  |   1791  |   1773  |   1867   |   1858   |
| Block List   |  655    |   661   |   638   |  638    |   645   |   644   |   615   |   530   |   532    |   532    |

### Summary

| Blob Payload | Avg. ETH Transfers per Batch | Avg. ERC20 Transfers per Batch |
| ------------ | ---------------------------- | ------------------------------ |
| State Diff   |          2298                |                1870            |
| Block List   |          965                 |                609             |

## Conclusion

Sending block lists in blobs instead of state diffs decreases the number of transactions that can fit in a single blob by approximately 2x for ETH transfers and 3x for ERC20 transfers.

## How these measurements were done

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
use ethrex_common::{
    Address, U256,
    types::{EIP1559Transaction, Transaction, TxKind},
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_l2_sdk::send_generic_transaction;
use ethrex_rpc::EthClient;
use tokio::time::sleep;
use url::Url;

#[tokio::main]
async fn main() {
    let chain_id = 65536999;
    let senders = vec![
        "7a738a3a8ee9cdbb5ee8dfc1fc5d97847eaba4d31fd94f89e57880f8901fa029",
        "8cfe380955165dd01f4e33a3c68f4e08881f238fbbea71a2ab407f4a3759705b",
        "5bb463c0e64039550de4f95b873397b36d76b2f1af62454bb02cf6024d1ea703",
        "3c0924743b33b5f06b056bed8170924ca12b0d52671fb85de1bb391201709aaf",
        "6aeeda1e7eda6d618de89496fce01fb6ec685c38f1c5fccaa129ec339d33ff87",
    ]
    .iter()
    .map(|s| Signer::Local(LocalSigner::new(s.parse().expect("invalid private key"))))
    .collect::<Vec<Signer>>();
    let eth_client: EthClient =
        EthClient::new(Url::parse("http://localhost:1729").expect("Invalid URL"))
            .expect("Failed to create EthClient");
    let mut nonce = 0;
    loop {
        for sender in senders.clone() {
            let signed_tx = generate_signed_transaction(nonce, chain_id, &sender).await;
            send_generic_transaction(&eth_client, signed_tx.into(), &sender)
                .await
                .expect("Failed to send transaction");
            sleep(std::time::Duration::from_millis(10)).await;
        }
        nonce += 1;
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
        let (first, last) = fetch_batch(batch).await;
        let mut txs = 0u64;
        for number in first as u64..=last as u64 {
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
        .send()
        .await
        .expect("Failed to send request")
        .json::<Value>()
        .await
        .unwrap();

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
        .send()
        .await
        .expect("Failed to send request")
        .json::<Value>()
        .await
        .unwrap();

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
use ethrex_common::{
    Address, U256,
    types::{EIP1559Transaction, GenericTransaction, Transaction, TxKind, TxType},
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_l2_sdk::{
    build_generic_tx, calldata::encode_calldata, create_deploy, send_generic_transaction,
    wait_for_transaction_receipt,
};
use ethrex_rpc::{EthClient, clients::Overrides};
use tokio::time::sleep;
use url::Url;

// ERC20 compiled artifact generated from this tutorial:
// https://medium.com/@kaishinaw/erc20-using-hardhat-a-comprehensive-guide-3211efba98d4
// If you want to modify the behaviour of the contract, edit the ERC20.sol file,
// and compile it with solc.
const ERC20: &str = include_str!("./TestToken.bin").trim_ascii();

#[tokio::main]
async fn main() {
    let chain_id = 65536999;
    let signer = Signer::Local(LocalSigner::new(
        "39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d"
            .parse()
            .expect("invalid private key"),
    ));
    let eth_client: EthClient =
        EthClient::new(Url::parse("http://localhost:1729").expect("Invalid URL"))
            .expect("Failed to create EthClient");
    let contract_address = erc20_deploy(eth_client.clone(), &signer)
        .await
        .expect("Failed to deploy ERC20 contract");

    let senders = vec![
        "7a738a3a8ee9cdbb5ee8dfc1fc5d97847eaba4d31fd94f89e57880f8901fa029",
        "8cfe380955165dd01f4e33a3c68f4e08881f238fbbea71a2ab407f4a3759705b",
        "5bb463c0e64039550de4f95b873397b36d76b2f1af62454bb02cf6024d1ea703",
        "3c0924743b33b5f06b056bed8170924ca12b0d52671fb85de1bb391201709aaf",
        "6aeeda1e7eda6d618de89496fce01fb6ec685c38f1c5fccaa129ec339d33ff87",
    ]
    .iter()
    .map(|s| Signer::Local(LocalSigner::new(s.parse().expect("invalid private key"))))
    .collect::<Vec<Signer>>();
    claim_erc20_balances(contract_address, eth_client.clone(), senders.clone())
        .await
        .expect("Failed to claim ERC20 balances");
    let mut nonce = 1;
    loop {
        for sender in senders.clone() {
            let signed_tx =
                generate_erc20_transaction(nonce, chain_id, &sender, &eth_client, contract_address)
                    .await;
            send_generic_transaction(&eth_client, signed_tx.into(), &sender)
                .await
                .expect("Failed to send transaction");
            println!(
                "Sent transaction with nonce {} for address {}",
                nonce,
                sender.address()
            );
            sleep(std::time::Duration::from_millis(10)).await;
        }
        nonce += 1;
    }
}

// Given an account vector and the erc20 contract address, claim balance for all accounts.
async fn claim_erc20_balances(
    contract_address: Address,
    client: EthClient,
    accounts: Vec<Signer>,
) -> eyre::Result<()> {
    for account in accounts {
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
        wait_for_transaction_receipt(tx_hash, &client, 1000)
            .await
            .unwrap();
    }

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

async fn generate_erc20_transaction(
    nonce: u64,
    chain_id: u64,
    signer: &Signer,
    client: &EthClient,
    contract_address: Address,
) -> GenericTransaction {
    let send_calldata = encode_calldata(
        "transfer(address,uint256)",
        &[
            ethrex_l2_common::calldata::Value::Address(Address::random()),
            ethrex_l2_common::calldata::Value::Uint(U256::one()),
        ],
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
    .await
    .unwrap();

    tx
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
ethrex-l2-common = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
ethrex-blockchain = { git = "https://github.com/lambdaclass/ethrex.git", tag = "v6.0.0" }
url = "2"
hex = "0.4"
eyre = "0.6"
```

#### `TestToken.bin`

```
608060405234801561000f575f5ffd5b506040518060400160405280600881526020017f46756e546f6b656e0000000000000000000000000000000000000000000000008152506040518060400160405280600381526020017f46554e0000000000000000000000000000000000000000000000000000000000815250816003908161008b9190610598565b50806004908161009b9190610598565b5050506100b83369d3c21bcecceda10000006100bd60201b60201c565b61077c565b5f73ffffffffffffffffffffffffffffffffffffffff168273ffffffffffffffffffffffffffffffffffffffff160361012d575f6040517fec442f0500000000000000000000000000000000000000000000000000000000815260040161012491906106a6565b60405180910390fd5b61013e5f838361014260201b60201c565b5050565b5f73ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff1603610192578060025f82825461018691906106ec565b92505081905550610260565b5f5f5f8573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205490508181101561021b578381836040517fe450d38c0000000000000000000000000000000000000000000000000000000081526004016102129392919061072e565b60405180910390fd5b8181035f5f8673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2081905550505b5f73ffffffffffffffffffffffffffffffffffffffff168273ffffffffffffffffffffffffffffffffffffffff16036102a7578060025f82825403925050819055506102f1565b805f5f8473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f82825401925050819055505b8173ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef8360405161034e9190610763565b60405180910390a3505050565b5f81519050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52604160045260245ffd5b7f4e487b71000000000000000000000000000000000000000000000000000000005f52602260045260245ffd5b5f60028204905060018216806103d657607f821691505b6020821081036103e9576103e8610392565b5b50919050565b5f819050815f5260205f209050919050565b5f6020601f8301049050919050565b5f82821b905092915050565b5f6008830261044b7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff82610410565b6104558683610410565b95508019841693508086168417925050509392505050565b5f819050919050565b5f819050919050565b5f61049961049461048f8461046d565b610476565b61046d565b9050919050565b5f819050919050565b6104b28361047f565b6104c66104be826104a0565b84845461041c565b825550505050565b5f5f905090565b6104dd6104ce565b6104e88184846104a9565b505050565b5b8181101561050b576105005f826104d5565b6001810190506104ee565b5050565b601f82111561055057610521816103ef565b61052a84610401565b81016020851015610539578190505b61054d61054585610401565b8301826104ed565b50505b505050565b5f82821c905092915050565b5f6105705f1984600802610555565b1980831691505092915050565b5f6105888383610561565b9150826002028217905092915050565b6105a18261035b565b67ffffffffffffffff8111156105ba576105b9610365565b5b6105c482546103bf565b6105cf82828561050f565b5f60209050601f831160018114610600575f84156105ee578287015190505b6105f8858261057d565b86555061065f565b601f19841661060e866103ef565b5f5b8281101561063557848901518255600182019150602085019450602081019050610610565b86831015610652578489015161064e601f891682610561565b8355505b6001600288020188555050505b505050505050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f61069082610667565b9050919050565b6106a081610686565b82525050565b5f6020820190506106b95f830184610697565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6106f68261046d565b91506107018361046d565b9250828201905080821115610719576107186106bf565b5b92915050565b6107288161046d565b82525050565b5f6060820190506107415f830186610697565b61074e602083018561071f565b61075b604083018461071f565b949350505050565b5f6020820190506107765f83018461071f565b92915050565b610e8c806107895f395ff3fe608060405234801561000f575f5ffd5b506004361061009c575f3560e01c80635b70ea9f116100645780635b70ea9f1461015a57806370a082311461016457806395d89b4114610194578063a9059cbb146101b2578063dd62ed3e146101e25761009c565b806306fdde03146100a0578063095ea7b3146100be57806318160ddd146100ee57806323b872dd1461010c578063313ce5671461013c575b5f5ffd5b6100a8610212565b6040516100b59190610b05565b60405180910390f35b6100d860048036038101906100d39190610bb6565b6102a2565b6040516100e59190610c0e565b60405180910390f35b6100f66102c4565b6040516101039190610c36565b60405180910390f35b61012660048036038101906101219190610c4f565b6102cd565b6040516101339190610c0e565b60405180910390f35b6101446102fb565b6040516101519190610cba565b60405180910390f35b610162610303565b005b61017e60048036038101906101799190610cd3565b610319565b60405161018b9190610c36565b60405180910390f35b61019c61035e565b6040516101a99190610b05565b60405180910390f35b6101cc60048036038101906101c79190610bb6565b6103ee565b6040516101d99190610c0e565b60405180910390f35b6101fc60048036038101906101f79190610cfe565b610410565b6040516102099190610c36565b60405180910390f35b60606003805461022190610d69565b80601f016020809104026020016040519081016040528092919081815260200182805461024d90610d69565b80156102985780601f1061026f57610100808354040283529160200191610298565b820191905f5260205f20905b81548152906001019060200180831161027b57829003601f168201915b5050505050905090565b5f5f6102ac610492565b90506102b9818585610499565b600191505092915050565b5f600254905090565b5f5f6102d7610492565b90506102e48582856104ab565b6102ef85858561053e565b60019150509392505050565b5f6012905090565b6103173369d3c21bcecceda100000061062e565b565b5f5f5f8373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20549050919050565b60606004805461036d90610d69565b80601f016020809104026020016040519081016040528092919081815260200182805461039990610d69565b80156103e45780601f106103bb576101008083540402835291602001916103e4565b820191905f5260205f20905b8154815290600101906020018083116103c757829003601f168201915b5050505050905090565b5f5f6103f8610492565b905061040581858561053e565b600191505092915050565b5f60015f8473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f8373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054905092915050565b5f33905090565b6104a683838360016106ad565b505050565b5f6104b68484610410565b90507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8110156105385781811015610529578281836040517ffb8f41b200000000000000000000000000000000000000000000000000000000815260040161052093929190610da8565b60405180910390fd5b61053784848484035f6106ad565b5b50505050565b5f73ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff16036105ae575f6040517f96c6fd1e0000000000000000000000000000000000000000000000000000000081526004016105a59190610ddd565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168273ffffffffffffffffffffffffffffffffffffffff160361061e575f6040517fec442f050000000000000000000000000000000000000000000000000000000081526004016106159190610ddd565b60405180910390fd5b61062983838361087c565b505050565b5f73ffffffffffffffffffffffffffffffffffffffff168273ffffffffffffffffffffffffffffffffffffffff160361069e575f6040517fec442f050000000000000000000000000000000000000000000000000000000081526004016106959190610ddd565b60405180910390fd5b6106a95f838361087c565b5050565b5f73ffffffffffffffffffffffffffffffffffffffff168473ffffffffffffffffffffffffffffffffffffffff160361071d575f6040517fe602df050000000000000000000000000000000000000000000000000000000081526004016107149190610ddd565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff160361078d575f6040517f94280d620000000000000000000000000000000000000000000000000000000081526004016107849190610ddd565b60405180910390fd5b8160015f8673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f8573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20819055508015610876578273ffffffffffffffffffffffffffffffffffffffff168473ffffffffffffffffffffffffffffffffffffffff167f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b9258460405161086d9190610c36565b60405180910390a35b50505050565b5f73ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff16036108cc578060025f8282546108c09190610e23565b9250508190555061099a565b5f5f5f8573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054905081811015610955578381836040517fe450d38c00000000000000000000000000000000000000000000000000000000815260040161094c93929190610da8565b60405180910390fd5b8181035f5f8673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2081905550505b5f73ffffffffffffffffffffffffffffffffffffffff168273ffffffffffffffffffffffffffffffffffffffff16036109e1578060025f8282540392505081905550610a2b565b805f5f8473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f82825401925050819055505b8173ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef83604051610a889190610c36565b60405180910390a3505050565b5f81519050919050565b5f82825260208201905092915050565b8281835e5f83830152505050565b5f601f19601f8301169050919050565b5f610ad782610a95565b610ae18185610a9f565b9350610af1818560208601610aaf565b610afa81610abd565b840191505092915050565b5f6020820190508181035f830152610b1d8184610acd565b905092915050565b5f5ffd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610b5282610b29565b9050919050565b610b6281610b48565b8114610b6c575f5ffd5b50565b5f81359050610b7d81610b59565b92915050565b5f819050919050565b610b9581610b83565b8114610b9f575f5ffd5b50565b5f81359050610bb081610b8c565b92915050565b5f5f60408385031215610bcc57610bcb610b25565b5b5f610bd985828601610b6f565b9250506020610bea85828601610ba2565b9150509250929050565b5f8115159050919050565b610c0881610bf4565b82525050565b5f602082019050610c215f830184610bff565b92915050565b610c3081610b83565b82525050565b5f602082019050610c495f830184610c27565b92915050565b5f5f5f60608486031215610c6657610c65610b25565b5b5f610c7386828701610b6f565b9350506020610c8486828701610b6f565b9250506040610c9586828701610ba2565b9150509250925092565b5f60ff82169050919050565b610cb481610c9f565b82525050565b5f602082019050610ccd5f830184610cab565b92915050565b5f60208284031215610ce857610ce7610b25565b5b5f610cf584828501610b6f565b91505092915050565b5f5f60408385031215610d1457610d13610b25565b5b5f610d2185828601610b6f565b9250506020610d3285828601610b6f565b9150509250929050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52602260045260245ffd5b5f6002820490506001821680610d8057607f821691505b602082108103610d9357610d92610d3c565b5b50919050565b610da281610b48565b82525050565b5f606082019050610dbb5f830186610d99565b610dc86020830185610c27565b610dd56040830184610c27565b949350505050565b5f602082019050610df05f830184610d99565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f610e2d82610b83565b9150610e3883610b83565b9250828201905080821115610e5057610e4f610df6565b5b9291505056fea2646970667358221220c2ace90351a6254148d1d6fc391d67d42f65e41f9290478674caf67a0ec34ec964736f6c634300081b0033
```
