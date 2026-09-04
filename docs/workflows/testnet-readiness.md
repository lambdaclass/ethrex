# Testnet readiness checklist

A testnet is ready when a stranger with no access to our servers can join it,
use it, and leave it, using only what we published. Every item below is
phrased as something you *verify from outside*, because the failures that
matter are the ones invisible from a host that has been in the chain since
genesis.

Run the whole list before announcing a testnet. Re-run sections A, C and D
after any re-genesis, and section E after any change to the chain config.

The **Gotchas** at the end are real failures caught by this checklist on
`frames-testnet` (2026-08-28). Each one shipped a broken user experience that
looked fine from the deploying host.

---

## A. Chain configuration (before genesis)

- [ ] **Chain ID and network name are unique** and not shared with any other
      devnet you run. Collisions silently cross-wire wallets and explorers.
- [ ] **Fork schedule is the one you meant.** Print the client's own fork table
      at startup and read it, rather than trusting the input YAML:
      `docker logs <el> | grep -A20 "hard forks"`
- [ ] **The feature under test is actually active at the fork epoch you set.**
      A fork epoch far in the future produces a chain that looks healthy and
      exercises nothing.
- [ ] **`SHARD_COMMITTEE_PERIOD` is compatible with the testnet's lifetime.**
      This is the minimum number of epochs between a validator activating and
      it being *allowed* to exit. The mainnet preset value of `256` is roughly
      13.6 hours at 6 s slots, which makes validator egress untestable on a
      short-lived devnet. Lower it deliberately, or accept that exits cannot
      be exercised.
- [ ] **Deposit gating configured and its policy written down** — which
      withdrawal-credential prefixes are allowed, which require a token, and
      who holds the admin role. Verify with `gating-cli status`, not from the
      template.
- [ ] **Genesis admin keys are not derived from a public mnemonic.** A genesis
      admin is granted *sticky* rights; a default-mnemonic admin is an
      unrevokable public mint.

## B. Genesis and published artifacts

- [ ] **Bundle is complete**: `genesis.json`, `genesis.ssz`, `config.yaml`,
      `deposit_contract.txt`, `deposit_contract_block.txt`,
      `genesis_validators_root.txt`, EL bootnodes, CL bootnode ENRs.
- [ ] **Every artifact is reachable over HTTP from off-site** — fetch all of
      them from a machine outside the deployment, not with `curl localhost`.
- [ ] **`chainId` in the published `genesis.json` matches the running chain**
      (`eth_chainId`), and the genesis hash matches what the nodes log.
- [ ] **The advertised P2P address is externally routable.** See gotcha 3 —
      this is the single most common way a testnet becomes unjoinable, and
      for the CL it is unrecoverable by the joiner.

## C. Node software and feature verification

- [ ] **The published image tag corresponds to a known commit**, and that
      commit is the one that passes the feature's test suite. Record both.
- [ ] **The feature works on-chain, observed in a real block** — not only in
      unit tests. Send the transaction type or opcode the testnet exists for
      and cite the block number.
- [ ] **The EL serves the Engine API methods the CL asks for.** Check the CL's
      startup warnings; a "does not support some requested engine methods"
      line is a real gap even when the chain runs.
- [ ] **A brand-new node can sync and reach head.** Test from a clean datadir
      on a different host, and test *each* path you intend to document:
      - genesis sync
      - checkpoint sync (`--checkpoint-sync-url`)
      A node that cannot reach head cannot serve RPC, so this gates everything
      downstream. Confirm the EL reaches head, not just the CL: compare
      `eth_blockNumber` against the remote node, and require the CL to report
      `sync_distance: 0` **and** `is_optimistic: false`. See gotcha 5.
- [ ] **All nodes in the deployment agree with each other.** Compare
      `eth_blockNumber` across every execution node and the finalized epoch
      across every beacon node. Per-node health checks cannot see a partition:
      every node reports `eth_syncing: false`, a beacon head at the current
      slot and `is_optimistic: false` while sitting on its own chain. See
      gotcha 8 — this is the check that would have caught it.
- [ ] **The joiner *holds* head for at least an hour**, sampled every minute.
      Reaching head once is not the same as staying there, and a node that
      drifts is worse than one that never synced, because it looks healthy at
      the moment you check it.

## D. Public endpoints

- [ ] **RPC** answers from off-site, and the guard (if any) permits exactly
      the methods the user guide tells people to call.
- [ ] **Faucet delivers**, and the delivered amount is stated on the page.
- [ ] **Faucet funds are sufficient for what the guide asks users to do.** If
      the guide describes running a validator, a 1 ETH drip does not fund a
      32 ETH deposit; say who funds operators instead. See gotcha 2.
- [ ] **Faucet page is the user guide**: chain ID, RPC URL, explorer URL,
      bootnodes, the artifact bundle, and a correct worked example of the
      feature under test.
- [ ] **The page lists only EIPs that are actually active.** A stale bundle
      list copied from a previous testnet is worse than no list.
- [ ] **Explorer is live, follows head, and renders the feature correctly** —
      the new transaction type is decoded, not shown as raw bytes or an
      "unknown type". Cite a block.
      Send one *now* rather than citing an old one, and check the explorer's
      decode against the client's own view of the same hash: two independent
      decoders agreeing is the evidence, and a stale citation proves only that
      it worked once. On `frames-testnet` a freshly sent type-`0x06` rendered
      as `Frame (EIP-8141) (6)`, `2 frames · 1 signature`, with
      `VERIFY / APPROVE execution+payment` and `SENDER / APPROVE none`,
      matching `rex frame send`'s report field for field.
- [ ] **Explorer indexes validator lifecycle, not just blocks.** Look up a
      validator that was *deposited after genesis* and a deposit transaction.
      Following head proves the beacon indexer works and says nothing about
      the execution-layer indexers. See gotcha 7.
- [ ] **Client tooling exists and is pinned.** If the released CLI predates
      the feature, publish a branch or commit that supports it and name it on
      the page.

## E. Validator lifecycle

Ingress and egress are separate tests; passing one says nothing about the
other.

- [ ] **Ingress, end to end, from a host that is not the deployment host:**
  - [ ] operator EOA funded, and granted a deposit token by the admin
  - [ ] deposit **without** a token reverts (`Not enough tokens`) — proves the
        gate is on
  - [ ] deposit **with** a token succeeds and **burns** it (check `balanceOf`
        before and after)
  - [ ] the EL emits the EIP-6110 deposit request and the CL queues it
        (`/eth/v1/beacon/states/head/pending_deposits`)
  - [ ] the deposit dequeues after its slot's epoch finalizes, and the
        validator appears with an `activation_epoch`
  - [ ] the validator attests once active
- [ ] **Egress:** submit an exit (EIP-7002 execution-layer request or a CL
      voluntary exit), and confirm the CL sets `exit_epoch`. Budget for
      `SHARD_COMMITTEE_PERIOD` — the request is silently ignored, not
      rejected, if the validator is too young.
- [ ] **Deposit queue timing is understood and documented.** A pending deposit
      is only processed once the epoch containing its slot is *finalized*, so
      ingress is never instant. Tell operators the expected wait.

## F. Documentation handover

- [ ] The guide is reproducible by someone with no access to our
      infrastructure, start to finish.
- [ ] Gas guidance is explicit wherever the network deviates from the usual
      assumptions (see gotcha 1).
- [ ] What is gated, and who to ask for access, is stated.
- [ ] Known limitations are written down rather than discovered by users.

---

## Gotchas caught by this checklist

**1. A hardcoded 21,000 gas transfer always fails under EIP-8037.**
State gas is charged from the same limit as execution gas for ordinary
transactions, so the canonical 21,000 is never enough. Measured on
`frames-testnet`: 21,165 to an existing account, and **204,600** to an address
that does not exist yet (21,000 execution + 183,600 `NEW_ACCOUNT` state gas).
Any script or wallet with a hardcoded limit fails, and the receipt looks like
a revert (`status: 0`, `gasUsed == gasLimit`) rather than an out-of-gas.
Always `eth_estimateGas`, and say so on the faucet page.

**2. The faucet cannot fund a validator.**
The faucet delivers 1 ETH; a deposit needs 32. If the guide invites people to
run validators, name the funding path — on a permissioned testnet the admin
who mints the deposit token should fund the operator in the same step.

**3. Published bootnodes must advertise a routable address.**
An EL `enode://` is unsigned, so a joiner can rewrite the host and connect
anyway. A CL **ENR is signed and cannot be rewritten** — if the advertised IP
is unreachable, the joiner has no workaround at all. Verify by fetching the
bundle and peering from a host outside the deployment network before
publishing. Symptom: `net_peerCount` stays `0x0` with correct-looking
bootnodes.

UPnP mappings are gateway *state*, not config, and a gateway may expire "permanent"
ones that are never renewed — this one did, silently, with no reboot. If forwards are
UPnP-created, renew them on a timer and test reachability from outside on a schedule,
not once. See gotcha 9.

The second-order cost is worse than the first. A joiner that cannot use the
published ENRs has no *discovery*, only the handful of static peers someone
hand-fed it, so it cannot heal: when one of those peers drops, it degrades to
`failed to dial` and drifts off head, and no new peers arrive to replace it.
On `frames-testnet` a joiner pinned to three static peers reached head, lost
one peer, and fell 180 slots behind with its validator silent. Peer *counts*
therefore need watching over time on a joiner, not just at first connection —
though note that a correctly-addressed deployment would not have this problem,
because discovery would keep refilling the peer set.

**4. `SHARD_COMMITTEE_PERIOD` decides whether egress is testable.**
At the mainnet preset of 256 epochs, no validator may exit for ~13.6 hours
after activation. Exit requests before that are *ignored silently* by the CL,
which is easy to misread as a broken exit path.

Confirmed from both sides on the same validator, with the same request. It
activated at epoch 51, so it became eligible at 307:

| Submitted | Execution layer | Consensus layer |
|---|---|---|
| epoch 53 | accepted, `status 1`, 281,785 gas | ignored; `exit_epoch` stayed `FAR_FUTURE` |
| epoch 419 | accepted, `status 1`, 251,785 gas | `active_exiting`, `exit_epoch 424`, `withdrawable_epoch 680` |

A successful execution transaction is therefore not evidence the exit took, and
the failure mode leaves nothing to find — no revert, no log, no error. Only the
validator's own `exit_epoch` answers the question. Note also `withdrawable_epoch`
lands a further `MIN_VALIDATOR_WITHDRAWABILITY_DELAY` beyond the exit, so the
balance is not recoverable when the exit is.

**5. One sync path working does not mean the other does — document the one you
tested.**
On `frames-testnet`, a clean joiner on a separate host behaved very
differently by path:

| Path | Result |
|---|---|
| Genesis sync | Stalls. The CL (prysm `glamsterdam-devnet-8`) fails ePBS payload-envelope backfill with `beacon block root ... not found in forkchoice`, crawls at a few slots/minute, and the EL is left at block 7. |
| Checkpoint sync | Works. CL reaches head with `sync_distance: 0` and `is_optimistic: false`, and the EL follows to exactly the remote head. |

So the testnet *is* joinable, but only by the path we tested — the guide has
to say `--checkpoint-sync-url` and point at a beacon endpoint, or newcomers
will follow the obvious route and conclude the chain is broken. Check the EL
independently of the CL: a CL can be at head while the EL is still at genesis,
and only `eth_blockNumber` tells you which.

Reaching head is also not the same as holding it. The same joiner repeatedly
caught up to `sync_distance: 0`, ran correctly for a few minutes, then fell
back into envelope backfill and drifted tens of slots behind. Its validator
went silent each time, and resumed immediately when pointed at a beacon node
that had been in the chain since genesis — which is how we know the validator
and its keys were never the problem. Sample sync distance over time, and if a
deposited validator is part of the test, watch that it keeps signing rather
than that it signed once.

*Attribution caveat:* gotcha 8 was found later, and the deployment this joiner
was syncing from turned out to be partitioned into three chains. Being fed
conflicting heads by three static peers is sufficient on its own to explain
the drift, so do not read the above as a proven client defect. The backfill
errors were real; the cause of the drift was not isolated. Rule out a
partition first.

**6. Verify from off-site, not from the deployment host.**
Gotchas 1, 2, 3 and 5 are all invisible from the machine that deployed the
testnet, because that machine has funded accounts, local network access, and
a node that has been in the chain since genesis.

**7. An explorer at head can still be indexing nothing.**
Dora on `frames-testnet` tracked the chain head exactly and rendered blocks and
type-`0x06` frame transactions correctly, while every execution-layer indexer
was failing:

```
level=error msg="deposit indexer error: finalized block not found in cache or db" indexer=deposit
                                        …same for withdrawals, consolidations, builder_deposits, builder_exits
```

The visible symptom is narrow and easy to miss: a validator deposited after
genesis returns *Validator not found*, while genesis validators render fine. A
restart did not clear it. The execution client was not at fault — it answered
`latest`, `safe` and `finalized` correctly (1257 / 1135 / 1018) while the
indexer claimed it could not find the finalized block. Check a post-genesis
validator explicitly; "the explorer is up and at head" does not cover it.

**8. Every node healthy, three separate chains.**
The frames deployment passed every per-node check while being three networks.
Node 1 was at execution block 1846 with finality at epoch 81; nodes 2 and 3
were at blocks 462 and 464 with finality still at epoch 0 — and all three
reported `eth_syncing: false`, `sync_distance: 0` and `is_optimistic: false`.
Node 2 was not stalled, it was *building its own blocks*.

The cause was that the beacon nodes had no peers at all (`connected: 0`) and
discovery was failing with `find peers with subnets: context deadline
exceeded`. The host sits behind NAT, the nodes advertise `nat_exit_ip`, and
the router forwarded only 80 and 443 — so nothing could reach them at the
address they published, *including each other on the same host*. Only node 1
finalized because it happened to hold 128 of the 192 validators.

Two lessons. Compare nodes against each other, never just against their own
health endpoints. And on a NAT'd host, the P2P port-forwards are not a
nice-to-have for external joiners — without them the deployment silently
splits into one chain per node. The healthy reference on a directly-addressed
host had all three execution nodes at an identical block and all three beacon
nodes finalized at the same epoch.

**9. "Permanent" UPnP mappings that quietly expire.**
Every P2P forward on `frames-testnet` was a UPnP mapping requested with
`LeaseDuration=0`; the gateway reported each as permanent. Within a day all
sixteen were gone — no reboot (WAN uptime 130 days), no error, and the
tailscale mapping on the same gateway untouched, because tailscale renews
its own. An external team's node dropped to 0 EL peers and reported the
enodes "TCP-refused"; the enodes were correct and the nodes healthy. Two
lessons: reachability is a thing to *monitor*, since a passing check at
publish time says nothing about tomorrow; and anything a gateway can forget
must be re-asserted by a timer, exactly as the beacon peering already is.
