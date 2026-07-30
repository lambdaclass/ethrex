# EIP-7906 on the Hegotá Devnet — Notes for the Spec Author

ethrex runs a public devnet integrating EIP-7906 with EIP-8141 (frame
transactions), EIP-8250 (keyed nonces), and EIP-8272 (recent roots). This note
summarizes where our 7906 implementation diverges from the draft, what
integrating it with the rest of the family required, and the spec questions we
would most like a ruling on.

- **Spec read:** `eips.ethereum.org` master as of 2026-07-28, including the
  merged updates PR #11829 (POST_TX frame mode) and PR #11830 (TXDIFF), the
  2026-06-29 TXDIFF pricing revision, the 2026-07-06 POST_TX revert revision and
  the 2026-07-09 per-address view params.
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

### 2. Provisional gas values, and fork-aware EIP-2929 costs

- `TXTRACE = 100` — the EIP's own example value; it doubles as the flat cost of
  TXDIFF params `0x06`-`0x0A`.
- TXDIFF params `0x00`-`0x05` are priced through the EIP-2929 access lists as the
  spec's 2026-06-29 revision requires, but with the **fork-aware** cold/warm
  costs rather than the literal `2100` / `2600` / `100`. EIP-8038 reprices cold
  state access at Amsterdam and Hegotá is post-Amsterdam, so the hardcoded
  numbers would make TXDIFF a strictly cheaper cold-state read than `SLOAD`.
  Pre-Amsterdam the two agree exactly.

  **Ask:** express the cost as the fork's cold/warm access cost rather than as
  fixed integers.

### 3. "before" needs a defined granularity — this one bit us

The spec says "before" is the value prior to the transaction, and leaves the
sourcing to implementations. That is a trap, because a client is likely to
already have a cache that looks like a prestate and is not one. In ethrex the
obvious candidate held the *block* prestate on the building path, a
flush-boundary prestate on the sequential import path, and nothing at all on the
concurrent path that re-executes transactions to validate the EIP-7928 Block
Access List — where every `*_before` read then silently fell back to the live
value, so `before == after` always.

The failure is not a wrong answer, it is a **path-dependent** answer. An
assertion that branches on its own diff takes different branches while a block is
built and while it is validated; gas diverges, the BAL no longer matches, the
block is rejected by its own producer, and block production stops. This halted
our public devnet for two hours with nothing logged above `WARN`.

**Asks:**
- State explicitly that "before" is the transaction prestate, and that it must be
  identical under sequential and concurrent re-execution.
- Add a test vector: a POST_TX frame that branches on `slot_before != slot_after`
  for a slot its own body wrote. Any client sourcing "before" from a block-scoped
  or BAL-seeded cache fails it, and the failure is invisible to single-path tests.

### 4. Partial revert: implemented per the 2026-07-06 revision

A reverted POST_TX frame reverts the execution **body** while the transaction
stays valid: it is included, reports `status = 0`, the validation prefix (notably
the APPROVE gas payment) stays committed, and the payer is charged. Three
sub-questions remain open, all consensus-relevant cross-client:

1. **Gas on retroactively-unwound body frames.** EIP-8141 charges a *failed*
   frame its full `gas_limit`, but a POST_TX revert unwinds frames that
   *succeeded*. We charge actual consumption, reading "the payer is fully charged
   for the gas consumed up to the point of the revert" literally.
2. **EIP-8037 state gas on a reverted body.** We drop it, mirroring the
   atomic-batch unroll: the body was unrolled, so it grew no state.
3. **Whether the remaining POST_TX frames in the suffix still execute** once one
   reverts. We stop at the first. They are read-only, so it cannot change state,
   but it changes gas.

**Ask:** the Security Considerations sentence "unconditionally invalidates the
entire transaction, including any gas payment already approved" predates the
07-06 revision and still contradicts the normative text. It is the most likely
cause of a client implementing exclusion, which is what we did first — and under
exclusion the anti-DoS hole the revision names is wide open, since the same
transaction can be resubmitted indefinitely at the builder's expense.

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

### 4. The partial revert cuts across the other EIPs' write boundaries

`consume_keyed_nonces` (EIP-8250) runs inside the APPROVE handler, i.e. in the
validation prefix, so committing the prefix means the keyed nonce **is** spent on
a POST_TX revert and the transaction is not replayable. That is what closes the
DoS: under the earlier exclude-the-transaction reading the nonce was rolled back
and the same transaction could be resubmitted indefinitely, costing a builder a
full execution each time.

**Ask:** the safety argument for committing the prefix ("the wallet controls the
validation prefix, so it is inherently safe to commit it") is stated in EIP-8141
terms only. It does not obviously extend to what EIP-8250 / 8272 / 8037 write
across that boundary. A ruling on which of those writes survive a POST_TX revert
would settle it.

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

Verified end-to-end on the live public devnet, on a three-client chain with
concurrent BAL validation enabled:

- A POST_TX frame executes and its assertions hold: a multi-frame transaction
  whose body writes a storage slot and whose POST_TX frame reads `slot_before` /
  `slot_after` across that write is included with every frame succeeding, and the
  body's write is committed.
- A reverting POST_TX frame reverts the body only: the transaction is included
  with `status = 0`, the body's storage write is back at its prestate value, and
  the payer is charged.
- The prestate is path-independent — the same transaction produces identical
  results and identical gas whether executed sequentially or re-executed
  concurrently for BAL validation. This is the property whose absence halted the
  chain (§A.3).

The APPROVE-in-POST_TX rejection, the trailing-suffix structural rule and the
per-address views are covered by tests (`test/tests/levm/eip7906_tests.rs` and the
frame-tx suites).
