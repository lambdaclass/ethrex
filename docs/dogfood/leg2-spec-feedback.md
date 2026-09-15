# Leg 2 spec feedback: implementing Profile 2 from `docs/eip-focil-frametx.md`

This log records every place the revised specification left an implementer
working from the text alone with a decision to make, a contradiction with the
base code, or a rule that turned out unnecessary. Each entry names the section
it concerns, what had to be known, what the text says or fails to say, what was
decided, and how the text could say it. The final section lists the parts that
were clear enough to implement verbatim.

The base this leg started from carried EIP-8141 frame transactions with the
mempool validation-prefix simulation and its observer, EIP-8250 keyed nonces,
EIP-8272 recent roots, and EIP-7805 Profile 1 (builder, validator, engine
endpoints), and deliberately no enforcement of frame-transaction omissions.
Everything below was written before, during, or right after implementing the
enforcement on that base. No other implementation was consulted.

## Gaps and decisions

### 1. `protocol_verifier_frames(tx)` is used but never defined

Section: Profile 2 candidates, `prefix_and_verifier_frame_cost`.

Needed: which frames count as protocol verifier frames for pricing when one of
them is not in its required position.

Text: the pseudocode sums `f.limits.execution for f in protocol_verifier_frames(tx)`
and the prose says protocol verifier frames "both precede the prefix". Nothing
says whether an expiry-shaped or recent-root-shaped frame that sits elsewhere
(condition 4 fails, or it is a body frame) is one of the frames this function
returns. The occurrence is still priced, because `shape_of(validation_prefix(tx))`
can succeed with such a frame in the body, and "otherwise: charged, not
admitted" applies to it.

Decided: `protocol_verifier_frames(tx)` returns only the frames in the required
positions: an expiry verifier frame at index 0, and a recent-root verifier frame
at index 0 or at index 1 behind an expiry frame. A frame with either shape
anywhere else is a body frame and is not priced.

Suggested text: define the function next to the pseudocode: "the expiry
verifier frame when it is the first frame, and the recent-root verifier frame
when it is the first frame or the second behind an expiry verifier frame; a
frame with either shape in any other position is a body frame for every rule in
this document, including pricing."

### 2. Whether a validation-surface entry is a protocol read or an `SLOAD` permission

Section: Validation surface, the keyed-nonce bullet.

Needed: whether prefix code may `SLOAD` the NONCE_MANAGER slot of
`(tx.sender, nonce_key)`, or whether the entry only describes what the protocol
reads for eligibility condition 2.

Text: the surface lists "EIP-8250 keyed nonce state at `(tx.sender, nonce_key)`
for every nonzero key" as state "replay may read", but the paragraph after it
says a storage read is owned by "the account whose storage the executing
context reads", which for NONCE_MANAGER storage is a third account. The
recent-root bullet resolves the same question explicitly ("readable only while
that frame executes `RECENT_ROOT_CODE` at the top level"); the keyed-nonce
bullet does not.

Decided: permit an `SLOAD` of exactly those NONCE_MANAGER slots, because the
normative sentence is "Replay may read the following state and no other" and
that state is in the list. In this base the permission is unreachable: the
NONCE_MANAGER predeploy's runtime is a bare `REVERT`, so no code can execute
with its storage as context. The protocol reads the keyed nonces natively before
any frame runs.

Suggested text: say for the keyed-nonce bullet, as for the recent-root one,
whether it is read by the protocol only (condition 2) or also readable by
prefix code, and under what context.

### 3. Which opcodes "load" a code body for the code bound

Section: Code bound; Validation surface, last bullet.

Needed: whether `EXTCODESIZE` and `EXTCODEHASH`, which resolve an account's
code hash or length without executing the body, count as loading a body.

Text: "Replay MUST count each distinct `codeHash` once and sum each distinct
code body's byte length once" and the surface names "code and `codeHash` of
every account reached during validation". Neither says what "reached" or
"loaded" means at the opcode level.

Decided: charge on every code resolution: frame target dispatch (and its
EIP-7702 delegate), every `CALL`-family target (and its delegate), and every
`EXTCODESIZE`/`EXTCODECOPY`/`EXTCODEHASH` target. This is the wider reading; it
excuses more, which is the direction the document says every ambiguity resolves
toward, but a client charging only executed bodies would reach a different
verdict on a prefix that `EXTCODESIZE`s seventeen accounts.

Suggested text: name the opcodes and the frame dispatch as the load points, and
say that a delegation indicator is a body of its own (the Rationale implies it
through "eight bodies", the normative text does not).

### 4. Whether the load that trips the code bound is itself charged

Section: Code bound.

Needed: the budget state after a replay exceeds it, since later replays of the
same list continue from it.

Text: "A replay whose next load would exceed the budget makes its transaction
ineligible for that list; no further code is charged for that replay" and
"Charges survive the verdict". "Would exceed" suggests the offending load is not
charged; "survive" is about the earlier ones.

Decided: the offending load is not charged, the earlier ones stay, and the
budget is otherwise unchanged for the next replay. The `exhausted` flag is per
replay and cleared before each one.

Suggested text: "The load that would exceed the budget is not charged."

### 5. Budget charges made by a replay that turns out undecidable

Section: Code bound; Omission check, "A verdict that cannot be computed".

Needed: whether bodies loaded before an internal failure (for example a code
body missing from the database part-way through) stay charged.

Text: charges "survive the verdict"; an undecidable replay produces "not a
verdict". Nothing says which rule wins.

Decided: charges made before the failure stay, on the same reasoning the text
gives for failed replays (every attester read them). A replay that fails in the
pre-frame checks charges nothing, since nothing ran.

Suggested text: one sentence in the undecidable paragraph saying whether the
code budget keeps what an aborted replay loaded.

### 6. Replaying `S_start` after an undecidable `S_end`

Section: Omission check.

Needed: whether "only if the transaction is not eligible at `S_end`" covers an
`S_end` that could not be decided.

Text: the `or` short-circuits on eligibility only, and cases 27 and 28 are
consistent with replaying `S_start` after an undecidable `S_end`, but the
sentence about a replay "run when this order skips it" made me stop and check
that an undecidable `S_end` does not count as a skip.

Decided: undecidable is "not eligible", so `S_start` is replayed. Confirmed
against cases 27 and 28.

Suggested text: "not eligible, or not decided, at `S_end`".

### 7. The judging order between profiles is stated as one order over all transactions

Section: Omission check, "Absent listed transactions are judged in the order of
their first occurrence in the inclusion list, once each".

Needed: whether the order matters between a Profile 1 transaction and a
Profile 2 transaction, or only among Profile 2 replays.

Text: the order is stated over all absent listed transactions and called
normative "because the code budget is shared". Profile 1 consumes neither
budget, so its position relative to Profile 2 transactions cannot change any
Profile 2 verdict, and the verdict for `B` is "unsatisfied if any omission is
unjustified" regardless of which transaction is found first.

Decided: the base's Profile 1 pass runs first, unchanged, and the Profile 2
pass then runs in list order. Verdicts are identical; only which transaction a
local log names can differ, and the text already says that record is local.

Suggested text: say the order is normative among Profile 2 replays, and that
where a Profile 1 omission is found first the evaluator MAY stop before any
replay.

### 8. "Same verdict from every later call" versus recomputation

Section: Engine API.

Needed: how an execution client that reports `inclusionListSatisfied` from
`engine_forkchoiceUpdatedV5` guarantees the same verdict it gave from
`engine_newPayloadV6`.

Text: "MUST report the same verdict from every later call about the same
payload." The base recomputes from the retained list against committed state
on each call. Recomputation is deterministic while both states are readable,
but the undecidable rule makes it not idempotent: a state root that was
readable at import and has since been pruned turns a decided replay into an
excused one, and the two calls then disagree.

Decided: record the verdict alongside the retained list the first time it is
computed and report the recorded value afterwards.

Suggested text: "An execution client SHOULD retain the verdict with the list
rather than recompute it", with the pruning case as the reason.

### 9. `FORK_TIMESTAMP` on a chain whose client has no separate schedule

Section: Constants; Activation.

Needed: what activates this EIP in a client whose fork schedule is a single
timestamp for the four prerequisite EIPs.

Text: `FORK_TIMESTAMP` is "the activation timestamp of EIP-8272, unless the
chain schedules its own". Clear. The base gates the whole inclusion-list check
on the Hegotá timestamp, which is where EIP-8272 activates, so nothing had to
be added, and there is no field to schedule a later activation. A chain that
wanted one would need a new chain-config field.

Decided: activation is the Hegotá timestamp. No separate field.

Suggested text: none needed; noting that "a chain that wants a later activation
MUST schedule it explicitly" implies a configuration surface the client may not
have.

### 10. The activation block at `S_start` for the other two predeploys

Section: Evaluation states, the paragraph on `RECENT_ROOT_CODE` at the
activation block.

Needed: what happens at the activation block's `S_start` for an expiry
verifier frame and for keyed nonces, since EIP-8141 and EIP-8250 install their
predeploys at the same block in this chain.

Text: names only `RECENT_ROOT_CODE`. An expiry frame at `S_start` of the
activation block targets an account with no code, runs the default code in
`VERIFY` mode with scope `0`, and reverts, so the transaction is ineligible at
`S_start` and the verdict is `S_end`'s. Keyed-nonce state is empty at both
states, so condition 2 behaves the same at both. The outcome matches the
recent-root paragraph's, so no divergence, but an implementer has to work it
out.

Decided: read `P`'s post-state directly, as the text permits, and let the
default-code revert produce the `S_start` verdict.

Suggested text: generalise the paragraph to "any predeploy the chain installs at
that block".

### 11. `MAX_VALIDATION_CODE_BYTES` changes with the fork

Section: Constants.

Needed: the value of `MAX_CODE_SIZE` "defined by the active fork".

Text: correct and sufficient. Worth noting that on a chain with EIP-7954 the
value is `16 * 65536`, and that a consensus constant which depends on the fork
is unusual enough to deserve the number in the table.

Decided: `MAX_CODE_SIZE` is `0x10000` from Amsterdam on, `0x6000` before.

### 12. The mempool undercounts the expiry frame, as the spec predicts

Section: Profile 2 candidates, the paragraph after the pseudocode;
Implementation notes, "The gas sum".

Observation: the base's mempool prefix check sums the recognised prefix frames
plus the recent-root frame and does not add the expiry frame. The spec says the
mempool "SHOULD be brought to agree" and that EIP-8141 rule 6 should be
clarified. Both observations hold for this base. The mempool was not changed:
it is local policy and outside this task, and changing it would alter admission
on a running network. The Profile 2 price counts both frames as the spec
requires (cases 15 and 16 are tested).

### 13. Condition 5 and the target half of condition 3 are unreachable, as stated

Section: Profile 2 candidates, the paragraph after condition 8.

Observation: confirmed. The base's static constraints reject `ATOMIC_BATCH_FLAG`
on a frame followed by a `VERIFY` frame and constrain `APPROVE_EXECUTION`
targets to the sender, so a deploy or verify frame in the prefix cannot carry
the flag, and the shape match's mode and scope checks complete condition 3. A
check for condition 5 is kept anyway, as the text asks; it costs one loop over
the prefix indices.

### 14. What "statically valid" means when the client's static rules are stricter than EIP-8141's

Section: Profile 2 candidates, condition 1.

Observation: the base rejects a zero sender address and frame gas limits above
`2**63 - 1` as static errors, neither of which EIP-8141 lists. Candidacy uses
the client's static check, so a transaction another client considers
statically valid could be excused here. This is a base divergence the base
documents, not a spec gap, but the spec could say that "statically valid" means
EIP-8141's constraints and only those, so that stricter local checks are not
allowed to widen the excused set.

### 15. The `payer` of a `self_verify` shape versus the account that approves payment

Section: Profile 2 eligibility, `payer` resolution.

Needed: whether the surface's `payer` must equal the account whose `APPROVE`
sets `payer` during replay.

Text: `payer` is resolved from the shape before the first frame, and condition
5 requires the prefix to set `payer` through `APPROVE`. EIP-8141 binds `payer`
to the `pay` frame's resolved target, which is exactly what the shape resolves,
so the two cannot differ. Clear once traced through EIP-8141; a sentence saying
"the account replay binds as `payer` is necessarily this one" would save the
trace.

### 16. The Test Cases rows that cannot be built on a chain

Section: Test Cases.

Rows 27 and 28 need a state that cannot be opened, which no block on a healthy
chain produces. They are pinned here with a scripted replayer against the
omission-check driver rather than a chain. Row 23 (forty transactions sharing
one verifier) is pinned on the code budget alone: charging one hash forty times
costs one body. Row 10 (canonical paymaster reservation storage) is covered by
the same rule as rows 8 and 9 (a `keccak`-derived slot of a third account) and
was not given its own scenario; the base's canonical paymaster exemption is
never enabled in a Profile 2 replay, which is the load-bearing part, and is
tested by construction.

## Rules the base makes impossible or contradicts

* None contradicted. The one near miss is entry 12: the mempool prices a prefix
  without the expiry frame, so the same transaction is admitted to the public
  mempool at one price and charged another by the fill. The spec anticipates
  this exact disagreement.

## Rules that turned out unnecessary in this base

* Condition 5 (atomic batch flag in the prefix) and the target half of
  condition 3, as the text itself says (entry 13).
* Condition 7 (defined modes), established by condition 1 in this base since
  static validity rejects reserved mode bytes.
* The `SLOAD` permission for keyed-nonce slots (entry 2), unreachable because
  the NONCE_MANAGER predeploy has no read path.

## Parts clear enough to implement verbatim

* The two-endpoint rule and its formula, `omission_unjustified`, and the
  short-circuit order (`S_end`, then `S_start`).
* `fee_valid` and `gas_fits`, including "once, at the end of the payload, for
  both profiles" and `total_gas_limit` being EIP-8141's `max_gas`.
* `S_end` read through the committed post-state with withdrawal credits
  discounted, and the EIP-161 consequence for an account created by the credit.
  The withdrawal test pins it.
* `S_start` as `P`'s post-state read directly, with the MAY on system
  operations.
* The block context of both replays: `B`'s base fee, timestamp, gas limit,
  chain id and `slotNumber` as `current_slot`.
* Profile 1 unchanged, judged at `S_end` only, not consuming the budgets.
* The four admitted shapes disregarding protocol verifier frames, and the
  position rule for those frames (condition 4).
* The `payer` resolution rule and that it fixes the surface before any read.
* Full `APPROVE` semantics with maximum-cost collection and the prohibition on
  a closed-form solvency test (row 24 is tested).
* The validation surface: `sender` and `payer` low slots by `AA_VOPS_SLOT_COUNT`,
  storage owner as the executing context, the `DELEGATECALL` versus `CALL`
  distinction (row 9 tested both ways), no canonical paymaster exemption, and
  writes confined to what EIP-8141 permits and additionally to the surface.
* The recent-root permission being scoped to the frame executing
  `RECENT_ROOT_CODE` at the top level.
* The code bound's per-list sharing across occurrences and both states, and
  charging each distinct `codeHash` once.
* Replay semantics: warm set, protocol verifier frames executed first, every
  change discarded, replays independent, fork rules `B`'s, body frames never run.
* The budget fill pseudocode, including the two-stage debit, "unpriceable:
  ignored, no debit", and "charged, not admitted". Rows 6, 17, 18 and 19 came
  out exactly as tabulated.
* The Implementation notes' four differences between the mempool use and the
  Profile 2 use of the shared simulation, which mapped one to one onto changes
  in the observer, the gas parameter, the storage rule, and the header passed
  to the replay.
* The undecidable rule: not ineligibility, not unsatisfied, recorded locally.
* `AA_VOPS_SLOT_COUNT` MAY be a chain parameter; the base already exposes one
  with the table's default.
* The Engine API section, apart from entry 8: no new method or version, the
  field is set only for `VALID`, and the verdict is computed in the call that
  executes `B`.
* The builder's two-pass construction, which the base lacked and now has (a
  test builds a block whose listed transaction lands only on the retry).
