# Hegotá Frame Transactions Devnet — User Guide

A public devnet for testing the Hegotá frame-transaction EIP family on ethrex:

- [EIP-8141 Frame Transactions](https://eips.ethereum.org/EIPS/eip-8141) — the type `0x06` transaction: multiple execution frames, on-chain authorization via the `APPROVE` opcode, native gas sponsorship.
- [EIP-8250 Keyed Nonces](https://eips.ethereum.org/EIPS/eip-8250) — replay protection on independent nonce keys (`nonce_keys` + `nonce_seq`), backed by the `NONCE_MANAGER` predeploy.
- [EIP-8272 Recent Roots](https://eips.ethereum.org/EIPS/eip-8272) — commit and reference recent roots (e.g. for privacy proofs) via the `RECENT_ROOT_ADDRESS` predeploy and signed envelope references.
- [EIP-7906 Post-Transaction Assertion Frames](https://eips.ethereum.org/EIPS/eip-7906) — trailing read-only `POST_TX` frames whose revert invalidates the whole transaction, plus the `TXTRACE`/`TXDIFF` introspection opcodes.

## Network Details

| Field | Value |
|-------|-------|
| **Chain ID** | `3151908` (`0x301824`) |
| **RPC URLs** | `https://rpc1.hegota.ethrex.xyz` (also `rpc2` and `rpc3`) |
| **Block Explorer (Dora)** | `https://dora.hegota.ethrex.xyz` |
| **Faucet** | `https://faucet.hegota.ethrex.xyz` — 1 ETH per claim (rate-limited per IP) |
| **Slot Time** | 6 seconds |
| **Fork** | Hegotá (all four EIPs activate together at epoch 1) |
| **Consensus** | 3× ethrex EL + 3× Lighthouse CL |

> All endpoints serve HTTPS (Let's Encrypt); plain-HTTP requests redirect. The raw
> HTTP ports (`:32003/:32010/:32017` RPC, `:32774` Dora, `:8080` faucet) remain open
> for tools that need them.

### Predeploys

| Contract | Address | Purpose |
|----------|---------|---------|
| `EXPIRY_VERIFIER` | `0x…8141` | Frame-tx expiry deadlines (VERIFY frame with an 8-byte BE deadline) |
| `NONCE_MANAGER` | `0x…8250` | Keyed-nonce sequence storage (non-zero keys) |
| `RECENT_ROOT_ADDRESS` | `0x…8272` | Recent-root commitments (empty code — the 64-byte write is handled natively) |

## Connect MetaMask

1. MetaMask → Settings → Networks → Add Network
2. **RPC URL:** `https://rpc1.hegota.ethrex.xyz` · **Chain ID:** `3151908` · **Symbol:** `ETH`

> MetaMask can send regular EIP-1559 transactions on this network. Frame transactions
> (type `0x06`) must be submitted programmatically — see the scripts below.

## Get Test ETH

Open the faucet in a browser and paste your address, or:

```bash
curl https://faucet.hegota.ethrex.xyz/api/claim \
  -H "Content-Type: application/json" \
  -d '{"address": "0xYourAddress"}'
```

## Frame Transaction Wire Format

A frame transaction is `0x06 ‖ rlp(envelope)` with an 11-field envelope:

```
[chain_id, nonce_keys, nonce_seq, sender, frames, signatures,
 max_priority_fee_per_gas, max_fee_per_gas, max_fee_per_blob_gas,
 blob_versioned_hashes, recent_root_references]
```

- `frame = [mode, flags, target_or_empty, gas_limit, value, data]`
- `signature = [scheme, signer, msg, signature_bytes]` — scheme 0 = secp256k1 (65-byte `v‖r‖s`, v ∈ {27,28}), scheme 1 = P256 (128-byte `r‖s‖qx‖qy`)
- `recent_root_reference = [source_id, slot, root]`
- `sig_hash = keccak256(0x06 ‖ rlp(envelope with every empty-msg signature's bytes elided))`

### Frame modes

| Mode | Name | Semantics |
|------|------|-----------|
| 0 | DEFAULT | General call, caller = ENTRY_POINT (`0x…aa`) |
| 1 | VERIFY | Static validation frame — grants approval via `APPROVE` |
| 2 | SENDER | Executes as `tx.sender` (requires execution approval); only mode that may carry `value` |
| 3 | POST_TX | EIP-7906 trailing read-only assertion — a revert invalidates the whole transaction |

### Frame flags

- Bits 0–1: APPROVE scope restriction (`0x1` payment, `0x2` execution, `0x3` both)
- Bit 2 (`0x04`): atomic-batch member — the batch reverts together. Payment-scoped
  APPROVE is forbidden inside a batch.

## Send a Frame Transaction (Verified Scripts)

This directory ships the byte-exact encoder and a self-verified-transfer submitter
(validated against the repo golden vector and this devnet):

```bash
cd scripts/hegota-devnet
python3 -m venv .venv && .venv/bin/pip install "eth-hash[pycryptodome]" eth-keys

# Self-verified transfer: frame[0] VERIFY(target=sender, scope 0x3) + frame[1] SENDER(transfer)
.venv/bin/python3 frametx_submit.py \
  https://rpc1.hegota.ethrex.xyz \
  <YOUR_PRIVATE_KEY_HEX> \
  0xRecipientAddress \
  1000000000000000    # amount in wei
```

The receipt is type `0x6` with per-frame `frameReceipts` (status, gas, logs — ETH
transfers emit EIP-7708 logs from `0x…fffe`).

> **Inclusion tip:** frame-tx gossip between the devnet nodes is best-effort. If your
> transaction hasn't mined within ~30 s, submit the SAME raw transaction to the other
> two RPCs as well (idempotent — same hash).

## Sponsored Transactions (Trustless Paymaster)

A frame transaction can have its gas paid by a **distinct paymaster** contract
(`payer != sender`) — the canonical-paymaster `[only_verify, pay]` shape. The
exec frame targets the sender (who approves execution via the outer signature)
and the pay frame targets a paymaster contract that calls `APPROVE(scope=1)`.

`contracts/OpenSponsor.yul` is a minimal, observer-friendly sponsor: its
`verify()` just calls `APPROVE(APPROVE_PAYMENT)`, so it sponsors any sender. It
makes no external calls and reads no storage in the verify path, so it is
admissible via the public mempool (unlike a balance-gated sponsor whose external
`STATICCALL` the ERC-7562 validation observer would reject for a non-canonical
paymaster).

```bash
cd scripts/hegota-devnet

# 1. Compile the sponsor (Yul).
solc --strict-assembly --bin contracts/OpenSponsor.yul

# 2. Deploy it with the owner address appended as a 32-byte constructor arg, then
#    fund it with ETH (any tool that sends a plain type-2 tx works, e.g. cast):
#      initcode = <bin> || left-padded-32-byte owner
#    cast send --create <initcode> --private-key <OWNER_KEY> --rpc-url https://rpc1.hegota.ethrex.xyz
#    cast send <SPONSOR_ADDR> --value 1ether --private-key <OWNER_KEY> --rpc-url ...

# 3. Send a sponsored transfer. The sender needs only the transferred `value`
#    (not gas): a successful run with a gas-starved sender proves the sponsor paid.
.venv/bin/python3 frametx_sponsor_submit.py \
  https://rpc1.hegota.ethrex.xyz \
  <SENDER_PRIVATE_KEY_HEX> \
  0xSponsorAddress \
  0xRecipientAddress \
  1000000000000000    # amount in wei
```

The receipt's top-level `payer` field is the **sponsor**, not the sender, and the
sender's balance drops only by the transferred `value`. The withdrawal function
`withdraw(address,uint256)` (`0xf3fef3a3`, owner-only) reclaims the sponsor's ETH.

> A sender-restricted trustless sponsor (authorizing specific senders) can
> ecrecover an owner signature over a domain that **excludes** the signature
> — e.g. `keccak(sender ‖ chain_id ‖ nonce_seq ‖ expiry)`. Do **not** sign over
> `sig_hash` and carry the signature in frame data: `sig_hash` now commits frame
> data verbatim, so that construction is a circular (unsatisfiable) fixed point.

## EIP-8250: Keyed Nonces

The envelope carries `nonce_keys` (1–16 strictly-increasing u256 keys) and one
`nonce_seq` checked against every selected key. Key `0` is the account's regular
nonce; non-zero keys live in `NONCE_MANAGER` storage
(`slot = keccak256(pad32(sender) ‖ key)`), letting independent workflows send in
parallel without nonce races.

> **Mempool policy:** the public mempool admits only `nonce_keys == [0]`
> transactions (spec-permitted minimal policy). Non-zero-key transactions are valid
> at consensus and can be included by a block builder directly.

## EIP-7906: POST_TX Assertion Frames

Append `POST_TX` frames (mode 3) as a trailing suffix. They run read-only with
ENTRY_POINT as caller after the main body; if any of them reverts, the whole
transaction is invalidated — including the already-approved gas payment. `APPROVE`
is forbidden inside them. `TXTRACE`/`TXDIFF` let assertion code inspect the
transaction's own execution.

## EIP-8272: Recent Roots

**Write:** call `RECENT_ROOT_ADDRESS` (`0x…8272`) with exactly 64 bytes of calldata
(`salt ‖ root`) and zero value — from a frame or any contract call. The entry is
committed under `source_id = keccak256(pad32(caller) ‖ salt)` for the current slot
(cost: 22100 gas). Static contexts and `DELEGATECALL`/`CALLCODE` revert.

**Reference:** declare `[source_id, slot, root]` tuples in the envelope's
`recent_root_references`. Each must satisfy `1 ≤ current_slot − slot ≤ 8191` and
match the committed entry — an invalid or forged reference invalidates the
transaction (the mempool also rejects it at admission).

> **Current devnet limitation:** the consensus client does not yet deliver the
> EIP-7843 slot number to the execution layer, so writes land at slot 0 and
> references cannot validate end-to-end yet. Writes, forged-reference rejection,
> and all consensus rules are active; the full write→reference round trip will
> work once CL support lands.

## Divergences From the Draft Specs

The four EIPs are drafts with TBD sections; every convention ethrex adopted (opcode
bytes, predeploy addresses, `source_id` derivation, write gas, TXPARAM indices) is
documented with rationale in the repo:
[`docs/eip-8141.md`](../../docs/eip-8141.md) ·
[`docs/eip-8250.md`](../../docs/eip-8250.md) ·
[`docs/eip-8272.md`](../../docs/eip-8272.md) ·
[`docs/eip-7906.md`](../../docs/eip-7906.md)
