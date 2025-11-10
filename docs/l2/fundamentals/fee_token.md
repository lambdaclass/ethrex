# Fee Token Overview

Ethrex lets L2 transactions pay execution costs with an ERC-20 instead of ETH. A fee-token-enabled transaction behaves like a normal call or transfer, but the sequencer locks fees in the ERC-20 and distributes them (sender refund, coinbase priority fee, base-fee vault, operator vault, L1 data fee) using the hooks in `l2_hook.rs`.

Key requirements:
- The token must implement `IFeeToken` (see `crates/l2/contracts/src/example/FeeToken.sol`), which extends `IERC20L2` and adds the `lockFee` / `payFee` entry points consumed by the sequencer.
- `lockFee` must reserve funds when invoked by the fee collector (the L2 bridge/`COMMON_BRIDGE_L2_ADDRESS`), and `payFee` must release or burn those funds when the transaction finishes.
- The token address must be registered in the L2 `FeeTokenRegistry` system contract (`0x…fffc`). Registration happens through the L1 `CommonBridge` by calling `registerNewFeeToken(address)`; only the bridge owner can do this, and the call queues a privileged transaction that the sequencer forces on L2. Likewise, `unregisterFeeToken(address)` removes it.

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

Compile and deploy:

```shell
rex deploy 0 <PRIVATE_KEY> \
    --rpc-url http://localhost:1729 \
    --contract-path crates/l2/contracts/src/example/FeeToken.sol \
    --remappings "@openzeppelin=https://github.com/OpenZeppelin/openzeppelin-contracts.git" \
    -- "constructor(address)" 0000000000000000000000000000000000000000
```

## Operator Workflow

1. Deploy or reuse an `IFeeToken` implementation and note its L2 address. (The devnet deployer can optionally register an initial fee token by passing `--l1.initial-fee-token <address>` when running `cmd/ethrex l2 deploy`; this queues the privileged registration automatically.)
2. Ensure the token address is registered in the `FeeTokenRegistry` through the L1 `CommonBridge` (see next section).
3. Instantiate an `EthClient` and a signer (local or remote) that will send transactions.
4. Build a `TxType::FeeToken` transaction with `ethrex_l2_sdk::build_generic_tx`, setting `Overrides::fee_token` and the desired `value`/calldata.
5. Submit the transaction with `ethrex_l2_sdk::send_generic_transaction` and wait for the receipt.

> ⚠️ **Warning:** Inclusions and refunds still depend on the L1 watcher/sequencer infrastructure. This workflow assumes the sequencer is running, the fee token has been registered on L2, and the privileged queue is being processed. Without those pieces, fee-token transactions will remain stuck even if locally signed.

That is all the sequencer needs; fee locking and distribution happen automatically via `l2_hook.rs`.

If the token is not yet registered, the bridge owner can queue the privileged call from L1. You can use the SDK helpers:

```rust
let calldata = ethrex_l2_sdk::calldata::encode_calldata(
    "registerNewFeeToken(address)",
    &[Value::Address(fee_token_address)],
)?;
let tx = build_generic_tx(
    &l1_client,
    TxType::EIP1559,
    ethrex_l2_sdk::bridge_address()?, // CommonBridge proxy on L1
    owner_signer.address(),
    calldata.into(),
    Overrides {
        gas_limit: Some(21000 * 10),
        ..Default::default()
    },
).await?;
let hash = send_generic_transaction(&l1_client, tx, &owner_signer).await?;
wait_for_transaction_receipt(hash, &l1_client, 100).await?;
// L1 watcher will include the privileged tx and the registry will emit FeeTokenRegistered.
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
