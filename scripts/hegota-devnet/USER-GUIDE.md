# Hegotá Frame Transactions Devnet — User Guide

A public devnet for the frame-transaction family of EIPs on ethrex:

- [EIP-8141 Frame Transactions](https://eips.ethereum.org/EIPS/eip-8141) — the type `0x06` transaction: multiple execution frames, on-chain authorization via the `APPROVE` opcode, native gas sponsorship.
- [EIP-8250 Keyed Nonces](https://eips.ethereum.org/EIPS/eip-8250) — replay protection on independent nonce keys (`nonce_keys` + `nonce_seq`), backed by the `NONCE_MANAGER` predeploy.
- [EIP-8272 Recent Roots](https://eips.ethereum.org/EIPS/eip-8272) — commit and reference recent roots (e.g. for privacy proofs) via the `RECENT_ROOT_ADDRESS` predeploy and signed envelope references.
- [EIP-7906 Post-Transaction Assertion Frames](https://eips.ethereum.org/EIPS/eip-7906) — trailing read-only `POST_TX` frames whose revert invalidates the whole transaction, plus the `TXTRACE`/`TXDIFF` introspection opcodes.
- [EIP-7805](https://eips.ethereum.org/EIPS/eip-7805) + [EIP-8369](https://github.com/ethereum/EIPs/pull/12110) — FOCIL inclusion lists, with frame-transaction eligibility rules.
- [Native UTXOs](https://ethresear.ch/t/native-utxos-on-ethereum/25368) — UTXO frames (mode 5) and the vault predeploy. A draft in progress rather than an assigned EIP; ethrex tracks it as EIP-8312.

Implementation notes and every deviation from the drafts are in
[`docs/hegota-devnet.md`](../../docs/hegota-devnet.md).

## Network Details

| Field | Value |
|-------|-------|
| **Chain ID** | `3151908` (`0x301824`) |
| **RPC URLs** | `https://rpc1.hegota.ethrex.xyz` (also `rpc2` and `rpc3`) |
| **Block Explorer (Dora)** | `https://dora.hegota.ethrex.xyz` |
| **Faucet** | `https://faucet.hegota.ethrex.xyz` — 1 ETH per claim (rate-limited per IP) |
| **Slot Time** | 6 seconds |
| **Consensus** | 3× ethrex EL + 3× Lighthouse CL |

> All endpoints serve HTTPS (Let's Encrypt); plain-HTTP requests redirect. The raw
> HTTP ports remain open for tools that need them: `:32003`/`:32010`/`:32017` for the
> three ELs, `:32774` for Dora.

The `ethrex` namespace is exposed alongside `eth`/`net`/`web3`/`debug`, so
`ethrex_simulateFrameTransaction` is available for dry-running a validation prefix.

### Activations

| What | Chain-config field | Timestamp |
|------|--------------------|-----------|
| Hegotá — EIP-8141/8250/8272/7906 and EIP-7805 together (the CL calls this fork `heze`) | `hegotaTime` | `1786026080` (2026-08-06) |
| UTXO frames — frame mode 5 and the vault predeploy | `utxoFramesTime` | `1786028128` (2026-08-06) |
| Two-dimensional frame gas (`limits = [execution, state]`) | `frameLimitsTime` | `1787775374` (2026-08-26 20:16 UTC) |

`debug_chainConfig` on any of the three RPCs prints the schedule this chain is running.

### Predeploys

| Contract | Address | Purpose |
|----------|---------|---------|
| `EXPIRY_VERIFIER` | `0x…8141` | Frame-tx expiry deadlines (VERIFY frame with an 8-byte BE deadline) |
| `NONCE_MANAGER` | `0x…8250` | Keyed-nonce sequence storage (non-zero keys) |
| `RECENT_ROOT_ADDRESS` | `0x…8272` | Recent-root commitments (empty code — the 64-byte write is handled natively) |
| UTXO vault | `0x…8312` | Holds UTXO-frame value; the sender of every UTXO spend |

## Get Test ETH

Open the faucet in a browser and paste your address, or:

```bash
curl https://faucet.hegota.ethrex.xyz/api/claim \
  -H "Content-Type: application/json" \
  -d '{"address": "0xYourAddress"}'
```

Dispenses 1 ETH. Test ETH has no value.

- An all-lowercase or all-uppercase address is accepted. A mixed-case one is treated as
  an EIP-55 checksum and verified, so a typo comes back as `address checksum mismatch`.
- Rate limiting is per source IP, not per address. CI should fund from a prefunded
  account instead, since shared egress ranges hit the limit.
- `faucet is empty` means the dispensing account is below its reserve. Ask an operator.

## Connect MetaMask

1. MetaMask → Settings → Networks → Add Network
2. **RPC URL:** `https://rpc1.hegota.ethrex.xyz` · **Chain ID:** `3151908` · **Symbol:** `ETH`

> MetaMask can send regular EIP-1559 transactions on this network. Frame transactions
> (type `0x06`) must be submitted programmatically — see below.

## Send a Frame Transaction

[`rex`](https://github.com/lambdaclass/rex) has first-class frame-transaction support
(`rex frame send` / `build` / `inspect`). Install it from the **`hegota-devnet`** branch:

```bash
cargo install --force --locked --git https://github.com/lambdaclass/rex --branch hegota-devnet
```

`rex` on `main` depends on the *published* ethrex crates, which speak the EIP-8141-only
9-field envelope with a scalar frame `gas_limit` and the `27`/`28` signature form — all
three of which this chain rejects. The `hegota-devnet` branch git-pins ethrex's
`hegota-devnet` branch, so it speaks the exact wire format the devnet runs. Reinstall
rather than reusing an older binary: `rex frame --help` listing a `receipt` subcommand
instead of `inspect` means the binary is stale.

```bash
# Self-verified transfer: VERIFY frame approving execution+payment, then a SENDER
# frame that transfers the value. Prints the tx hash, then the decoded frames once
# it mines.
rex frame send \
  --to 0xRecipientAddress \
  --value 1gwei \
  --private-key <YOUR_PRIVATE_KEY_HEX> \
  --rpc-url https://rpc1.hegota.ethrex.xyz

# Preview the raw 0x06 bytes without submitting:
rex frame send --to 0x… --value 1gwei --private-key <KEY> --dry-run
```

`rex frame build --frames '[…]'` constructs a raw unsigned envelope from explicit
frames — the way to reach modes and flags `send` does not expose. See the
[rex CLI reference](https://github.com/lambdaclass/rex/blob/hegota-devnet/cli/README.md#rex-frame).

> **Inclusion tip:** frame-tx gossip between the devnet nodes is best-effort. If your
> transaction hasn't mined within ~30 s, submit the SAME raw transaction to the other
> two RPCs as well (idempotent — same hash).

## Frame Gas: Two Budgets

```
frame = [mode, flags, target, [execution, state], value, data]
```

`execution` pays for running code. `state` pays EIP-8037's charge for state growth:
183,600 gas to create an account, 97,920 per new storage slot. The pools are
independent, so execution gas cannot cover state growth.

A frame that writes new state with `state: 0` halts on that write, consuming its whole
execution budget. That looks like an execution limit set too low. Check `stateGasUsed`
on the receipt first: if it is `0x0`, the missing budget is the state one.

Unused state gas is refunded, so an over-generous bound costs only the reservation.

In `rex`, `--frame-gas-limit`/`--sponsor-gas-limit` set `limits.execution`; in
`rex frame build --frames`, `gasLimit` is `limits.execution` and `stateGasLimit` is
`limits.state`, which defaults to `0` — correct only for a frame that writes no new
state.

The validation prefix (the leading frames simulated before admission) is bounded by:

| | Spec | Here | Applies to |
|---|---|---|---|
| `MAX_VERIFY_GAS` | 100,000 | 500,000 | Σ prefix `limits.execution` + signature intrinsic |
| `MAX_VERIFY_STATE_GAS` | 500,000 | 500,000 | Σ prefix `limits.state` |

These nodes run `--mempool.max-verify-gas 500000`. Both bounds are mempool policy, not
consensus, so a prefix that fits here can still be rejected by a node on spec defaults —
do not treat 500,000 as portable. Frames outside the prefix are bounded by neither.

## Simulate Before Sending

`eth_call`/`eth_estimateGas` cannot represent a frame transaction (their input is a
single flat call, with no frames or validation prefix). ethrex adds a dedicated method
that dry-runs the SAME raw bytes you would submit — no submission, no mempool effect:

```bash
# rawTx is the 0x-prefixed canonical type-0x06 bytes you'd pass to
# eth_sendRawTransaction (`rex frame send --dry-run` prints these).
curl -s https://rpc1.hegota.ethrex.xyz \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"ethrex_simulateFrameTransaction","params":["0x06...","latest"]}'
```

Response fields: `valid`, `prefixShape`, `payer`, `maxCost`, `violation`, `gasUsed`,
`frames` (per frame, each `{gasUsed, stateGasUsed, succeeded}`), `executionStatus`,
`executionError`. Use `gasUsed`/`frames` to size both budgets, and `valid`/`violation`
to debug a prefix the mempool rejects.

`valid` covers every frame-specific gate — EIP-8141 static constraints and signature
authentication, the EIP-8250 key rules, EIP-8272 references, EIP-8312 openings, and the
validation-prefix simulation — in the order the mempool runs them, so a `false` never
under-rejects. It does not replay the gates shared with every other transaction type
(linear nonce, fee floor, wire size) or the per-sender pending-frame-transaction rule, so
a `true` is necessary but not sufficient for admission. This is an ethrex extension in
the `ethrex_*` namespace, not a standard `eth_` method.

## Reading Results

`eth_getTransactionReceipt` returns a type `0x6` receipt whose top-level `payer` is the
account that paid, plus one `frameReceipts` entry per frame:

```json
{
  "type": "0x6",
  "payer": "0xea4e13a1107b002aa7547df6681b80ecc191b13e",
  "frameReceipts": [
    { "status": "0x1", "gasUsed": "0x0", "stateGasUsed": "0x0", "logs": [] },
    { "status": "0x1", "gasUsed": "0x0", "stateGasUsed": "0x0", "logs": [ /* EIP-7708 transfer */ ] }
  ]
}
```

`gasUsed` is the execution dimension, `stateGasUsed` the state one. A frame that only
moves value runs no code, so its `gasUsed` is `0x0`, and its `stateGasUsed` is non-zero
only when the transfer creates the recipient account. ETH transfers emit EIP-7708 logs
from `0xffff…fffe`.

`rex frame inspect <TX_HASH>` fetches the transaction and its receipt and prints a
unified, decoded view — each frame's mode, APPROVE scope, target, value and data paired
with its per-frame status, gas and logs, plus the resolved payer:

```bash
rex frame inspect <TX_HASH> --rpc-url https://rpc1.hegota.ethrex.xyz
```
```
Frame transaction (type 0x06)
  status:    SUCCESS   block: 0x2f6b6   gas used: 0x52fe
  payer:     0xe255…6425 (self)   sender: 0xe255…6425   nonceKeys: [0x0] seq: 0xe
  frames:    2
    [0] VERIFY [APPROVE execution+payment] -> 0xe255…6425  value 0x0  data 0B    ✓ gas 0x0, 0 logs
    [1] SENDER [APPROVE none]               -> 0xe255…6425  value 0x1  data 0B    ✓ gas 0x0, 0 logs
```

## Common Errors

**`frame N uses the Scalar encoding, but this block requires Limits`**

Slot 3 is being sent as a scalar `gas_limit`. Since `frameLimitsTime` the chain accepts
only the `[execution, state]` form — the encoder is from before the activation.

**A frame reverts having used exactly its `execution` budget, `stateGasUsed: 0x0`**

Missing state budget; see [Frame Gas](#frame-gas-two-budgets). Raising `execution` will
not help.

**`Invalid frame transaction signature`**

For SECP256K1 the layout is `v || r || s` (65 bytes) with `v` a bare recovery id (`0` or
`1`), not the `27`/`28` form `ecrecover` takes. `r` and `s` must also be canonical:
`0 < r < n` and low-s (`0 < s <= n/2`).

**`Frame transaction prefix gas budget (frames + sig cost) exceeds MAX_VERIFY_GAS`**

The prefix exceeds the execution bound above. Move work out of the prefix, or lower the
prefix frames' `limits.execution`. The related
`Frame transaction signature verification cost exceeds MAX_VERIFY_GAS` means the
signature intrinsic alone already exceeds it — reduce the number of signatures.

## Wire Format

A frame transaction is `0x06 ‖ rlp(envelope)` with an 11-field envelope:

```
[chain_id, nonce_keys, nonce_seq, sender, frames, signatures,
 max_priority_fee_per_gas, max_fee_per_gas, max_fee_per_blob_gas,
 blob_versioned_hashes, recent_root_references]
```

- `frame = [mode, flags, target_or_empty, [execution, state], value, data]` — blocks
  before `frameLimitsTime` carry a scalar `gas_limit` in slot 3 instead. Decoding accepts
  both, and each frame re-encodes in the form it was decoded from, so historical
  transactions keep their hashes; a block admits only its own era's form.
- `signature = [scheme, signer, msg, signature_bytes]` — scheme 0 = ARBITRARY, scheme 1 = secp256k1 (65-byte `v‖r‖s`, where `v` is a bare recovery id ∈ {0,1}, `r` is canonical and `s` is low-s per EIP-2), scheme 2 = P256 (128-byte `r‖s‖qx‖qy`)
- `recent_root_reference = [source_id, slot, root]`
- `sig_hash = keccak256(0x06 ‖ rlp(envelope with every empty-msg signature's bytes elided))`

### Frame modes

| Mode | Name | Semantics |
|------|------|-----------|
| 0 | DEFAULT | General call, caller = ENTRY_POINT (`0x…aa`) |
| 1 | VERIFY | Static validation frame — grants approval via `APPROVE` |
| 2 | SENDER | Executes as `tx.sender` (requires execution approval); only mode that may carry `value` |
| 3 | POST_TX | EIP-7906 trailing read-only assertion — a revert invalidates the whole transaction |
| 5 | UTXO | Spends and creates UTXOs against the vault predeploy |

### Frame flags

- Bits 0–1: APPROVE scope restriction (`0x1` payment, `0x2` execution, `0x3` both)
- Bit 2 (`0x04`): atomic-batch member — the batch reverts together. Payment-scoped
  APPROVE is forbidden inside a batch.

## Sponsored Transactions (Trustless Paymaster)

A frame transaction can have its gas paid by a **distinct paymaster** contract
(`payer != sender`) — the canonical-paymaster `[only_verify, pay]` shape. The exec frame
targets the sender (who approves execution via the outer signature) and the pay frame
targets a paymaster contract that calls `APPROVE(scope=1)`.

`contracts/OpenSponsor.yul` is a minimal, observer-friendly sponsor: its `verify()`
(`0xfc735e99`) just calls `APPROVE(APPROVE_PAYMENT)`, so it sponsors any sender. It makes
no external calls and reads no storage in the verify path, so it is admissible via the
public mempool — unlike a balance-gated sponsor, whose external `STATICCALL` the ERC-7562
validation observer rejects for a non-canonical paymaster.

```bash
# 1. Compile the sponsor (Yul).
solc --strict-assembly --bin contracts/OpenSponsor.yul

# 2. Deploy it with the owner address appended as a 32-byte constructor arg, then fund
#    it with ETH (any tool that sends a plain type-2 tx works):
#      initcode = <bin> || left-padded-32-byte owner
#    rex deploy --bytecode <initcode> --print-address \
#      --private-key <OWNER_KEY> --rpc-url https://rpc1.hegota.ethrex.xyz
#    rex transfer 1000000000000000000 <SPONSOR_ADDR> --private-key <OWNER_KEY> --rpc-url ...

# 3. Send a sponsored transfer. The sender needs only the transferred `value`, not gas:
#    a successful run with a gas-starved sender proves the sponsor paid.
rex frame send \
  --to 0xRecipientAddress --value 0.01ether \
  --sponsor <SPONSOR_ADDR> --sponsor-calldata 0xfc735e99 \
  --private-key <SENDER_PRIVATE_KEY_HEX> \
  --rpc-url https://rpc1.hegota.ethrex.xyz
```

`--sponsor-calldata` matters: with empty calldata OpenSponsor's `receive()` path runs and
never calls `APPROVE`, so the transaction has no payer. The receipt's top-level `payer`
is then the **sponsor**, and the sender's balance drops only by the transferred `value`.
`withdraw(address,uint256)` (`0xf3fef3a3`, owner-only) reclaims the sponsor's ETH.

> A sender-restricted trustless sponsor (authorizing specific senders) can ecrecover an
> owner signature over a domain that **excludes** the signature — e.g.
> `keccak(sender ‖ chain_id ‖ nonce_seq ‖ expiry)`. Do **not** sign over `sig_hash` and
> carry the signature in frame data: `sig_hash` commits frame data verbatim, so that
> construction is a circular (unsatisfiable) fixed point.

## EIP-8250: Keyed Nonces

The envelope carries `nonce_keys` (key `0` alone, or 1–16 strictly-increasing non-zero
u256 keys) and one `nonce_seq` checked against every selected key. Key `0` is the
account's regular nonce; non-zero keys live in `NONCE_MANAGER` storage
(`slot = keccak256(pad32(sender) ‖ key)`), letting independent workflows send in parallel
without nonce races.

The public mempool admits both domains. Keyed transactions are tracked per
`(sender, key set)` — disjoint key sets are independent — and their per-key nonce
validity comes from the validation-prefix simulation rather than the linear account
nonce. One sender cannot hold pending transactions in both domains at once
(`A frame transaction in the other nonce-key domain is already pending for this sender`).

## EIP-7906: POST_TX Assertion Frames

Append `POST_TX` frames (mode 3) as a trailing suffix. They run read-only with
ENTRY_POINT as caller after the main body; if any of them reverts, the whole transaction
is invalidated — including the already-approved gas payment. `APPROVE` is forbidden
inside them. `TXTRACE`/`TXDIFF` let assertion code inspect the transaction's own
execution.

## EIP-8272: Recent Roots

**Write:** call `RECENT_ROOT_ADDRESS` (`0x…8272`) with exactly 64 bytes of calldata
(`salt ‖ root`) and zero value — from a frame or any contract call. The entry is
committed under `source_id = keccak256(pad32(caller) ‖ salt)` for the current slot (cost:
22100 gas). Static contexts and `DELEGATECALL`/`CALLCODE` revert.

**Reference:** declare `[source_id, slot, root]` tuples in the envelope's
`recent_root_references`. Each must satisfy `1 ≤ current_slot − slot ≤ 8191` and match
the committed entry — an invalid or forged reference invalidates the transaction (the
mempool also rejects it at admission).

> **Current devnet limitation:** the reference side is not usable end to end here. The
> current slot comes from EIP-7843, which the CL delivers over
> `engine_forkchoiceUpdatedV4`; the devnet's Lighthouse drives ethrex over V3, which
> carries no slot field. ethrex can derive the slot from the block timestamp behind the
> `derivedSlotTime` chain-config knob, but this chain does not set it
> (`debug_chainConfig` reports `derivedSlotTime: null`), so writes land at slot 0 and no
> reference can satisfy the window. Writes, forged-reference rejection and every
> consensus rule are active.

## UTXO Frames

Mode-5 frames spend and create UTXOs held by the vault predeploy at `0x…8312`, which is
the `tx.sender` of every spend. Active on this chain since `utxoFramesTime`.
`utxo-demo/` is an interactive demo of deposits, self-funded spends, multi-actor
consolidation and sponsored spends; in live mode every step is a real transaction on a
devnet with UTXO frames activated. See [`utxo-demo/README.md`](utxo-demo/README.md) and
[`docs/eip-8312.md`](../../docs/eip-8312.md).

## Scripts in This Directory

`frametx.py` (a dependency-light byte-exact encoder), `frametx_submit.py`,
`frametx_sponsor_submit.py` and `utxo_itest.py` predate `frameLimitsTime`: they encode
slot 3 as a scalar `gas_limit`, which every block since the activation rejects. They
remain an accurate reference for the envelope and `sig_hash` construction — `python3
frametx.py` checks itself against the repo's scalar golden vector — but use `rex` to
submit to this devnet until they are updated to `limits`.

## Divergences From the Draft Specs

The EIPs are drafts with TBD sections; every convention ethrex adopted (opcode bytes,
predeploy addresses, `source_id` derivation, write gas, TXPARAM indices) is documented
with rationale in the repo:
[`docs/eip-8141.md`](../../docs/eip-8141.md) ·
[`docs/eip-8250.md`](../../docs/eip-8250.md) ·
[`docs/eip-8272.md`](../../docs/eip-8272.md) ·
[`docs/eip-7906.md`](../../docs/eip-7906.md) ·
[`docs/eip-8312.md`](../../docs/eip-8312.md)
