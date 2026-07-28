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

## C. Findings from building the proof-of-concept scenarios

We built attack reproductions for the published PoC list against the live devnet
(see [POC-GUIDE.md](POC-GUIDE.md), results in [EVIDENCE.md](EVIDENCE.md)). Four
findings came out of it. The first two would each cause a PoC written the
intuitive way to silently demonstrate nothing, so they are the ones we would most
like reflected in the note.

### 1. `TXDIFF`'s before/after is intra-transaction, so the oracle-TOCTOU and proxy-swap PoCs need *absolute* assertions

`*_before` reads the transaction prestate and `*_after` the live post-body state,
so a key the transaction never wrote reads the **same value both ways**. But in
the oracle-TOCTOU and proxy/implementation-swap scenarios the adverse change
happens in a **prior** transaction. A differential assertion ("this slot did not
change") therefore passes while the attack succeeds.

Confirmed on-chain rather than argued: in scenario P6 the implementation was
swapped in an earlier block, and a differential guard over the implementation slot
**mined, with the victim's deposit diverted**. Only an absolute assertion
(`slot_after == value committed at signing time`) stopped it.

This implies a wallet-side requirement worth stating in the note: for those two
scenarios the signer must **capture and commit expected environment values at
signing time**. A guard that merely asks for a diff check cannot work. If that
pattern is expected to be common, a standard encoding for "expected environment"
may belong in the EIP rather than in each guard's ad-hoc calldata.

### 2. For the proxy PoC, asserting the proxy's code hash never detects an upgrade

An upgrade does not change a proxy's bytecode — only the storage slot holding the
implementation address. Also confirmed on-chain in P6: a guard asserting the
proxy's code hash **mined, with the deposit diverted**. The assertion has to
target the implementation slot (or the implementation address's code hash).

### 3. Guard provenance is unaddressed in the PoC list, and it is load-bearing for items 1-3

Every scenario in the note assumes the POST_TX assertion is present on the
victim's transaction. Under EIP-8141 whoever composes the transaction composes the
frame list — so in the threat models items 1-3 describe (a phishing frontend, a
compromised Safe interface) the adversary simply ships a **guardless** frame
transaction, or one whose guard truthfully blesses the drain, and the honest
wallet signs it. Item 2 in particular, as written, is prevented only if the
assertion cannot be omitted.

**The good news: this is closable today with primitives EIP-8141 already ships, and
needs no spec change.** A VERIFY frame can read `TXPARAM(0x09)` for the frame
count, `FRAMEPARAM` for each frame's mode and resolved target, and
`FRAMEDATACOPY` for its calldata; and a reverting VERIFY frame invalidates the
transaction. An account can therefore *require* a correctly parameterized POST_TX
assertion and refuse to transact without one.

We built that account and ran six attempts at the same malicious intent: an
honest transaction mined, while guard-omitted, guard-substituted-with-a-no-op,
guard-present-but-weakened, and non-owner-signed were all refused before the body
ran; with the correct guard present the assertion itself fired. The VERIFY frame
reads only its own storage and makes no external calls, so it stayed admissible
through the public mempool under the ERC-7562 validation observer.

We would suggest the note say explicitly whether the guard is expected to be
attached by the wallet or mandated by the account, because the two give very
different security properties.

### 4. A POST_TX-reverting transaction is admitted and then silently never mined — wallets get no failure signal

Mempool validation simulates only the validation prefix, and POST_TX is body,
never prefix. So a transaction whose assertion will fire is **accepted by
`eth_sendRawTransaction`**, never mined, produces no receipt, and costs no gas.
From the submitter's side it is indistinguishable from a transaction that was
simply dropped.

Two consequences:

- **Diagnostics misattribute the cause.** `ethrex_simulateFrameTransaction`
  reports `VERIFY frame did not call APPROVE or payer not approved` with
  `frames: null` for a POST_TX revert, even when the VERIFY frame is byte-identical
  to one that succeeds — the revert rolls back the APPROVE and the error surfaces
  against the wrong frame. A developer would debug the wrong frame, and simulation
  cannot be used to identify *which* assertion failed. We are treating this as an
  ethrex-side diagnostic bug, but the underlying ambiguity is worth a spec note on
  what a client should report.
- **Mandating the guard at the account has strictly better failure UX**, which is a
  second, independent argument for finding 3. Because VERIFY frames *are* in the
  simulated prefix, the account's refusals came back as immediate rejections at
  submission, rather than a silent non-inclusion.

### 5. Block building and simulation disagree on the transaction's storage-change set, which makes deny-by-default assertions unusable

This is the finding we would flag most urgently, because it silently breaks one of the
natural assertion shapes.

For a frame transaction whose body performs a single ERC-20 transfer,
`ethrex_simulateFrameTransaction` reports **2** storage-slot changes via
`TXTRACE(0x01)`, while **block building observes 5**. Measured directly: a probe
guard asserting an exact count was submitted for counts 2, 3, 4 and 5; simulation
accepted only the assertion of 2, and only the assertion of **5** was ever
included in a block.

The consequence for assertion authors is severe. A deny-by-default guard — "every
slot this transaction wrote must be one I expected", which is the natural form
whenever the adversarial slot is a mapping entry keyed by an attacker-chosen
address and so cannot be enumerated in advance — **passes simulation and then
reverts during block building**. Combined with finding 4, the transaction is
admitted, silently dropped, and never mined. Worse, it *looks* like the assertion
worked: the malicious transaction did not land. Only a negative control reveals
that the identical guard also rejects the honest transaction.

We found this exactly that way. An earlier version of our hidden-side-effect
scenario used a deny-by-default guard and appeared to defend correctly; its
negative control showed it was rejecting every transaction, including the
legitimate one. The scenario now uses a targeted assertion over a named slot
instead, at the cost of having to know the exfiltration address in advance.

Two asks: (a) treat the divergence as a bug to reconcile, so that what an
assertion observes during block building is what simulation reports; and (b)
consider saying in the EIP which state changes are in scope for the transaction
diff — the frame-transaction machinery's own writes (keyed nonces, payer
accounting, predeploy bookkeeping) are plausibly the extra three, and whether
assertion authors should see them at all is a specification question, not just an
implementation one.

### 6. Suggested additional PoC: allowance *elimination*, not just detection

Item 1 catches a malicious `approve(MAX_UINT)` the user never intended. It cannot
help the victims of the arbitrary-external-call router drains (two aggregators in
January 2026, ~$13.4M and ~$3.67M; a helper contract in 2025, ~$5M) — those users
**intended** their approvals and were drained weeks later through them.

That class needs the allowance not to survive the transaction at all: one frame
transaction bundling `approve(exact)`, the use, and a POST_TX assertion that the
allowance slot is zero afterwards. We built it as scenario P7: the attacker's
identical drain then finds nothing to take. Worth noting honestly that this does
*not* revert the attacker's transaction — it removes the surface — and that
detection and elimination together cover the class where either alone leaves half
of it exposed.

### Scope note on where EIP-7906 does and does not reach

We classified 81 documented 2025-2026 incidents against "would a POST_TX assertion
have prevented this". 72 were not defendable: attacker-signed protocol-logic bugs
(reentrancy, oracle manipulation, arithmetic, access control), key compromise, and
bridge/cross-chain message forgery — all cases where the losing transaction is
signed by the attacker or by a legitimate key holder, so a sender-authored
postcondition cannot reach it. We think that is a feature worth stating plainly in
the EIP: it defends transaction-intent integrity precisely, and a narrower claim is
much harder to refute than a broad one.
