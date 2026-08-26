# Hegotá devnet — user guide

A public devnet running the EIP-8141 frame-transaction family: EIP-8141 (frame transactions),
EIP-8250 (keyed nonces), EIP-8272 (recent roots), EIP-7906 (post-transaction assertions),
EIP-8312 (UTXO frames), EIP-7805/8369 (FOCIL). It exists so these EIPs can be exercised against
a real chain rather than a local harness.

For what this branch does differently from the specs, see `hegota-devnet.md`. This file is for
people *using* the devnet.

## Endpoints

| What | Where |
|---|---|
| JSON-RPC | `https://rpc1.hegota.ethrex.xyz` (also `rpc2`, `rpc3`) |
| Faucet | `https://faucet.hegota.ethrex.xyz` |
| Explorer | `https://dora.hegota.ethrex.xyz` |
| Chain ID | `3151908` |

The `ethrex` RPC namespace is exposed alongside `eth`/`net`/`web3`, so
`ethrex_simulateFrameTransaction` is reachable. Use it to dry-run a validation prefix before
paying for it.

## Getting ETH

```bash
curl -X POST https://faucet.hegota.ethrex.xyz/api/claim \
  -H 'content-type: application/json' \
  -d '{"address":"0xYourChecksummedAddress"}'
```

Two things surprise people:

- **The address must be EIP-55 checksummed.** A lowercase address is rejected with
  `address checksum mismatch`, not silently accepted.
- **Rate limiting is per source IP, not per address.** `rate limited, try again in N minute(s)`
  means someone sharing your egress IP claimed recently. Automated suites should fund from a
  prefunded account instead of the faucet, or they become unrunnable from shared CI ranges.
- `faucet is empty, ask an operator to top it up` means the dispensing account fell below its
  reserve. It is an operator problem, not something a caller can retry around.

## Sending a frame transaction

The easiest path is [`rex`](https://github.com/lambdaclass/rex), which builds the envelope from
ethrex's own types, so its encoding cannot drift from what the node expects.

**Install from the `hegota-devnet` branch — the default branch will not work here:**

```bash
cargo install --git https://github.com/lambdaclass/rex --branch hegota-devnet --locked
```

The branch is load-bearing, not a preference. `main` still emits the pre-revision frame layout
(a scalar `gas_limit` in slot 3) and the `27`/`28` signature form, so every frame transaction it
builds is rejected here — on the encoding first, and on the signature after that. Both are fixed
on `hegota-devnet`.

Then:

```bash
rex frame send \
  --to 0xRecipient \
  --value 1gwei \
  --private-key $PRIVATE_KEY \
  --rpc-url https://rpc1.hegota.ethrex.xyz
```

Add `--dry-run` to print the raw `0x06` bytes without sending, and see
[the CLI README](https://github.com/lambdaclass/rex/blob/hegota-devnet/cli/README.md) for
`frame build` (construct an envelope from explicit frames) and `frame inspect` (decode a
transaction and pair its frames with their results).

## The two budgets — the thing that trips everyone up

A frame declares gas in **two dimensions**, and they never mix:

```
frame = [mode, flags, target, [execution, state], value, data]
                              ^^^^^^^^^^^^^^^^^^
```

- `execution` pays for running EVM code.
- `state` pays EIP-8037's charge for *growing* the state: creating an account, writing a
  storage slot that did not exist. Roughly 183,600 gas to create an account and ~98,000 per
  fresh storage slot on this chain.

Execution gas cannot pay for state growth. A frame that writes new state while declaring
`state: 0` halts on that write — and because a halt consumes the frame's whole execution
budget, **it looks like the execution limit was too low**. If a frame reverts having spent
exactly its `execution` budget, check `stateGasUsed` on the receipt before raising the
execution number: if it reads `0x0`, the state budget is what is missing.

Unused state gas is refunded, so declaring a generous bound costs nothing but the reservation.

Two constants bound the *validation prefix* (the leading frames a node simulates before
admitting a transaction):

| Constant | Spec default | **On this devnet** | Applies to |
|---|---|---|---|
| `MAX_VERIFY_GAS` | 100,000 | **500,000** | Σ prefix `limits.execution` + signature intrinsic |
| `MAX_VERIFY_STATE_GAS` | 500,000 | 500,000 | Σ prefix `limits.state` |

Frames outside the prefix are not bound by these.

The nodes here run `--mempool.max-verify-gas 500000`, so the execution bound is five times the
spec default. That override predates the two-dimensional split — it existed because account
creation had to fit in a single combined budget — and it is no longer load-bearing now that the
state dimension carries growth. It is called out because a prefix that fits here can still be
rejected by a node running spec defaults: **do not treat 500,000 as portable**.

## Errors you are likely to hit

**`frame N uses the Scalar encoding, but this block requires Limits`**

Your tooling is emitting the older frame format, where slot 3 was a bare `gas_limit` scalar.
The chain crossed its `frameLimitsTime` activation on 2026-08-26 and now accepts only
`[execution, state]`. Update your client. `rex` on `hegota-devnet` emits the current form; a
frontend building frames by hand needs slot 3 changed to a two-element list.

**A frame reverts having used exactly its `execution` budget, with `stateGasUsed: 0x0`**

Missing state budget — see above. Raising `execution` will not help.

**`Invalid frame transaction signature`**

For SECP256K1 the wire layout is `v || r || s` (65 bytes) and `v` must be a **bare recovery id**
(`0` or `1`), not the `27`/`28` EVM form that `ecrecover` takes. Anything above `1` is rejected.

**`Frame transaction signature verification cost exceeds MAX_VERIFY_GAS`**

The validation prefix is over the execution budget above. Move work out of the prefix, or
reduce the number of signatures.

## Reading results

`eth_getTransactionReceipt` returns `frameReceipts`, one per frame, each with:

```json
{ "status": "0x1", "gasUsed": "0x...", "stateGasUsed": "0x...", "logs": [] }
```

`gasUsed` is the execution dimension and `stateGasUsed` the state dimension, reported apart
because the pools are apart. A frame that only moves value does no EVM work, so its `gasUsed`
is legitimately `0x0` and `stateGasUsed` carries its whole cost.

`rex frame inspect <txhash>` pairs the decoded frames with their per-frame results, which is
usually easier to read than the raw receipt.
