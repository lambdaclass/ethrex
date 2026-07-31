# EIP-8312: where native UTXOs earn their keep

An assessment of what EIP-8312 is actually *for*, written after implementing it and running it on
a devnet. Sources: the [spec](https://github.com/nerolation/EIPs/blob/a5da3f608c6dfbf353bea264054d99fc164ab10c/EIPS/eip-8312.md)
@ `a5da3f60`, the ethresear.ch thread *Native UTXOs on Ethereum*, and measurements from our own
chain. Implementation detail lives in [eip-8312.md](eip-8312.md).

The short version: the value is **concentrated, not general**. This is a payments primitive, and
it is strongest exactly where payments are one-shot, high-volume, and directed at recipients who
do not yet exist on chain. It is not a replacement for accounts, and for some uses it is strictly
worse than one.

## The measured baseline

Everything below rests on four numbers, all from receipts on our devnet:

| Flow | Gas | Permanent state |
|------|-----|-----------------|
| Vault deposit (create one UTXO) | 36,334 | none |
| Spend (1 input, depth-1 proof, 1 account output + change) | 56,094 | one spent bit |
| Full cycle | 92,428 | ~0.3 B |
| Plain transfer to a fresh account | 204,600 | ~120 B account leaf |

Figures are from one measured run; spend gas varies by a few tens of gas with the recipient
address's zero-byte count in the frame's calldata.

So the complete create-and-spend cycle is ~2.2× cheaper than a *single* fresh-account transfer.
The state-dimension gap is much wider than the gas gap, and under EIP-8037 it is the state
dimension that saturates a block first — so the throughput difference exceeds the cost difference.

We also confirmed the headline claim directly rather than trusting it: an address holding **zero
wei** spent its UTXO, paid a third party, and the fee came out of the payment. No faucet, no
sponsor, no pre-funding.

## Where it is transformative: one-time and stealth addresses

This is the case where the properties *compound* instead of merely adding up, and it is the one
worth leading with.

ERC-5564 stealth addresses have a chronic structural problem. Each stealth address is a fresh
account, so it (a) writes a permanent account leaf and (b) holds no ETH — meaning it cannot spend
until someone funds it, and that funding transaction links back to the payer, which is precisely
the linkage the stealth address existed to break. The usual workaround is a relayer: more
infrastructure, more trust.

EIP-8312 removes both halves at once. The recipient has no account leaf at all, and the spend is
self-funding, so the stealth address never holds ETH and never appears as `tx.sender`. Then the
multi-actor spend goes further: several stealth payments to the same owner merge into **one spend,
one fee, one signature set** — simultaneously a cost win and a privacy win, since batching
conceals which payment is being moved.

That is not an efficiency tweak. It removes the main reason stealth addresses see little use.

## Where it is clearly valuable: payouts that may never be claimed

Airdrops, exchange withdrawals, payroll, faucets, refunds. Two wins stack:

**Unclaimed costs nothing permanent.** The state charge lands at *consumption*, not creation. An
airdrop to 100,000 addresses where 30% never claim writes zero permanent state for that 30%,
forever. Both of today's options pay upfront for claims that may never happen: direct transfers
write 100,000 account leaves, and a claim contract writes a storage slot per claimer.

**Recipients need no gas to claim.** An airdrop to unfunded addresses is close to unusable today —
the recipient needs ETH to claim ETH. Here the claim pays for itself out of the payment.

Because blocks are state-gas-limited, this is arithmetic rather than rhetoric: a payout size that
simply does not fit in a block as plain transfers fits comfortably as UTXOs.

## Where it is a quiet operational win: sponsored relaying at scale

Two things that only became apparent while implementing the mempool and admission paths.

A sponsor verifies its repayment by reading **declared data** — the spend's outputs and fee caps —
so it needs no simulation and cannot be griefed by execution behavior.

Less obvious, and more interesting: a vault-sender transaction consumes **no nonce**. A relayer
today is throughput-bounded by its own nonce sequence, which is why relayer fleets exist. That
ceiling disappears — one sponsor can serve unlimited concurrent spends.

## Where it matters later: post-quantum signatures

Underrated in the discussion so far. Hash-based post-quantum signatures are kilobytes, and
payments are the highest-volume traffic class — exactly where signature size hurts most. Because
the spend hash commits to no signature bytes, and a vault-sender envelope is unsigned, signatures
can be aggregated **after signing, while in flight**. Very few designs leave that door open, and
it costs nothing to preserve now.

## Where it does *not* shine

Worth stating plainly; overselling this would be a mistake.

- **Anything with persistent state.** A recipient must be a keyholder, so a contract recipient is
  silently unspendable. No DeFi positions, no NFTs, no ongoing contract relationships.
- **Long-term holding is strictly worse than an account.** You must custody the opening *and*
  refresh the witness before the ring wraps (~27 h at 12 s slots), and after EIP-4444 the history
  needed to rebuild a proof may not be retrievable. An account balance needs only a key; a UTXO
  needs a key plus data you can lose. This is the sharpest objection raised in the thread, and it
  is a real trade rather than a rounding error: permanent state is exchanged for a
  data-availability assumption. It wants a defined retention domain for openings.
- **Chained or high-frequency payments.** One-block minimum latency, and no spending of an
  unconfirmed output — unlike Bitcoin, where mempool chaining is routine.
- **Small volumes to established users.** If the recipient already has an account and will keep
  using it, a plain transfer is simpler and the deposit overhead is not repaid.
- **Tokens.** ETH only in this draft; token-carrying openings are deferred.
- **Discovery pollution.** Discovery is "scan `topics[2]` for my address", so an attacker can
  cheaply mint zero-value UTXOs to bloat a victim's tracking set. That griefing lands specifically
  on the wallet UX of the flagship stealth use case.

## A note on the design, from having built it

The thread argues that frames beat a `SPEND_UTXO` opcode because an opcode runs *after* gas
prepayment, so a zero-ETH recipient could never start. Implementing it made that concrete rather
than theoretical: the payer is assigned **inside** the frame, after conservation proves the inputs
cover `max_cost`, and the vault is debited at that point. There is no way to express "the inputs
pay the fee" once the fee has already been collected.

The frames choice is load-bearing, not stylistic — and correspondingly, the self-funded path was
the fiddliest part of the implementation to get right.
