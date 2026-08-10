# Hegotá testnet — divergence ledger

Every place this branch's behaviour differs from the specification a second client
would implement from, plus the upstream drift since each pin. A row whose
**Consensus-visible** column reads `yes` and whose **Action** is not `closed` is a
chain split waiting for the first block that exercises it, which is why this ledger
gates bring-up.

Spec lookups go through `eipmcp`; PR state through `gh`. Audited against
`ethereum/EIPs@4093c2184` (indexed 2026-08-09) on 2026-08-10.

## 1. Drift since the pinned revisions

`diff_eip(n, since=<pin>)` for each core EIP, against the ```pins``` block in
`docs/hegota-devnet.md`.

| EIP | Pin | Drift to head | Consensus-visible | Action | Owner |
| --- | --- | --- | --- | --- | --- |
| 8141 | `4a9ad32cf2` | none — byte-identical | no | closed | — |
| 7805 | `9a345f96c2` | none — byte-identical | no | closed | — |
| 8250 | `81b976ac01` | one Abstract sentence: privacy applications sharing one sender | no | bump pin to `4093c2184` | Edgar |
| 8272 | `d8636a330d` | one Abstract sentence: proofs against recent commitment roots | no | bump pin to `4093c2184` | Edgar |
| 8369 | `ad8571028a` | two commits on PR #12110 (2026-08-10), extension-requirements paragraph expanded | no (prose about a future extension) | bump pin to `6f818e27d`; the prose is already reflected in `docs/hegota-testnet.md` §"Profile 2 enforcement judges two fixed endpoints" | Edgar |
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
| EIP-8141 | banned list does not include it; the ban is open PR #12066 |
| Consensus-visible | **yes** |

ethrex adopted the proposed ban ahead of merge (`#7108`, commits `d43ebdda2`,
`fc018b526`). A validation prefix branching on `SLOTNUM` is admitted by a spec-literal
client and rejected here. The ban is right — EIP-8272 makes the beacon slot load-bearing,
which gives `SLOTNUM` exactly the admission-time/inclusion-time divergence the banned
list exists to prevent — but it is a divergence until #12066 merges.

**Note:** `docs/eip-8272.md` still claims ethrex "keeps following the list as written
until it merges", which the code contradicts. Corrected in this pass.

**Action:** carry; track #12066. **Owner:** Edgar.

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

## 6. Open upstream PRs (Task 6.3)

All confirmed **open** on 2026-08-10 via `pending_prs_for_eip`.

| Upstream PR | Subject | ethrex | Ships meanwhile |
| --- | --- | --- | --- |
| EIPs#12066 | ban `SLOTNUM` in validation prefix | `#7108`, transplanted | the ban (see §3.6) |
| EIPs#12041 | canonical paymaster reference bytecode | none | `FRAME_CANONICAL_PAYMASTER_CODE_HASH = H256::zero()` sentinel (`mempool.rs:63`) + fallback branch (`blockchain.rs:3825`) — **Task 6.4, open** |
| EIPs#12039 | keyed mempool concurrency | none | `keyed_concurrency_verdict` |
| EIPs#12109 | atomic-batch approval scope | none | `docs/eip-8250.md` divergence #4 |
| EIPs#12091 | block inclusion gating and payer solvency | none | — |
| EIPs#12113 | initial `accessed_addresses` set | none | **Task 6.5, open** |
| EIPs#12026 | floor repricing, signature validation, `frame.value` gas | none | **Task 6.5, open** |
| EIPs#12061 | frame receipt has no transaction-level status | none | — |
| EIPs#12110 | VOPS profiles for FOCIL eligibility (EIP-8369 itself) | implemented ahead of merge | §3.2–3.5 |
| EIPs#12131 | specify `RECENT_ROOT_CODE` | `#7120` | §5 — bytes verified identical |

Not in the plan's list, found this pass:

| Upstream PR | Subject | Bearing |
| --- | --- | --- |
| EIPs#12062 (draft) | explicit second dimension for state gas on frames | would change frame gas accounting; watch |
| EIPs#12121 | link first reference to each cited proposal | editorial |
| EIPs#11681, #11555, #11580, #11482 | guarantors, payer-approves-first, precompiles in VERIFY frames | long-idle drafts, no ethrex surface |

## 7. Phase 6 task status

| Task | Status |
| --- | --- |
| 6.1 record drift since pins | **done** — §1, and no core EIP moved normatively |
| 6.2 reclassify adopted items, bump pins | **partly** — §4 confirms all five, plus three more; `docs/eip-8272.md`/`docs/hegota-devnet.md` already carry them as annotated "no longer a divergence" entries rather than deletions, which reads better and is left as is. Pin bumps outstanding. |
| 6.3 open-PR rows | **done** — §6 |
| 6.4 resolve EIPs#12041 paymaster hash | **open** |
| 6.5 EIPs#12026 BAL clause + EIPs#12113 warm-set clause | **open** |
| 6.6 EIP-8272 BAL read record + EIP-8250 bookkeeping exclusion | **open** |
| 6.7 every `yes` row closed or moved to Open Questions with a fallback | **open** — every `yes` row has an action and §3.1 is closed conformant. Remaining blockers are 6.4-6.6. |
| 6.8 checkpoint | **open** |
