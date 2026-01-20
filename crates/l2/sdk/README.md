# ethrex-sdk

Developer SDK for interacting with ethrex L2 rollup.

## Overview

This crate provides a high-level SDK for building applications on ethrex L2. It includes utilities for bridge operations (deposits, withdrawals), contract deployment, transaction building, and L1/L2 message passing.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        ethrex-sdk                                │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │    Bridge       │  │    Contract     │  │   Transaction   │ │
│  │   Operations    │  │   Deployment    │  │    Building     │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐ │
│  │    Calldata     │  │    L1 ↔ L2     │  │   Fee Token     │ │
│  │    Encoding     │  │   Messages      │  │   Operations    │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
┌─────────────────┐ ┌─────────────────┐ ┌─────────────────────────┐
│   Ethereum L1   │ │    ethrex L2    │ │   Bridge Contracts      │
└─────────────────┘ └─────────────────┘ └─────────────────────────┘
```

## Quick Start

```rust
use ethrex_l2_sdk::{transfer, deposit_through_transfer, withdraw};
use ethrex_rpc::clients::eth::EthClient;

// Create clients
let l1_client = EthClient::new("http://localhost:8545")?;
let l2_client = EthClient::new("http://localhost:8546")?;

// Transfer ETH on L2
let tx_hash = transfer(amount, from, to, &private_key, &l2_client).await?;

// Deposit from L1 to L2
let tx_hash = deposit_through_transfer(amount, from, &private_key, &l1_client).await?;

// Withdraw from L2 to L1
let tx_hash = withdraw(amount, from, private_key, &l2_client, None, None).await?;
```

## Bridge Operations

### Deposits (L1 → L2)

```rust
// Simple ETH deposit via transfer to bridge
let tx_hash = deposit_through_transfer(amount, from, &pk, &l1_client).await?;

// ERC20 deposit
let tx_hash = deposit_erc20(
    token_l1, token_l2, amount, from, &signer, &l1_client
).await?;
```

### Withdrawals (L2 → L1)

```rust
// Initiate withdrawal on L2
let tx_hash = withdraw(amount, from, pk, &l2_client, None, None).await?;

// Wait for proof and claim on L1
let proof = wait_for_l1_message_proof(&l2_client, tx_hash, 1000).await?;
let claim_hash = claim_withdraw(amount, from, pk, &l1_client, &proof).await?;
```

## Contract Deployment

### CREATE Deployment

```rust
use ethrex_l2_sdk::create_deploy;

let (tx_hash, address) = create_deploy(
    &client,
    &signer,
    init_code,
    Overrides::default(),
).await?;
```

### CREATE2 Deployment

```rust
use ethrex_l2_sdk::{create2_deploy_from_path, create2_deploy_from_bytecode};

// Deploy from file path
let (tx_hash, address) = create2_deploy_from_path(
    &constructor_args,
    Path::new("./contract.bytecode"),
    &signer,
    &salt,
    &client,
).await?;

// Deploy from bytecode
let (tx_hash, address) = create2_deploy_from_bytecode(
    &constructor_args,
    &bytecode,
    &signer,
    &salt,
    &client,
).await?;
```

### Proxy Deployment (ERC1967)

```rust
use ethrex_l2_sdk::deploy_with_proxy;

let deployment = deploy_with_proxy(
    &signer,
    &client,
    Path::new("./Implementation.bytecode"),
    &salt,
).await?;

println!("Proxy: {:?}", deployment.proxy_address);
println!("Implementation: {:?}", deployment.implementation_address);
```

## System Contracts

| Contract | Address | Description |
|----------|---------|-------------|
| `COMMON_BRIDGE_L2_ADDRESS` | `0x...ffff` | L2 bridge for ETH/ERC20 |
| `L2_TO_L1_MESSENGER_ADDRESS` | `0x...fffe` | L2 → L1 message passing |
| `FEE_TOKEN_REGISTRY_ADDRESS` | `0x...fffc` | Custom fee token registry |
| `FEE_TOKEN_PRICER_ADDRESS` | `0x...fffb` | Fee token pricing oracle |

## Transaction Building

```rust
use ethrex_l2_sdk::build_generic_tx;
use ethrex_common::types::TxType;

let tx = build_generic_tx(
    &client,
    TxType::EIP1559,
    to_address,
    from_address,
    calldata,
    Overrides {
        value: Some(amount),
        gas_limit: Some(100_000),
        ..Default::default()
    },
).await?;

let tx_hash = send_generic_transaction(&client, tx, &signer).await?;
```

## Calldata Encoding

```rust
use ethrex_l2_sdk::calldata::{encode_calldata, Value};

let calldata = encode_calldata(
    "transfer(address,uint256)",
    &[
        Value::Address(recipient),
        Value::Uint(amount),
    ],
)?;
```

## Fee Token Operations

```rust
use ethrex_l2_sdk::{register_fee_token_no_wait, get_fee_token_ratio};

// Register a new fee token
let tx_hash = register_fee_token_no_wait(
    &client, bridge_address, fee_token, &signer, Overrides::default()
).await?;

// Get fee token conversion ratio
let ratio = get_fee_token_ratio(&fee_token, &client).await?;
```

## Module Structure

| Module | Description |
|--------|-------------|
| `calldata` | ABI calldata encoding utilities |
| `l1_to_l2_tx_data` | L1 → L2 transaction data handling |
| `privileged_data` | Privileged transaction data (deposits) |

## Helper Functions

### Waiting for Receipts

```rust
use ethrex_l2_sdk::wait_for_transaction_receipt;

let receipt = wait_for_transaction_receipt(tx_hash, &client, 100).await?;
```

### Gas Bumping

```rust
use ethrex_l2_sdk::send_tx_bump_gas_exponential_backoff;

// Automatically retry with higher gas if tx gets stuck
let tx_hash = send_tx_bump_gas_exponential_backoff(&client, tx, &signer).await?;
```

### Contract Calls

```rust
use ethrex_l2_sdk::call_contract;

let tx_hash = call_contract(
    &client,
    &signer,
    contract_address,
    "setOwner(address)",
    vec![Value::Address(new_owner)],
).await?;
```

## Querying L1 State

```rust
use ethrex_l2_sdk::{get_last_committed_batch, get_last_verified_batch};

let committed = get_last_committed_batch(&client, proposer_address).await?;
let verified = get_last_verified_batch(&client, proposer_address).await?;
```

## Error Types

```rust
pub enum SdkError {
    FailedToParseAddressFromHex,
}

pub enum DeployError {
    FailedToReadInitCode(std::io::Error),
    FailedToDecodeBytecode(hex::FromHexError),
    FailedToDeploy(EthClientError),
    ProxyBytecodeNotFound,
}
```

## Feature Flags

Contract compilation is optional. Set `COMPILE_CONTRACTS=1` to enable:
- ERC1967 proxy deployment
- Other embedded contract bytecodes

## Dependencies

- `ethrex-common` - Core Ethereum types
- `ethrex-rpc` - RPC client (`EthClient`)
- `ethrex-l2-common` - L2 shared types
- `ethrex-l2-rpc` - L2 RPC client extensions
- `ethrex-sdk-contract-utils` - Contract compilation utilities

## Address Utilities

```rust
use ethrex_l2_sdk::{get_address_alias, get_erc1967_slot, address_to_word};

// Get L1 → L2 address alias
let aliased = get_address_alias(l1_address);

// Get ERC1967 storage slot
let slot = get_erc1967_slot("eip1967.proxy.implementation");
```

For detailed API documentation:
```bash
cargo doc --package ethrex-sdk --open
```
