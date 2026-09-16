# MATCHA in ethrex

An implementation of *Mempool Account Transaction Capacity from Historical Activity*:
letting one `sender` hold several pending EIP-8141 frame transactions with independent
EIP-8250 nonce keys, without the client whitelisting the application, paymaster, verifier
or proof system behind them.

The proposal: https://ethresear.ch/t/mempool-account-transaction-capacity-from-historical-activity-matcha/25949

## The problem it solves

EIP-8250 lets one address send transactions under independent nonce keys, which is what a
shared-sender privacy application needs: many users, one `sender`. Relaxing the
one-transaction-per-sender rule opens two doors at once.

A malicious application can submit many valid transactions whose prefixes read the same
shared state, then change that state once and invalidate all of them, making the client
revalidate the lot. And a malicious user of an honest application can fill the shared
sender's capacity with cheap transactions so nobody else gets in.

Both are cheap for the attacker and expensive for the client. MATCHA prices them.

## Width

**Width** is capacity for pending frame transactions beyond the one EIP-8141 already
allows. Three properties make it a defence rather than a fee.

**Earned, not granted.** A sender starts with none and accrues it from the gas its own
frame transactions used in newly finalized blocks, capped. Buying capacity means first
paying for blockspace that was actually included.

**Spent irreversibly.** Width is not returned when a transaction is included, invalidated,
replaced or evicted. The client did the work either way. Refunding on invalidation would
make mass invalidation free for whoever caused it.

**Charged for work.** The charge tracks admission gas, which is what the client spends
deciding whether to keep a transaction, including every later revalidation.

## What is charged

```
charge = ceil(safety_factor * admission_gas)
```

`admission_gas` is the intrinsic and per-frame cost, the signature verification cost, the
transaction's data cost, the validation prefix's declared `limits.execution` including the
EIP-8272 recent-root verifier frame, and one cold storage read per EIP-8250 nonce key. It
excludes application execution after the payer is established, the state dimension, and
the value-transfer cost, because none of those is work the client does at admission.

The prefix figure is the same sum `MAX_VERIFY_GAS` is measured against, passed in rather
than recomputed, so the charge and the budget cannot drift about which frames count as
validation work.

The safety factor is above one because the client's real cost exceeds the gas it can
attribute: admission also touches the pool's structures, the reservation maps and the
eviction bookkeeping, none of which appears in a gas figure.

## When it is spent

| Event | Width |
| --- | --- |
| The baseline transaction | free |
| An additional transaction | one charge |
| Re-announcing a transaction already pending | free, one pool record |
| Replacing the baseline | free |
| Replacing an additional transaction | another charge |
| Revalidating after a new head | its stored charge, before the work |
| Removal, inclusion, eviction, expiry | nothing returned |
| Past the maximum pending lifetime | dropped; re-admission spends again |

Revalidation is charged only when a rerun actually happens. If the new head's state cannot
be opened the prefix is not re-simulated, so there is no work and nothing is spent. The
charge is taken after the cheap drops (expiry, recent-root window, structure) and before
any EVM is built, which is what "before the work begins" means in practice.

## Where it lives

`crates/blockchain/matcha.rs` holds the ledger and is pure bookkeeping: no I/O, no locks.
The mempool owns an instance and calls it under its own write lock, so the balance check
and the insert are atomic. Two concurrent additional transactions from one sender cannot
both pass against the same balance.

The charge is computed before the lock, where the validation prefix has just been
validated, and carried into the locked section as a `MatchaCharge`. The decision about
whether a transaction is *additional* is made under the lock, because it depends on what
else the sender has pending at that instant, and the spend is the last check before any
insertion or removal, so a transaction the lock rejects for any other reason has spent
nothing. The effective priority fee the optional floor is judged against is computed
against the *next* block's base fee, since that is the block the transaction is admitted
for.

Credit comes from finality, never from the head: `Blockchain::credit_finalized_width`
walks newly finalized blocks and credits each sender the gas its own frame transactions
used. A head-based credit would let a sender earn from a block that is later reorged out
and spend the width on work the chain never paid for.

## Paymasters

EIP-8141 exempts a canonical paymaster (the pinned runtime, one instance per sponsor) from
the one-pending rule that binds non-canonical ones, and bounds it by balance alone:
`available = balance - reserved_cost`. Composed with the free sender baseline, that
exemption was the one place the mechanism did not reach. Every fresh sender gets a free
pending transaction, a canonical paymaster can fund an unbounded number of fresh senders,
and the capital behind them is only reserved, never spent: `balance / max_cost` pending
transactions per node, re-simulated on every head, all invalidated by one ordinary
transaction that moves the paymaster's balance, and the same balance funds the next batch.
Sender width never sees it, because none of those transactions is additional for its
sender.

So the ledger is keyed on the payer too. A canonical paymaster earns width from the gas
of the transactions it paid for in finalized blocks (`credit_finalized_width` credits the
receipt's payer alongside the sender when they differ). Its first pending sponsored
transaction is its free baseline in that role; every further one spends the same charge
from the paymaster's width, and every revalidation debits it again. A transaction that is
additional for both its sender and its paymaster pays both, and both ledgers are checked
before either is debited, so a refusal for one never costs the other. The refusal is its
own error, naming the paymaster and its figures.

What that buys: parking six thousand sponsored transactions would need about 3.4 billion
width, which the cap makes impossible, so a paymaster holds about fifty per node at the
default charge, and each drain-and-refill cycle debits all of them. Idle capital stops
counting; gas actually paid for in finalized blocks is what buys sponsoring capacity.
Self-payers are charged as senders only, and non-canonical paymasters keep their
one-pending rule, the structural gate width stacks behind.

## Reading the ledger

A wallet that relays for many senders wants to schedule rather than probe, so the two
figures admission judges are readable before a broadcast.

`ethrex_matchaWidth(address)` returns this node's ledger for one account: `width` (earned
and not spent), `widthCap`, `lastCreditedBlock`, `pendingFrameTxs` (the first of which is
the free baseline), `pendingCharges` (what those pending additional transactions paid),
`pendingSponsored` and `pendingSponsoredCharges` (the same two in the paymaster role),
`load` (the pool-wide term the linear fee reads), and the policy knobs. The answer is
node-local by construction: width is credited from finalized blocks, which every node
sees alike, and spent by what this node admitted, so it is exact for this node's
admission and only approximate for another's.

`ethrex_simulateFrameTransaction` gained three fields, priced from the prefix it already
validates: `matchaCharge`, the width admitting the transaction as an additional pending
one would spend, computed by the same function admission uses; `matchaAdmissible`, whether
admission would take it right now, as a baseline or against the sender's width, the
canonical paymaster's width when one pays, and the fee floor; and `matchaRefusal`, the error admission would return when it would not. Like
`valid`, admissible is necessary rather than sufficient, and it can go stale the moment it
is answered, so the refusal on `eth_sendRawTransaction` remains the authority.

## Local policy

`MatchaConfig` is local policy, not consensus, and nothing in it is observable to other
nodes. Every field has a CLI flag under `--mempool.matcha-*`, plus `--mempool.no-matcha`
to switch the mechanism off and fall back to the structural EIP-8250 rule alone.

| Field | Default | Flag |
| --- | --- | --- |
| `width_cap` | 30,000,000 gas | `--mempool.matcha-width-cap` |
| `safety_factor` | 3/2 | fixed |
| `base_price` (linear fee) | 0, off | `--mempool.matcha-base-price` |
| `min_validity_slots` | 0, off | `--mempool.matcha-min-validity-slots` |
| `max_pending_lifetime` | 3 hours | `--mempool.matcha-max-lifetime-secs` |

The two optional deterrence policies the proposal describes are both implemented and both
off by default, since it leaves open whether FOCIL alone makes them unnecessary. The
linear fee raises the priority-fee floor for additional transactions by `base_price` per
pending charge of the same size. The minimum validity period refuses an additional
transaction whose EIP-8141 expiry deadline or any EIP-8272 recent root would stop being
valid within `min_validity_slots` of the next block, so a transaction cannot be admitted
only to expire before it can be built.

The maximum pending lifetime is on by default and applies to every pending frame
transaction, baseline included, because the proposal says "every pending transaction".
A removed transaction is unaffected on chain and may be resubmitted; if it is additional
that costs another charge.

## Divergences and judgement calls

Recorded in full, with reasoning, in the implementation notes kept for the proposal's
author. In summary:

- The post names the EIP-8250 nonce checks as a term of `admission_gas` without pricing
  them. Charged here per key, as one cold storage read each, because the work is per key.
- The linear fee's step is defined only for equal charges. Generalised here as
  `base_price * (1 + load / charge)`, which reproduces the stated behaviour exactly when
  charges are equal.
- "Newly finalized" needs a rule when a node has been offline. Catch-up is bounded to the
  last 64 finalized blocks; withholding width is always the safe direction.
- "Credited once" is keyed on block number with a high-water mark, not on block hash, so
  two blocks at one height after a reorg cannot both mint width.
- Finalized gas is the transaction's `cumulative_gas_used` delta, which for a frame
  transaction sums both gas dimensions. Width is earned from what was paid for.
- Width is required for additional transactions regardless of whether the prefix is
  structurally independent, following the post's reply that the balance-drain vector
  applies independently of mass invalidation. The structural EIP-8250 eligibility test is
  retained as well: a structurally dependent sender is still held to one pending
  transaction however much width it holds. That is the conservative of the two readings
  the post admits; the other, width as the whole mechanism with no structural cap, is
  what the post argues for, and which is intended is an open question put to its author.

## Bootstrapping

A newly deployed shared sender has no finalized history, so it has no width, so it gets
one pending transaction until its first transaction finalizes. This is the design working
as intended, and it is sharp enough to surprise: it applies exactly when an application is
trying to attract its first users. Tests that exercise concurrency credit the sender
first, which is what a real deployment reaches by running.

The devnet conformance script, `scripts/hegota-testnet/verify_devnet.py`, asserts this
against a live node: a fresh contract sender's second key is refused with the width error.
With `HEGOTA_VERIFY_MATCHA_EARN=1` it goes on to mine three more baseline transactions,
wait for the finalized tag to pass them, and then admit and mine two keys at once, which
is the only check that proves the credit actually fires at finality.
