# Fee Token Overview

Ethrex lets L2 transactions pay execution costs with an ERC-20 instead of ETH. A fee-token-enabled transaction behaves like a normal call or transfer, but the sequencer locks fees in the ERC-20 and distributes them (sender refund, coinbase priority fee, base-fee vault, operator vault, L1 data fee) using the hooks in `l2_hook.rs`.

Key requirements:
- The token must implement `IFeeToken` (see `crates/l2/contracts/src/example/FeeToken.sol`), which extends `IERC20L2` and adds the `lockFee` / `payFee` entry points consumed by the sequencer.
- `lockFee` must reserve funds when invoked by the fee collector (the L2 bridge/`COMMON_BRIDGE_L2_ADDRESS`), and `payFee` must release or burn those funds when the transaction finishes.

### Minimal Contract Surface

```solidity
contract FeeToken is ERC20, IFeeToken {
    
    ...

    modifier onlyFeeCollector() {
        require(msg.sender == FEE_COLLECTOR, "only fee collector");
        _;
    }
    function lockFee(address payer, uint256 amount)
        external
        override(IFeeToken)
        onlyFeeCollector
    {
        _transfer(payer, FEE_COLLECTOR, amount);
    }

    function payFee(address receiver, uint256 amount)
        external
        override(IFeeToken)
        onlyFeeCollector
    {
        if (receiver == address(0)) {
            _burn(FEE_COLLECTOR, amount);
        } else {
            _transfer(FEE_COLLECTOR, receiver, amount);
        }
    }
}
```

Compile and deploy:

```shell
rex deploy 0 <PRIVATE_KEY> \
    --rpc-url http://localhost:1729 \
    --contract-path crates/l2/contracts/src/example/FeeToken.sol \
    --remappings "@openzeppelin=https://github.com/OpenZeppelin/openzeppelin-contracts.git" \
    -- "constructor(address)" 0000000000000000000000000000000000000000
```

## Operator Workflow

1. Deploy or reuse an `IFeeToken` implementation and note its L2 address.
2. Instantiate an `EthClient` and a signer (local or remote) that will send the transaction.
3. Build a `TxType::FeeToken` transaction with `ethrex_l2_sdk::build_generic_tx`, setting `Overrides::fee_token` and the desired `value`/calldata.
4. Submit the transaction with `ethrex_l2_sdk::send_generic_transaction` and wait for the receipt.

That is all the sequencer needs; fee locking and distribution happen automatically via `l2_hook.rs`.

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
