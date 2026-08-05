# EIP-8312 UTXO Frames — interactive demos

Interactive demos for [EIP-8312 "UTXO Frame"](https://github.com/nerolation/EIPs/blob/a5da3f608c6dfbf353bea264054d99fc164ab10c/EIPS/eip-8312.md)
and its companion [ethresear.ch post](https://ethresear.ch/t/native-utxos-on-ethereum/25368).

Lives here rather than at the repo root because live mode drives a real devnet, which
puts it in the same category as `frametx_submit.py` and `utxo_itest.py` beside it. See
[`../../docs/eip-8312.md`](../../../docs/eip-8312.md) for the implementation and
[`../UPGRADE-GUIDE.md`](../UPGRADE-GUIDE.md) for activating EIP-8312 on a devnet.

Two modes, same UI, same backend API:

- **Live** — every deposit, self-funded spend, multi-actor consolidation, and sponsored
  spend is a **real transaction** on a devnet with EIP-8312 activated (frame mode 5 on
  EIP-8141 frame transactions, vault predeploy at `0x…8312`), witnessed from on-chain
  logs and settled by the protocol itself.
- **Simulated** — an in-process engine reproduces the same state transitions instantly.
  It uses **pseudo-hashes, not keccak**, and is not driven by a node: treat it as an
  illustration of the mechanics, never as evidence about the implementation. Live mode
  is the one that proves anything.

## Run

```sh
cd scripts/hegota-devnet/utxo-demo

# live mode: point EIP8312_RPC at an EL whose chain has EIP-8312 activated. If the
# devnet is remote, forward its EL RPC port first, e.g.
#   ssh -L 8545:127.0.0.1:<el-rpc-port> -N -f <host>
# (see ../UPGRADE-GUIDE.md for activating EIP-8312 on a devnet).
EIP8312_LIVE=1 node server/server.js          # → http://localhost:8000

# simulated mode (no devnet needed)
node server/server.js
```

Env: `PORT` (default 8000), `EIP8312_RPC` (default `http://localhost:8545`),
`EIP8312_MASTER_KEY` (default: line 2 of `fixtures/keys/private_keys_l1.txt` in the repo root,
a genesis-funded account on the lambdaclass devnets).

Live demos advance forward-only (the chain is real; Reset starts a fresh scenario with
new keys). Each step waits for devnet blocks (~6 s slot).

## Test

```sh
node --test                                   # simulated-mode e2e over HTTP + engine unit tests
EIP8312_LIVE=1 EIP8312_LIVE_TEST=1 node --test test/devnet.e2e.test.mjs   # live e2e against the devnet
```

Requires Node 18+ and the Python venv (`python3 -m venv .venv && .venv/bin/pip install eth-account eth-keys eth-hash`).

## Architecture

```
shared/chain.js      Simulated protocol engine (vault, openings roots, spent bitfield,
                     VERIFY → APPROVAL → SETTLEMENT, conservation). Isomorphic JS.
shared/scripts.js    Demo scripts: setup/steps/captions/view, chain-implementation agnostic.
server/livechain.js  Chain-compatible adapter that performs every operation for real:
                     reads via JSON-RPC, writes via txforge.
devnet/txforge.py    Transaction forging for the PoC: vault deposits, witness building
                     from on-chain logs, spend-hash + EIP-8141 signature list, type-0x06
                     frame envelopes (self-funded vault-sender and sponsor-as-sender).
server/server.js     node:http static + JSON API; sim replay or forward-only live sessions.
js/                  Thin frontend: buttons call the API, rendering consumes payloads.
test/                Simulated-mode e2e (default) + gated live devnet e2e.
```

### API

- `GET  /api/demos` → `{ live, rpc, demos: [{ id, title, totalSteps }] }`
- `POST /api/demo/:id/goto` — `{ step: -1..N, flags: {} }` → full payload (caption, view,
  chain snapshot). Simulated mode replays deterministically to any step; live mode is
  forward-only. `flags.attack` on demo 4 removes the sponsor's repayment output.
- `POST /api/demo/5/scale` — `{ count }` → the scale-model numbers.

## Demos

1. **Pay someone who has nothing** — a zero-ETH recipient spends a self-funded UTXO (gas inside the conservation rule).
2. **Stealth payments** — ERC-5564-style fresh addresses, discovery by view tag, one multi-actor consolidation frame.
3. **Batch payroll** — 12 first-time recipients: UTXO cycle vs fresh-account transfers (gas + permanent state).
4. **Trustless sponsorship** — a sponsor co-signs the spend envelope and is repaid by a signed output; attack toggle included (full rejecting-sponsor path in simulated mode).
5. **State that never grows** — scale explorer from 1 to 1 B one-shot payments.
