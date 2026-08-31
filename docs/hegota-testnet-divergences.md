# Hegotá testnet — divergence ledger

Every place this branch's behaviour differs from the specification a second client
would implement from, plus the upstream drift since each pin. A row whose
**Consensus-visible** column reads `yes` and whose **Action** is not `closed` is a
chain split waiting for the first block that exercises it, which is why this ledger
gates bring-up.

Spec lookups go through `eipmcp`; PR state through `gh`. Audited against
`ethereum/EIPs@4093c2184` (indexed 2026-08-09) on 2026-08-10.

## 1. Drift since the pinned revisions

`diff_eip(n, since=<pin>)` for each core EIP, against the pinned revisions in the rule-set
table of `docs/hegota-testnet-joining.md`.

Re-audited 2026-08-25 against `ethereum/EIPs@master`, 70 commits past the pin. The four
core EIPs were pinned at `4093c21847`; 8250, 8272 and 7805 are byte-identical to it still,
8141 is not, and EIP-8369's PR head has moved.

EIP-8141 is pinned at `7d1c8bfb94` (2026-08-24, "account block execution gas before refund
(EIP-7778)"), verified byte-identical to the copy this implementation was written against.
That revision is the reason the frame path reports a pre-refund `gas_used` for the block and
a post-refund `gas_spent` for the payer.

**Re-audited 2026-08-31.** EIP-8141 and EIP-7805 have not moved. EIP-8250 and EIP-8272 both
moved that day, and both moves are consensus-visible renumberings caused by v2's `0x0C` and
`0xB5` claims — see §5.1. Implemented and re-pinned to `e5cf246ff1` and `0231fb05f5`.

| EIP | Pin | Drift to head | Consensus-visible | Action | Owner |
| --- | --- | --- | --- | --- | --- |
| 8141 | `4093c21847` → **`7d1c8bfb94`** | **+326/−96 — a new envelope, see §6** | **yes** | **v2 adopted and implemented**, pin bumped; re-genesis required | — |
| 7805 | `4093c21847` | none — byte-identical | no | closed | — |
| 8250 | `81b976ac01` → **`e5cf246ff1`** | **TXPARAM ids shifted up by one** (2026-08-31), plus an Abstract sentence | **yes** — every prefix reading a keyed-nonce id | **implemented**, pin bumped | — |
| 8272 | `d8636a330d` → **`0231fb05f5`** | **reference count `0x0F`→`0x11`, `RECENTROOTREFLOAD` `0xB5`→`0xB6`** (2026-08-31), plus an Abstract sentence | **yes** | **implemented**, pin bumped; the opcode byte already matched | — |
| 8369 | `6f818e27dd` → **`33724bd7da`** | three commits, +17/−13: Profile 1 candidacy stated by transaction type, `MAX_VERIFY_GAS_PER_TX` demoted to "up to `MAX_VERIFY_GAS_PER_IL`", and **budget fill split into a two-stage debit** | **yes** — it decides Profile 2 admission | **implemented**, see §7; pin bumped | — |
| 8312 | `a5da3f608c` | not diffable — author's fork only, no upstream `EIPS/eip-8312.md` | n/a while `utxoFramesTime` is unset | inert; re-check if the EIP is upstreamed | Edgar |

The core EIPs have not moved normatively since the branch reconciled against them.
The only expanded text is EIP-8369's, and it expands the list of things the *enforcing
extension* must define — which is this branch's two-endpoint rule, already written up.

## 2. Fork membership: what "Hegotá" means to a joining client

EIP-8081 (*Hardfork Meta - Hegotá*, Draft, `requires: 7723, 7773`) is the authoritative
statement of the fork's contents, and it is much narrower than this branch:

| EIP | EIP-8081 status | On this branch |
| --- | --- | --- |
| 7805 FOCIL | **Scheduled for Inclusion** | active |
| 8141 Frame Transaction | Considered for Inclusion | active |
| 8250 Keyed Nonces | Proposed for Inclusion | active |
| 8272 Recent Roots | Proposed for Inclusion | active |
| 8312 UTXO Frames | not listed | present, inert (`utxoFramesTime` unset) |
| 8369 FOCIL Eligibility | not listed (open PR #12110) | active, no off switch |

| Item | ethrex behaviour | Spec behaviour | Consensus-visible | Action | Owner |
| --- | --- | --- | --- | --- | --- |
| Fork rule set | `hegotaTime` activates 8141 + 8250 + 8272 + 7805 + 8369 together | EIP-8081 schedules only 7805 under this name | **yes** | A client implementing "Hegotá" from EIP-8081 implements FOCIL alone and would reject every frame transaction. Publication must name the exact five-EIP set and the pins, never the fork name alone. Feeds Phase 8's artifact set. | Edgar |
| Amsterdam prerequisite | `validate_fork_schedule` rejects Hegotá without Amsterdam at genesis load | EIP-8081 `requires: 7773` (Amsterdam meta) | yes | closed — `0a1ccaed8`, `2a2aab360` | — |

## 3. Consensus-visible implementation divergences

### 3.1 VERIFY budgets take EIP-8369's constant — closed

| | |
| --- | --- |
| ethrex | `MAX_VERIFY_GAS_PER_IL = MAX_VERIFY_GAS_PER_TX = 1 << 20` (`crates/blockchain/focil_eligibility.rs`) |
| EIP-8369 | `MAX_VERIFY_GAS_PER_IL = MAX_VERIFY_GAS_PER_TX = 2**20` |
| Consensus-visible | **yes** — it decides Profile 2 candidacy |
| Status | **conformant** |

The branch previously derived both as `parent_gas_limit / COMMITTEE_VERIFY_GAS_FRACTION
/ IL_COMMITTEE_SIZE`, on the reasoning that a fixed constant changes meaning as the gas
limit moves. That reasoning was calibrated against a 60,000,000 block, where the
derivation yields 937,500 and sits within 11% of `2**20`.

**This testnet does not run at 60M.** It schedules `gloas_fork_epoch: 1`, and the pinned
`ethereum-package` (`b5b3af65`) defaults both `genesis_gaslimit` and `gas_limit` to
**200,000,000** whenever Gloas is scheduled, which the testnet config does not override.
At 200M the derivation gives:

| | per IL / per tx | committee-wide (×16) | share of a 200M block |
| --- | ---: | ---: | ---: |
| derivation | 3,125,000 | 50,000,000 | 25% |
| EIP-8369 `2**20` | 1,048,576 | 16,777,216 | 8.4% |

So the departure on the chain we actually run is ~3×, not 11%, and it is in the
permissive direction: ethrex would admit candidates the reference model excludes, and
`COMMITTEE_VERIFY_GAS_FRACTION = 4` puts committee replay at a quarter of the block by
construction. Nothing has benchmarked 50M gas of attester replay against the attestation
deadline.

Two reasons to take the constant instead. EIP-8369 is the only published document that
names these values, so for this network it is the de facto shared reference whatever its
Informational header says — and unlike the two-endpoint rule (§3.2), the budget is not a
divergence the missing extension EIP forces on us. And on an unbenchmarked number whose
failure mode is attesters missing the deadline, the smaller value is the conservative
one.

The derivation remains the right long-run answer and belongs in the EIP-7805 extension
draft as a proposal, where the fraction can be argued with benchmarks behind it.

**Action:** closed. **Owner:** —

### 3.2 No claimed insertion index; two fixed endpoints instead

| | |
| --- | --- |
| ethrex | omission unjustified if the candidate is eligible at `S_start` **or** `S_end`; `gas_fits` at `S_end` only |
| EIP-8369 | builder commits an index in `[0, len(block.transactions)]`; missing/malformed/out-of-range defaults to `len(block.transactions)` |
| Consensus-visible | **yes** |

Deliberate, and documented at length in `docs/hegota-testnet.md`. EIP-8369 defers the
whole index mechanism to an enforcing extension that does not exist, so the enforcement
point had to come from somewhere. ethrex's `S_end` is exactly the spec's default index;
`S_start` is strictly additional, so ethrex enforces a superset and never excuses an
omission the spec would judge unjustified. The split direction is one-way: ethrex
rejects blocks a spec-literal client accepts, never the reverse.

**Action:** carry, with the write-up published as the Standards Track extension to
EIP-7805 that EIP-8369 asks for. **Owner:** Edgar.

### 3.3 Attester state reconstruction

| | |
| --- | --- |
| ethrex | two real states the evaluator already holds, read through `header.state_root` |
| EIP-8369 | parent VOPS state + pre-execution system updates + EIP-7928 BAL changes for transactions before the index |
| Consensus-visible | no (same states, different derivation) |

Equivalent at `S_end`. No BAL reconstruction dependency, which is what lets Profile 2
run before the "canonical diff validated by consensus" the EIP names as an activation
prerequisite. **Action:** closed, note in the extension draft. **Owner:** —

### 3.4 Per-inclusion-list code-byte budget

| | |
| --- | --- |
| ethrex | 16 distinct bodies and 16 × 64 KiB per list, charged inside the replay, shared across candidates and both endpoints |
| EIP-8369 | specifies no such bound; requires the enforcing extension to "bound the number and total bytes of claims" |
| Consensus-visible | **yes** — the budget ends a replay early, which decides a verdict |

ethrex bounds *code loaded during replay*; the EIP asks the extension to bound *claims*.
Related but not the same quantity, and ethrex's is the one that actually bounds attester
work. **Action:** carry; state the bound in the extension draft and in the published
artifact set, since a client with a different bound reaches a different verdict on the
same list. **Owner:** Edgar.

### 3.5 `AA_VOPS_SLOT_COUNT = 4` with no off switch

| | |
| --- | --- |
| ethrex | `ChainConfig::aa_vops_slot_count()` defaults to 4 when `aaVopsSlotCount` is absent |
| EIP-8369 | value left unset, candidate range 2–4; "No implementation can classify Profile 2 for enforcement until the enforcing Standards Track EIP selects a value" |
| Consensus-visible | **yes** |

The published genesis leaves the field unset, and absent means **4, not off** — a joining
client MUST NOT read the missing key as "Profile 2 disabled". Pinned by
`aa_vops_slot_count_defaults_to_four`. 4 is the top of the range, so it is the permissive
choice: eligible-at-4 is a superset of eligible-at-2. **Action:** carry; must appear
explicitly in the published artifact set, since it is a consensus input that the genesis
file does not contain. **Owner:** Edgar.

### 3.6 `SLOTNUM` is banned in the validation prefix

| | |
| --- | --- |
| ethrex | `SLOTNUM` (`0x4B`) is in the banned set (`crates/vm/levm/src/vm.rs:3474`) |
| EIP-8141 | banned list now includes it: PR #12066 **merged 2026-08-11** |
| Consensus-visible | was yes; now **conformant** |

ethrex adopted the proposed ban ahead of merge (`#7108`, commits `d43ebdda2`,
`fc018b526`). A validation prefix branching on `SLOTNUM` is admitted by a spec-literal
client and rejected here. The ban is right — EIP-8272 makes the beacon slot load-bearing,
which gives `SLOTNUM` exactly the admission-time/inclusion-time divergence the banned
list exists to prevent — and #12066 merged on 2026-08-11, so the early adoption is now
plain conformance rather than a divergence.

**Note:** `docs/eip-8272.md` still claims ethrex "keeps following the list as written
until it merges", which the code contradicts. Corrected in this pass.

**Action:** closed — #12066 merged; nothing to carry.

## 5.1 Opcode and TXPARAM ids v2 collides with — relocated

Adopting EIP-8141 v2 forced two id moves, both consensus-visible, both because v2 claims
an id this chain had already assigned to another EIP in the same set. **Both were then
settled upstream on 2026-08-31, and the upstream resolution is not the one ethrex guessed.**

| id | v2 assigns | ethrex had | Upstream resolution (2026-08-31) |
| --- | --- | --- | --- |
| `0xB5` | `SIGDATACOPY` | EIP-8272 `RECENTROOTREFLOAD` | `RECENTROOTREFLOAD` → `0xB6` (`0231fb05f5`) — matches what ethrex shipped |
| `0x0C` (TXPARAM) | `state_gas_left` | EIP-8250 `legacy_sender_nonce` | EIP-8250 shifts **all three** of its indices up by one (`e5cf246ff1`) — ethrex had moved only the nonce read, to `0x12` |

ethrex guessed one of the two. The nonce read went to `0x12` here because `0x11` was taken
by the resolved-payer param; upstream instead moved EIP-8250's whole block, so
`legacy_nonce` is `0x0D`, `len(nonce_keys)` is `0x0E` and `nonce_keys_hash` is `0x0F`.
EIP-8272's reference count, which ethrex had at `0x0F`, moved to `0x11` to make room — and
that displaced ethrex's resolved-payer param, which is ours and therefore yields, to `0x12`.

The current map, matching upstream:

| id | Value | EIP |
| --- | --- | --- |
| `0x0C` | `state_gas_left` | 8141 v2 |
| `0x0D` | pre-state legacy sender nonce | 8250 |
| `0x0E` | `len(nonce_keys)` | 8250 |
| `0x0F` | `nonce_keys_hash` | 8250 |
| `0x10` | `nonce_keys[0]` | 8250 |
| `0x11` | `len(recent_root_references)` | 8272 |
| `0x12` | resolved payer | ethrex only, knob-gated |

EIP-8141 gets the disputed id in every case: it is the EIP the rest of the set extends, and
each *other* EIP had already been relocated once for the same reason — `RECENTROOTREFLOAD`
moved off `0xB4` when it collided with `SIGPARAM`, and `nonce_keys[0]` moved off `0x0B` to
`0x10`. That is now **seven** relocations across three EIPs, which is the real finding:
**EIP-8141 needs a shared registry for the frame-surface ids its extensions claim.** Raised
with the authors; recorded here because a second client that picks different bytes diverges
on every validation prefix touching any of them.

The lesson for this branch is narrower and worth stating: **do not invent an id to resolve a
collision upstream has not resolved yet.** `0x12` was a reasonable guess and it was wrong,
and because the chain had not launched on v2 it cost only a rename. On a live chain it would
have cost a re-genesis.

Four compile-time asserts in `crates/vm/levm/src/opcodes.rs` now pin the opcode bytes and
forbid sharing, so a fifth relocation is a compile error rather than a chain split.

**Action:** closed for this chain; upstream registry request open.

## 6. EIP-8141 v2: a new envelope, and therefore a re-genesis

EIP-8141 moved +326/−96 since the pin. Six of the new sections are Rationale and one is
Security Considerations, so they need no code; what is normative is:

- the envelope nests the fee fields: `[chain_id, nonce, sender, frames, signatures, fees,
  blob_versioned_hashes]`;
- each frame carries `limits = [execution, state]` in place of one `gas_limit`, giving
  per-frame two-dimensional budgets that never mix;
- `FRAME_TX_INTRINSIC_COST` drops from 15 000 to 12 000;
- `SIGPARAM`'s copy operation becomes a new `SIGDATACOPY` opcode at `0xb5`;
- receipts carry `gas_used = [execution, state]`, and a cross-frame state-gas refill
  retroactively reduces an *earlier* frame's `gas_used.state`;
- EIP-2929 warm/cold is charged at frame entry, before the balance check and dispatch;
- the EIP-7825 cap applies to `intrinsic + Σ limits.execution`, state gas excluded;
- `ecrecover` and `P256VERIFY` must not appear in the block access list;
- extra expiry-frame constraints, and an atomic-batch `APPROVE_SCOPE_MASK` assert.

**The envelope change means this cannot be an in-place upgrade of a running chain.**
Adopting it is a re-genesis, and it also invalidates the published tooling: the Python
frame-tx encoder, the go-ethereum fork the explorer decodes with, and every joiner's
transaction builder.

Cherry-picking is the one option to avoid: taking `12 000` without the two-dimensional
model produces a rule set that matches no published revision, which is exactly what this
ledger exists to prevent.

**Action:** adopted, and implemented. The two-dimensional model is enforced, not merely
encoded — see §6.1 for what a joiner has to change because of it.

### 6.1 What the second dimension costs a transaction builder

Enforcement is the part with user-visible consequences. Three of them:

**Every frame that creates state must declare `limits.state` for it.** A charge past the
declared budget halts that frame — a `VERIFY` frame halting invalidates the whole
transaction — and no frame can borrow state gas from its own execution budget, from
another frame, or from a reservoir. The charges a plain transaction meets:

| what the frame does | state gas |
| --- | --- |
| funds an address that does not exist yet | `120 * 1530` = 183 600 |
| creates a storage slot (`0 → non-zero`) | `64 * 1530` = 97 920 |
| installs a 7702 delegation | `23 * 1530` = 35 190 |

**Every frame's execution budget must cover one account access.** The resolved target's
EIP-2929 warm/cold access is charged at frame entry from `limits.execution`, so the
minimum viable per-frame budget rises by 3 000 (cold, this base's Amsterdam rate) or 100
where an earlier frame already touched the address. This applies to the expiry-verifier
frame too: protocol-defined evaluation is an optimization, never a discount, so a frame
that declared 1 000 gas because a client evaluates its deadline directly now halts.

**Over-declaring is nearly free, under-declaring is fatal.** Unused budget in either pool
is refunded at settlement; it only raises the `max_cost` collected from the payer up
front. So a builder that cannot predict a frame's state growth should over-declare.

Two changes a joiner will see rather than cause: receipts report `stateGasUsed` per frame
(net of refills, and zero for a frame that reverted), and `TXPARAM 0x0C` returns the
executing frame's remaining state budget — the read a paymaster needs to check that a
frame it is about to sponsor can afford what it intends to do.

### 6.2 Cross-checked against the other v2 implementation line

`frames-devnet-0` (131 commits, not merged anywhere) is a second implementation of the same
rule set, developed against the frame-transaction fixtures added to execution-specs#3047.
Diffing its frame-entry work against this branch's found one bug here and confirmed one
divergence that is only apparent:

- **Found:** the EIP-7702 delegate of a frame's target was resolved *before* the frame was
  known to afford its entry access charge, so an unaffordable frame still warmed the
  delegatee and filed it as touched in the EIP-7928 access list. Nothing in the receipts can
  contradict that — an unaffordable frame forfeits its whole gas limit either way — so the
  access list is the only place it shows, and a builder that files a stray touch writes a
  list its own block contradicts. Fixed by peeking at the delegation to pick the dispatch
  branch and following it only once the charge is paid, with
  `an_unaffordable_frame_files_no_delegatee_in_the_block_access_list` covering both halves.
- **Not a divergence:** that branch runs the default code only for a *codeless VERIFY*
  target and sends every other frame through the EVM. EIP-8141 v2 orders the branches
  differently — precompile first, then empty code hash for any mode, then EIP-7702 — so on
  v2 the observable difference between the two readings is precompiles alone, which this
  branch dispatches (§6.1). Its account-creation charge likewise spills into execution gas,
  which is correct for v1's derived split and wrong for v2's declared one.

Its remaining value is the fixture harness: it runs the released frame-transaction suite (44
frame fixtures, 14 908 blockchain fixtures) which this branch does not. Worth taking on its
own, separately from the rule set.

## 6.3 Fixed: frame transactions accepted by RPC and silently dropped

Found while verifying EIP-8250 concurrency on a devnet. **Not an EIP-implementation defect —
a mempool admission one — and it should be fixed before the live relaunch, because it loses
user transactions without telling anyone.**

Two shapes, one symptom. In both, `ethrex_simulateFrameTransaction` reports the transaction
valid, `eth_sendRawTransaction` returns its hash with no error, and the transaction then
never enters the pool: `eth_getTransactionByHash` reports it unknown within the same second,
`txpool_status` stays empty, and the sender's balance is untouched, so it did not execute
under a different hash. No log line marks the drop. The returned hash is the correct
`keccak(raw)` — verified against `cast keccak` — so this is not a hash-computation mismatch.

| Shape | Behaviour | Reproducer |
| --- | --- | --- |
| Prefix-only: a frame tx whose sole frame is its `VERIFY` prefix | **Always** dropped | `scripts/hegota-testnet/probe_prefix_only_tx.py` |
| Contract sender, two frames (`VERIFY` + `SENDER`), disjoint nonce keys | **Intermittently** dropped — roughly one run in three | `scripts/hegota-testnet/probe_contract_sender_tx.py` |

The second one is the one that matters, because it is the shape EIP-8250 concurrency depends
on and it usually works: both transactions are admitted and mine in the *same block*, which
is the feature's whole claim. It was verified working standalone (block 74 of a fresh chain)
and in a full verifier run that passed 35/35, then failed in the very next run on that same
chain. So the feature is implemented correctly and the admission path drops it at random.

Ruled out along the way: chain age (a chain seconds old reproduces it), base fee (flat 7 wei
throughout), the mempool size cap (10 000, pool empty), hash mismatch (hashes match), and
EOA senders (an EOA frame transaction submitted in the same run mined normally).

**Root cause: `revalidate_frame_txs_after_block` evicted on a non-verdict.** It replays every
pending frame transaction's prefix against each new head, and three outcomes are possible —
the prefix ran and rejected the transaction, the prefix ran and accepted it, or the replay
could not be performed. Only the first is evidence about the transaction. The function
already handled the third case correctly in three places (a `StoreError` from the
recent-root check, an unopenable head state, a failed balance read all keep the transaction)
and then evicted on any error from the simulation itself, under a comment calling that
conservative.

It is the opposite. A read that fails, or a head whose state is not yet readable, says
something about the node, not the transaction — and evicting on it destroys a transaction the
node has already acknowledged, silently, and intermittently, because whether the read fails
is timing. That is the whole shape of the symptom: all pending frame transactions at once,
about one run in three, no log.

Both error branches now keep the transaction and log why, and eviction logs its reason. A
genuinely invalid transaction is still caught by the `passed` flag, by the next block's pass,
or at block building.

## 7. The glamsterdam-devnet-8 base reprices Amsterdam

The testnet is now built on `origin/glamsterdam-devnet-8`, whose commit `fe6b15abb`
updates the EIP-8038 vectors to the v8.1.0 schedule:

| primitive | before | on this base |
| --- | --- | --- |
| cold storage access (Amsterdam) | 3000 | 2100 |
| `ACCESS_LIST_ADDRESS_COST` | 3000 | 2400 |
| `ACCESS_LIST_STORAGE_KEY_COST` | 3000 | 1900 |
| `TX_VALUE_COST` (Amsterdam) | 4244 + 1756 transfer log | 6000 combined |

EIP-8272 defines its charges by formula over the access-list costs, so the spec-faithful
values follow the schedule: the reference-address charge goes 3000 → 2400 and the
reference charge 3102 → 2002. **Recent roots are live on the running chain**, so this
reprices a rule already in force; a node on this base disagrees with that chain's history.
The pinned EIP-8272 write measurement moved by exactly the cold-access delta,
127 256 → 126 356.

EIP-8312 instead publishes absolute totals, and its Rationale decomposes them over the
old schedule (13 000 = 3000 + 10 000). Under v8.1.0 the same decomposition yields 12 100.
The published totals are what a second client implements, so `GAS_UTXO_FRAME` and
`GAS_UTXO_INPUT` stay pinned at 13 000 and 16 048 and the stale decomposition is recorded
in `crates/vm/levm/src/gas_cost.rs`. `GAS_UTXO_ACCOUNT_OUT` still reproduces its published
9000, because the new `TX_VALUE_COST` absorbs the transfer log exactly. Worth raising with
the EIP-8312 author.

**Action:** this is the second independent reason the live chain needs a re-genesis rather
than an upgrade. Owner: unassigned.

## 8. EIP-8369's two-stage budget debit — implemented

The re-pin from `6f818e27dd` to `33724bd7da` changed budget fill normatively. Fill used to
price an occurrence from its shape, then debit the whole cost before any signature check.
It now computes the signature and validation-prefix costs, ignores the occurrence outright
if their sum does not fit, debits the **signature half** before checking protocol
signatures, keeps only that debit when a signature fails, and debits the prefix half only
once the signatures pass.

`fill_il_budget` therefore verifies protocol signatures (it takes the fork and a crypto
backend), and a new `FillOutcome::SignatureFailed` records the signature-only debit. The
behavioural difference is pinned by
`an_invalid_signature_debits_only_the_signature_half`: two occurrences at half the list
budget each, where the old single debit would have starved the second and the new rule
admits it.

**Action:** closed.

## 4. Items upstream has closed in ethrex's favour

Confirmed against `get_eip` / the PR head this pass; no action beyond deleting the stale
divergence text.

| Item | Resolution |
| --- | --- |
| `RECENTROOTREFLOAD = 0xB5` | spec assigns `0xB5`; conformant |
| TXPARAM recent-root count `0x0F` | spec's two tables reconciled to `0x0F`; conformant |
| `RECENT_ROOT_ADDRESS = 0x…8272` | spec pins it; conformant |
| EIP-8250 TXPARAM `nonce_keys[0] = 0x10` | spec assigns `0x10`; conformant |
| `NONCE_MANAGER = 0x…8250` | spec pins it; conformant |
| EIP-8272 calldata charge and floor placement | additive rewrite matches ethrex; conformant |
| EIP-8272 delegate-style calls write foreign storage and succeed | **adopted upstream 2026-08-10** (PR #12131 commit `9f3c60261`), in the exact terms ethrex shipped and tested; the EIP now also covers the EIP-7702 delegation case |
| EIP-8272 activation requires EIP-7843 | **added upstream 2026-08-10**; satisfied here transitively, since Hegotá requires Amsterdam and `SLOTNUM` is Amsterdam |

## 5. `RECENT_ROOT_CODE` — verified byte-for-byte

PR ethereum/EIPs#12131 gained commit `9f3c60261` on 2026-08-10 promoting
`RECENT_ROOT_CODE` from `TBD` to a pinned 144-byte runtime with an assembly listing.

**ethrex's `RECENT_ROOT_RUNTIME_BYTECODE` is byte-identical to it** — all 144 bytes,
compared programmatically this pass. `keccak256` stays
`0x432c8b183d17d5e9939623833203b9a5b62325246cfcd9307982bfde8f18c6fb`, pinned by
`recent_root_bytecode_matches_spec`.

Both authorial questions `docs/eip-8272.md` tracks are now resolved in ethrex's favour:
the spec's own assembly comments endorse the `0x1fff` mask ("RECENT_ROOT_LENGTH is a
power of two, so the modulus is a mask"), and install-at-activation stands with no
deployment transaction.

Still open, from the PR review and not yet addressed upstream or here:

| Item | Consensus-visible | Action | Owner |
| --- | --- | --- | --- |
| Static-context gas: malformed input exits via `REVERT`, but a valid-shaped `STATICCALL` reaches `SSTORE` and exceptional-halts, consuming all remaining gas in the subcontext | **yes** (exact gas) | add the test; ethrex has the `DELEGATECALL` case pinned but not this one | Edgar |
| PR's broader test matrix (calldata 0/63/64/65, value, `CALL`/top-level/`STATICCALL`/`DELEGATECALL`/`CALLCODE`/7702, cold/warm/overwrite, enclosing revert, repeated same-slot writes, slots `0`/`1`/`8191`/`8192`/`8195`/`2**64-1`, reference ages `0`/`1`/`8191`/`8192`) | yes | fill the gaps against `test/tests/levm/eip8272_tests.rs` | Edgar |
| `RECENT_ROOT_CODE` is still an unmerged PR | yes — a byte change moves the code hash and the write's gas, a fork for a running chain | track #12131 to merge | Edgar |

## 6. Upstream PRs this rule set depends on (Task 6.3)

Re-audited **2026-08-26**. Six of the eleven have merged since the 2026-08-10 pass, and
every merged one is implemented here — several were adopted ahead of merge, which is why
this table exists: an early adoption is a divergence until the text lands, and then it
silently stops being one.

| Upstream PR | Subject | State | ethrex |
| --- | --- | --- | --- |
| EIPs#12066 | ban `SLOTNUM` in validation prefix | **merged 2026-08-11** | conformant; adopted ahead in `#7108` (§3.6) |
| EIPs#12109 | atomic-batch approval scope | **merged 2026-08-14** | conformant — `APPROVE_SCOPE_MASK` asserted statically |
| EIPs#12026 | floor repricing, signature validation, `frame.value` gas | **merged 2026-08-14** | conformant — uniform floor tokens, no precompile in the BAL (§9.2), and the value-frame account-creation charge (§9.2) |
| EIPs#12061 | frame receipt has no transaction-level status | **merged 2026-08-14** | conformant — the receipt is `[tx_type, cumulative_gas_used, payer, [frame_receipt…]]` with no top-level `succeeded` |
| EIPs#12062 | explicit second dimension for state gas on frames | **merged 2026-08-13** | implemented — this is v2's two-pool model (§6, §6.1). It was a *draft to watch* at the last pass |
| EIPs#12113 | initial `accessed_addresses` set | **merged 2026-08-17** | conformant — five clauses, one test each (§9.1) |
| EIPs#12091 | block inclusion gating and payer solvency | **closed unmerged** | nothing owed; drop from the watch list |
| EIPs#12041 | canonical paymaster reference bytecode | open | implemented ahead of merge; the pinned 355-byte runtime's hash (§8 — Task 6.4 closed) |
| EIPs#12039 | keyed mempool concurrency | open | ships `keyed_concurrency_verdict`; devnet-verified for a contract sender |
| EIPs#12110 | VOPS profiles for FOCIL eligibility (EIP-8369 itself) | open | implemented ahead of merge, pinned at `33724bd7da` (§3.2–3.5, §8) |
| EIPs#12131 | specify `RECENT_ROOT_CODE` | open | `#7120`; bytes verified identical (§5) — a byte change would move the code hash and the write's gas, so this one is still a fork risk for a running chain |

Editorial or idle, tracked so a later pass need not rediscover them:

| Upstream PR | Subject | Bearing |
| --- | --- | --- |
| EIPs#12121 | link first reference to each cited proposal | editorial |
| EIPs#11681, #11555, #11580, #11482 | guarantors, payer-approves-first, precompiles in VERIFY frames | long-idle drafts, no ethrex surface |

## 7. Phase 6 task status

| Task | Status |
| --- | --- |
| 6.1 record drift since pins | **done** — §1, and no core EIP moved normatively |
| 6.2 reclassify adopted items, bump pins | **done** — §4 confirms all five, plus three more; pins bumped. `docs/eip-8272.md`/`docs/hegota-devnet.md` carry them as annotated "no longer a divergence" entries rather than deletions, which reads better and is left as is. |
| 6.3 open-PR rows | **done** — §6 |
| 6.4 resolve EIPs#12041 paymaster hash | **done** — §8 |
| 6.5 EIPs#12026 BAL clause + EIPs#12113 warm-set clause | **done** — §9.1, §9.2 |
| 6.6 EIP-8272 BAL read record + EIP-8250 bookkeeping exclusion | **done** — §9.3, §9.4 |
| 6.7 every `yes` row closed or moved to Open Questions with a fallback | **done** — §10.1 |
| 6.8 checkpoint | **done** — §10.2 |

## 8. Canonical paymaster (EIPs#12041) — resolved

ethereum/EIPs#12041 pins a 355-byte canonical paymaster runtime and its per-fork
`keccak256`. Verified independently this pass rather than taken from the PR text: the
runtime is 355 bytes and hashes to
`0xda42f0d11838c4c0c3129b8b8e93e9718127ad6b315e517e1088125707c4d45c`, which is the value
the PR states.

ethrex previously shipped `FRAME_CANONICAL_PAYMASTER_CODE_HASH = H256::zero()`, a
sentinel no real code can hash to, so every paymaster was non-canonical and the whole
canonical path was unreachable. That was the conservative interim (it only over-rejects)
but it left two behaviours inert:

| Behaviour | Before | Now |
| --- | --- | --- |
| Pending-tx cap | every sponsor capped at `MAX_PENDING_TXS_USING_NON_CANONICAL_PAYMASTER = 1` | a canonical instance is bounded by the payer's reserved balance alone; every other sponsor stays capped |
| Validation-trace exemption | `canonical_pay_frame` hard-coded `None`, so the ERC-7562 access-restriction skip never fired | resolved from the `pay` frame's target code hash, at all four simulation call sites |

Recognition is on the **runtime** code hash, so the canonical paymaster is not a
singleton: many instances may be deployed, one per sponsor, differing only in the
`signer` their constructor writes to slot 0. An instance whose slot 0 is zero authorizes
nothing, so a mis-deployment is inert rather than open.

The exemption is passed to the Profile 2 replay as well as to mempool admission,
revalidation and the RPC simulation. EIP-8369's Profile 2 surface says "the canonical
paymaster exception does not expand this range", which only has meaning if the exception
is in force during eligibility replay; the storage surface remains the binding
constraint there.

| Item | Consensus-visible | Action | Owner |
| --- | --- | --- | --- |
| The hash is per-fork and #12041 is unmerged | **yes** — a byte change moves the hash, and an instance canonical today demotes if the pin moves | track #12041 to merge | Edgar |

## 9. Initial access sets and protocol bookkeeping (Tasks 6.5, 6.6)

### 9.1 EIPs#12113 — frame-transaction initial access sets

Five clauses, one test each in `test/tests/levm/eip8141_tests.rs`. Each is a gas
differential between two runs differing only in the probed address or slot, so it
survives a repricing of the warm/cold spread itself.

| Clause | Verdict |
| --- | --- |
| `accessed_addresses` starts as EIP-2929/EIP-3651 (`tx.sender`, coinbase, precompiles) | conformant — `env.origin` is `tx.sender` for a frame tx |
| `accessed_storage_keys` starts empty | conformant — `FrameTransaction` returns `EMPTY_ACCESS_LIST` |
| being a frame target does not warm an address | conformant — appearing in `tx.frames` warms nothing; the target is warmed when its own frame runs and is charged for it (v2 §Behavior), and stays warm for later frames because the journal is shared across them |
| `ENTRY_POINT` is not pre-warmed | conformant — it is only ever the frame *caller*, never inserted |
| the payer is added when a payment-scope `APPROVE` collects `max_cost` | **was divergent, now fixed** — and v2 makes it structural: the payer is always its frame's resolved target, so the frame-entry access charge warms it and no separate rule is needed |

The payer clause was a real gap: `APPROVE` scopes `0x1` and `0x3` debited the payer's
balance via `decrease_account_balance` without adding it to `accessed_addresses`, so a
later access to the payer was charged cold where a conforming client charges warm. That
is a per-transaction `gas_used` difference, hence a receipts and state-root difference.
Fixed by warming the payer at both scopes, which is free, as for `tx.sender` and the
coinbase.

### 9.2 EIPs#12026 — signature validation records no precompile

**Conformant.** ethrex's `validate_signatures` calls `precompiles::ecrecover` and the
P256 verifier as ordinary Rust functions; it never routes through the EVM call path, so
no precompile account is loaded, warmed, or recorded. Pinned by
`a_validated_signature_records_no_precompile_in_the_block_access_list`, which runs a
frame transaction carrying a real secp256k1 signature under a live BAL recorder and
asserts no address at or below `0x100` appears.

#12026 carries two further clauses this ledger should track:

| Clause | Bearing |
| --- | --- |
| `floor_cost` is the calldata floor function *of the fork in force* — EIP-7623's 10/40 per token, or EIP-7976's flat 64 gas per byte where scheduled | consensus-visible through the frame-tx calldata floor; re-check when EIP-7976 lands on this chain's fork schedule |
| `frame.value` follows ordinary `CALL` value-transfer semantics, charged inside the frame's budgets including the fork's account-creation cost; a frame that cannot cover it halts without transferring, so no EIP-7708 log | **implemented and asserted** — v2 puts the account-creation charge in `limits.state`, after the balance check and before the frame's code; `a_value_bearing_frame_pays_account_creation_from_its_state_budget` covers both the charge and the one-gas-short halt |

### 9.3 EIP-8272 reference reads in the BAL — keep the record

EIP-8272 says a valid reference "MUST add `RECENT_ROOT_ADDRESS` and its `storage_key` to
the transaction's accessed address and storage-key sets. This affects warm/cold gas
accounting only." ethrex additionally records each key as an EIP-7928 `storage_reads`
entry, which raised the question of whether that sentence forbids the record.

**Decision: keep it.** The sentence scopes what *adding to the accessed sets* does; it
does not speak to EIP-7928 at all. Three reasons the record is required rather than
merely permitted:

1. The read genuinely happens. Validating a reference compares
   `RECENT_ROOT_ADDRESS[storage_key]` against `entry_hash`, so a real storage slot is
   read from state.
2. EIP-7928's own precedents record protocol-performed reads, not just EVM `SLOAD`s: the
   EIP-7002 and EIP-7251 dequeues read queue slots "which appear as storage_reads".
   The reference check is the same shape — a protocol-level read outside any EVM frame.
3. A BAL reconstructor that omitted it would not reproduce the access list execution
   produces, which is what `the_access_list_commitment_survives_a_rebuild` requires.

Recorded as a **read**, never a change, under the transaction's own
`block_access_index`, and only after the whole validity pass succeeds. **Action:**
closed, with this justification. **Owner:** —

### 9.4 EIP-8250 keyed nonces are protocol bookkeeping

EIP-8250: keyed-nonce reads and writes "do NOT add `NONCE_MANAGER` or its slots to
EIP-2929 `accessed_addresses` or `accessed_storage_keys`, are NOT charged under EIP-2200
`SSTORE` pricing, and do NOT warm the address or slot for later user-level access."

**Conformant on all three.** `consume_keyed_nonces` never calls `add_accessed_address`
or its storage equivalent, and charges only `KEYED_NONCE_FIRST_USE_GAS` on a key's first
use. Two tests in `test/tests/levm/eip8250_tests.rs`:
`consuming_a_keyed_nonce_does_not_warm_the_nonce_manager` (probing `NONCE_MANAGER` after
a keyed consumption costs exactly what probing a never-touched account costs) and
`a_keyed_nonce_is_not_priced_as_an_sstore` (a second first-use key costs one surcharge
plus envelope data, with no storage charge layered on).

The third clause — the *slot* not entering `accessed_storage_keys` — is not separately
observable from the EVM, since `SLOAD` only ever reads the executing account's own
storage and `NONCE_MANAGER` has no code path that would expose it. It follows from the
same absence of warming calls that the address clause is asserted on.

## 10. Sweep and checkpoint (Tasks 6.7, 6.8)

### 10.1 Every consensus-visible row (Task 6.7)

Task 6.7's bar: zero rows may end the phase as "yes / unresolved / no fallback".

| Row | Consensus-visible | Disposition |
| --- | --- | --- |
| §2 fork rule set is broader than EIP-8081 | yes | **carried** — publication must name the five-EIP set and the pins, never the fork name. Feeds Phase 8's artifact set. |
| §2 Amsterdam prerequisite | yes | **closed** — enforced at genesis load |
| §3.1 VERIFY budgets | yes | **closed** — conformant, `2**20` |
| §3.2 two fixed endpoints, no claimed index | yes | **carried** — one-way (ethrex stricter); fallback is the EIP-7805 extension draft, which EIP-8369 explicitly asks for |
| §3.3 attester state reconstruction | no | closed |
| §3.4 per-IL code-byte budget | yes | **carried** — published in the artifact set and the extension draft |
| §3.5 `AA_VOPS_SLOT_COUNT = 4`, absent means 4 not off | yes | **carried** — published; a joining client MUST NOT read the missing key as "off" |
| §3.6 `SLOTNUM` banned in the validation prefix | yes | **closed** — EIPs#12066 merged 2026-08-11 |
| §5 `RECENT_ROOT_CODE` bytes | yes | **carried** — byte-identical to EIPs#12131; unmerged, so track it |
| §5 static-context gas not pinned by a test | yes | **open test gap**, behaviour is spec-conformant; see §10.2 |
| §8 canonical paymaster hash | yes | **carried** — matches EIPs#12041; unmerged, so track it |
| §9.1 payer warming | yes | **closed** — fixed and tested |
| §9.2 signature validation records no precompile | yes | **closed** — conformant and tested |
| §9.2 `floor_cost` follows the fork's calldata floor | yes | **carried** — re-check if EIP-7976 joins this chain's fork schedule |
| §9.3 EIP-8272 reference reads in the BAL | yes | **closed** — kept, with justification |
| §9.4 keyed nonces are protocol bookkeeping | yes | **closed** — conformant and tested |

No row is unresolved without a stated fallback, so Task 6.7's bar is met.

Four rows are carried by deliberate choice rather than closed, and they share one root
cause: **EIP-8369 defers the whole consensus integration to an extension EIP that does
not exist**. §3.2, §3.4 and §3.5 are all consequences of having to pick an enforcement
point anyway. Publishing that extension draft is what converts them from divergences
into a specification a second client can implement, and it is the single highest-value
piece of upstream work this branch has outstanding.

### 10.2 Checkpoint (Task 6.8)

Verified on 2026-08-10:

| Check | Result |
| --- | --- |
| every row has a non-empty action | yes |
| no consensus-visible row unresolved without a fallback | yes — §10.1 |
| pins match head for 8141 / 8250 / 8272 / 7805 | yes — bumped to `4093c21847` |
| pin matches head for 8369 | yes — `6f818e27dd`, PR #12110 head |
| `cargo clippy --workspace` (l2/prover/guest excluded) | clean |
| `cargo test -p ethrex-test --test ethrex_tests` | 1229 passed / 0 failed |
| `cargo test -p ethrex-rpc --lib` | 122 passed / 0 failed |
| `make -C tooling/ef_tests/blockchain test` | 14 744 + 3 138 passed / 0 failed |

`make -C tooling/ef_tests/engine test` is clean at 74 458 passed / 0 failed, with the
24 FOCIL fixtures skipped: `tests-focil@v0.1.0` was filled against an Amsterdam
predating EIP-8282, so its `pre` omits the builder deposit/exit predeploys and every
payload fails EIP-8282's empty-code rule before the inclusion-list check runs. The
fixtures commit to a genesis hash computed without those predeploys, so supplying them
is not available either. Diagnosis and evidence are under "Not complete" in
`docs/hegota-testnet.md`. That crate is its own cargo workspace, so it must be run
through its Makefile.

**Phase 6 is complete.** Bring-up is unblocked as far as the ledger is concerned; the
FOCIL fixture failures are the remaining item that predates it.
