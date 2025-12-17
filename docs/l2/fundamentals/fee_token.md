# Fee Token Overview

Ethrex lets L2 transactions pay execution costs with an ERC-20 instead of ETH. A fee-token-enabled transaction behaves like a normal call or transfer, but the sequencer locks fees in the ERC-20 and distributes them (sender refund, coinbase priority fee, base-fee vault, operator vault, L1 data fee) using the hooks in `l2_hook.rs`.

Key requirements:
- The token must implement `IFeeToken` (see `crates/l2/contracts/src/example/FeeToken.sol`), which extends `IERC20L2` and adds the `lockFee` / `payFee` entry points consumed by the sequencer.
- `lockFee` must reserve funds when invoked by the l2 bridge (the L2 bridge/`COMMON_BRIDGE_L2_ADDRESS`), and `payFee` must release or burn those funds when the transaction finishes.
- The token address must be registered in the L2 `FeeTokenRegistry` system contract (`0x…fffc`). Registration happens through the L1 `CommonBridge` by calling `registerNewFeeToken(address)`; only the bridge owner can do this, and the call queues a privileged transaction that the sequencer forces on L2. Likewise, `unregisterFeeToken(address)` removes it.

Fee token ratios are also updated through the same privileged transaction path (deposits from L1 to L2). This is because we want the changes to be done through the L1, and via an owner that we want to be the same as the owner in the L1 bridge.

### Minimal Contract Surface

```solidity
contract FeeToken is ERC20, IFeeToken {
    address internal constant BRIDGE = 0x000000000000000000000000000000000000FFFF;

    ...

    modifier onlyBridge() {
        require(msg.sender == BRIDGE, "only bridge");
        _;
    }
    function lockFee(address payer, uint256 amount)
        external
        override(IFeeToken)
        onlyBridge
    {
        _transfer(payer, BRIDGE, amount);
    }

    function payFee(address receiver, uint256 amount)
        external
        override(IFeeToken)
        onlyBridge
    {
        if (receiver == address(0)) {
            _burn(BRIDGE, amount);
        } else {
            _transfer(BRIDGE, receiver, amount);
        }
    }
}
```

For deployment and operator steps, see [Deploying a Fee Token](../deployment/fee_token.md).

## User Workflow

Once a token is registered, users can submit fee-token transactions:

1. Instantiate an `EthClient` connected to L2 and create a signer.
2. Build a `TxType::FeeToken` transaction with `build_generic_tx`, setting `Overrides::fee_token = Some(<token>)` and the desired `value` / calldata.
3. Send the transaction with `send_generic_transaction` and wait for the receipt.

Fee locking and distribution happen automatically inside `l2_hook.rs`.

### Minimal `Cargo.toml`

```toml
[package]
name = "fee-token-client"
version = "0.1.0"
edition = "2024"

[dependencies]
anyhow = "1.0.86"
hex = "0.4.3"
secp256k1 = { version = "0.30.0", default-features = false, features = ["global-context", "recovery", "rand"] }
tokio = { version = "1.41.1", features = ["macros", "rt-multi-thread"] }
url = { version = "2.5.4", features = ["serde"] }
ethrex_l2_sdk = { package = "ethrex-sdk", git = "https://github.com/lambdaclass/ethrex", tag = "v6.0.0" }
ethrex-rpc = { git = "https://github.com/lambdaclass/ethrex", tag = "v6.0.0" }
ethrex-common = { git = "https://github.com/lambdaclass/ethrex", tag = "v6.0.0" }
ethrex-l2-rpc = { git = "https://github.com/lambdaclass/ethrex", tag = "v6.0.0" }
```

```rust
use anyhow::Result;
use ethrex_l2_sdk::{build_generic_tx, send_generic_transaction};
use ethrex_rpc::clients::eth::{EthClient, Overrides};
use ethrex_common::types::TxType;
use ethrex_common::{Address, Bytes, U256};
use ethrex_l2_sdk::wait_for_transaction_receipt;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use secp256k1::SecretKey;
use url::Url;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Connect and create the signer.
    let l2 = EthClient::new(Url::parse("http://localhost:1729")?)?;
    let private_key = SecretKey::from_slice(&hex::decode("<hex-private-key>")?)?;
    let signer = Signer::Local(LocalSigner::new(private_key));

    // 2. Build the fee-token transaction.
    let fee_token: Address = "<fee-token-address>".parse()?;
    let recipient: Address = "<recipient-address>".parse()?;
    let mut tx = build_generic_tx(
        &l2,
        TxType::FeeToken,
        recipient,
        signer.address(),
        Bytes::default(),
        Overrides {
            fee_token: Some(fee_token),
            value: Some(U256::from(100_000u64)),
            ..Default::default()
        },
    )
    .await?;

    // 3. Send and wait for the receipt.
    let tx_hash = send_generic_transaction(&l2, tx, &signer).await?;
    wait_for_transaction_receipt(tx_hash, &l2, 100).await?;
    Ok(())
}
```
