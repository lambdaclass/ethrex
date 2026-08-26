# Hegotá devnet: user guide

A public devnet for the frame-transaction family of EIPs:

| | |
|---|---|
| [EIP-8141](https://eips.ethereum.org/EIPS/eip-8141) | Frame transactions (type `0x06`) |
| [EIP-8250](https://eips.ethereum.org/EIPS/eip-8250) | Keyed nonces |
| [EIP-8272](https://eips.ethereum.org/EIPS/eip-8272) | Recent roots |
| [EIP-7906](https://eips.ethereum.org/EIPS/eip-7906) | Post-transaction assertions |
| [EIP-7805](https://eips.ethereum.org/EIPS/eip-7805) + [EIP-8369](https://github.com/ethereum/EIPs/pull/12110) | FOCIL inclusion lists |
| [Native UTXOs](https://ethresear.ch/t/native-utxos-on-ethereum/25368/23) | UTXO frames |

Implementation notes and deviations from the specs are in `hegota-devnet.md`.

## Endpoints

| | |
|---|---|
| Chain ID | `3151908` |
| JSON-RPC | `https://rpc1.hegota.ethrex.xyz`, `rpc2`, `rpc3` |
| Faucet | `https://faucet.hegota.ethrex.xyz` |
| Explorer | `https://dora.hegota.ethrex.xyz` |

The `ethrex` namespace is exposed alongside `eth`/`net`/`web3`, so
`ethrex_simulateFrameTransaction` is available for dry-running a validation prefix.

## Getting ETH

```bash
curl -X POST https://faucet.hegota.ethrex.xyz/api/claim \
  -H 'content-type: application/json' \
  -d '{"address":"0xYourChecksummedAddress"}'
```

Dispenses 1 ETH. Test ETH has no value.

- The address must be EIP-55 checksummed; lowercase is rejected with `address checksum mismatch`.
- Rate limiting is per source IP, not per address. CI should fund from a prefunded account
  instead, since shared egress ranges hit the limit.
- `faucet is empty` means the dispensing account is below its reserve. Ask an operator.

## Sending a frame transaction

[`rex`](https://github.com/lambdaclass/rex) builds the envelope from ethrex's own types.
Install from the `hegota-devnet` branch:

```bash
cargo install --git https://github.com/lambdaclass/rex --branch hegota-devnet --locked
```

The default branch emits the pre-revision frame layout and the `27`/`28` signature form, both
of which this chain rejects.

```bash
rex frame send \
  --to 0xRecipient \
  --value 1gwei \
  --private-key $PRIVATE_KEY \
  --rpc-url https://rpc1.hegota.ethrex.xyz
```

`--dry-run` prints the raw `0x06` bytes without sending. `rex frame build` constructs an
envelope from explicit frames and `rex frame inspect` decodes a transaction alongside its
per-frame results; see the
[CLI README](https://github.com/lambdaclass/rex/blob/hegota-devnet/cli/README.md).

## Frame gas: two budgets

```
frame = [mode, flags, target, [execution, state], value, data]
```

`execution` pays for running code. `state` pays EIP-8037's charge for state growth: 183,600 gas
to create an account, 97,920 per new storage slot. The pools are independent, so execution gas
cannot cover state growth.

A frame that writes new state with `state: 0` halts on that write, consuming its whole execution
budget. That looks like an execution limit set too low. Check `stateGasUsed` on the receipt
first: if it is `0x0`, the missing budget is the state one.

Unused state gas is refunded, so an over-generous bound costs only the reservation.

The validation prefix (the leading frames simulated before admission) is bounded by:

| | Spec | Here | Applies to |
|---|---|---|---|
| `MAX_VERIFY_GAS` | 100,000 | 500,000 | Σ prefix `limits.execution` + signature intrinsic |
| `MAX_VERIFY_STATE_GAS` | 500,000 | 500,000 | Σ prefix `limits.state` |

These nodes run `--mempool.max-verify-gas 500000`. A prefix that fits here can still be rejected
by a node on spec defaults, so do not treat 500,000 as portable. Frames outside the prefix are
unbounded by both.

## Common errors

**`frame N uses the Scalar encoding, but this block requires Limits`**

Slot 3 is being sent as a scalar `gas_limit`. The chain activated the `[execution, state]` form
on 2026-08-26 and accepts only that.

**A frame reverts having used exactly its `execution` budget, `stateGasUsed: 0x0`**

Missing state budget. Raising `execution` will not help.

**`Invalid frame transaction signature`**

For SECP256K1 the layout is `v || r || s` (65 bytes) with `v` a bare recovery id (`0` or `1`),
not the `27`/`28` form `ecrecover` takes. `r` and `s` must also be canonical: `0 < r < n` and
low-s (`0 < s <= n/2`).

**`Frame transaction signature verification cost exceeds MAX_VERIFY_GAS`**

The prefix exceeds the execution bound above. Move work out of the prefix or reduce signatures.

## Reading results

`eth_getTransactionReceipt` returns one `frameReceipts` entry per frame:

```json
{ "status": "0x1", "gasUsed": "0x...", "stateGasUsed": "0x...", "logs": [] }
```

`gasUsed` is the execution dimension, `stateGasUsed` the state dimension. A frame that only
moves value runs no code, so `gasUsed` is `0x0` and `stateGasUsed` carries its cost.

`rex frame inspect <txhash>` renders the same data with the frames decoded.
