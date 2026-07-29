# EIP-7906 on the Hegotá Devnet — Notes for the Spec Author

ethrex runs a public devnet integrating EIP-7906 with EIP-8141 (frame
transactions), EIP-8250 (keyed nonces), and EIP-8272 (recent roots). This note
summarizes where our 7906 implementation diverges from the draft, what
integrating it with the rest of the family required, and the spec questions we
would most like a ruling on.

- **Spec read:** `eips.ethereum.org` master as of 2026-06-24, including the two
  merged updates PR #11829 (POST_TX frame mode) and PR #11830 (TXDIFF).
- **Implementation:** ethrex branch `hegota-devnet`; detailed notes in
  [`docs/eip-7906.md`](../../docs/eip-7906.md).
- **Try it live:** endpoints, faucet, and a working frame-tx submitter are in
  the [USER-GUIDE](USER-GUIDE.md).

## A. Divergences from the 7906 draft

### 1. Opcode bytes renumbered on the integrated devnet

The draft assigns `TXTRACE / EVENTDATACOPY / TXDIFF = 0xB5 / 0xB6 / 0xB7`. On
the integrated devnet they ship at **`0xB6 / 0xB7 / 0xB8`**.

Cause: a collision cascade inside the 8141 family. EIP-8272's Constants table
assigns `RECENTROOTREFLOAD = 0xB4`, which collides with EIP-8141's shipped
`SIGPARAM = 0xB4`; ethrex moved `RECENTROOTREFLOAD` to the next free byte
`0xB5`, which displaces 7906's three opcodes by one. Our standalone
`eip-7906` branch (no 8272) keeps the spec bytes — the renumber exists only
where the EIPs coexist.

**Ask:** an authoritative opcode (and TXPARAM-index) registry for the
8141-family EIPs, so implementations stop colliding pairwise.

### 2. Provisional gas values

- `TXTRACE = 100` — the EIP's own example value.
- `TXDIFF = 2100` — PR #11830 marks the cost TBD; ethrex prices the keyed
  before/after lookup as a cold `SLOAD`, since it may touch a cold
  account/slot.

### 3. TXDIFF "after" reads the live post-body state

"Before" reads the transaction prestate; "after" reads the live state through
the execution diff caches rather than a separately materialized post-tx
snapshot. Because POST_TX frames are the trailing suffix, the live state *is*
the post-body state, so this is equivalent to the spec's intent — flagged for
cross-client confirmation of edge cases. TXDIFF reads deliberately do **not**
trigger EIP-2929 warm/cold accounting.

### 4. Whole-body revert is implemented as exclusion — underspecified

A reverted POST_TX frame invalidates the transaction through the same path as
a reverted VERIFY frame: the transaction is **excluded from the block** and
the approved gas payment is fully rolled back. Internally consistent, but the
draft leaves three things open that we'd most like a ruling on:

1. **Receipt representation.** Exclude entirely (our choice: no receipt, not
   in the block body) vs. include-but-mark-reverted (a status-0 receipt
   occupying a block slot). The two disagree on the receipts root, so this is
   consensus-relevant across clients.
2. **Validation-prefix payment interaction.** The spec also describes the
   validation prefix as "not reverted in a mempool-compatible way", which is
   in tension with rolling back the approved payment on POST_TX revert (we
   roll it back — the payer pays nothing).
3. **Anti-DoS.** With exclusion + full payment rollback, a block builder bears
   the execution cost of POST_TX-reverting transactions with no compensation.

## B. What integration with 8141 / 8250 / 8272 required

### 1. Frame-mode namespace

`POST_TX = 3` in the shared 8141 frame-mode enum; static validation admits
modes `0..=3` (mode 4 is reserved for the deferred EIP-8288). Enforced at
consensus: POST_TX frames must form a **contiguous trailing suffix**, and they
are rejected from the 8141 mempool validation prefix (they are body, never
prefix).

### 2. APPROVE is explicitly forbidden inside POST_TX

`APPROVE` inside a POST_TX frame exceptional-halts. The gate is on the
**POST_TX mode specifically, not on staticness** — VERIFY frames are also
static, and `APPROVE` is precisely how they grant authorization, so a naive
"no APPROVE in static context" rule would break 8141. Found in our second
implementation audit.

**Ask:** state the APPROVE prohibition explicitly in the spec rather than
implying it via "no state manipulation".

### 3. POST_TX revert vs. atomic-batch revert

8141/8250 atomic batches revert *as a batch* while the transaction survives; a
POST_TX revert is a *transaction-level* failure. The two revert sites are
distinct in the implementation, and a POST_TX revert overrides any atomic
batch unrolling that preceded it.

### 4. Whole-body revert must unwind the other EIPs' effects

Exclusion rolls back EIP-8250 keyed-nonce consumption and the EIP-8141
payment through the transaction-level backup: the nonce is **not** consumed
and the transaction is replayable — consistent with "invalidates the entire
transaction, including any gas payment already approved".

### 5. TXTRACE gas-pre-charge (`0x14`) reports the 8141 maximum cost

EIP-8141 requires APPROVE to collect the transaction's **maximum** cost
(`max_fee_per_gas × total_gas_limit` plus max-rate blob cost). We aligned
TXTRACE's `0x14` with that: it reports the same quantity as `TXPARAM(0x06)`,
the actual APPROVE debit, and the mempool paymaster reservation — one
definition of "pre-charge" everywhere.

**Ask:** once 8141's fee rule is pinned, specify that `0x14` means "the amount
actually debited from the payer at approval".

### 6. 8272 interplay comes free from staticness

Assertion code **can** read verified recent roots (`RECENTROOTREFLOAD` over
the signed envelope references) and **cannot** write them — a call from a
POST_TX subtree to `RECENT_ROOT_ADDRESS` fails because the frame is static.
Assertions over recent-root commitments therefore need no extra rules.

### 7. EIP-8037 (two-dimensional gas) interplay

Frames are gas-isolated: the state-gas reservoir/spill is captured at frame
entry and reset afterwards, so a state-gas refund earned in a body frame
cannot subsidize a POST_TX frame's charges. One question remains open in
[`docs/eip-7906.md`](../../docs/eip-7906.md): whether frame transactions
participate in the 8037 intrinsic state-gas split at all. Until pinned, budget
POST_TX frames generously.

## Validation status

Verified end-to-end on the live public devnet (through the public RPC
endpoints): POST_TX frames execute (multi-frame transaction, all frames
succeed) and a reverting POST_TX excludes the whole transaction with the body
transfer rolled back. The APPROVE-in-POST_TX rejection and the
trailing-suffix structural rule are covered by unit tests
(`test/tests/levm/eip7906_tests.rs` and the frame-tx suites).
