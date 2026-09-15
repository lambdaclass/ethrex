# Leg 1 spec feedback: implementing Profile 2 from `docs/eip-focil-frametx.md`

This log was written while implementing the frame-transaction enforcement
layer of the FOCIL frame-transaction EIP into ethrex from the spec text alone,
on a base that already carried EIP-7805 Profile 1, EIP-8141, EIP-8250 and
EIP-8272 and deliberately excused every frame-transaction omission. One entry
per gap, in the order hit. Each entry names the spec section, what the
implementer needed to know, what the text says or fails to say, what was
decided, and how the text could say it. Line numbers refer to
`docs/eip-focil-frametx.md` at the revision implemented.

The implementation lives in `crates/blockchain/focil_profile2.rs` (candidacy,
budget fill, two-state omission check), `crates/vm/levm/src/validation_observer.rs`
and `crates/vm/levm/src/vm.rs` (validation surface, code budget), and
`crates/vm/backends/levm/mod.rs` (`replay_profile2_validation_prefix`).

---

## 1. The Engine API delivers one flat list, so "per inclusion list" is not computable

**Section:** Budget fill (lines 197 to 224), Code bound (line 182),
Terminology (line 57), Engine API (lines 267 to 273).

**Needed:** the list boundaries the fill and the code budget are defined over.

**Text:** "Occurrences are admitted per inclusion list, in list order, before
deduplication across lists" and "One budget of `MAX_VALIDATION_CODE_BODIES`
bodies and `MAX_VALIDATION_CODE_BYTES` bytes is maintained per inclusion list".
The Engine API section says "Inclusion lists reach the execution layer through
the `inclusionListTransactions` parameter", and that parameter (engine-bogota,
`engine_newPayloadV6`) is a single `Array of DATA` with no list boundaries. The
consensus layer aggregates the committee's lists into that array and, in the
consensus specs, deduplicates them. The execution layer therefore cannot know
which committee member listed a transaction, how many lists it appeared in, or
where one list ends and the next begins.

**Decided:** the delivered array is treated as the one inclusion list: one gas
budget of `MAX_VERIFY_GAS_PER_IL`, one code budget, occurrences metered in the
order delivered. Test case 19 ("listed by two committee members, admitted in
one list only") cannot be exercised through the Engine API at all.

**Could say:** either define the fill and the code budget over the
`inclusionListTransactions` array as delivered (and then the Rationale's
"across EIP-7805's sixteen committee members it bounds the metered replay at
`2**24`" is wrong: the bound is `2**20` per payload), or require the consensus
layer to deliver per-committee-member lists, which contradicts "No Engine API
method, parameter, structure, or version is added". The current text asserts
both a per-list rule and an unchanged API; an implementer can honour only one.

## 2. `FORK_TIMESTAMP` is `TBD`

**Section:** Constants (line 67), Activation (lines 275 to 281).

**Needed:** when to switch from "every frame transaction omission is justified"
to the omission check defined here.

**Text:** `FORK_TIMESTAMP | TBD`. Activation says "This EIP MUST activate at or
after" the four prerequisite EIPs, nothing more.

**Decided:** the fork that activates the prerequisites on this chain (Hegotá,
`ChainConfig::is_hegota_activated`) is also the activation of this EIP. The
base already gated the Profile 1 check on that fork, so no separate placeholder
was introduced.

**Could say:** "Unless a chain schedules a separate `FORK_TIMESTAMP`, this EIP
activates with the fork that activates EIP-8272." A `TBD` in a consensus
constant table is an invitation to diverge.

## 3. `S_end` is not a state the execution layer holds after import

**Section:** Evaluation states (lines 97 to 102).

**Needed:** how to open `S_end` on a client that decides satisfaction after
the block has been imported and its post-state root committed.

**Text:** "`S_end` = the state after `B`'s last transaction, before `B`'s
end-of-block system operations". The committed post-state root is after the
withdrawals, which credit arbitrary accounts, so it is not `S_end`. The text
gives the `S_start` MAY (apply the pre-execution system operations or not)
but nothing for `S_end`.

**Decided:** `S_end` is read from the committed post-state with the block's
withdrawal credits subtracted per recipient (`PreWithdrawalsDb` in
`focil_profile2.rs`), exactly what the base's Profile 1 code does with
`discount_withdrawals`. Withdrawals only add balance, so the subtraction is
exact for balances. One edge remains: an account created by a withdrawal
exists in the committed state and did not exist at `S_end`, which the EIP-8037
existence rule can observe through the `APPROVE` account-creation charge.

**Could say:** either "an evaluator reading the committed post-state MUST
discount the block's withdrawal credits from each recipient's balance", or
define `S_end` as the committed post-state and argue in the Rationale that
withdrawals are monotone like gas. The first keeps EIP-7805's check point; the
second removes a reconstruction step every client has to get right.

## 4. `S_start` can observe the pre-execution system operations at the fork block

**Section:** Evaluation states (line 102).

**Text:** "`B`'s pre-execution system operations write only the storage of
system contracts, which lies outside the validation surface, so a Profile 2
replay cannot observe whether they have been applied".

**Observed:** at the first block of the fork, the pre-execution operations
also install `RECENT_ROOT_CODE` at `RECENT_ROOT_ADDRESS` (EIP-8272 Activation)
and the expiry verifier code. Eligibility condition 3 reads "the code at
`RECENT_ROOT_ADDRESS` in `S` is `RECENT_ROOT_CODE`", so at `S_start` of the
activation block, read as `P`'s post-state, that condition fails for every
recent-root transaction while it holds at `S_end`. The verdict is unaffected in
practice (no root can be committed before activation, so condition 3 fails
either way), but the claim that the replay "cannot observe" the operations is
false for that one block.

**Decided:** `S_start` is `P`'s post-state, no system operations applied.

**Could say:** restrict the claim to blocks after the activation block, or
say the code at the two predeploys is judged at `S_end` only.

## 5. Chain id is a Profile 1 condition but not a Profile 2 one

**Section:** Profile 1 (line 115), Profile 2 candidates (lines 123 to 132).

**Needed:** whether a frame transaction declaring another chain can have an
unjustified omission.

**Text:** Profile 1 candidacy requires "`tx.chain_id`, when present, matches
the chain". Profile 2 candidacy has no chain id condition and eligibility has
none either; the replay preamble does not check it. A frame transaction for
another chain whose prefix replays cleanly would be eligible, and the honest
block that could not include it would be unsatisfied.

**Decided:** a listed frame transaction whose `chain_id` differs from the
chain's is excused, at the point where `gas_fits` is judged. Logged as a
decision the text does not support directly.

**Could say:** add "`tx.chain_id` matches the chain" to the Profile 2
candidacy list. It is decidable from the bytes alone, like the rest of it.

## 6. The base mempool's budget sum and the fill's "MUST agree" cannot both hold

**Section:** Profile 2 candidates (line 141), Rationale "Protocol verifier
frame gas" (line 362).

**Text:** "This is the same sum EIP-8141 rule 6 and EIP-8272 require for public
mempool admission, and the two MUST agree, or a transaction the mempool admitted
at one price is charged another by the fill." The Rationale then admits
"EIP-8141 rule 6's 'across the validation prefix' is silent on the expiry frame
and SHOULD be clarified the same way".

**Observed in the base:** `FrameTransaction::validate_prefix_structure` sums the
prefix frames plus the recent-root frame and not the expiry frame, which is a
faithful reading of EIP-8141 rule 6 as written. So the base mempool disagrees
with this EIP's `verify_budget_cost` for any transaction carrying an expiry
frame, and the MUST is unsatisfiable without changing a rule this EIP says it
leaves alone ("Nothing about ... public mempool admission changes").

**Decided:** the fill uses this EIP's sum (expiry frame counted); the mempool
is untouched. Test cases 15 and 16 pin the fill's sum.

**Could say:** drop the MUST, or make it "SHOULD agree, and EIP-8141 rule 6
needs amending for that to be possible". A consensus rule cannot MUST-agree
with a local policy it has no authority over.

## 7. What makes an occurrence "unpriceable" is not defined

**Section:** Budget fill (line 208 to 210).

**Text:** `prefix_cost = prefix_and_verifier_frame_cost(occ)   # decoding and
shape only` and `if prefix_cost is None: continue  # unpriceable: ignored, no
debit`. Nothing says when the function returns `None`.

**Decided:** `None` when the transaction fails EIP-8141 static validity or
matches none of the four shapes. Both are "decoding and shape". A statically
invalid transaction therefore pays nothing, which is consistent with the
two-stage debit sentence ("a structurally valid transaction with a bad
signature pays for the signature check").

**Could say:** define `prefix_and_verifier_frame_cost` in the pseudocode, or
state "`None` when the transaction is not statically valid or its prefix
matches no admitted shape".

## 8. Candidacy conditions 5 and 7, and the target half of 3, are restatements

**Section:** Profile 2 candidates (lines 127 to 131).

**Observed:** EIP-8141's static constraints already assert `frame.mode < 3`
(condition 7), forbid `ATOMIC_BATCH_FLAG` on a VERIFY frame and on any frame
followed by one (so every prefix frame, the four shapes all ending in VERIFY
frames, trips condition 1 before condition 5), and require `APPROVE_EXECUTION`
frames to target the sender (the target half of condition 3 for `self_verify`
and `only_verify`). None of the three is reachable past condition 1.

**Decided:** implemented as written, and the tests for cases 11 and 13 assert
the `StaticallyInvalid` outcome that actually fires. Condition 7's paragraph
(line 143) is valuable as a forward-compatibility statement about future modes.

**Could say:** keep 7 for its forward-compatibility point, mark 5 and the
target rule as "implied by condition 1, stated for clarity", so an implementer
does not look for a code path that cannot execute.

## 9. "S_end first, skip S_start" interacts with the shared code budget

**Section:** Omission check (line 243), Code bound (line 182).

**Needed:** an order of replays that every evaluator reproduces.

**Text:** "An evaluator MAY evaluate `S_end` first and skip `S_start` when the
transaction is eligible there, since the result is the same." The code budget
is "shared across every replay of that list's occurrences and across both
evaluation states" and "a replay that would exceed the budget does not proceed
and its transaction is ineligible for that list".

**Observed:** skipping `S_start` for one transaction charges fewer bodies to
the list's budget than replaying both, so a later transaction in the same list
can be ineligible on one evaluator (budget exceeded) and eligible on another
(budget not exceeded). The result is the same for the transaction that
skipped, not for the list. The order in which absent transactions are replayed
is also unstated; the fill says "in list order", the omission check does not.

**Decided:** list order, one verdict per transaction (first occurrence),
`S_end` first, `S_start` skipped when `S_end` is eligible. Logged as a
determinism risk.

**Could say:** fix the order ("in list order, `S_end` then `S_start`") and
either remove the skip or make the code budget insensitive to it, for example
by charging code at `S_start` and `S_end` from the same accounting whether or
not the second replay is run, or by making the code bound per replay rather
than per list.

## 10. `gas_fits` is one-dimensional under a two-dimensional gas model

**Section:** Fee and gas conditions (lines 87 to 93).

**Text:** `gas_fits(tx, B): return tx.total_gas_limit <= B.gas_limit -
B.gas_used`, with `total_gas_limit` "EIP-8141's `max_gas`, the intrinsic cost
plus the sum of every frame's `limits.execution` and `limits.state`".

**Observed:** EIP-8141 requires EIP-8037, under which a frame transaction is
includable when its execution reservation and its state reservation each fit
their own dimension; the header's `gas_used` is one number. Summing both
dimensions against one remaining budget is stricter than the inclusion rule,
so a transaction the builder could have appended (each dimension fits) can
have its omission excused because the sum does not fit. Test case 21 relies on
exactly this: a transaction declaring more state gas than the block holds is
statically valid and excused by `gas_fits`.

**Decided:** implemented as written (`max_gas` against `gas_limit - gas_used`),
the same reading the base's Profile 1 code uses.

**Could say:** state that `gas_fits` is deliberately the one-dimensional sum
because the header carries one `gas_used`, and that it therefore excuses more
than EIP-8037's per-dimension rule would.

## 11. "does not proceed" for the code budget

**Section:** Code bound (line 182).

**Text:** "A replay that would exceed the budget does not proceed and its
transaction is ineligible for that list."

**Decided:** the observer records `CodeBudgetExceeded`, takes no further
charges, and the transaction is ineligible. The EVM is not halted mid-frame:
the remaining opcodes of the frame run, bounded by the frame's declared gas,
and load nothing more. The verdict is identical; only the work differs.

**Could say:** "the transaction is ineligible and no further code is charged;
an evaluator MAY abort the replay". A halt is an implementation choice, not a
consensus-visible one.

## 12. Pre-frame check order

**Section:** Profile 2 eligibility (line 156), Implementation notes (line 294).

**Text:** "The pre-frame checks (keyed nonces, recent-root tuples, signatures)
run before the EVM is constructed, in that order".

**Decided:** the recent-root tuples are judged first, in the blockchain layer
against the state root, and the keyed nonces and signatures next, in the VM
preamble shared with mempool admission. The order changes only how much work a
malformed transaction costs, never the verdict.

**Could say:** make the order a SHOULD and say why (bounding work), so an
implementer reusing an existing preamble is not left wondering whether the
order is consensus.

## 13. Case 9 names `DELEGATECALL` for a read it cannot perform

**Section:** Test Cases, case 9 (line 388).

**Text:** "prefix reads third-account storage reached by `DELEGATECALL`".

**Observed:** a delegatecalled library executes in the caller's storage
context; its `SLOAD` reads the sender's storage, inside the surface. Third
account storage is reached by `CALL`, `STATICCALL` or `CALLCODE` into a
contract that reads its own storage. Two tests pin both readings:
`third_account_storage_reached_by_a_call_is_outside_the_surface` and
`delegatecalled_library_reads_the_senders_storage`.

**Could say:** "reached by a call into a third contract", and in the
Validation surface section, "the storage owner is the account whose storage
the executing context reads, not the account whose code runs".

## 14. Recording undecided verdicts

**Section:** Omission check (line 245).

**Text:** "It SHOULD record which transactions it could not decide, so that
the failure is visible rather than silently excused."

**Decided:** each undecided state is logged at warn level with the transaction
hash and the reason, and returned in `IlSatisfaction::undecided` to the
caller. Nothing reaches the Engine API, which has no field for it.

**Could say:** nothing more is needed for a SHOULD, but a sentence that the
record is local (a log or metric) would save the question of whether the
consensus layer is meant to see it.

## 15. "MUST evaluate both states within the call that executes `B`" versus the skip

**Section:** Engine API (line 273), Omission check (line 243).

**Text:** "Execution clients MUST evaluate both states within the call that
executes `B`" and, earlier, an evaluator "MAY evaluate `S_end` first and skip
`S_start`".

**Observed:** the two cannot both be read literally. Also, a payload returned
`ACCEPTED` is executed in a later call; the verdict is then computed in that
later call, which execution-apis permits and the base does.

**Decided:** the verdict is computed once the block is imported, in whichever
engine call imports it, from the block's committed states; a later call about
the same payload recomputes from the same states and reaches the same verdict,
because nothing in the check depends on when it is asked.

**Could say:** "Execution clients MUST compute the verdict from `B`'s own
states, so that every call about the same payload reports the same field", and
drop "both" and "within the call that executes".

## 16. Transaction identity is the envelope bytes; the implementation uses their digest

**Section:** Terminology (line 59).

**Decided:** presence in `B`, deduplication and admission are keyed by the
keccak of the canonical EIP-2718 encoding, which the base uses everywhere as
the transaction hash. Equivalent to byte identity; a byte-distinct variant has
a distinct hash.

**Could say:** "or a collision-resistant digest of them".

## 17. Builders: the RECOMMENDED placement does not achieve the MUST

**Section:** Includers and builders (line 265).

**Text:** "Builders MUST include every listed transaction whose omission would
be unjustified. ... A builder placing all listed transactions at the front of
its payload satisfies the rule for every transaction eligible at `S_start`;
this is RECOMMENDED but not required".

**Observed in the base:** the payload builder applied the inclusion list once,
at the front, and never returned to an entry it had skipped. A listed
transaction ineligible at the front and eligible at the end (case 2: the queued
transaction whose predecessor the mempool supplies later in the same payload)
was skipped and the builder produced an unsatisfied block by its own rule.

**Decided:** a second pass over the entries the first pass skipped, after the
mempool fill, at the end of the payload. With both passes, a transaction
eligible at either endpoint the builder can produce is included. The pass is
bounded by the list's byte cap.

**Could say:** name the two-pass construction as the way to meet the MUST:
"placing all listed transactions at the front, and retrying the skipped ones
after the rest of the payload, satisfies the rule for every transaction
eligible at either endpoint".

## 18. The Test Cases are descriptions, not vectors

**Section:** Test Cases (lines 374 to 409), Motivation (line 35).

**Text:** "the Test Cases are the ones the implementation is checked against",
and EIP-8369 asked the enforcing EIP to define "conformance test vectors".

**Observed:** the table gives one sentence and a verdict per case; no
transaction bytes, no state, no expected budget balances. Each case had to be
reconstructed as a scenario, and two evaluators reading the same row can build
different scenarios (see entry 13). Cases 5, 20, 24, 27 and 28 were covered by
existing base behaviour or VM-level tests rather than block-level scenarios;
case 19 cannot be built (entry 1).

**Could say:** publish the vectors, or at least the transaction encodings and
state seeds per row, so "checked against" means the same thing on every client.

## 19. `AA_VOPS_SLOT_COUNT` as chain configuration

**Section:** Constants (line 78).

**Text:** "`AA_VOPS_SLOT_COUNT` MAY be exposed as a chain configuration
parameter ... The value binding a chain is the one its chain configuration
pins, and the value binding this specification is the one in the table."

**Observed:** the base already carried `ChainConfig::aa_vops_slot_count` with
a default of 4. It was used as the surface width. Helpful and unambiguous.

## 20. Things the text got right that an implementer notices

Not gaps; recorded because they saved time.

* The Implementation notes' list of exactly what MUST differ between the
  mempool simulation and the replay (gas budget, storage rule, gas sum,
  `current_slot`) mapped one to one onto the code: the observer gained a
  surface and a budget, the harness dropped the canonical exemption and the
  operator budget, and the fill counts the verifier frames.
* "Each evaluation state opens its own EVM over a state root, with `B`'s header
  as context and `B`'s fork rules" is precisely the construction used.
* The two-stage debit pseudocode was implementable verbatim.
* "A verdict that cannot be computed is not a verdict" gave the three-way
  `Eligible | Ineligible | Undecided` result type directly, and the distinction
  between a transaction-validation refusal (ineligible) and an evaluator failure
  (undecided) fell out of the existing error types.

## 21. Things the text says that turned out unnecessary here

* The `MAX_VERIFY_GAS_PER_TX` bound inside eligibility: candidacy prices the
  declared limits, and the replay runs each frame within its declared limit, so
  no separate gas assertion is needed after the replay (the mempool path has
  one against the operator budget; the Profile 2 path has none, deliberately).
* The instruction that `payer` "MUST be resolved from the prefix shape before
  the first frame executes": EIP-8141 already guarantees that `APPROVE` only
  succeeds in the frame's resolved target, so the payer `APPROVE` sets is the
  one the shape resolves. The implementation resolves it first, as told, and
  also asserts the two agree, which cannot fail.
