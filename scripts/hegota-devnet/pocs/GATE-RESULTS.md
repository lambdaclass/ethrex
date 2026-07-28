# Phase-0 gate results

Three assumptions had to hold before building the assertion-guard library and the
attack scenarios. All three were verified against the live public devnet rather than
argued from the specification. Each gate records what was tested, what happened, and
what it means for the design.

## Gate 0.1 — devnet health

`chainId 3151908`, head advancing, and all three execution clients agreeing on the head
hash. Verified before any deployment.

## Gate 0.2a — can assertion guards be written in Solidity?

**Why it mattered.** Solidity cannot emit the EIP-7906 opcodes: `verbatim_*` is available
only in pure Yul, not inside a Solidity `assembly { }` block. Verified on solc 0.8.31
against both the legacy and `--via-ir` pipelines — both reject it. Without a workaround the
entire guard library would have to be hand-written Yul.

**What was tested.** A minimal Yul shim (`contracts/TxIntrospection.yul`) exposes the three
introspection opcodes as ABI functions. A Solidity guard (`contracts/guards/ShimProbe.sol`)
reads through it via `STATICCALL` from inside a POST_TX frame. Two complementary assertions
were run against a value-moving transaction:

| Assertion | Expected | Result |
|---|---|---|
| `assertBalanceChangesAtLeast(shim, 1)` | mines | **mined** |
| `assertBalanceChangesEquals(shim, 0)` | does not mine | **not mined, body rolled back** |

**Verdict: PASSED.** A broken shim, or an opcode gate that did not survive the
`STATICCALL`, could not produce that pair — it would fail or halt both. The gate is
confirmed to hold across the POST_TX call subtree, so guards are written in Solidity and
raw opcodes are confined to the ~40-line shim.

## Gate 0.3 / 0.4 — the evidence model

**Why it mattered.** A POST_TX-reverting transaction is *excluded*, so there is no receipt
and no explorer page for a successful defense. The evidence channel had to be established
before any scenario relied on it.

**What was tested.** The same frame transaction with an always-pass POST_TX frame, then an
always-revert one.

| POST_TX frame | Submission | Mined | Body landed |
|---|---|---|---|
| always pass (`STOP`) | accepted | yes, status `0x1` | **yes** |
| always revert | **accepted** | **never** | **no** |

**Verdict: PASSED**, with two findings:

1. **A POST_TX-reverting transaction is admitted to the mempool and then silently never
   mined** — no receipt, no error, no gas charged. The submitter gets no failure signal.
   Mempool validation simulates only the validation prefix, and POST_TX is body, never
   prefix, so nothing rejects it up front.
2. **Simulation misattributes the cause.** `ethrex_simulateFrameTransaction` reports
   `VERIFY frame did not call APPROVE or payer not approved` and `frames: null` for a
   POST_TX revert — even though the VERIFY frame is byte-identical to one that succeeds.
   The POST_TX revert invalidates the transaction, which rolls back the APPROVE, and the
   error surfaces against the wrong frame. A developer would debug the wrong frame, and
   simulation cannot be used to identify *which* assertion tripped.

Consequence for the design: which assertion failed is established by local `forge` tests
against a mocked introspection source, not by devnet simulation. On-chain evidence is the
admitted-but-never-mined outcome plus unchanged victim state.

## Gate 0.5 — account-mandated guards, and mempool admissibility

**Why it mattered.** Every published proof-of-concept assumes the POST_TX guard is present.
Under EIP-8141 the transaction composer controls the frame list, so a hostile composer can
simply omit it. If an account could not *enforce* guard presence, the intent-integrity
scenarios would be voluntary rather than real defenses.

**What was tested.** `contracts/accounts/GuardMandatingAccount.yul` verifies, inside its
VERIFY frame, that the final frame is POST_TX, targets the configured guard, and carries
exactly the committed calldata — reverting otherwise. Five transactions from that account:

| Case | Expected | Result |
|---|---|---|
| A correctly guarded | mines | **mined** |
| B guard omitted | invalid | **rejected** |
| C guard substituted with a no-op | invalid | **rejected** |
| D genuine guard, weakened parameters | invalid | **rejected** |
| E non-owner signature | invalid | **rejected** |

**Verdict: PASSED, 5/5.** Guard provenance is closable on-chain with primitives EIP-8141
already ships — `TXPARAM(0x09)`, `FRAMEPARAM`, `FRAMEDATACOPY`, and a reverting VERIFY
frame. No specification change is required. The VERIFY frame reads only its own storage and
makes no external calls, so it stayed admissible through the public mempool.

**Third finding — the account-mandated design has strictly better failure UX.** Cases B–E
were rejected **at mempool admission** with an immediate error, because VERIFY frames run in
the validation prefix that admission simulates. Compare gate 0.3, where a bare POST_TX
revert was admitted and silently dropped. So mandating the guard at the account buys not
only unstrippability but an actionable rejection at submission time.

## Incidental finding — EIP-8037 state gas inflates plain transactions too

`setPolicy` performs two cold `SSTORE`s and required **232,455** gas by
`eth_estimateGas`, against a naive 200,000 budget that failed. The frame-level state-gas
over-billing documented in `docs/eip-7906.md` has a counterpart on the ordinary
transaction path: any storage-writing call needs a generous budget. Scenario helpers
budget accordingly and record why at each call site.

## Later finding — block building and simulation disagree on the storage-change set

Found by a negative control while building the hidden-side-effect scenario, not by a gate,
but it belongs with the gate results because it constrains which assertion shapes are usable.

For a frame transaction whose body performs one ERC-20 transfer, `TXTRACE(0x01)` reports **2**
storage-slot changes under `ethrex_simulateFrameTransaction` and **5** during block building.
Measured with a probe guard asserting an exact count for 2, 3, 4 and 5: simulation accepted
only the assertion of 2, and only the assertion of **5** was ever included in a block.

**Consequence.** Deny-by-default assertions over the slot-change enumeration — the natural
form when the adversarial slot is a mapping entry keyed by an attacker-chosen address — pass
simulation and then revert during block building. Combined with the admitted-then-silently-
dropped behaviour above, such a guard *appears* to work: the malicious transaction does not
land. Only a negative control shows that the identical guard also rejects the honest
transaction, which means it is not a defense at all.

Until the two paths agree, only targeted assertions over explicitly named slots are
dependable. This is why the hidden-side-effect scenario asserts a single named balance slot
rather than enumerating everything the transaction wrote.
