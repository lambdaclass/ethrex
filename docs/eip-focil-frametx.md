---
eip: XXXX
title: FOCIL Enforcement for Frame Transactions
description: Extends inclusion-list omission checks to frame transactions, judged at both endpoints of the payload
author: Edgar Luque (@edg-l), Iván Litteri (@ilitteri)
status: Draft
type: Standards Track
category: Core
created: 2026-08-07
requires: 161, 1559, 2718, 2930, 3607, 4844, 7702, 7805, 7843, 7954, 8037, 8141, 8250, 8272
---

## Abstract

[EIP-7805](./eip-7805.md) lets attesters refuse a block that omits a listed transaction the builder could have included. It decides "could have included" with a nonce and balance check against the state at the end of the payload. That check answers the right question for ordinary transactions and the wrong one for [EIP-8141](./eip-8141.md) frame transactions, whose validity is decided by code the transaction itself names.

This EIP extends the omission check so it covers frame transactions, including those that use [EIP-8250](./eip-8250.md) keyed nonces and [EIP-8272](./eip-8272.md) recent roots. It sorts listed transactions into two profiles. Profile 1 is every transaction EIP-7805 already handles, unchanged. Profile 2 is a frame transaction whose validation reads only a fixed, small state surface; its omission is judged by replaying that validation at two states the evaluator already holds, the one the payload started from and the one it ended at:

```text
omission unjustified  <=>  eligible at S_start  or  eligible at S_end
```

Neither state is chosen by any party, so nothing is encoded in the block, the Engine API is not extended, and no state is reconstructed from a block access list. A per-list gas budget and a per-list code budget bound the work an attester does on anyone's behalf.

## Motivation

FOCIL works because an attester can cheaply answer one question about a transaction the builder left out: would it have been valid had the builder appended it? For a legacy or [EIP-1559](./eip-1559.md) transaction that is a nonce and a balance read against the post-state, and EIP-7805 specifies exactly that.

A frame transaction does not have a nonce and a balance in that sense. Its sender may be a contract whose authorization logic is arbitrary code. Its fee may be paid by a different account chosen by a `pay` frame. Its replay protection may be a set of [EIP-8250](./eip-8250.md) keyed nonces rather than one sequence number. It may bind itself to an [EIP-8272](./eip-8272.md) recent root that expires. It may install its own code in a deploy frame before any of that runs. Each of these is a validity condition, and none is visible to a nonce and balance check. So a client that implements both EIP-7805 and EIP-8141 has two choices, and both are bad: excuse every frame transaction omission, which leaves the transactions privacy protocols and smart accounts actually send outside FOCIL's protection, or invent a rule, which splits the network the first time two clients invent differently.

[EIP-8369](./eip-8369.md) describes the shape of a good rule. It divides listed transactions into a profile that keeps EIP-7805's check and a profile whose validation is confined to a fixed state surface small enough for every attester to hold and replay. It is Informational. It defines no consensus rule, leaves its constants as candidates pending benchmarks, and defers the enforcement mechanism, the encoding, and the test vectors to "an extension to EIP-7805 and to EIP-8141". This EIP is that extension. It restates the model as normative text rather than by reference, so that no binding rule depends on a document that declares none.

Where EIP-8369 says a Profile 2 omission is judged at an index the builder claims, this EIP judges it at the two fixed endpoints of the payload instead. That is stronger, because the builder can no longer pick the point at which the transaction looks invalid. It is cheaper, because both states already exist in the evaluator's memory. And it needs no new field anywhere, because there is nothing to claim. The [Rationale](#rationale) gives the argument in full.

Everything in this document has been implemented and run on a public test network. Where the text names a constant, a rule, or an ordering, it is the one that is running, and the [Test Cases](#test-cases) are the ones the implementation is checked against.

## Specification

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in RFC 2119 and RFC 8174.

Terms not defined here, including `FrameTx`, frame modes, `VERIFY`, `APPROVE`, `payer`, validation prefix, and the expiry verifier frame, have the meanings defined in [EIP-8141](./eip-8141.md). The recent-root verifier frame has the meaning defined in [EIP-8272](./eip-8272.md).

Let `B` be the block being judged and `P` its parent.

### What this changes, and where

This EIP is a delta against two documents. Readers implementing one of them need only the paragraph that names it.

**For an implementer of [EIP-7805](./eip-7805.md).** The execution-layer omission check gains a second branch. A listed transaction that is absent from `B` is still judged justified or unjustified, and the result still reaches the consensus layer through the same `inclusionListSatisfied` field on the same Engine API methods. What changes is how the judgement is reached for one class of transaction. For every transaction EIP-7805 already covers, nothing changes: the nonce and balance check at the end of the payload is kept verbatim as [Profile 1](#profile-1). For a frame transaction, the check becomes [Profile 2](#profile-2-candidates): a bounded replay of its validation prefix at two states, under a gas budget and a code budget that every evaluator computes identically from the inclusion list alone. Frame transactions that do not fit Profile 2 have their omission excused, exactly as they would be today. No consensus-layer rule, no committee behaviour, and no engine method, parameter or version changes.

**For an implementer of [EIP-8141](./eip-8141.md).** Nothing about frame transaction execution, encoding, or public mempool admission changes. This EIP reuses the machinery EIP-8141 already requires for mempool admission, the simulation of a validation prefix under validation-trace rules, and runs it a second time for a different purpose with two differences. The gas budget is a consensus constant rather than the operator's `MAX_VERIFY_GAS`, and the storage a prefix may read is the [validation surface](#validation-surface) defined here rather than EIP-8141's `sender`-only rule. Because a shared code path is the likeliest source of divergent verdicts, [Implementation notes](#implementation-notes) lists what must differ between the two uses.

**What does not change anywhere.** Block validity. A block that fails to satisfy its inclusion lists is `VALID`; attesters withhold their vote on the satisfaction field, as EIP-7805 already specifies. Transaction encodings, receipts, and the [EIP-8272](./eip-8272.md) and [EIP-8250](./eip-8250.md) rules are untouched.

### Terminology

For the execution layer, **the inclusion list** of `B` is the `inclusionListTransactions` array delivered with `B` through the Engine API, in the order delivered. The consensus layer aggregates the committee members' lists into that array; how it aggregates and deduplicates them is defined by [EIP-7805](./eip-7805.md) and its consensus specifications, not here, and the execution layer never learns which committee member listed what or where one member's list ends. A **listed transaction** is a transaction that appears in that array. An **occurrence** is one element of the array; a transaction the array names more than once has several occurrences. Every rule below that says "per inclusion list" or "in list order" is defined over that array.

Transaction identity, for presence in `B`, for deduplication, and for the budget fill, is the exact [EIP-2718](./eip-2718.md) envelope bytes; a client MAY key by their keccak-256 digest, which is the transaction hash. A byte-distinct variant does not satisfy the listing.

A frame transaction's frames fall into three groups. Its **validation prefix** is the frames EIP-8141 defines as such: the optional deploy frame and the `VERIFY` frames that establish `sender` approval and `payer`. Its **protocol verifier frames** are the optional expiry verifier frame of EIP-8141 and the optional recent-root verifier frame of EIP-8272, in the positions those EIPs require: the expiry verifier frame when it is the first frame, and the recent-root verifier frame when it is the first frame or the second behind an expiry verifier frame. Both precede the prefix, both are executed, and neither is part of the prefix for shape matching. A frame with either shape in any other position is not a protocol verifier frame for any rule in this document, pricing included; it is a body frame, and candidacy condition 4 rejects the transaction. Every remaining frame is a **body frame**.

### Constants

| Name | Value |
| --- | ---: |
| `FORK_TIMESTAMP` | the activation timestamp of [EIP-8272](./eip-8272.md), unless the chain schedules its own |
| `AA_VOPS_SLOT_COUNT` | `4` |
| `MAX_VERIFY_GAS_PER_IL` | `2**20` |
| `MAX_VERIFY_GAS_PER_TX` | `MAX_VERIFY_GAS_PER_IL` |
| `MAX_VALIDATION_CODE_BODIES` | `16` |
| `MAX_VALIDATION_CODE_BYTES` | `MAX_VALIDATION_CODE_BODIES * MAX_CODE_SIZE` (`2**20` under EIP-7954's `MAX_CODE_SIZE = 0x10000`) |

`MAX_CODE_SIZE` is defined by the active fork. `IL_COMMITTEE_SIZE` and `MAX_BYTES_PER_INCLUSION_LIST` are defined by [EIP-7805](./eip-7805.md) and are not used by any rule here.

`FORK_TIMESTAMP` is not left open. A chain that activates the four prerequisite EIPs together, as the test network does, activates this EIP at that same timestamp; a chain that wants a later activation MUST schedule it explicitly. An unset consensus constant is an invitation to diverge.

These are consensus constants. They MUST NOT be read from node-local configuration. In particular a client that also implements EIP-8141's public mempool cap `MAX_VERIFY_GAS` MUST NOT use that value, or any operator-tunable override of it, on this path; the two budgets answer different questions and are permitted to differ.

`AA_VOPS_SLOT_COUNT` MAY be exposed as a chain configuration parameter so that a test network can sweep the range EIP-8369 left open. The value binding a chain is the one its chain configuration pins, and the value binding this specification is the one in the table.

### Fee and gas conditions

```python
def fee_valid(tx, B):
    return (tx.max_fee_per_gas >= B.base_fee_per_gas
            and tx.max_priority_fee_per_gas <= tx.max_fee_per_gas)

def gas_fits(tx, B):
    return tx.total_gas_limit <= B.gas_limit - B.gas_used
```

For legacy and [EIP-2930](./eip-2930.md) transactions, `gas_price` stands in for both fee fields. For a frame transaction, `total_gas_limit` is EIP-8141's `max_gas`: the larger of the transaction's standard gas limit (its intrinsic cost plus the sum of every frame's `limits.execution` and `limits.state`) and its calldata floor gas plus the sum of every frame's `limits.state`. For a transaction whose floor is not binding, that is the standard gas limit; a floor-bound transaction reserves more, and it is the reservation that has to fit.

`gas_fits` is not part of eligibility at a state and MUST be evaluated once, at the end of the payload, for both profiles.

`gas_fits` is deliberately one-dimensional. `B.gas_used` is one number, while [EIP-8037](./eip-8037.md), which EIP-8141 requires, admits a frame transaction when its execution reservation and its state reservation each fit their own dimension. Judging the sum against the one remaining budget excuses more omissions than the per-dimension rule would, never fewer, which is the direction every ambiguity in this document resolves toward.

### Evaluation states

```text
S_start = the state B executes from
S_end   = the state after B's last transaction, before B's end-of-block system operations
```

`S_end` is the state [EIP-7805](./eip-7805.md) already specifies. It is not a state a client holds once `B` is imported: the committed post-state root lies after `B`'s withdrawals, which credit arbitrary accounts. An evaluator that reads `S_end` through the committed post-state MUST discount the block's withdrawal credits from each recipient's balance. Withdrawals credit balance and nothing else, so the discounted view is exact, with one consequence: an account whose discounted balance is zero, whose nonce is zero and whose code is empty was created by the credit, is empty under [EIP-161](./eip-161.md), and MUST be treated as nonexistent, as EIP-161 already requires of an empty account.

`S_start` is `P`'s post-state. From the block after activation onward, `B`'s pre-execution system operations write only the storage of system contracts, which lies outside the [validation surface](#validation-surface), so a Profile 2 replay cannot observe whether they have been applied; an evaluator MAY apply them or MAY read `P`'s post-state directly, and MUST reach the same verdict either way. At a block whose pre-execution operations install a predeploy the replay executes or reads, which on a chain activating the prerequisites together is the activation block for `RECENT_ROOT_CODE`, the expiry verifier and the nonce manager, `S_start` read as `P`'s post-state lacks that code: eligibility condition 3 fails for a transaction leading with a recent-root verifier frame, an expiry verifier frame runs the default code and reverts, and keyed-nonce state is empty at both states. In each case the transaction is ineligible at `S_start` and the verdict is the one `S_end` reaches, so the MAY holds at that block too.

Both states are evaluated under `B`'s block context: `B.base_fee_per_gas`, `B.timestamp`, `B.gas_limit`, `B`'s chain id, and `B`'s [EIP-7843](./eip-7843.md) `slotNumber` as `current_slot`. Each endpoint asks whether the transaction could have been included in this block, so the state differs between them and nothing else does.

No other evaluation state is defined. Clients MUST NOT accept an evaluation index or state selector from the block, the beacon block, the Engine API, or any out-of-band channel.

### Profile 1

A listed transaction is a Profile 1 candidate if all of the following are true:

1. it is a structurally valid legacy, [EIP-2930](./eip-2930.md), [EIP-1559](./eip-1559.md), or [EIP-7702](./eip-7702.md) transaction;
2. `blob_versioned_hashes` is empty or absent;
3. its signature validates against the recovered sender;
4. `tx.chain_id`, when present, matches the chain.

A Profile 1 candidate is eligible at a state `S` if `fee_valid(tx, B)`, the sender satisfies [EIP-3607](./eip-3607.md) at `S` with a valid [EIP-7702](./eip-7702.md) delegation indicator treated as the delegated EOA case, and the sender's nonce and balance at `S` admit the transaction.

Profile 1 candidates are judged at `S_end` only, exactly as EIP-7805 specifies, and do not consume the budgets defined here.

### Profile 2 candidates

Candidacy is decided from the transaction bytes alone, with no state access. A listed transaction is a Profile 2 candidate if all of the following are true:

1. it is a statically valid [EIP-8141](./eip-8141.md) frame transaction whose `chain_id` matches the chain;
2. `blob_versioned_hashes` is empty;
3. disregarding protocol verifier frames for shape matching, the modes, flags, and targets of its validation prefix match one of `self_verify`, `deploy | self_verify`, `only_verify | pay`, or `deploy | only_verify | pay`;
4. its protocol verifier frames, if present, are in the positions EIP-8141 and EIP-8272 require: an expiry verifier frame first, a recent-root verifier frame immediately after it or first in its absence, and at most one of each;
5. no validation prefix frame has `ATOMIC_BATCH_FLAG` set;
6. no body frame has mode `VERIFY`;
7. every frame has a mode defined by [EIP-8141](./eip-8141.md);
8. `verify_budget_cost(tx) <= MAX_VERIFY_GAS_PER_TX`.

```python
def prefix_and_verifier_frame_cost(tx):
    # Decoding and shape only. None when the transaction is not statically
    # valid or its validation prefix matches none of the four admitted shapes:
    # such an occurrence is unpriceable and the budget fill ignores it.
    if not statically_valid(tx) or shape_of(validation_prefix(tx)) is None:
        return None
    return (sum(f.limits.execution for f in validation_prefix(tx))
            + sum(f.limits.execution for f in protocol_verifier_frames(tx)))

def verify_budget_cost(tx):
    return signature_verification_cost(tx) + prefix_and_verifier_frame_cost(tx)
```

`signature_verification_cost` is the intrinsic cost EIP-8141 assigns to validating `tx.signatures`. Protocol verifier frames are disregarded only by condition 3. Their declared gas counts, because replay executes them; a client that builds a prefix-frame list with those frames filtered out for shape matching MUST add their gas back before pricing. This is the sum [EIP-8272](./eip-8272.md) requires for public mempool admission. EIP-8141 rule 6 sums "across the validation prefix" and does not name the expiry verifier frame, so a mempool reading it literally charges less than this fill for a transaction carrying one. The fill's sum is fixed here and is consensus; the mempool's is local policy this EIP has no authority over. It SHOULD be brought to agree, so that a transaction admitted at one price is not charged another by the fill, and EIP-8141 rule 6 SHOULD be clarified to name the expiry frame.

"Statically valid" in condition 1 means EIP-8141's static constraints and only those. A client whose public mempool applies stricter static checks, such as rejecting a zero sender address or capping frame gas limits below what EIP-8141 allows, MUST NOT apply them to candidacy, or it excuses omissions every other client enforces.

Condition 5, condition 7, and the target half of condition 3 for the `self_verify` and `only_verify` frames are already implied by condition 1: EIP-8141's static constraints reject `ATOMIC_BATCH_FLAG` on any frame followed by another, reject undefined modes, and require `APPROVE_EXECUTION` frames to target the sender. They are stated so that a later relaxation of those constraints cannot widen Profile 2 by default, not because an implementation needs a code path for them; an implementer will find that none of the three is reachable past condition 1.

Condition 7 admits the modes EIP-8141 defines and no others. A future EIP that adds a frame mode settling after the validation prefix, whose failure could invalidate a transaction that replayed cleanly, is outside condition 7 until an extension to this EIP admits it explicitly. Profile 2 replay never observes such a frame, so admitting one by default would make an unenforceable omission look enforceable.

### Profile 2 eligibility

A Profile 2 candidate is eligible at a state `S` if all of the following are true:

1. `fee_valid(tx, B)`;
2. for every nonzero key in `tx.nonce_keys`, the [EIP-8250](./eip-8250.md) keyed nonce at `(tx.sender, key)` in `S` equals `tx.nonce_seq`, and for the zero key the sender's account nonce in `S` equals `tx.nonce_seq`;
3. if the transaction leads with a recent-root verifier frame, the code at `RECENT_ROOT_ADDRESS` in `S` is `RECENT_ROOT_CODE`, and every `(source_id, slot, root)` tuple in that frame's data satisfies the three [EIP-8272](./eip-8272.md) conditions against `S` at `current_slot`: `slot < current_slot`, `current_slot - slot <= RECENT_ROOT_USABLE_WINDOW`, and the entry hash committed under the tuple's storage key equals the tuple's;
4. every protocol-validated signature in `tx.signatures` validates;
5. the protocol verifier frames and the validation prefix execute to completion against `S` under [Replay semantics](#replay-semantics), stay within the [validation surface](#validation-surface) and the [code bound](#code-bound), and set `payer` through `APPROVE_PAYMENT` (`0x1`) or `APPROVE_EXECUTION_AND_PAYMENT` (`0x3`);
6. a deploy frame, if present, is the first frame after the protocol verifier frames, installs code or an [EIP-7702](./eip-7702.md) delegation indicator at `sender`, and touches only storage inside the surface.

Conditions 2, 3, and 4 are decided before any frame executes and are conditions of eligibility, not outcomes of replay. They are cheap, they need no EVM, and evaluating them first bounds the work a malformed transaction can cause. Their order among themselves is not consensus: an evaluator SHOULD check them before constructing an EVM, in whatever order its existing admission path uses, because the verdict does not depend on that order and only the work does. Replaying the validation prefix without applying them first reports invalid transactions as includable, because the prefix code has no reason to re-check what the protocol checks.

`payer` is `sender` for the `self_verify` shapes and the `pay` frame's EIP-8141 `resolved_target` otherwise; a null `pay` target resolves to `sender`. `payer` MUST be resolved from the prefix shape before the first frame executes, so the surface is known before any read is judged against it. The account replay binds as `payer` through `APPROVE` is necessarily this one, since EIP-8141 lets only the `pay` frame's resolved target, or `sender` in its absence, approve payment; the resolution is a pre-computation of that outcome, not a second check that could disagree with it.

Replay MUST use full EIP-8141 and EIP-8250 `APPROVE` (`0xaa`) semantics including maximum-cost collection, charging `payer` `max_gas * max_fee_per_gas + blob_gas * blob_base_fee` and refunding after execution. Clients MUST NOT substitute a closed-form solvency test. A closed-form test priced at the effective gas price declares solvent a payer that reverts in `APPROVE`.

### Validation surface

Replay may read the following state and no other:

* `address`, `nonce`, and `balance` of `sender` and `payer`;
* storage slots `0` through `AA_VOPS_SLOT_COUNT - 1` of `sender` and `payer`;
* [EIP-8250](./eip-8250.md) keyed nonce state at `(tx.sender, nonce_key)` for every nonzero key in `tx.nonce_keys`, read by the protocol for eligibility condition 2 before any frame executes; this is not an `SLOAD` permission, and prefix code reading the nonce manager's storage reads a third account;
* the [EIP-8272](./eip-8272.md) recent-root entries named by the tuples in the recent-root verifier frame, readable only while that frame executes `RECENT_ROOT_CODE` at the top level, as EIP-8272 already permits for public mempool admission;
* code and `codeHash` of every account reached during validation, including [EIP-7702](./eip-7702.md) delegation target code.

Any other read makes the transaction ineligible rather than expensive. This covers `keccak256`-derived locations, which is what mappings, proxy slots, and namespaced layouts use. The owner of a storage read is the account whose storage the executing context reads, not the account whose code runs: a library reached by `DELEGATECALL` reads its caller's storage, which is inside the surface when the caller is `sender` or `payer`, while a contract reached by `CALL`, `STATICCALL`, or `CALLCODE` that reads its own storage reads a third account's, which is outside it. The restriction applies transitively through `CALL`, `DELEGATECALL`, and any other reachable frame, and it applies inside the `pay` frame whether or not the payer is the canonical paymaster: EIP-8141's canonical paymaster exemption is a public mempool rule about trusted shared state, and it does not widen this surface. A canonical paymaster whose `pay` frame reads its own reservation storage is therefore outside Profile 2, exactly as EIP-8369 states.

Writes are restricted to what EIP-8141, EIP-8250, and EIP-8272 permit for the validation prefix, and MUST additionally stay inside the surface.

### Code bound

Replay MUST count each distinct `codeHash` once and sum each distinct code body's byte length once. An account with empty code is not a body and costs nothing. A body is loaded, and charged on first load, wherever replay resolves an account's code: the dispatch of each executed frame to its target, every `CALL`, `CALLCODE`, `DELEGATECALL` and `STATICCALL` target, and every `EXTCODESIZE`, `EXTCODECOPY` and `EXTCODEHASH` target. An [EIP-7702](./eip-7702.md) delegation indicator is a body of its own, and the code it designates is a second body, loaded when execution follows the delegation.

A transaction whose replay would load more than `MAX_VALIDATION_CODE_BODIES` distinct code bodies, or more than `MAX_VALIDATION_CODE_BYTES` in total, is ineligible.

One budget of `MAX_VALIDATION_CODE_BODIES` bodies and `MAX_VALIDATION_CODE_BYTES` bytes is maintained per inclusion list, shared across every replay of that list's occurrences and across both evaluation states. A distinct `codeHash` is charged once, on first load by any replay of that list, and is free to every later replay of the same list. A replay whose next load would exceed the budget makes its transaction ineligible for that list; that load is not charged, no further code is charged for that replay, and an evaluator MAY abort it, since nothing it does afterwards can change the verdict. Charges survive the verdict: a replay that loaded bodies and then failed still made every attester read them. The same holds for a replay that ends undecided: what it loaded before failing stays charged, and a replay that fails before its first frame executes charges nothing.

Because the budget is shared, the verdict for one transaction depends on which replays ran before it. The [Omission check](#omission-check) therefore fixes the order in which replays run.

### Replay semantics

Each replay runs against one evaluation state and MUST NOT observe or persist anything else.

* Replay starts with the warm set EIP-8141 defines for a frame transaction's own execution and nothing more: `tx.sender`, the coinbase, and the precompiles are warm; `ENTRY_POINT` is cold; there is no access list. Every replay, at either state, starts from this set.
* The protocol verifier frames execute before the validation prefix, in transaction order, with the permissions EIP-8141 and EIP-8272 grant them and no others. A client MAY evaluate a protocol verifier frame directly instead of executing its code, as EIP-8141 and EIP-8272 already allow for mempool admission, provided the result, the gas, and the warm-state effects are identical.
* Every state change is discarded when the replay ends, including the `APPROVE` charge, keyed nonce consumption, and deploy frame installation.
* Replays are independent of each other, whether or not their transactions conflict, and independent across the two states.
* Fork rules are `B`'s.
* Body frames MUST NOT be executed and their outcome MUST NOT affect the verdict.

The only state shared across replays is the per-inclusion-list budgets.

### Budget fill

Occurrences are admitted over the inclusion list as delivered, in list order, before any deduplication, so every occurrence is metered even when the same transaction appears more than once. The evaluator computes this from the inclusion list alone, so every evaluator admits the same set. The debit is in two stages so that a structurally valid transaction with a bad signature pays for the signature check it caused and nothing more.

```python
def admitted(il):
    remaining = MAX_VERIFY_GAS_PER_IL
    out = []
    for occ in il.transactions:
        if not is_frame_transaction(occ) or occ.blob_versioned_hashes:
            continue                                  # not metered
        prefix_cost = prefix_and_verifier_frame_cost(occ)   # decoding and shape only
        if prefix_cost is None:
            continue                                  # unpriceable: ignored, no debit
        sig_cost = signature_verification_cost(occ)
        if sig_cost + prefix_cost > MAX_VERIFY_GAS_PER_TX or sig_cost + prefix_cost > remaining:
            continue                                  # does not fit: ignored, no debit
        remaining -= sig_cost
        if not signatures_valid(occ):
            continue                                  # keeps the signature debit only
        remaining -= prefix_cost
        if is_profile_2_candidate(occ):
            out.append(occ)                           # admitted
        # otherwise: charged, not admitted
    return out
```

A candidate that fails after its cost is deducted keeps the debit. Profile 1 candidates are not processed by this fill.

### Omission check

For a listed transaction `T` absent from `B`:

```python
def omission_unjustified(T, B):
    if not gas_fits(T, B):
        return False
    if is_profile_1_candidate(T):
        return eligible_profile_1(T, S_end)
    if is_profile_2_candidate(T) and any_occurrence_admitted(T):
        return eligible_profile_2(T, S_end) or eligible_profile_2(T, S_start)
    return False
```

If any listed transaction's omission is unjustified, `B` does not satisfy its inclusion lists and attesters MUST NOT vote for it.

Absent Profile 2 candidates are judged in the order of their first occurrence in the inclusion list, once each. For each, the evaluator replays at `S_end` first and then, only if the transaction was not found eligible at `S_end`, whether ineligible or undecided, at `S_start`; the `or` in `omission_unjustified` short-circuits in that order. An eligible replay ends the check for `B`. Profile 1 candidates consume neither budget, so their position relative to Profile 2 candidates is not consensus: an evaluator MAY judge them first and, on finding an unjustified omission, stop without opening any replay. This order is normative, not an optimisation: the code budget is shared across the list's replays, so a different order, or a replay run when this order skips it, charges a different set of bodies and can change a later verdict, or the same transaction's own verdict at the other state. An evaluator that has found `B` unsatisfied MAY continue replaying to record further omissions; nothing it finds afterwards changes the verdict.

A verdict that cannot be computed is not a verdict. If an evaluator cannot decide eligibility at a state, because the state cannot be opened, a code body matching a `codeHash` is missing, or replay fails for a reason internal to the evaluator, it MUST NOT treat that as ineligibility, and it MUST NOT report the payload unsatisfied on that account alone. It SHOULD record which transactions it could not decide, so that the failure is visible rather than silently excused; the record is local, a log entry or a metric, since the Engine API carries no field for it and the consensus layer is not meant to see it. An omission is unjustified only when a computed replay found the transaction eligible.

### Transactions outside enforcement

A listed transaction for which `omission_unjustified` is false is not enforced. Its omission is justified, attesters MUST NOT withhold a vote on account of it, and builders remain free to include it. This covers:

* any transaction with non-empty `blob_versioned_hashes`, including [EIP-4844](./eip-4844.md) transactions and frame transactions carrying blobs, since blob gas has its own target and maximum and no omission check is defined over it. The Engine API separately forbids an execution client from returning a blob transaction for an inclusion list, so a listed one indicates a non-conforming includer;
* frame transactions whose prefix reads outside the surface, violates EIP-8141 validation trace rules, or does not match an admitted shape, where the shape decides and not the frame count, since the admitted `only_verify | pay` shapes themselves contain two `VERIFY`-mode frames;
* frame transactions whose protocol verifier frames are out of position, duplicated, or refer to a predeploy whose code is not the canonical one;
* frame transactions whose `verify_budget_cost` exceeds `MAX_VERIFY_GAS_PER_TX`, or that no occurrence of the budget fill admitted;
* frame transactions sponsored by a canonical paymaster whose `pay` frame reads outside the surface.

### Includers and builders

Includers MAY source eligible transactions from the public mempool, a custom mempool, or direct submission. Public mempool admission is neither required for nor implied by eligibility.

For public mempool admission clients apply EIP-8141's storage rule, which admits `sender` storage at any slot and exempts the canonical paymaster's `pay` frame. For Profile 2 eligibility clients apply the surface in this EIP, which admits `sender` and `payer` below `AA_VOPS_SLOT_COUNT` and exempts nothing. Neither rule contains the other. Clients implementing both MUST apply the rule belonging to the question being asked and MUST NOT intersect them, so the same read may be permitted for one question and refused for the other.

Includers SHOULD simulate a candidate's validation prefix before listing it. This is local policy and does not affect enforceability.

Builders MUST include every listed transaction whose omission would be unjustified. Builders commit no evaluation index. A builder that places every listed transaction it can at the front of its payload, and retries the ones it had to skip after the rest of the payload, satisfies the rule for every transaction eligible at either endpoint the builder itself produces. A single pass at the front does not: a listed transaction whose predecessor the builder appends later in the same payload is ineligible at the front and eligible at the end, and its omission is unjustified. The two-pass construction is RECOMMENDED but not required, and EIP-7805's anywhere-in-block property is unchanged.

### Engine API

No Engine API method, parameter, structure, or version is added. Inclusion lists reach the execution layer through the `inclusionListTransactions` parameter EIP-7805's fork adds to `engine_newPayload`, and the execution layer reports whether they were satisfied through the `inclusionListSatisfied` field that fork adds to the payload status. This EIP extends the conditions under which that field is `false`.

Satisfaction is a field, not a status. [EIP-7805](./eip-7805.md) describes an `INCLUSION_LIST_UNSATISFIED` status; the Engine API defines none. `PayloadStatusV2` extends `PayloadStatusV1` with `inclusionListSatisfied`, a `BOOLEAN|null` that is set when the payload is `VALID` and `null` otherwise. An unsatisfied payload is therefore `VALID` with the field `false`, and the consensus layer withholds its vote on the field. Where EIP-7805's summary and the Engine API disagree, the Engine API governs.

Both `engine_newPayloadV6` and `engine_forkchoiceUpdatedV5` report the field, the latter for the payload named by `forkchoiceState.headBlockHash`, from the inclusion list the execution client retained when that payload was `ACCEPTED`. Retention lets the consensus layer re-read a verdict, not request a different one: the retained list is the same list, and this specification fixes the states it is judged at. Execution clients MUST compute the verdict from `B`'s own states, in whichever call imports `B`, and MUST report the same verdict from every later call about the same payload. A payload returned `ACCEPTED` is judged in the call that executes it. A client SHOULD retain the verdict alongside the list rather than recompute it on each call: recomputation is deterministic only while both states remain readable, and a state pruned between two calls turns a decided replay into an excused one, so the two calls would disagree.

### Activation

This EIP MUST activate at or after [EIP-7805](./eip-7805.md), [EIP-8141](./eip-8141.md), [EIP-8250](./eip-8250.md), and [EIP-8272](./eip-8272.md).

If `timestamp < FORK_TIMESTAMP`, clients MUST apply EIP-7805's omission check unchanged and MUST treat every frame transaction omission as justified.

If `timestamp >= FORK_TIMESTAMP`, clients MUST apply the omission check defined here.

### Implementation notes

These are not additional rules. They record how the existing implementations arrived at verdicts every evaluator must reproduce, and where a natural implementation goes wrong.

The Profile 2 replay is the same simulation EIP-8141 requires for public mempool admission, run with two things attached: the [validation surface](#validation-surface), which replaces the mempool's `sender`-only storage rule for the duration of the replay, and the per-list [code budget](#code-bound), which the mempool does not have. Everything else, the banned opcodes, the deploy-frame write rules, the protocol verifier frame permissions, and `APPROVE` semantics, is shared. The two uses MUST differ in exactly these places:

* The gas budget. The mempool uses the operator's `MAX_VERIFY_GAS`; the fill uses `MAX_VERIFY_GAS_PER_TX`. A shared budget parameter reads the wrong one.
* The storage rule. The mempool admits any `sender` slot and exempts the canonical `pay` frame; Profile 2 admits `sender` and `payer` below `AA_VOPS_SLOT_COUNT` and exempts nothing. A check that returns early for the canonical `pay` frame before consulting the surface applies the mempool rule to a Profile 2 replay.
* The gas sum. Shape matching filters the protocol verifier frames out of the prefix; the budget sums them back in. An implementation that prices the filtered list undercounts by the expiry frame and the recent-root frame, and disagrees with its own mempool.
* `current_slot`. The mempool judges a recent root against the head slot plus one, the earliest block that could carry the transaction; Profile 2 judges it at `B`'s own `slotNumber`, because the question is whether the root held in this block.

The pre-frame checks (keyed nonces, recent-root tuples, signatures) run before the EVM is constructed, so that a transaction failing any of them costs no replay; their order among themselves changes the work, not the verdict. Payer resolution follows, so the surface is fixed before the first opcode. Each evaluation state opens its own EVM over a state root, with `B`'s header as context and `B`'s fork rules.

### Interaction with EIP-7732

Under EIP-7732 the payload is revealed after beacon attestations and the payload timeliness committee does not validate execution, so no attester holds `S_end` when it votes. This is the expected case: FOCIL is specified as a fork layered on the one enshrining proposer-builder separation, and that fork's builder duties are already defined over a signed payload bid rather than a revealed payload.

Where EIP-7732 is active, Profile 2 omission checking belongs to the post-reveal payload validity duty and MUST NOT influence the beacon attestation. Profile 1 omission checking follows whatever EIP-7805 specifies for that fork. This is a statement about where the consensus layer consumes the verdict; the execution layer's computation of it is unchanged.

`S_start` is computable before reveal from the parent state alone. A future revision MAY use that to restore a pre-reveal half of the check. This EIP does not, because a pre-reveal check on `S_start` alone is weaker than the post-reveal check on both states and would give a builder a second verdict to target.

## Rationale

### Two endpoints

Judging at one point admits a payload where the transaction fails at that point. The complete rule is the union over every index, and it is unaffordable, since each distinct index is its own state reconstruction inside the attestation deadline. The question is which affordable subset to take.

A builder-chosen index is the worst affordable subset. End of payload is claimable, so the omissions it excuses are a superset of those the end-of-payload rule excuses, and the claim carries no cost, proof, or verification to offset that. The weakening reaches ordinary traffic: a sender submitting two frame transactions in sequence produces a queued transaction whose keyed nonce makes it invalid early in the payload and valid late, which the reference model's own position-stability analysis places outside the protected class.

The two endpoints are the subset that strengthens it. Both are states the evaluator already holds, so reconstruction cost is zero, and the number of states is two regardless of how many committee members list a transaction. It also removes the encoding, the deduplication rule across disagreeing includers, and the per-index reconstruction a claim requires. The union closes a hole in the end-of-payload rule alone: a transaction whose payer is drained by later payload transactions is valid at `S_start` and invalid at `S_end`.

### Gas fit

Block space is not position-dependent state. Gas remaining decreases monotonically within a payload, so a transaction that does not fit at the end never fitted less at the start in a way a builder could exploit, and evaluating fit at `S_start` would make every full block unsatisfied. That would convert the inclusion list into a hard priority claim on block space and remove EIP-7805's conditional-inclusion property. Each condition is evaluated where its dependency is decided: gas at the end because it is monotone, recent roots and the fee conditions at either because they are constant within the payload, and keyed nonces, payer balance, and installed code at both because another payload transaction can move them.

### No block access list dependency

The reference model reconstructs state at an arbitrary index by applying [EIP-7928](./eip-7928.md) changes for the transactions before it, and therefore requires every Profile 2 state update to appear in the block access list or be given an equivalent canonical diff as an activation prerequisite. Both endpoints are produced by ordinary execution, so that requirement does not arise here.

### A constant budget

`MAX_VERIFY_GAS_PER_IL = 2**20` is the value EIP-8369 proposed and the value the implementation runs. The budget is defined over the inclusion list as the execution layer receives it, one array per payload, so it bounds the metered Profile 2 replay at `2**20` gas per slot however many committee members contributed to that array, roughly one sixtieth of a 60,000,000 gas block, and it does not move when the gas limit does. An earlier draft defined the budget per committee member's list and bounded the replay at `2**24`; the Engine API delivers no list boundaries, so that rule was not computable by the execution layer.

A budget derived from the parent's gas limit was considered and set aside. It keeps the committee's share of the block constant as the limit grows, which is attractive, and an earlier draft of this document specified it. But it makes the admitted set depend on a header field, so a client that derives from the wrong block, or from a limit that changed between the list's construction and the payload, admits a different set from one that does not; it changes the budget's meaning with every limit vote; and no measurement yet says whether the attestation deadline scales with the gas limit at all. A constant is the value every evaluator can compute from nothing. Moving to a derivation is a one-line change for a later revision once the full pipeline has been benchmarked.

Setting `MAX_VERIFY_GAS_PER_TX` equal to `MAX_VERIFY_GAS_PER_IL` lets one transaction consume a list's budget. A lower per-transaction share would make the most expensive legitimate validation shapes unenforceable.

### Two-stage debit

Charging an occurrence's whole cost before checking its signatures lets a list full of structurally valid transactions with bad signatures exhaust the budget without any of them being enforceable. Charging nothing until the signatures pass lets the same list force sixteen committee members' worth of signature checks for free. Debiting the signature half first, and the prefix half only once the signatures pass, makes each occurrence pay for exactly the work it caused. This is EIP-8369's rule, restated.

### Code accounting

EVM gas does not bound bytes loaded: at a cold-account price of a few thousand gas, a list's gas budget admits hundreds of cold accesses, each able to pull a maximum-size code body. A separate byte bound is therefore required.

Charging each distinct `codeHash` once per list rather than once per transaction is what keeps the bound usable. The motivating pattern is many transactions validating against one shared verifier contract. Under per-transaction accounting that contract's bytes are charged once per transaction and a list of forty such transactions exceeds any workable bound, though an attester loads the code once.

`MAX_VALIDATION_CODE_BODIES` follows from the prefix shape: at most four frames, each resolving a target that may carry an [EIP-7702](./eip-7702.md) delegation indicator, giving eight bodies, doubled to admit one shared library per frame reached by `DELEGATECALL`.

### Slot count

The reference model leaves `AA_VOPS_SLOT_COUNT` unset with a candidate range of 2 to 4 pending benchmarks, and states that no implementation can classify Profile 2 for enforcement until it is chosen. Four is the top of the range. It is the worst case for replay, so a benchmark that fits the attestation deadline at 4 fits at 2 and 3. It is a superset, so no transaction eligible at a lower value becomes unreachable, whereas a low value makes wallets ineligible and presents as fewer enforcement obligations. It covers the realistic surface: one slot for an address owner, two for a P256 public key, a third for a threshold or module word, a fourth as headroom. Keyed nonces and recent roots live in protocol state and cost no slots.

### Storage rule selection

EIP-8141 admits `sender` storage at any slot and trusts the canonical paymaster's `pay` frame with shared state; this EIP admits `sender` and `payer` below `AA_VOPS_SLOT_COUNT` and trusts nothing. Neither contains the other. Intersecting them would produce a third rule that neither document specifies, excluding the paymaster reads Profile 2 exists to admit as well as the high-slot sender reads the public mempool has always allowed. Selecting by the question keeps each document authoritative for its own decision.

The canonical paymaster exemption does not carry over because Profile 2's surface is a promise about what an attester must hold, and a paymaster's reservation ledger lives at `keccak256`-derived slots no attester holds. A canonical paymaster can still sponsor Profile 2 transactions; it cannot read its ledger while doing so.

### Recent roots in the frame

An earlier draft of this document, and of EIP-8272, carried recent-root references as a field of the transaction envelope. EIP-8272 now carries them in a canonical `VERIFY` frame that leads the transaction and that the protocol checks before any other code runs. For this EIP the change is mostly a relocation: the tuples are read from that frame's data instead of the envelope, and the three conditions on them are unchanged. Two consequences are worth stating. The frame is executed by replay, so its declared gas is part of the budget, alongside the expiry frame's, which has always been. And the storage the frame reads is the predeploy's own, which is outside the surface for every other frame; it is readable only while that frame runs the canonical code at the top level, exactly as EIP-8272 permits for the mempool.

### Undecidable verdicts

An evaluator that cannot open a state, or cannot find a code body it needs, has learned nothing about the transaction. Reporting the payload unsatisfied on that basis withholds an attestation from a block that may be honest, on the strength of a local failure that other evaluators may not share; that is a network split originating in one node's disk. Reporting it satisfied is an excused omission that should not have been excused, but it is the same verdict every evaluator without the data reaches, and it is the direction EIP-7805 already errs in for every ambiguity. The rule therefore excuses and records, and asks the implementation to make the record visible rather than to pretend the verdict was computed.

### Protocol verifier frame gas

Shape matching is defined over the frames carrying the sender and payer decision, so the natural implementation builds a prefix-frame list with the protocol verifier frames filtered out and prices that list, leaving the budget short by their declared gas while replay executes them. The implementation this document describes made exactly that error for the recent-root frame, after having fixed it for the expiry frame, and caught it only by comparing its fill against its own mempool. EIP-8272 states the rule for the mempool; EIP-8141 rule 6's "across the validation prefix" is silent on the expiry frame and SHOULD be clarified the same way.

## Backwards Compatibility

This EIP changes block validity as seen by the fork choice: a block previously attested to may now fail to satisfy its inclusion lists. It requires a hard fork.

Before activation, frame transaction omission is excused unconditionally, which is the correct interim behavior, since an evaluator that cannot decide must not withhold a vote from an honest block.

No transaction encoding, receipt encoding, or Engine API signature changes. A client implementing [EIP-7805](./eip-7805.md) and none of the frame transaction EIPs is unaffected, since Profile 1 restates EIP-7805's existing check.

The engine method version carrying inclusion lists is the same version carrying every other execution-payload addition of its fork. A fork adding both inclusion lists and a block access list field to `engine_newPayload` has one new version, not two.

## Test Cases

Each case names a listed transaction absent from `B`. `E` means the omission is unjustified and `B` is unsatisfied; `J` means it is justified and `B` is satisfied.

| # | Case | Verdict |
| ---: | --- | :---: |
| 1 | Profile 2 candidate eligible at both states | `E` |
| 2 | queued frame transaction, `nonce_seq == 1`, predecessor executes in `B`: keyed nonce fails at `S_start`, passes at `S_end` | `E` |
| 3 | payer solvent at `S_start`, drained by a later transaction of `B` | `E` |
| 4 | keyed nonce already consumed before `B` | `J` |
| 5 | recent-root verifier frame names a tuple whose entry is not committed, or whose slot is outside the window at `B.slotNumber` | `J` |
| 6 | signature in `tx.signatures` does not verify, structurally valid otherwise | `J`; only the signature half of the budget is debited |
| 7 | prefix reads storage slot `AA_VOPS_SLOT_COUNT` of `sender` | `J` |
| 8 | prefix reads a `keccak256`-derived slot of `payer` | `J` |
| 9 | prefix reads a third account's storage through a call into that account; a `DELEGATECALL`ed library reading its caller's slots below `AA_VOPS_SLOT_COUNT` stays inside the surface | `J` |
| 10 | canonical paymaster `pay` frame reads its reservation storage | `J` |
| 11 | frame with a mode EIP-8141 does not define | `J` |
| 12 | non-empty `blob_versioned_hashes`, eligible otherwise | `J` |
| 13 | `ATOMIC_BATCH_FLAG` on a validation prefix frame | `J` |
| 14 | `VERIFY`-mode body frame | `J` |
| 15 | `verify_budget_cost` exceeds `MAX_VERIFY_GAS_PER_TX` only once the expiry frame's gas is counted | `J` |
| 16 | `verify_budget_cost` exceeds `MAX_VERIFY_GAS_PER_TX` only once the recent-root verifier frame's gas is counted | `J` |
| 17 | second occurrence in a list whose first consumed all of `MAX_VERIFY_GAS_PER_IL` | `J` for the second |
| 18 | two occurrences at half the list budget each, the first with a bad signature | `E` for the second; the first debits only its signature half |
| 19 | two occurrences of one transaction in the delivered list, the first admitted, the second not fitting the remaining budget | `E`; one verdict per transaction, every occurrence metered |
| 20 | a byte-distinct variant is present in `B` | `E` |
| 21 | `total_gas_limit` exceeds `B.gas_limit - B.gas_used` but not `B.gas_limit`, eligible at both states | `J` |
| 22 | replay loads a 17th distinct code body | `J` |
| 23 | forty transactions in one list sharing one verifier, distinct code under `MAX_VALIDATION_CODE_BYTES` | `E` for each |
| 24 | payer holds more than `gas_limit * effective_price` but less than `max_gas * max_fee_per_gas` | `J`, `APPROVE` reverts |
| 25 | recent-root verifier frame present but not in the leading position | `J` |
| 26 | recent-root verifier frame present and the code at `RECENT_ROOT_ADDRESS` is not `RECENT_ROOT_CODE` | `J` |
| 27 | state at `S_start` cannot be opened, transaction eligible at `S_end` | `E` |
| 28 | state at neither endpoint can be opened | `J`, recorded as undecided |

Cases 2 and 3 distinguish this EIP from an end-of-payload rule; case 2 also distinguishes it from a builder-claimed index, which excuses that omission. Case 21 pins the boundary: state validity is judged at two points, block space at one. Cases 15 and 16 pin the budget sum against the shape-matched prefix. Cases 27 and 28 pin that an undecidable state is neither evidence for nor against.

Rows 27 and 28 need a state that cannot be opened, which no block on a healthy chain produces; they are checked by driving the omission check with an evaluator that reports the failure, not by building a block. Row 23 is a statement about the code budget (one shared verifier is one body however many transactions load it) and is checked there.

These rows are scenario descriptions, not vectors. The transaction encodings, state seeds and expected budget balances that would make "checked against" mean the same thing on every client are still to be published; until they are, two implementers can build different scenarios from one row.

## Security Considerations

Two endpoints are not a proof. A builder controlling the payload can construct one where a listed transaction fails at both, with an unmet dependency at `S_start` and a drained payer at `S_end`. Only the union over every index is a proof, and it is unaffordable. This EIP claims a better point on the cost and guarantee curve than an end-of-payload rule or a builder-chosen index, not censorship resistance in the strong sense.

A transaction whose payer is shared with other sponsored transactions holds no reservation on that payer's balance. It is protected whenever it is solvent at `S_start`, which the end-of-payload rule does not give it, but a payload draining the payer before `B`'s first transaction defeats both.

Budget fill is griefable. A transaction may consume a list's gas budget while valid and then be invalidated by a cheaper conflicting transaction included earlier in `B`, denying the rest of that list. Includer simulation does not prevent it, since the conflicting transaction need not exist when the list is built. The per-list code budget has the same property. Both are accepted in exchange for a fill every evaluator computes identically from the list alone; a stateful fill would have to be recomputed per state and would reintroduce the ordering problem this EIP removes. Because the execution layer sees one list per payload, one committee member's occurrences can consume the budget every other member's transactions needed, and the consensus layer's aggregation order decides which are denied.

Total attester work exceeds the metered budget. Transaction decoding, signature verification of rejected candidates, the pre-frame keyed nonce and recent-root checks, and code loading sit outside `MAX_VERIFY_GAS_PER_IL`; the code bound covers the largest of those. The full path including both replays must be benchmarked against the attestation deadline before activation, since the constants fix the budget's size, not its wall-clock cost.

The globally held surface is bounded by field type, not total size. Holding the first `AA_VOPS_SLOT_COUNT` slots of every account, the code corpus, keyed nonce entries, and recent-root sources all grow. A test network measures replay time only and is no evidence about this, so parameter selection must account for both.

Verdicts diverge if evaluators disagree on warm-state initialization, `current_slot`, the base fee, whether the protocol verifier frames' gas counts, whether the canonical paymaster exemption applies, or which storage rule applies. Each is fixed above for that reason. A code path shared with public mempool admission is the likeliest source of divergence, since the operator-tunable budget, the EIP-8141 storage rule, the canonical paymaster exemption, and the head-plus-one `current_slot` all live there; [Implementation notes](#implementation-notes) names each.

Every ambiguity resolves toward excusing the omission, since an unsatisfied verdict withholds an attestation from a block that may be honest. An evaluator that cannot compute a verdict excuses and records rather than guessing in either direction.

A transaction submitted through a custom mempool or direct endpoint is enforceable only if a committee member received it. Direct endpoints carry denial-of-service, private access market, and metadata risks and need bounded admission policies; none of that is consensus, and none of it changes the eligibility of a transaction that reaches a list.

Privacy protocols remain responsible for binding their proofs. For single-use nullifiers expressed as keyed nonces, validation must authenticate `sender`, the full `nonce_keys` set or its canonical hash, `nonce_seq == 0`, the exact recent-root tuple the verifier frame carries, the chain and contract domain, and the authorized execution and payment intent. The eligibility surface does not substitute for that binding.

## Copyright

Copyright and related rights waived via [CC0](../LICENSE.md).
