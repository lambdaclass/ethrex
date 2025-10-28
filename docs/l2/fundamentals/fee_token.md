# Fee Token Overview

Ethrex lets L2 transactions pay execution costs with an ERC-20 instead of ETH. A fee-token-enabled transaction behaves like a normal call or transfer, but the sequencer locks fees in the ERC-20 and distributes them (sender refund, coinbase priority fee, base-fee vault, operator vault, L1 data fee) using the hooks in `l2_hook.rs`.

Key requirements:
- The token must implement `IFeeToken`, which combines the standard ERC-20 surface plus the `IERC20L2` bridge interface and the `lockFee`/`payFee` entry points the hook calls.
- `lockFee` **must** reserve funds when invoked by the fee collector (the L2 bridge/COMMON_BRIDGE_L2_ADDRESS), and `payFee` must release/burn them when the hook settles the transaction.

## Using the Token with the SDK

From an operator point of view the flow mirrors the integration test in `crates/l2/tests/tests.rs`:
1. Deploy or point to an `IFeeToken` implementation (the example contract and the fixture both satisfy the interface).
2. Build a `TxType::FeeToken` transaction with `ethrex_l2_sdk::build_generic_tx`, setting:
   - `Overrides::fee_token` to the fee token address,
   - `Overrides::value` and any calldata/context just like a regular transaction.
3. Sign and submit the transaction with `ethrex_l2_sdk::send_generic_transaction`.

That is all that is needed to run fee-token transactions; the sequencer handles the rest as long as the token exposes the expected interface.

```rust
use ethrex_l2_sdk::{build_generic_tx, send_generic_transaction};
use ethrex_rpc::clients::eth::Overrides;
use ethrex_common::types::TxType;
let mut tx = build_generic_tx(
    l2_client,
    TxType::FeeToken,
    recipient,
    origin,
    ethrex_common::Bytes::default(),
    Overrides {
        fee_token: Some(fee_token),
        value: Some(value),
        ..Default::default()
    },
)
.await;

let tx_hash = send_generic_transaction(l2_client, tx, signer)
    .await;

```
