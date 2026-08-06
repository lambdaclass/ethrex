# EIP-8250 on the Hegotá Devnet — Notes for the Spec Author

ethrex implements EIP-8250 alongside EIP-8141, EIP-8272 and EIP-7906 and runs the
combination on a devnet. Most of what we found has already been reconciled — the
TXPARAM index, the `NONCE_MANAGER` address and code, the gas quantities, and the
revalidation requirements all match what we ship. What follows is what has not.

- **Spec read:** `ethereum/EIPs` `EIPS/eip-8250.md` @ `8ff5c1359`, with
  `EIPS/eip-8141.md` at the same point.

## 1. §Nonce consumption contradicts EIP-8141's atomic-batch rule

EIP-8250 requires the five effects of a successful payment-scoped `APPROVE` —
nonce consumption, maximum-cost collection, payer recording, first-use gas
charging, and the approval-context updates — to be

> journaled outside the current frame's revert journal and outside any EIP-8141
> atomic-batch snapshot […] MUST NOT be reverted by a later frame revert, by
> skipping later frames, or by restoring an atomic-batch state snapshot.

EIP-8141 §Behavior says the opposite about the same event:

> When a frame in the batch fails, the state must be rolled back to the condition
> it was immediately before the atomic batch began.

8141 states that unconditionally and does not acknowledge an exception; 8250
asserts one. Since 8250 requires 8141, an implementer is handed both rules, and
the two produce different state for the same block — a payment `APPROVE` inside a
batch whose later member fails is either rolled back (transaction invalid, no
payer) or preserved (transaction valid, normal refund), and both readings are
internally consistent.

**This looks settled in 8141's favour, and 8250 is the side left behind.** Open PR
#12109 against EIP-8141 adds the missing unroll rule: the approval context is
discarded together with the batch's state, nonce increment and `max_cost`
collection included, and the transaction is invalid if `payer` ends unset. If that
merges as written, 8250 §Nonce consumption still says the reverse, and the family
ships two contradictory normative sentences rather than one gap.

**Suggested resolution:** narrow 8250's durability sentence to the cases 8141
agrees on — a *later* frame's revert, and skipped frames — and drop the
atomic-batch-snapshot clause, deferring to 8141's unroll rule. The protection that
motivated the sentence survives: a payment `APPROVE` in a non-batch frame keeps its
consumption when a later batch reverts, because that frame's effects are already
committed outside the batch's scope, so the replay/DoS vector the rule closes stays
closed.

If instead the intent is that the effects genuinely do survive a batch unroll, then
#12109 is the wrong direction and 8141 needs the carve-out rather than the unroll —
in which case 8250 would also need to say how the maximum-cost collection is
re-applied after the snapshot restore, exactly once, consistently with the
end-of-transaction refund and the EIP-7928 access list. Either answer is
implementable; what is not is both.

## 2. `APPROVE_PAYMENT` is reachable from any frame mode

Both of the above hinge on this and it is worth stating separately, because it is
what makes the conflict reachable at all: the `APPROVE` instruction carries no
frame-mode restriction, so a DEFAULT or SENDER frame may carry `ATOMIC_BATCH_FLAG`
and grant payment. Batches cannot contain VERIFY frames (#11987), so if payment
approval were VERIFY-only the whole case would be structurally impossible.

It is not reachable through the public mempool — payment is granted in the
validation prefix, and the prefix may not carry the batch flag — so in practice it
takes a crafted block.

**Suggested resolution, if the durability sentence is kept:** restrict
`APPROVE_PAYMENT` to `VERIFY` frames. One sentence in EIP-8141, and the conflict in
§1 becomes moot instead of contradictory. It appears free: every validation-prefix
shape grants payment from a `VERIFY` frame, as does the EIP-7906 paymaster frame, so
no legitimate flow needs a non-`VERIFY` payer. This also completes what #11987
started — #11652's "batching with any frames" is what left a non-`VERIFY` frame able
to grant payment inside a batch.

## 3. Concurrent keyed transactions per sender (deliberate divergence)

Recorded for completeness; it is already documented on our side and is admission
policy, never consensus. §Mempool restates the pending identity as
`(sender, nonce_keys, nonce_seq)` while keeping EIP-8141's one-pending-frame-tx-per-sender
limit verbatim. ethrex admits disjoint non-zero key sets from one sender
concurrently, which is the keyed-aware mempool the Motivation describes as future
work — holding to one pending transaction per sender would leave the EIP with no
observable effect on this node, since replay-independent keys are the whole point.
It strictly admits a superset of what the spec's rule admits, so blocks stay
mutually valid. Worth a ruling on whether the inherited limit is deliberately
reaffirmed or merely carried over.
