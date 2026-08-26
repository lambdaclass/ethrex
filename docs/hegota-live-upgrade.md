# Upgrading the live Hegotá devnet to the two-dimensional frame format

`spec/hegota-eips-refresh` brings the frame-tx EIPs up to their current specs. One of
those changes — EIP-8141 replacing a frame's scalar `gas_limit` with
`limits = [execution, state]` — is a wire-format break, so the upgrade procedure every
previous devnet change used does not apply. This is the decision and the runbook.

## Why the usual procedure does not work

The in-place binary swap (`docs/`, "in-place binary swap") assumes a node can read the
history already in its datadir. Measured against a **copy** of the live datadir, with the
live node untouched:

1. The new binary **starts fine** — opens the datadir, loads chain 3151908, serves the head.
2. It **cannot decode any block containing a frame tx**:
   `Error decoding field 'transactions' … 'frames' … 'limits': UnexpectedString`.
   That is at block-body level, so every such block is unreadable — and the demo has been
   submitting frame transactions throughout the chain's life.
3. A **format-tolerant decoder** — RLP distinguishes a list from a string by its prefix
   byte, so slot 3 can accept both forms with no fork gate and no block context, in about
   fifteen lines — makes those blocks readable again and returns their frames correctly
   (`executionGasLimit: 0x13880`, `stateGasLimit: 0x0`).
   **But the transaction hash comes back wrong**: querying `0x7dcb84…` returns
   `0x982949…`. `Transaction::hash()` is `keccak256(encode_canonical_to_vec())`, so the
   node re-derives the hash from a re-encoding, and the re-encoding is the new format.

The third point is the load-bearing one. A tolerant decoder alone yields a node that
serves plausible-looking history under wrong identities, which is worse than failing
loudly. Correct hashes require the *encoder* to reproduce each historical frame's original
bytes, which means a per-frame format discriminator living permanently on the consensus
hash path — and `transactions_root` is a trie over the same encoding, so it inherits the
problem.

## Confirmed fixed

The fork gate (`spec/hegota-eips-refresh`, `52fb06427` + `1c496e8ae`) closes exactly this
gap. Re-ran the same measurement against a **fresh copy** of the live datadir (2.1 GB,
copied from the running `el-1` container's volume; the live enclave was never stopped or
touched):

1. The fork-gated binary opens the copy and serves the head (289,869) — as before.
2. `eth_getTransactionByHash` on the same historical frame tx now returns
   **`hash: 0x7dcb84…`, matching the query exactly.** This is the assertion the
   tolerant-decoder prototype failed on.
3. `eth_getBlockByNumber` on its block decodes, is fully populated, and the tx appears in
   its `transactions` list.
4. `eth_getTransactionReceipt` reads back the frame receipts (3 frames, logs intact) with
   no decode error, exercising the second wire surface (`FrameReceipt`'s own encoding
   marker).

Not yet run: a full re-sync from genesis (acceptance-test item 4 in the fork plan), which
is the only check that exercises the *execution-side* era gates (intrinsic repricing, the
per-frame state pool, receipt production) rather than just decode/encode fidelity. That is
the remaining pre-flight step before scheduling `frame_limits_time` on the live devnet —
expensive (hours, ~290k blocks) and not done as part of this check.

## Decision: ship it as a fork

The options below were written before that was on the table. **The chosen path is a fork** —
gate the new encoding on a new activation timestamp, so pre-fork blocks keep the old encoding
and their hashes, and post-fork blocks use the new one. It preserves the chain and its
history, needs no exception to the never-re-genesis rule (new-fork decoupling is one of the
techniques that rule allows, and EIP-8312 shipped exactly this way), and is cheap here
because this is an ethrex-only devnet with no cross-client coordination.

It is not option C. C carried both formats forever with no way to tell which era a block
belonged to; a fork makes the era explicit and bounded, which is how every client already
handles a wire change. The design is in the frame-limits fork plan.

The three options below are kept for the reasoning behind them, and because the measurements
in the previous section are what ruled out the naive paths.

## The three options (superseded)

### A. Promote (recommended)

Make the office-2 chain the public devnet; keep the current chain running read-only as an
archive. Nothing is destroyed, so this needs no exception to the never-re-genesis rule —
the old chain is retired, not wiped. Costs: a new genesis hash and chain history for
anyone already integrated, and DNS/endpoint churn.

1. Confirm the office-2 enclave is healthy: producing, finalizing, all ELs at one head.
2. Repoint `rpc1/2/3.hegota.ethrex.xyz` and `dora.hegota.ethrex.xyz` at office-2's
   published ports. The Caddyfile on the current host hardcodes kurtosis ports that
   re-stale on every enclave restart — see the devnet caddy notes before editing.
3. Put the current enclave into read-only service (stop the validator clients, leave the
   ELs serving RPC) and announce it as the archive endpoint.
4. Re-deploy the demo contracts against the new chain and rebuild the demo frontend with
   the new addresses; its addresses are build-time Vite env vars.

### B. Re-genesis

Wipe the live enclave and relaunch it on the new branch, keeping the name and endpoints.
Simplest for consumers — same URLs, same chain id — but it destroys 288k blocks of history
and **requires explicitly relaxing the never-re-genesis rule**, whose premise (that our
upgrades can always be made state-preserving) upstream broke by re-architecting the EIP.

1. Announce the wipe and a cutover time; the demo and the EIP authors are active users.
2. `kurtosis enclave rm hegota --force` on the devnet host.
3. Relaunch from `fixtures/networks/hegota-devnet.yaml` with `ethrex:local` built from this
   branch. **The CL image must be the `focil` build** — `glamsterdam-devnet-7` speaks only
   `engine_forkchoiceUpdatedV4` and the chain halts the instant heze activates.
4. Re-deploy the demo contracts and rebuild its frontend, as in A.

### C. Dual-format encoder

Preserve the live chain and its history by teaching the codec both formats. Only option
that keeps hashes intact. Cost is permanent: a format discriminator on `Frame`, honoured by
`RLPEncode`, on the path that computes transaction hashes and the transactions root — a
dead format carried in consensus code indefinitely, for a devnet exercising a draft EIP.

1. Add the format marker to `Frame`, set by the tolerant decoder described above.
2. Make `RLPEncode for Frame` reproduce the original encoding when the marker says old.
3. Prove it: re-run the measurement above and require the queried and returned hashes to
   match, then re-validate a historical block's `transactions_root`.
4. Only then perform the ordinary in-place binary swap.

## Recommendation (superseded)

A was the recommendation while the choice was between these three: it cost nothing
irreversible and needed no exception to a standing rule. The fork approach above is better
than all of them — it keeps the chain, keeps the history, keeps the endpoints, and bounds the
dual-format handling to an explicit era boundary rather than carrying it indefinitely.
