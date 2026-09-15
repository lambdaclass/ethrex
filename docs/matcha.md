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
excludes application execution after the payer is established, and excludes the state
dimension entirely, because neither is work the client does at admission.

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

## Where it lives

`crates/blockchain/matcha.rs` holds the ledger and is pure bookkeeping: no I/O, no locks.
The mempool owns an instance and calls it under its own write lock, so the balance check
and the insert are atomic. Two concurrent additional transactions from one sender cannot
both pass against the same balance.

The charge is computed before the lock, where the validation prefix has just been
validated, and carried into the locked section as a `MatchaCharge`. The decision about
whether a transaction is *additional* is made under the lock, because it depends on what
else the sender has pending at that instant.

Credit comes from finality, never from the head: `Blockchain::credit_finalized_width`
walks newly finalized blocks and credits each sender the gas its own frame transactions
used. A head-based credit would let a sender earn from a block that is later reorged out
and spend the width on work the chain never paid for.

## Local policy

`MatchaConfig` carries the cap, the safety factor, the optional linear-fee `base_price`
and an on/off switch. The post is explicit that these are local policy rather than
consensus, and nothing here is observable to other nodes.

The linear fee is off by default. The post presents it as optional deterrence, and whether
FOCIL alone makes it unnecessary is an open question the post itself raises.

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
- Width is required for additional transactions regardless of whether the prefix is
  structurally independent, following the post's reply that the balance-drain vector
  applies independently of mass invalidation.

## Bootstrapping

A newly deployed shared sender has no finalized history, so it has no width, so it gets
one pending transaction until its first transaction finalizes. This is the design working
as intended, and it is sharp enough to surprise: it applies exactly when an application is
trying to attract its first users. Tests that exercise concurrency credit the sender
first, which is what a real deployment reaches by running.
