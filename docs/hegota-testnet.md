# Hegotá testnet — implementation plan

Working document. Plan for `hegota-testnet`: a permissioned, externally reachable
kurtosis devnet carrying **four transaction-layer EIPs plus EIP-8369 as FOCIL's
eligibility layer** on one ethrex binary, with a fixed chain ID and a reproducible
genesis, so another client can join it later without a re-genesis.

EIP-8369 is in the rule set rather than alongside it. EIP-7805 enforcement needs an
eligibility rule for frame transactions, EIP-8369 is that rule, and it has no activation
switch to turn off — so enforcing rather than excusing frame-transaction omissions means
publishing it. Its constants are consensus inputs and appear in the artifact set for that
reason.

## Starting state

Resolved at the time the branch was cut. `left-right` counts read as
`main-only / branch-only` against `origin/main`.

| Ref | SHA | vs `origin/main` |
| --- | --- | --- |
| `origin/main` | `797df5540` | — |
| `origin/hegota-devnet` | `dbcaeae78` | 8 / 196 |
| `origin/focil` | `667563366` | 9 / 37 |
| `origin/eip-8250` | `a441f27eb` | 20 / 15 |
| `origin/eip-8272` | `c1d57faa4` | 18 / 24 |
| `origin/eip-7906` | `1b6c6f15a` | out of scope |

`hegota-testnet` is cut from `origin/hegota-devnet`. Two traps:

- **A stale local `hegota-devnet` ref is 132 commits behind its remote.** Reasoning from
  it produces a wrong picture of the branch — it predates the FOCIL merge, EIP-8312,
  EIP-8369 and the divergence reclassification. Always resolve `origin/hegota-devnet`.
- **`origin/main` is not an ancestor of this branch**, and has 8 commits this branch
  lacks. A PR based on `main` therefore cannot be merged as a branch without dragging
  that divergence in; transplant its commits instead. Expect type-level breakage even
  when the text applies cleanly, because helper signatures differ between the two bases.

## Progress

Branch `hegota-testnet`, pushed to `origin`. Baseline for anything below: the whole
workspace compiles (`--exclude 'ethrex-l2*' --exclude ethrex-prover --exclude
ethrex-guest-program`) and `cargo test -p ethrex-test --test ethrex_tests` is **1217
passed / 0 failed**.

| Landed | Commit | Notes |
| --- | --- | --- |
| Branch cut (Task 2.1) | `87fb3f700` | from `origin/hegota-devnet`, not `main` |
| PR ledger (Phase 1) | `4b6563197` | `docs/hegota-testnet-prs.md` |
| EIP-8272 predeploy (Task 4.1) | `b964b318a` | `#7120` merged; native write and its flat 22100 gas gone |
| `SLOTNUM` ban | `d43ebdda2`, `fc018b526` | `#7108` transplanted; its test needed the FOCIL-surface argument |
| `NONCEKEYLOAD` removed (Task 2.3) | `f598130c0` | `0xB9` free again |
| Install phase written (Phase 9) | `a3f1b77be`, `a6b6c1033` | from a read-only survey of a running deployment |
| Kurtosis config (Tasks 7.1/7.2/7.4/7.5) | `40e8f11a4`, `70c8ebc4a` | `fixtures/networks/hegota-testnet.yaml` |
| Permissioning + runbook (Tasks 7.6/7.7) | `b27a9b999` | `docs/hegota-testnet-permissioning.md` |
| Generator pin + 8282 inhibitor | (in `70c8ebc4a` range) | generator `v6.1.6`; package default 6.0.7 would freeze at Amsterdam |
| EIP-7906 removed (Task 2.2) | `e862b2fa1` | −3054/+118; mode 3 unassigned, `Utxo` still 5 |
| `origin/main` merged (Task 2.6) | `b1c21e7b0` | 15 conflicts plus six silent auto-merge duplicates |
| Two-endpoint Profile 2 (Tasks 3.0/3.1/3.3) | `07092961c` | union of both endpoints; budgets derived from parent gas limit |
| Phase 4 closed (Tasks 4.2-4.8) | `a485332a6` | code hash + mask pinned, the three activation cases enforced, predeploy behaviour and the BAL write/read split tested |
| Per-IL code-byte budget (Task 3.2) | `138f75ce0` | charged inside the replay, not after it; 16 bodies / 16 x 64 KiB per list, shared across candidates and both endpoints |
| Phase 3 closed (Tasks 3.1/3.3-3.6) | | inert-surface and exact-set opcode tests, chain-config doc comments, absent-field table |

### Not complete

- **Phase 5** and **Phase 6** — not started.
- **Phase 7 Task 7.3** (generator revision) is satisfied by pinning the generator image
  in the config instead of bumping `ETHEREUM_PACKAGE_REVISION`; **Task 7.8** needs a
  live enclave.
- **Phase 8** — the artifact set and firewall surface: not started.
- **Phase 9 Task 9.9** — local kurtosis validation. Prerequisite is an `ethrex` image
  build, and the three `REPLACE` placeholders in the config filled with throwaway local
  values.

### Traps found the hard way — do not rediscover these

- **Resolve `origin/hegota-devnet`, never a local `hegota-devnet` ref.** A stale local
  ref was 132 commits behind and produced a wrong picture of the entire branch: it
  predated the FOCIL merge, EIP-8312, EIP-8369 and the divergence reclassification.
- **`origin/main` is not an ancestor of this branch.** A `main`-based PR cannot be
  merged as a branch; transplant its commits. Expect type-level breakage even when the
  text applies cleanly, because helper signatures differ between the two bases.
- **The `main` merge's real hazard was silent auto-merge duplication, not conflicts.**
  `--mempool.private` had already been cherry-picked here, so main's copy arrived as an
  independent addition and git kept both, producing six duplicate definitions with no
  conflict marker: `is_blob_carrying`, `private_mempool` (field and Default init),
  `add_transaction_no_broadcast`, `add_transaction_inner`, the `mempool.private` CLI
  flag and its Default init. Only the compiler finds these. And the cherry-pick was
  **not** a superset — it had no `private_pool` at all, so "take ours" would have
  shipped a private mempool that still leaks over P2P.
- **Two replacement policies now coexist by design.** `check_replacement_bid` applies
  the blob↔non-blob category gate to everything, routes frame transactions to
  EIP-8141's stricter rule (both dimensions ≥ `FRAME_TX_MIN_FEE_BUMP_PERCENT`, not
  operator-tunable), and applies main's configurable percentage bump to everything else.
- **Do not use index or brace arithmetic to delete Rust spans.** Two separate attempts
  broke the tree; use exact-match edits or `ast-grep --rewrite`.
- **Do not edit the working tree while a subagent is running in this repo.** One ran
  `git stash` for a test baseline, swallowed unrelated uncommitted work, never popped
  it, and reported "all changes fully restored". The reflog and `git stash list` are the
  record, not the agent's report.
- **`TxValidationError::InvalidFrameTransaction` lies about which check failed.** Static
  constraints and the VERIFY-approval check share the one variant, so a frame tx built
  without `nonce_keys` (a non-vault sender must select 1..=16) reports "VERIFY frame did
  not call APPROVE or payer not approved". Check the payload shape before the approval
  code.
- **The three EIP-8272 write-gas figures on record are one curve, not a disagreement.**
  127 256 (ours, 64 non-zero calldata bytes), 127 244 (ethereum/EIPs#12131) and 127 196
  (ethrex `#7120`) differ only by how many zero bytes the `salt ‖ root` payload carries:
  execution sits far above the EIP-7623 floor, so each zero byte is 12 gas cheaper.
  Reconcile a gas delta against calldata content before reading it as a pricing split.

The two things that decide whether this succeeds:

1. **Cross-client interop.** Every consensus-visible ethrex-only divergence must be
   closed or justified before a second client joins. A surviving divergence is not a
   bug report, it is a chain split on the first block that uses it.
2. **Permissioning like Sepolia.** Anyone may sync and peer; becoming a validator
   requires an access token to deposit. That asymmetry is what makes the network both
   open to a third party and controlled by us.

Companion documents on this branch: `docs/hegota-devnet.md` (branch composition,
opcode allocation, per-EIP divergences, spec pins), `docs/hegota-devnet-genesis.md`
(genesis requirements and the post-deploy verification pass), `docs/eip-8141.md`,
`docs/eip-8250.md`, `docs/eip-8272.md`, `scripts/hegota-devnet/USER-GUIDE.md`,
`scripts/hegota-devnet/UPGRADE-GUIDE.md`.

## Overview

Recut `origin/hegota-devnet` into `hegota-testnet` by deleting the two surfaces that
activate at `Fork::Hegota` and are not in scope (EIP-7906 and `NONCEKEYLOAD`), leaving
the two that have their own activation switch inert (EIP-8312, EIP-8369), then close
the frame-tx seams in FOCIL, replace the EIP-8272 native write with the specified
predeploy bytecode, close the divergence ledger against upstream, and launch three
ethrex+CL pairs under kurtosis on a public host behind a gated deposit contract.

## Requirements

Explicit:

- Branch named `hegota-testnet`.
- Four transaction-layer EIPs **active**: EIP-8141 (frame transactions), EIP-8250
  (keyed nonces), EIP-8272 (recent roots), EIP-7805 (FOCIL). All under the single
  existing `Fork::Hegota`, genesis aliases `hegotaTime` / `hezeTime` / `bogotaTime`.
- EIP-8369 (VOPS profiles for FOCIL eligibility) is **also active and published**: it
  has no activation switch, and EIP-7805 enforcement over frame transactions is
  undefined without it. Its consensus inputs are `AA_VOPS_SLOT_COUNT = 4`, the derived
  VERIFY budgets, the per-list code-byte bound, and the two-endpoint enforcement rule.
- EIP-7906 (transaction assertions) is out. `NONCEKEYLOAD` (`0xB9`) is out.
- EIP-8312 (UTXO frames) stays in the tree but inert behind an unset `utxoFramesTime`,
  including the defensive `Undecided` guard in the Profile 2 evaluator, which is
  unreachable while it is inert and is what keeps a prefix-only replay honest if it is
  ever switched on.
- Fixed chain ID and reproducible genesis, publishable without a re-genesis.
- Gated deposit contract wired into genesis; validator entry requires a token.
- 3 nodes under kurtosis on a production server, reachable from the public internet.
- The plan is written against ethereum/EIPs upstream text and the nine open upstream
  EIP PRs Nethermind is implementing.

Inferred:

- The published artifact set must be enough for a Nethermind EL **and** a consensus
  client, not just an EL. A joiner needs both layers or it cannot follow the chain.
- Nethermind is configured from a **chainspec**, not from `genesis.json`. The
  genesis generator emits both; the chainspec's per-EIP transition keys are the
  interop surface, and it does not currently emit keys for EIP-8250/EIP-8272
  (verified: `apps/el-gen/generate_genesis.sh:922-949` at generator `v6.1.6` emits
  only `eip7805TransitionTimestamp` and `eip8141TransitionTimestamp` for heze).
- FOCIL forces the engine API to V5/V6, so the consensus client must be FOCIL-aware.
  Execution and consensus upgrade together with no inert intermediate state
  (`docs/hegota-devnet.md`, EIP-7805 section).
- Proving stays disabled. `docs/eip-8272.md` records the prover caveat and no prover
  runs on this network.

Assumptions, each stated so it can be falsified:

- The production host has a single static public IPv4 address and we control its
  firewall. IPv6 peering is not required.
- We hold one EOA private key that becomes the gater admin, and it is funded in
  genesis via `network_params.prefunded_accounts`.
- Nethermind joins as a **non-validating** full node first. A Nethermind validator
  requires us to mint them a deposit token, which is a separate, later step.
- `--dev` single-node reproduction stays available via `fixtures/genesis/l1-hegota.json`.

## Out of Scope

- **EIP-7906.** Deleted from this branch, not merely disabled: `TXTRACE`,
  `EVENTDATACOPY`, `TXDIFF` and POST_TX frames all gate on `Fork::Hegota` with no
  separate switch, so leaving the code in would make them live on the published
  chain and split against any client that does not implement them.
- **`NONCEKEYLOAD` (`0xB9`).** Same reason: an ethrex-only opcode registered at
  `Fork::Hegota` that no other client implements. EIP-8250 exposes `len(nonce_keys)`
  (`TXPARAM 0x0D`), `nonce_keys_hash` (`0x0E`) and `nonce_keys[0]` (`0x10`) and
  deliberately provides no per-index accessor, so nothing in the four-EIP scope needs it.
- **EIP-8312 (UTXO frames) and EIP-8369 (FOCIL eligibility) code removal.** Both
  already have their own activation switch (`utxoFramesTime`, `AA_VOPS_SLOT_COUNT`)
  and are inert when unset, so they are handled by *leaving the switch unset*, not by
  deleting ~2000 lines. Phase 2 proves inertness instead. The rule this plan applies:
  **delete what activates at `Fork::Hegota` and is out of scope; leave inert what has
  its own switch.**
- **`payerTxparamTime` (resolved-payer `TXPARAM 0x11`) and `derivedSlotTime`.** Left
  unset for the same reason; `TXPARAM 0x11` keeps its `InvalidOpcode` halt and the
  slot arrives from the consensus client over the engine API.
- **EIP-8288** (PQ signatures + STARK aggregation) — upstream-blocked.
- **EIP-8025** stateless proving and any prover integration.
- **Bare-metal / non-kurtosis deployment.** No systemd units, no non-kurtosis node
  packaging.
- **A public faucet build-out beyond what `scripts/hegota-devnet/faucet/` already is.**
  It carries across unchanged; no new features, no hardening pass.
- **A long-lived testnet ops story**: no monitoring/alerting stack, no on-call
  runbook, no key rotation policy, no genesis re-spin automation.
- **EIP-8038 gas refitting or performance work.**
- **`#7038` blob-carrying frame transactions as a new feature.** It is already merged
  into `origin/hegota-devnet` (`d5192b82c`), so it carries across; no additional work.
- **`#6974` (`ethrex_simulateFrameTransaction`) and `#7091` (RLPx inbound hardening)
  as gates.** Both are already on `origin/hegota-devnet` or independent of interop;
  they are not allowed to block bring-up.

## Existing Patterns

- **`docs/hegota-devnet.md`** — the convention for this family of documents: a
  Composition block, an opcode-allocation table, a per-EIP divergence list where each
  entry says whether it is a divergence *or* explicitly "not a divergence", and a
  fenced ```pins``` block giving `eip-<n> <commit> <class> <date> [source]`. New docs
  follow that shape. The `/hegota-eips` skill reads the pins block.
- **`docs/hegota-devnet-genesis.md`** — genesis requirements written as "every item
  here is load-bearing", each with the failure mode it prevents and the misleading
  symptom it produces. Extend, do not replace.
- **`docs/CONTRIBUTING_DOCS.md`** — mdBook + `mdbook-linkcheck2`; `lychee.toml` runs
  `lychee --config lychee.toml docs/` over the **whole** directory, so every external
  link in a new doc must resolve (2xx, or 429/503). `docs/SUMMARY.md` does *not* list
  `docs/eip-8141.md` or `docs/hegota-devnet.md`, so Hegotá working notes are
  deliberately outside the book nav: **do not add `hegota-testnet.md` to `SUMMARY.md`.**
- **`crates/vm/system_contracts.rs`** — predeploys as `SystemContract { address, name,
  active_since_fork }` plus a `*_RUNTIME_BYTECODE` byte array, installed at the fork
  boundary by an `install_*_code` function called from both `prepare_block` and
  `apply_system_calls` (`crates/vm/backends/levm/mod.rs`, `crates/vm/backends/mod.rs`).
  `install_nonce_manager_code` is the reference implementation of the spec's
  three-case activation rule (create / adopt-empty / leave-alone).
- **`fixtures/networks/hegota-devnet.yaml`** — kurtosis config convention: every
  non-obvious knob carries a comment naming the failure it prevents. `port_publisher`
  blocks below the ephemeral floor; `additional_preloaded_contracts` for the EIP-8282
  inhibitor; `dora_params.image` pinned to a frame-tx-aware build.
- **`scripts/eip8141-devnet/Caddyfile`** — prior art for exposing an enclave's RPC and
  explorer through one reverse proxy with permissive CORS, driven by `$RPC_PORT` /
  `$DORA_PORT` environment variables.
- **`~/.claude/skills/hegota-branches`, `hegota-brief`, `hegota-eips`** — existing
  automation for the recurring branch/PR/spec drift checks. Point at them rather than
  re-specifying the procedure.

## Architecture Decision

### Base the branch on `origin/hegota-devnet`, not on `main`

`origin/hegota-devnet` is **132 commits ahead of the local `hegota-devnet` ref** and
already carries, verified by reading it:

- `d57568844 Merge branch 'focil' into hegota-devnet` — **FOCIL is already merged.**
- `c45a95df9 fix(l1): align SIGPARAM copy operands with CALLDATACOPY` — the `#7089`
  consensus fix. `main` still pops the pre-`4a9ad32cf` order at
  `crates/vm/levm/src/opcode_handlers/frame_tx.rs:477`
  (`let [length, data_offset, mem_offset] = *vm.current_call_frame.stack.pop()?;`).
- `d5192b82c` + `437884b54` + `a4bf24279` — blob-carrying frame transactions (`#7038`).
- `1b6b9df7a docs(l1): retire the pinned-address divergences and bump the spec targets`
  — the divergence reclassification is already recorded, pins now at
  `eip-8141 4a9ad32cf2`, `eip-8250 81b976ac01`, `eip-8272 d8636a330d`.
- `8b12baa9a fix(l1): exclude EIP-8141 frame txs from FOCIL IL satisfaction check`
  (inherited from `origin/focil`) plus `58d9831e3`, `a9e0a0528`, `b23e20684` refining
  frame-tx IL eligibility on top of it.
- EIP-8312 (UTXO frames, own `utxoFramesTime`) and EIP-8369 (FOCIL eligibility,
  `AA_VOPS_SLOT_COUNT`) — two EIPs beyond the four in scope, both inert when unset.
- `ghcr.io/lambdaclass/dora:heze-decode` as the explorer image (superseding
  `:frame-tx-view`).

Rebuilding this from `main` means re-resolving conflicts that were already resolved
and reviewed: `#6906` (`eip-8250`, +2468/−219), `#6907` (`eip-8272`, +2086/−113) and
`#7039` (`focil`, +4626/−86) are all **CONFLICTING against `main`** right now. Three
large three-way merges plus a FOCIL↔frame-tx integration that already exists is the
worst possible trade.

Rejected alternatives:

- **Recut from `main`, merge `origin/eip-8250` + `origin/eip-8272` + `origin/focil`.**
  Rejected: redoes three conflicting merges and discards the FOCIL↔frame-tx work.
- **Recut from `origin/focil` and merge 8250/8272 in.** Rejected for the same reason,
  and it additionally discards the FOCIL frame-tx eligibility refinements
  (`58d9831e3`, `a9e0a0528`, `b23e20684`) that only exist on `origin/hegota-devnet`.
- **Keep EIP-7906 and gate it behind a new timestamp.** Rejected: inventing a
  per-EIP knob to avoid a deletion is more code, more surface and one more thing
  Nethermind must agree is unset. Deleting is smaller and provable by grep.

### Chain ID `8141`

Free on `chainid.network/chains_mini.json` (checked against all 2681 registered
chains; `8272` is taken, `8141` is not). Mnemonic for the base EIP. Explicitly **not**
`3151908`, the kurtosis default: every local devnet a Nethermind developer runs uses
it, so wallets, RPC configs and datadirs collide.

### Replace the EIP-8272 native write with the specified predeploy

The native write is a real consensus split, quantified in `#7120`: the same
transaction costs **38 064 gas in ethrex and 127 196 on a client that executes the
predeploy**, because the native path skipped the EIP-8037 state-gas charge for the
created slot. Installing the specified bytecode makes gas *emergent from the EVM*
rather than a constant either side has to agree on, and deletes divergences #4, #6,
#7, #8 and #9 from `docs/eip-8272.md` at once. This is the root-cause fix; pricing the
native write differently would be the symptom fix.

Independently verified against ethereum/EIPs#12131's 144-byte runtime:

- Length is exactly 144 bytes.
- The two `PUSH32` domain constants are
  `8f42481679c8e6fefa040974b3c905e0ce3f2e464ba93acdb074a41181617efc` and
  `bdc897da2177d260ff5f4be5d4b2aad43f89c3347a305b584fa5a2546d053daa`, which are
  `keccak256("RECENT_ROOT_ENTRY")` and `keccak256("RECENT_ROOT_STORAGE")` — byte-for-byte
  the domains ethrex already uses in `RecentRootReference::{entry_hash, storage_key}`
  (`crates/common/types/transaction.rs`, `origin/eip-8272` lines 2115-2149).
- `source_id` is `KECCAK256(mem[0x0c..0x40))` over `CALLER` (unpadded 20 bytes) ‖
  32-byte salt — the same 52-byte preimage as ethrex's `recent_root_native_write`.
- `SLOTNUM` is `0x4B` (`crates/vm/levm/src/opcodes.rs:80`), reachable from any code
  path via `OpSlotNumHandler` (`crates/vm/levm/src/opcode_handlers/block.rs:244-252`),
  so the predeploy runs unmodified in levm.

So the swap changes gas and call semantics, **not** the hash layout: a root written
through the predeploy still validates its own reference with no change to the read side.

### Frame transactions are enforced through EIP-8369 Profile 2, not excused

**Corrected against the branch.** The blanket `TxType::Frame` skip is gone from
`crates/blockchain/inclusion_list_validator.rs`, and both callers wire a live Profile 2
evaluator: `crates/networking/rpc/engine/inclusion_list.rs:139` and
`crates/blockchain/blockchain.rs:2558` construct `BlockchainProfile2Evaluator` and call
`check_with_profile_2`. Frame-transaction omissions are therefore enforced on this
branch today, by replaying each candidate's validation prefix
(`crates/blockchain/focil_profile2.rs:125`).

This makes **EIP-8369 a fifth active EIP, not an inert one.** It has no activation
timestamp: `ChainConfig::aa_vops_slot_count()` falls back to
`DEFAULT_AA_VOPS_SLOT_COUNT = 4` (`crates/common/types/genesis.rs:35`), so leaving the
field unset selects the constant rather than disabling the profile. Nothing in the
chain config can turn it off.

Resolved: it stays on and is completed, not gated off or skipped. EIP-8369 is part of
this chain's published rule set, with `AA_VOPS_SLOT_COUNT = 4`, derived VERIFY budgets,
and the two-endpoint enforcement rule below. The plan no longer assumes inertness.

The original reasoning for skipping frame transactions, retained because it is why the
generic path must never be reached for type `0x06`: the satisfaction
check is a pure state comparison that must not call the EVM
(`crates/blockchain/inclusion_list_validator.rs`), but a frame transaction's
includability depends on executing its validation prefix to discover `payer`, on the
keyed-nonce domain in `NONCE_MANAGER` rather than the account nonce, and under
EIP-8272 on a slot-windowed storage read. Comparing the *declared sender's* nonce and
balance — which is what the generic path does, and which every existing skip fails to
catch for type `0x06` — yields `IlUnsatisfied` for transactions that could never have
been validly appended, making attesters reject a valid block on `engine_newPayloadV6`.
Excusing all of them is a strict subset of any future eligible set, so it can only
ever over-accept, never split. Symmetric decision for the **builder** side: exclude
frame transactions from the locally built inclusion list too, because an entry every
client is required to excuse buys no censorship resistance while consuming the 8 KiB
budget, and the builder's per-sender linear-nonce walk is actively wrong for them.

### Profile 2 enforcement judges two fixed endpoints

EIP-8369 specifies the eligibility *profiles* and explicitly not the consensus
integration: Profile 2 enforcement "can activate only after EIP-8141, EIP-8250, and
EIP-8272 are live and the enforcing extension defines how claimed indices are encoded,
carried, and committed". What it defers is substantial — bounded claim encoding, a
transaction identifier, lookup and deduplication, a block commitment, transport to the
execution layer, retention and pruning across branches, and a consensus-validated
canonical diff for every Profile 2 state update, with an equivalent diff an activation
prerequisite wherever the EIP-7928 BAL lacks one. So EIP-8369 alone is not implementable
as a chain rule, and the enforcement point has to come from somewhere.

We take the two-endpoint rule: an omission is unjustified when the candidate is eligible
at `S_start` **or** at `S_end`, where `S_start` is the parent post-state with `B`'s
pre-execution system operations applied and `S_end` is the state after `B`'s last
transaction. Both are states the evaluator already holds, so reconstruction costs
nothing, and the count is two however many committee members listed the transaction.
There is no claimed index, so no encoding, no block commitment, no engine-API change and
no BAL reconstruction dependency. It is strictly stronger than `S_end` alone: it closes
the case of a payer solvent early and drained by a later transaction of the same block,
and it stops excusing a queued keyed-nonce transaction that is invalid early in the
payload and valid late.

This is a deliberate departure from EIP-8369's builder-claimed index, which is the worst
affordable subset: end of payload is always claimable, so the omissions a claim excuses
are a superset of those the end-of-payload rule excuses, and the claim carries no cost,
proof or verification to offset that. `gas_fits` stays a single end-of-payload check
because gas remaining is monotone within a payload; evaluating it at `S_start` would mark
every full block unsatisfied and convert the inclusion list into a hard priority claim on
block space.

### The code-byte budget is charged inside the replay, not after it

The per-inclusion-list code bound exists because VERIFY gas does not bound how much code
a Profile 2 replay makes an attester read: at a few thousand gas per cold account, a
single candidate's budget admits hundreds of cold accesses, each able to pull a
maximum-size body. A ledger that only *reports* what a replay loaded, and charges once it
returns, therefore bounds a list but not a transaction — one candidate can exceed the
whole allowance by an order of magnitude before anything notices.

So the allowance travels *into* the replay (`Profile2Replay`, alongside the AA-VOPS
surface) and the observer charges each body as it is loaded, ending the replay on the
first one that does not fit. The evaluator holds the ledger in a `RefCell` and folds each
replay's result back in, so the charge is per list: shared across every candidate and
across both evaluation endpoints, with an already-charged `codeHash` free to load again.
Charges survive a failed verdict, because a replay that loaded bodies and then failed
still made every attester read them.

Bounds are 16 bodies and 16 × 64 KiB. Both bind: a count alone would admit sixteen
maximum-size bodies, and a byte total alone would let a list spend its allowance on many
tiny ones. The byte figure tracks EIP-7954's Amsterdam `MAX_CODE_SIZE` rather than the
pre-Amsterdam 24 KiB, since frame transactions exist only from Hegotá.

The charge order is list order, over the omitted candidates only: a listed transaction the
block included never replays and never charges, so the allowance is spent exactly on the
work omissions force. Both inputs — the list and the block's transaction set — are fixed
before judging, so every attester spends the allowance on the same candidates in the same
order and reaches the same verdicts.

The rule is written up as a Standards Track extension to EIP-7805 — the enforcing
extension EIP-8369 asks for — and that draft, not this document, is the normative
reference once it is published. Two commits already on this branch demonstrate the
motivating behaviour: `467450733` ("show Profile 2 eligibility flipping across a
payload") and `dbcaeae78` ("show eligibility non-monotonic across a payload").

### Activation is one fork timestamp; per-EIP chainspec keys are not our problem

All four EIPs activate together on `Fork::Hegota`, keyed off a single genesis field
(`hegotaTime`, with `hezeTime` and `bogotaTime` accepted as aliases). This matches how
each EIP defines its own activation — EIP-8250 and EIP-8272 are deltas that "MUST
activate at or after EIP-8141" — and it is what `ethereum-genesis-generator` writes into
the geth-style `genesis.json` and `besu.json`: one `bogotaTime` key.

The generator additionally writes a Nethermind-format `chainspec.json` whose activation
is per-EIP (`params.eipNNNNTransitionTimestamp`), and for this fork it emits only
`eip7805TransitionTimestamp` and `eip8141TransitionTimestamp` — nothing for EIP-8250 or
EIP-8272. That is a property of that file format and of what Nethermind currently reads,
not a gap in our activation. We do not add per-EIP transition timestamps to ethrex, and
we do not hand-patch `chainspec.json` to paper over it: doing either would encode another
client's configuration shape into our chain definition and give us two sources of truth
for one fork boundary.

What we publish is the fork timestamp and the EIP set. A client that needs additional
chainspec keys to configure those EIPs needs them added to its own configuration parser
and to the generator; that is upstream work for whoever owns that client, and it does not
block our genesis. Phase 8 states the EIP set, the single activation timestamp and the
constants explicitly, so nothing about the rule set has to be inferred from a
client-specific file.

### Adopt the `SLOTNUM` validation-prefix ban (`#7108` / EIPs#12066)

EIP-8141's banned-opcode list is written as a **public-mempool validation-trace rule**
("A public mempool node must simulate the validation prefix and reject the transaction
if … execution uses a banned opcode"), and it is enforced that way: the check runs only
behind `validation_observer.active` (`crates/vm/levm/src/vm.rs:3647`), the sole site
setting `active: true` is `ValidationObserver::new` at `vm.rs:3176`, and its sole caller
is `LEVM::simulate_frame_validation_prefix`. Block execution never reaches it, so the
ban cannot change a state root.

**It is nonetheless fork-choice visible on this branch, through FOCIL.** One of
`simulate_frame_validation_prefix`'s callers is `crates/blockchain/focil_profile2.rs:125`,
where `!outcome.passed` becomes `Profile2Eligibility::Ineligible`. A banned opcode in
the prefix therefore makes the transaction ineligible, which makes its omission
justified, which decides whether the payload satisfies its inclusion lists and whether
an attester votes for it. Banning `SLOTNUM` while another client does not means ethrex
excuses an omission that client enforces: ethrex attests to a block it withholds from.
The divergence is in attestation rather than in execution, so it splits the fork choice
without ever producing a state-root mismatch — which makes it harder to notice, not
more benign.

Adopting `#7108` is correct on the spec's own terms: EIP-8272 makes the beacon slot
load-bearing for transaction validity, which gives `SLOTNUM` exactly the property the
banned-opcode list exists to exclude — a prefix that passes at admission and fails at
inclusion — and EIPs#12066 is the upstream change that says so. That another client is
also implementing it is corroboration, not the reason. The operational consequence is
that the ban belongs in the **published rule set** rather than being treated as
node-local policy, because through Profile 2 it decides an omission verdict; a client
that follows the EIPs will agree, and a client that does not is the one that needs to
change.

EIP-8272 makes the beacon slot load-bearing,
which gives `SLOTNUM` exactly the property the list exists to exclude, and Nethermind
is implementing the ban. The interaction with `RECENT_ROOT_CODE` is benign: a VERIFY
frame is already forbidden to `SSTORE` under the STATICCALL restrictions, so calling
the recent-root predeploy from a validation prefix could never have written anything;
the ban turns a dead path from a revert into an admission rejection.

### Permissioning: pk910/gated-deposit-contract via the genesis generator

No ethrex-side work. Verified: `ChainConfig::deposit_contract_address`
(`crates/common/types/genesis.rs:374`) plus `Requests::from_deposit_receipts`
(`crates/common/types/requests.rs:99-118`) match deposit logs purely by `log.address ==
deposit_contract_address` and `topics[0] == DEPOSIT_TOPIC`
(`crates/common/constants.rs:47`, `0x649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5`). `GatedDepositContract` is the
mainnet deposit contract plus one `depositGater.check_deposit()` call, with the same
tree, the same `DepositEvent` and **no extra events**, so it is transparent to the EL.
(The pre-Pectra `protolambda/testnet-dep-contract` that Sepolia used emits from a
modified `deposit()` and is not a candidate; it is background only.)

Rejected alternative: write our own gater. Rejected because the pk910 pair is already
installed by `ethpandaops/ethereum-genesis-generator`
(`apps/el-gen/generate_genesis.sh:473-525`) under `DEPOSIT_CONTRACT_GATED=true`, and
has an existing admin CLI (`pk910/gated-deposit-contract-cli`). Writing our own means
writing the CLI too.

## Implementation Plan

### Phase 1: Reconcile state and produce the PR ledger

Why this phase: the branch cannot be cut until we know which of ~21 open frame-tx PRs
are already in `origin/hegota-devnet`, which must be transplanted, and which are
superseded. Cutting first and reconciling later means re-resolving the same conflicts.

- [x] Task 1.1: `git fetch origin` and record in `docs/hegota-testnet.md` a "Starting
      state" section stating the resolved SHAs of `origin/main`, `origin/hegota-devnet`,
      `origin/focil`, `origin/eip-8250`, `origin/eip-8272`, `origin/eip-7906`, and the
      three `git rev-list --left-right --count` figures against `origin/main`. Note
      explicitly that the local `hegota-devnet` ref is 132 commits behind its remote
      and must not be used.
- [x] Task 1.2: Create `docs/hegota-testnet-prs.md` with a table of every open
      lambdaclass/ethrex PR whose title or head branch matches
      `8141|8250|8272|7906|7805|focil|frame|sigparam|slotnum|recent.root|paymaster|nonce`,
      with columns: PR, head, base, review state, mergeable state, in-scope (yes/no),
      action, rationale. Generate the raw rows with
      `gh pr list --repo lambdaclass/ethrex --state open --limit 200 --json number,title,headRefName,baseRefName,reviewDecision,mergeable,additions,deletions,isDraft`.
      Seed it from the snapshot in the "PR reconciliation snapshot" section below and
      refresh every row against `gh` rather than trusting the snapshot.
- [x] Task 1.3: For each PR whose `base` is `hegota-devnet` (`#7120`, `#7086`, `#7084`,
      `#7085`, `#7121`), run `git log origin/hegota-devnet --oneline --grep=<subject>`
      and `git branch -r --contains <head sha>` to determine whether it is already
      merged into `origin/hegota-devnet`. Record "already in base" versus "needs
      transplant" per PR in `docs/hegota-testnet-prs.md`.
- [x] Task 1.4: Record in `docs/hegota-testnet-prs.md` the decision that **`#7120` is
      taken and `#7086` is closed as superseded**, with the one-sentence reason:
      `#7120` is stacked on `#7086` and deletes the native write wholesale (−190
      lines), so `#7086`'s extra interception paths are dead on arrival and landing
      `#7086` alone leaves us on the divergent 38 064-gas pricing.
- [x] Task 1.5: Record in `docs/hegota-testnet-prs.md` the resolution of the
      `#7082` / `#7058` overlap (both implement "frame receipts with the storage
      codec"; `#7058` additionally carries the Amsterdam-scheduling devnet-boot fix).
      Check `origin/hegota-devnet` for `f4f29c001`/`e02532512`-equivalents first; if
      both are already in the base, mark both PRs "already in base, close".
- [x] Task 1.6: Mark as out-of-scope-for-the-gate, with the reason, in
      `docs/hegota-testnet-prs.md`: `#7047` (base `frames-devnet-0`, needs a full
      re-target), `#6974` (`ethrex_simulateFrameTransaction`, useful but not on the
      interop-critical path), `#7091` (RLPx inbound hardening, not interop-critical),
      `#6891` (EIP-7906, deleted from this branch), `#6730` (EIP-8025), `#6625`,
      `#6325`. Each of these must still appear in the table.
- [x] Task 1.7: **Checkpoint: Verify Phase 1 complete.** Review Tasks 1.1-1.6. Confirm
      `docs/hegota-testnet-prs.md` has one row per open matching PR with a non-empty
      action column and no row reading "TBD". List each task and its status. Do not
      proceed until all are done.

### Phase 2: Cut `hegota-testnet`

Why this phase: every later phase edits this branch, so its exact construction and the
proof that the active EIP set is the intended one must land first.

- [x] Task 2.1: `git switch -c hegota-testnet origin/hegota-devnet`. Do not merge
      anything yet.
- [x] Task 2.2: Delete EIP-7906. Remove `crates/vm/levm/src/opcode_handlers/tx_trace.rs`,
      `test/tests/levm/eip7906_tests.rs`, `docs/eip-7906.md` and
      `scripts/hegota-devnet/NOTES-FOR-7906-AUTHOR.md`; drop `TXTRACE` (`0xB6`),
      `EVENTDATACOPY` (`0xB7`) and `TXDIFF` (`0xB8`) from the enum, the dispatch table
      and the Hegotá registration in `crates/vm/levm/src/opcodes.rs`; drop the
      `FrameMode::PostTx = 3` variant and its `from_u8`/`From<FrameMode> for u8` arms
      in `crates/common/types/transaction.rs` (enum and impls at lines 1900-1958 on
      `origin/hegota-devnet`, drifted from the range originally recorded here) so
      modes 3 and 4 are reserved again. **`FrameMode::Utxo = 5` (EIP-8312) is untouched
      and must not be removed** — ethrex deliberately placed EIP-8312's UTXO mode at
      the ethrex-local byte 5, not upstream EIP-8312's colliding `UTXO_MODE = 3`
      (already taken by EIP-7906's `POST_TX`), exactly so this deletion would not
      require renumbering it; drop the POST_TX trailing-suffix rule at
      `crates/common/types/transaction.rs:2523-2532`; drop the per-transaction prestate
      machinery `9fd0a851b` added to `crates/vm/levm/src/db/gen_db.rs`; drop the 7906
      arms in `crates/vm/levm/src/vm.rs`, `crates/vm/levm/src/gas_cost.rs` and
      `crates/vm/levm/src/opcode_handlers/mod.rs`. Use `ast-grep --rewrite` for the
      block deletions and `difft` to confirm each span.
- [x] Task 2.3: Delete `NONCEKEYLOAD`. Remove `OpNonceKeyLoadHandler` and
      `load_nonce_key` from `crates/vm/levm/src/opcode_handlers/frame_tx.rs`, the
      `NONCEKEYLOAD = 0xB9` enum entry, table slot and Hegotá registration plus its
      not-before-Hegota test from `crates/vm/levm/src/opcodes.rs`, and its gas constant
      from `crates/vm/levm/src/gas_cost.rs` (the surface `f27362cb4` added).
- [ ] Task 2.4: Prove the deletion is complete. `grep -rn -iE
      '7906|TXTRACE|EVENTDATACOPY|TXDIFF|POST_TX|PostTx|NONCEKEYLOAD|NonceKeyLoad'
      crates/ cmd/ test/ tooling/ fixtures/ docs/` must return nothing outside
      `docs/hegota-testnet*.md`. Run `cargo check --workspace` and `cargo clippy
      --workspace -- -D warnings`; use `cargo fix` for the cascade of unused imports.
- [ ] Task 2.5: Update `docs/hegota-devnet.md` on this branch: rewrite the Composition
      block to `main + eip-8250 + eip-8272 + eip-7805`, delete the EIP-7906 divergence
      section and the `0xB6`/`0xB7`/`0xB8`/`0xB9` rows from the opcode-allocation table
      (leaving `0xB5 = RECENTROOTREFLOAD` needing no dedup), delete the EIP-7906 pin
      line from the ```pins``` block, and remove the EIP-7906 paragraphs from
      `scripts/hegota-devnet/UPGRADE-GUIDE.md` and `scripts/hegota-devnet/USER-GUIDE.md`.
      Rename the branch references from `hegota-devnet` to `hegota-testnet`.
- [x] Task 2.6: `git merge origin/main`. Expect conflicts in
      `crates/vm/levm/src/vm.rs`, `crates/vm/levm/src/gas_cost.rs`,
      `crates/blockchain/blockchain.rs`, `crates/blockchain/mempool.rs`,
      `crates/blockchain/payload.rs` and `crates/common/types/transaction.rs`. Resolve
      keeping the branch side for frame-tx code and `main`'s side for everything else;
      confirm `crates/vm/levm/src/opcode_handlers/frame_tx.rs`'s `SIGPARAM 0x04` pops
      `[mem_offset, data_offset, length]` (the `c45a95df9` order) and not `main`'s
      pre-`4a9ad32cf` `[length, data_offset, mem_offset]`.
- [ ] Task 2.7: **Checkpoint: Verify Phase 2 complete.** Confirm Tasks 2.1-2.6 with:
      the grep in 2.4 clean; `cargo check --workspace`, `cargo clippy --workspace -- -D
      warnings`, `cargo fmt --check` clean; `cargo test --workspace --exclude
      'ethrex-l2*' --exclude ethrex-prover --exclude ethrex-guest-program` green;
      `make -C tooling/ef_tests/blockchain test` green; `make -C tooling/ef_tests/engine
      test` green (this pulls the FOCIL fixtures via `focil-vectors` from
      `.fixtures_url_focil`, `tests-focil@v0.1.0`). List each task and its status.

### Phase 3: Complete EIP-8369 enforcement, prove EIP-8312 inert, enumerate the surface

Why this phase: the active EIP set is an activation-gating claim, not a code-absence
claim, so it has to be tested rather than asserted. Deleting EIP-7906 is independent of
this work — the FOCIL eligibility path contains no reference to `PostTx` or EIP-7906 — but
the two EIPs the tree carries beyond the intended set are not alike:

- **EIP-8312 is properly inert.** `utxo_frames_time` is a distinct chain-config field
  (`crates/common/types/genesis.rs:346`) and `validate_static_constraints` rejects
  `FrameMode::Utxo` when it is unset (`crates/common/types/transaction.rs:2738`). Mode
  `5` also does not collide with any EIP-7906 mode, so removing EIP-7906 frees nothing
  it uses. This needs a pinning test, nothing more.
- **EIP-8369 is active, cannot be switched off, and specifies no enforcement point** —
  see [the enforcement decision](#profile-2-enforcement-judges-two-fixed-endpoints).
  Profile 2 enforcement is kept and completed rather than skipped.

- [x] Task 3.0: Implement the second evaluation state. Today
      `BlockchainProfile2Evaluator` replays against one state, the judged header's
      post-execution state (`crates/blockchain/focil_profile2.rs`), which is `S_end`.
      Add `S_start` — the parent post-state with `B`'s pre-execution system operations
      applied — and make an omission unjustified when the candidate is eligible at
      *either*. Both states use `B`'s `base_fee_per_gas`, `timestamp`, `gas_limit`,
      chain id and EIP-7843 `slotNumber`. `gas_fits` stays a single end-of-payload
      check: gas remaining decreases monotonically, so evaluating it at `S_start`
      would mark every full block unsatisfied and destroy EIP-7805's conditional
      inclusion. The evaluator already reads through `header.state_root` explicitly
      rather than by block number, so it is safe to run against a non-canonical
      header; `S_start` must be derived the same way and MUST NOT be the parent
      post-state alone.
- [x] Task 3.1: Derive the VERIFY budgets instead of hard-coding them. Replace
      `MAX_VERIFY_GAS_PER_TX` and `MAX_VERIFY_GAS_PER_IL`
      (`crates/blockchain/focil_eligibility.rs`, both `1 << 20`) with
      `P.gas_limit / COMMITTEE_VERIFY_GAS_FRACTION / IL_COMMITTEE_SIZE` at
      `COMMITTEE_VERIFY_GAS_FRACTION = 4`. Derive from the **parent** gas limit, never
      `B`'s: the parent is the head inclusion lists for `B`'s slot are built against,
      so includers, builders and attesters compute the same budget before `B` exists.
      At a 60,000,000 gas limit this yields 937,500 per list, within 11% of the current
      constant, so the operating point does not move but stops drifting with the gas
      limit. Keep both values equal, so one transaction may consume a list's budget.
      These are consensus inputs: they MUST NOT be read from node configuration, and
      MUST NOT reuse EIP-8141's operator-tunable `MAX_VERIFY_GAS` mempool cap.
- [x] Task 3.2: Add the per-inclusion-list code-byte budget, which does not exist yet.
      Count each distinct `codeHash` once and each distinct code body's byte length
      once, charged on first load by any replay of that list and shared across both
      evaluation states. Bound at `MAX_VALIDATION_CODE_BODIES = 16` bodies and
      `16 * MAX_CODE_SIZE` bytes. Per-list rather than per-transaction accounting is
      what makes the bound usable: the motivating shape is many transactions validating
      against one shared verifier contract, whose bytes an attester loads once. EVM gas
      alone does not bound bytes loaded — at a few thousand gas per cold account, a
      list's gas budget admits hundreds of cold accesses each able to pull a
      maximum-size code body.
- [x] Task 3.3: Keep `AA_VOPS_SLOT_COUNT` at 4 and pin it. It is the top of EIP-8369's
      candidate range and the worst case for replay, so a benchmark that fits the
      attestation deadline at 4 fits at 2 and 3, and no transaction eligible at a lower
      value becomes unreachable. `aa_vops_slot_count()` already defaults to 4
      (`crates/common/types/genesis.rs:35`); make the default explicit in the published
      genesis rather than implicit, so a joining client cannot infer a different value
      from an absent field.

- [x] Task 3.1: Add `test/tests/levm/hegota_active_surface_tests.rs` with a test that
      builds a `ChainConfig` with `hegota_time` set and `utxo_frames_time`,
      `payer_txparam_time`, `derived_slot_time` and `aa_vops_slot_count` all `None`,
      and asserts: a frame with `mode == 5` is rejected by
      `FrameTransaction::validate_static_constraints`; `TXPARAM(0x11)` halts with
      `InvalidOpcode`; `SLOTNUM` still resolves from `env.slot_number` supplied by the
      header rather than derived. Register the module in `test/tests/levm/mod.rs`.
- [x] Task 3.2: Add to `test/tests/blockchain/focil_tests.rs` a test over the entry
      point the consensus path actually uses. `InclusionListSatisfactionValidator::check`
      is the Profile-1-only wrapper and proves nothing about the live behaviour, because
      both real callers use `check_with_profile_2` with a `BlockchainProfile2Evaluator`.
      Assert against `check_with_profile_2`, and assert the outcome Task 3.0 chose: under
      (b), that an omitted frame transaction whose sender nonce and balance would
      otherwise satisfy every gate yields no `IlUnsatisfied`; under (a), that a Profile 2
      candidate eligible at the judged state does yield one, and that a prefix reading
      storage slot `aa_vops_slot_count()` of `sender` does not. Also assert that
      `aa_vops_slot_count()` returns `4` when the field is `None`, so the silent default
      is pinned rather than mistaken for a disabled feature.
      **Landed in `test/tests/blockchain/focil_profile2_tests.rs`, not `focil_tests.rs`.**
      The premise no longer holds: `focil_tests.rs` never calls the Profile-1-only
      `check` wrapper — every case there drives `add_block_pipeline_with_il`, which
      uses `check_with_profile_2` internally. Each named assertion is made against
      `check_with_profile_2` with a real `BlockchainProfile2Evaluator`:
      `self_verify_frame_tx_that_would_pass_replay_is_eligible` (an eligible candidate
      yields `IlUnsatisfied`), `self_verify_frame_tx_reading_outside_the_surface_is_ineligible`
      (a read at exactly `aa_vops_slot_count()` does not), and
      `aa_vops_slot_count_defaults_to_four` in the Task 3.1 module.
- [x] Task 3.3: Add `test/tests/levm/hegota_opcode_surface_tests.rs` asserting the
      exact set of opcodes registered at `Fork::Hegota` and absent before it:
      `APPROVE 0xAA`, `TXPARAM 0xB0`, `FRAMEDATALOAD 0xB1`, `FRAMEDATACOPY 0xB2`,
      `FRAMEPARAM 0xB3`, `SIGPARAM 0xB4`, `RECENTROOTREFLOAD 0xB5`, and that
      `0xB6`-`0xB9` decode as invalid at every fork.
- [x] Task 3.4: Modify `crates/common/types/genesis.rs:ChainConfig` doc comments on
      `utxo_frames_time`, `payer_txparam_time`, `derived_slot_time` and
      `aa_vops_slot_count` to state that the Hegotá testnet leaves each unset and what
      the surface does when set. No process or plan references in the comments.
- [x] Task 3.5: Update `docs/hegota-devnet.md` on this branch so the EIP-8312 and
      EIP-8369 sections say "present but inert on `hegota-testnet`: `utxoFramesTime` /
      `AA_VOPS_SLOT_COUNT` unset", and add a table listing every chain-config field
      that must remain absent from the published genesis, with the test that pins it.
- [x] Task 3.6: **Checkpoint: Verify Phase 3 complete.** Run `cargo test -p ethrex-vm
      --lib`, `cargo test --test levm`, `cargo test --test blockchain` and confirm the
      three new test modules run and pass. List each task and its status.

### Phase 4: Install the specified EIP-8272 `RECENT_ROOT_CODE`

Why this phase: it deletes the single largest quantified consensus divergence
(38 064 vs 127 196 gas) and it must land before the FOCIL seams are re-verified,
because it changes whether a frame transaction's includability depends on recent-root
state at all.

- [x] Task 4.1: Transplant `#7120`'s final commit onto `hegota-testnet` (its base is
      `hegota-devnet` and it is stacked on `#7086`, so cherry-pick the last commit
      only, not the branch). Confirm the resulting tree contains
      `RECENT_ROOT_RUNTIME_BYTECODE` in `crates/vm/system_contracts.rs` and no
      `RECENT_ROOT_WRITE_GAS` in `crates/vm/levm/src/gas_cost.rs`.
- [x] Task 4.2: Assert the bytecode byte-for-byte. In `crates/vm/system_contracts.rs`
      add a unit test that `RECENT_ROOT_RUNTIME_BYTECODE.len() == 144`, that
      `keccak256(RECENT_ROOT_RUNTIME_BYTECODE)` equals a hex literal recorded in the
      test, and that the two `PUSH32` immediates at byte offsets `0x23..0x43` and `0x5b..0x7b` equal
      `keccak256(b"RECENT_ROOT_ENTRY")` and `keccak256(b"RECENT_ROOT_STORAGE")`
      respectively. This is the check that catches a
      transcription error, and it is the same pair the read side already derives in
      `RecentRootReference::{entry_hash, storage_key}`.
- [x] Task 4.3: Pin the `push2 0x1fff` mask assumption. Add a compile-time assertion
      next to `RECENT_ROOT_RUNTIME_BYTECODE` that `RECENT_ROOT_LENGTH == 8192`, with a
      comment stating that the bytecode computes `i` as `SLOTNUM AND 0x1fff`, which
      equals `S mod RECENT_ROOT_LENGTH` only while the length is that power of two.
- [x] Task 4.4: Verify `crates/vm/backends/levm/mod.rs:install_recent_root_code`
      implements the spec's three-case activation: create with balance 0, nonce 1, code
      `RECENT_ROOT_CODE`, empty storage; adopt an existing empty-code empty-storage
      account by setting code and `nonce = max(existing, 1)` while preserving balance;
      and make the payload invalid when the address has non-empty code or storage in
      the parent state. Confirm it is called from both `prepare_block` and
      `apply_system_calls` and that it records the nonce and code change through
      `bal_recorder` so a BAL reconstructor reproduces the post-state.
- [x] Task 4.5: Add to `test/tests/levm/eip8272_tests.rs` the four cases the predeploy
      swap newly makes reachable, each asserting gas as well as effect: a plain EOA
      transaction with `to = 0x…8272` and 64 bytes of calldata writes the entry (the
      old divergence #9 was a silent no-op); a `DELEGATECALL` to the predeploy writes
      nothing and the outer frame observes the `SSTORE` failure; a call in a static
      context reverts; a write inside a reverting frame rolls back. Record the measured
      gas for the plain-EOA case in `docs/eip-8272.md` next to the figure
      ethereum/EIPs#12131 reports, and reconcile any difference before bring-up.
- [x] Task 4.6: Verify the block-access-list interaction. Add a test that a
      predeploy write records `RECENT_ROOT_ADDRESS` and the storage key as a **write**
      in the block access list under the writing transaction's `block_access_index`,
      and that a reference-carrying frame transaction still records its keys as
      **reads** (`storage_reads`, never a change) as `docs/eip-8272.md` specifies.
      Confirm `blockAccessListHash` is stable across build and re-import for a block
      containing both.
- [x] Task 4.7: Rewrite the EIP-8272 sections of `docs/hegota-devnet.md` and
      `docs/eip-8272.md`: delete divergences #4, #6, #7, #8 and #9; record
      `RECENT_ROOT_CODE` as implemented from ethereum/EIPs#12131 with the code hash
      from Task 4.2; record the two open upstream questions from that PR and the answer
      we ship (mask kept verbatim for byte-for-byte agreement, guarded by Task 4.3's
      assertion; install-at-activation with no deployment transaction, which is what
      the merged Activation section already mandates and what ethrex already does for
      `0x…8141` and `0x…8250`).
- [x] Task 4.8: **Checkpoint: Verify Phase 4 complete.** Run `cargo test -p ethrex-vm
      --lib`, `cargo test -p ethrex-common recent_root`, `cargo test --test levm
      eip8272`, `make -C tooling/ef_tests/blockchain test`. Confirm
      `RECENT_ROOT_WRITE_GAS`, `recent_root_native_write`,
      `run_top_level_recent_root_write` and `execute_recent_root_frame` are all absent
      from the tree. List each task and its status.

### Phase 5: Close the FOCIL ↔ frame-transaction seams

Why this phase: the highest-risk phase. The validator side is already correct on the
base; the builder side is not, and a builder that applies an inclusion list without
the frame-tx gates the mempool applies produces blocks its own peers reject.

- [ ] Task 5.1: Modify `crates/blockchain/payload.rs:apply_inclusion_list_transactions`
      to skip `TxType::Frame` entries, symmetric with the satisfaction check's
      exclusion. Justify in the code comment that an entry every client must excuse
      buys no censorship resistance, and that the per-sender nonce/balance walk the IL
      path relies on is the wrong domain for a keyed, payer-funded transaction.
- [ ] Task 5.2: Modify `crates/blockchain/inclusion_list_builder.rs:filter_and_cap` to
      exclude `TxType::Frame` alongside `EIP4844Transaction` and
      `PrivilegedL2Transaction`, before the `tx.nonce() != expected_nonce` walk. Add a
      test in `test/tests/blockchain/il_builder_tests.rs` that a pending frame
      transaction never appears in a built inclusion list and never displaces a
      regular transaction from the 8 KiB budget.
- [ ] Task 5.3: Modify `crates/blockchain/payload.rs:apply_inclusion_list_transactions`
      to apply the two gates `fill_transactions` applies at
      `crates/blockchain/payload.rs:753-777` and that the IL path currently omits: the
      `is_hegota_activated(payload.header.timestamp)` fork gate and the
      `frame_tx.expiry_deadline() < payload.header.timestamp` expiry gate. With Task
      5.1 these are unreachable for frame transactions, so implement them as debug
      assertions plus the fork gate applied to every IL entry, and add a test that an
      IL delivered across the Hegotá boundary cannot introduce a pre-fork-invalid
      transaction.
- [ ] Task 5.4: Verify BAL index consistency between IL-first sequencing and
      `fill_transactions`. `apply_inclusion_list_transactions` sets
      `context.vm.set_bal_index((transactions.len() + 1) as u32)` per entry; add a test
      in `test/tests/blockchain/focil_tests.rs` that a block containing IL-sequenced
      transactions followed by a reference-carrying frame transaction produces the same
      `blockAccessListHash` on build and on re-import, and that the frame transaction's
      EIP-8272 reference reads land under its own index.
- [ ] Task 5.5: Verify the engine-API surface a FOCIL consensus client actually sends,
      per the divergences already recorded in `docs/hegota-devnet.md`: that
      `engine_getInclusionListV1` tolerates the one ignored `parentHash` parameter every
      shipping FOCIL client sends, that `engine_forkchoiceUpdatedV5` accepts the
      16-byte `custodyColumns` third parameter and `null`, and that `newPayloadV5` /
      `forkchoiceUpdatedV4` are rejected from Hegotá on. Add or confirm one test per
      item in `test/tests/rpc/engine_fork_choice_tests.rs` and
      `test/tests/rpc/payload_attributes_tests.rs`.
- [ ] Task 5.6: Modify `crates/blockchain/inclusion_list_validator.rs` module docs to
      state the frame-tx exclusion as a standing rule with its reason, and add to
      `docs/hegota-devnet.md`'s EIP-7805 section the builder-side symmetry from Tasks
      5.1-5.2. Record that frame-tx IL eligibility is unspecified upstream, that
      ethereum/EIPs#12110 (VOPS profiles) is the candidate mechanism, and that ethrex
      ships full exclusion until it merges.
- [ ] Task 5.7: **Checkpoint: Verify Phase 5 complete.** Run `cargo test --test
      blockchain focil`, `cargo test --test blockchain il_builder`, `cargo test --test
      blockchain il_validator`, `cargo test --test rpc`, and `make -C
      tooling/ef_tests/engine test`. Confirm no test is `#[ignore]`d. List each task and
      its status.

### Phase 6: Close the divergence ledger against upstream

Why this phase: this is the gate on bring-up. A surviving consensus-visible divergence
becomes a chain split the moment a Nethermind node follows a block that uses it, which
is why the audit gates the bring-up and not the other way round.

All spec lookups in this phase go through the `eipmcp` MCP server — `get_eip`,
`get_spec`, `diff_eip`, `pending_prs_for_eip`, `recent_changes`, `sync_repo`. Do not
fetch EIP text from the web and do not write it from memory; library and spec text
churns and training data is stale.

- [ ] Task 6.1: `sync_repo("eips")`, then for each of `eip-8141`, `eip-8250`,
      `eip-8272`, `eip-7805` run `diff_eip(n)` against the pin recorded in
      `docs/hegota-devnet.md`'s ```pins``` block (`4a9ad32cf2`, `81b976ac01`,
      `d8636a330d`, `9a345f96c2`). Record every changed line in a new
      `docs/hegota-testnet-divergences.md` with columns: item, ethrex behaviour, spec
      behaviour, consensus-visible (yes/no), action, owner.
- [ ] Task 6.2: In `docs/hegota-testnet-divergences.md`, reclassify as **conformant**
      and delete from the divergence lists in `docs/eip-8250.md` and
      `docs/eip-8272.md` the five items upstream has adopted: EIP-8250 `TXPARAM
      nonce_keys[0] = 0x10`, EIP-8250 `NONCE_MANAGER = 0x…8250`, EIP-8272
      `RECENTROOTREFLOAD = 0xB5`, EIP-8272 `TXPARAM_RECENT_ROOT_REFERENCE_COUNT =
      0x0F`, EIP-8272 `RECENT_ROOT_ADDRESS = 0x…8272`. Confirm each against `get_eip`
      before deleting the entry, and bump the ```pins``` block to the SHAs `diff_eip`
      reports as head.
- [ ] Task 6.3: For each of the nine open upstream PRs, use `pending_prs_for_eip` to
      confirm it is still open, then add one row to
      `docs/hegota-testnet-divergences.md` giving the upstream PR, the ethrex PR that
      implements it (or "none"), merged/unmerged, and what we ship in the meantime:
      EIPs#12066 `SLOTNUM` ban → ethrex `#7108`; EIPs#12041 canonical paymaster
      bytecode → the `FRAME_CANONICAL_PAYMASTER_CODE_HASH` sentinel at
      `crates/blockchain/mempool.rs:63` and its `H256::zero()` branch at
      `crates/blockchain/blockchain.rs:3825`; EIPs#12039 keyed mempool concurrency →
      `keyed_concurrency_verdict`; EIPs#12109 atomic-batch approval scope →
      `docs/eip-8250.md` divergence #4; EIPs#12091 block inclusion gating and payer
      solvency; EIPs#12113 initial `accessed_addresses` set; EIPs#12026 floor
      repricing, signature validation and `frame.value` gas; EIPs#12061 frame receipt
      has no transaction-level status; EIPs#12110 VOPS profiles for FOCIL eligibility.
- [ ] Task 6.4: Resolve EIPs#12041 concretely. Replace
      `crates/blockchain/mempool.rs:63`'s `FRAME_CANONICAL_PAYMASTER_CODE_HASH =
      H256::zero()` sentinel with the code hash the PR pins, delete the
      `== H256::zero()` fallback branch at `crates/blockchain/blockchain.rs:3825`, and
      add a test that a `pay` frame whose target's runtime code hash matches is exempt
      from the generic validation-trace rules while a non-matching one stays capped by
      `FRAME_TX_MAX_PENDING_NONCANONICAL_PAYMASTER`. Confirm the bytecode and hash via
      `eipmcp` (`get_eip(8141)` and the assets file the PR adds) rather than from this
      document. Independently verified while planning: the PR's 355-byte runtime hashes
      to `0xda42f0d11838c4c0c3129b8b8e93e9718127ad6b315e517e1088125707c4d45c`, which is
      the value the PR states — use that as a cross-check, not as the source.
- [ ] Task 6.5: Resolve EIPs#12026's BAL clause and EIPs#12113's warm-set clause, both
      of which are consensus-visible through `blockAccessListHash`. Confirm that
      validating a frame transaction's `signatures` records no EIP-7951 precompile
      access in the block access list, and that frame-transaction processing starts
      with `accessed_addresses` = {`tx.sender`, coinbase, precompiles} and
      `accessed_storage_keys` empty, that a `resolved_target` is not warmed by being a
      frame target, that the payer is added when a payment-scope `APPROVE` collects
      `max_cost`, and that `ENTRY_POINT` is not pre-warmed. One test per clause in
      `test/tests/levm/eip8141_tests.rs`.
- [ ] Task 6.6: Resolve the two EIP-8272 items the merged text now pins that ethrex may
      implement differently. First: the merged Reference-validity section says a valid
      reference adds the address and storage key to the accessed sets and "affects
      warm/cold gas accounting only", while ethrex performs real storage reads and
      records each key as a BAL `storage_reads` entry (`docs/eip-8272.md`, Access
      warming and BAL). Decide whether the BAL read record is required or forbidden,
      confirm via `get_eip(7928)` and `get_eip(8272)`, and either keep it with a
      justification row or drop it. Second: confirm EIP-8250's clause that keyed-nonce
      reads and writes are protocol bookkeeping which must **not** enter
      `accessed_addresses` / `accessed_storage_keys`, must not be priced as `SSTORE`,
      and must not warm anything, and add a test asserting each of the three.
- [ ] Task 6.7: For every row in `docs/hegota-testnet-divergences.md` whose
      consensus-visible column reads "yes" and whose action is not "closed", either
      close it or move it verbatim into the Open Questions section of this document
      with the fallback we ship. Zero rows may end the phase as "yes / unresolved /
      no fallback".
- [ ] Task 6.8: **Checkpoint: Verify Phase 6 complete.** Confirm every row of
      `docs/hegota-testnet-divergences.md` has a non-empty action, that no
      consensus-visible row is unresolved without a stated fallback, that the ```pins```
      block in `docs/hegota-devnet.md` matches what `diff_eip` reports as head for
      EIP-8141, EIP-8250, EIP-8272, EIP-7805 and EIP-8369, and that `cargo test
      --workspace --exclude 'ethrex-l2*' --exclude
      ethrex-prover --exclude ethrex-guest-program` is green. List each task and its
      status.

### Phase 7: Permissioned, reachable genesis

Why this phase: the genesis must be right the first time. Chain ID, deposit gating and
the fork schedule are all baked into the genesis hash, and getting one wrong means a
re-genesis, which is exactly what we promised a third party we would not do.

- [x] Task 7.1: Create `fixtures/networks/hegota-testnet.yaml` from
      `fixtures/networks/hegota-devnet.yaml` with: `network_params.network_id: "8141"`;
      `seconds_per_slot: 6`; `fulu_fork_epoch: 0`, `gloas_fork_epoch: 1`,
      `heze_fork_epoch: 2`; three ethrex participants at `el_image: ethrex:local` with
      `el_extra_params` carrying `--syncmode full`, `--mempool.max-verify-gas 500000`,
      `--http.api eth,net,web3` and `--nat.extip <PUBLIC_IP>`; the FOCIL-aware
      consensus image from Task 7.2; `validator_count: 32` each; `supernode: true`;
      `dora_params.image: ghcr.io/lambdaclass/dora:heze-decode`; and the
      `additional_preloaded_contracts` block for the two EIP-8282 predeploys with slot
      0 set to `EXCESS_INHIBITOR`. Note in a comment that `--nat.extip` (not
      `--p2p.addr`, which is the *bind* address, `cmd/ethrex/cli.rs:376-393`) is the
      advertised address, and that it must be passed explicitly because the
      ethereum-package ethrex launcher is the only EL launcher that never emits an
      external-IP flag (verified: `src/el/ethrex/ethrex_launcher.star` has no
      `el_nat_exit_ip` reference, unlike geth, reth, besu, nethermind, erigon,
      nimbus-eth1 and ethereumjs).
- [x] Task 7.2: Pin the consensus client. Per `docs/hegota-devnet.md`, the client to
      move to is `sigp/lighthouse@focil` — a strict superset of the `unstable` base
      `ethpandaops/lighthouse:glamsterdam-devnet-7` builds from, with `ForkName` through
      `Heze`, `slot_number` and `target_gas_limit` on `JsonPayloadAttributesV4`/`V5`,
      plus `getInclusionListV1`, `forkchoiceUpdatedV5` and `newPayloadV6`. Build and
      publish that image, set it as `cl_image` on all three participants, and record
      the exact commit in a comment. Do **not** use
      `ethpandaops/lighthouse:glamsterdam-devnet-7` (no `Heze` ForkName, so it cannot
      drive FOCIL) and do not use a stock Lighthouse release (`unstable` rejects a Heze
      payload with `UnsupportedForkVariant`).
- [x] Task 7.3: Bump `Makefile:73`'s `ETHEREUM_PACKAGE_REVISION` to a revision whose
      `DEFAULT_ETHEREUM_GENESIS_GENERATOR_IMAGE` is `>= 6.1.4`, or override it in
      `fixtures/networks/hegota-testnet.yaml` with
      `ethereum_genesis_generator_params.image:
      ethpandaops/ethereum-genesis-generator:6.1.6`. The pinned revision
      `d47e98799c84a71d94371472e05f5e93030b3a7b` defaults to `6.0.7`
      (`ethereum-package/src/package_io/constants.star:111-113`) while
      `docs/hegota-devnet-genesis.md` requires 6.1.4+ for the EIP-8282 predeploys.
      Verified at 6.1.6: `apps/el-gen/system-contracts.yaml` still ships
      `eip8282_deposit` with the comment "No storage (no excess inhibitor)" while
      `eip8282_exit` has storage, so the `additional_preloaded_contracts` preload in
      Task 7.1 is still required and must not be dropped.
- [x] Task 7.4: Add the gated deposit contract to
      `fixtures/networks/hegota-testnet.yaml` via
      `ethereum_genesis_generator_params.extra_env`, with **every value a string**:
      `DEPOSIT_CONTRACT_GATED: "true"`, `DEPOSIT_CONTRACT_ADMINS: '["<ADMIN_ADDR>"]'`,
      `DEPOSIT_CONTRACT_SETTINGS: '{"0x00":"0x01","0x01":"0x00","0x02":"0x00","0x03":"0x00","0xffff":"0x02"}'`.
      The string requirement is load-bearing: the package JSON-encodes each value
      before templating it (`el_cl_genesis_generator.star:137-139` →
      `static_files/genesis-generation-config/el-cl/values.env.tmpl`'s
      `export {{ $key }}={{ $value }}`), so a Starlark *list* renders as
      `export X=["0x…"]` and bash strips the inner quotes, leaving invalid JSON for the
      generator's `jq -r '.[]'`. Also add `<ADMIN_ADDR>` to
      `network_params.prefunded_accounts` so it can pay for mint transactions.
- [x] Task 7.5: **Do not set `network_params.deposit_contract_address`.** Leave it at
      the package default `0x00000000219ab540356cBB839Cbe05303d7705Fa`
      (`ethereum-package/network_params.yaml:81`). The gater template hard-codes the
      `DEPOSIT_CONTRACT_ROLE` grant for exactly that address — storage key
      `0xc0de0000000000000000000000000000219ab540356cbb839cbe05303d7705fa = 1` in
      `apps/el-gen/gated-deposit-contract.yaml` — and `TokenDepositGater.check_deposit`
      opens with `require(hasRole(DEPOSIT_CONTRACT_ROLE, _msgSender()))`, so any other
      deposit address makes **every** deposit revert with "Only deposit contract can
      call this function". Add the address, and this reason, as a comment in the config.
- [x] Task 7.6: Document the deposit-gating policy in a new
      `docs/hegota-testnet-permissioning.md`: the gater lives at
      `0x00000000a11acc355c0de0000a11acc355c0de00` (the template's
      `deposit_gater_address`, also written into the deposit contract's storage slot
      `0x41`); it is an ERC-20 named "Deposit Token"/"Deposit" with `decimals() == 0`,
      burning one token per non-top-up deposit; the gate value bits are `0x01 = blocked`
      and `0x02 = noToken` (`contracts/TokenDepositGater.sol:11-13`); a deposit is
      classified as a top-up (`0xffff`) when the signature is 96 zero bytes **and** the
      withdrawal credentials are 32 zero bytes, otherwise by the first credential byte;
      the token is checked and burned against the **caller of `deposit()`**, not the
      validator. Record the chosen policy and its reasons: `0x00` (BLS) **blocked**
      because BLS credentials cannot withdraw to an execution address and only create
      validators nobody can exit cleanly; `0x01` (execution), `0x02` (compounding) and
      `0x03` (builder, needed because the chain runs Gloas) **allowed, token required**;
      `0xffff` (top-up) **allowed with no token**, because a top-up is not a new
      validator so gating it adds no permissioning value and would break EIP-7251
      consolidation. Record that the genesis-injected admin is granted with value `2`,
      which `SimpleAccessControl.isStickyRole` treats as non-revocable.
- [x] Task 7.7: Write the token-minting runbook into
      `docs/hegota-testnet-permissioning.md`, using `pk910/gated-deposit-contract-cli`:
      `docker run --rm -it pk910/gated-deposit-contract-cli -k $ADMIN_KEY -r $RPC
      status` to confirm the gater address, token supply, admin stickiness and the
      five per-prefix configurations; `… mint --to <THEIR_DEPOSITOR_EOA> --amount <N>`
      to hand a third party N validator slots; `… setConfig --prefix 0x01 --blocked
      true` as the kill switch; `… grantAdmin` / `revokeAdmin` for delegation. State
      that the recipient address is whoever will *send* the deposit transaction.
- [ ] Task 7.8: **Checkpoint: Verify Phase 7 complete.** Launch the enclave with
      `make localnet ENCLAVE=hegota-testnet
      KURTOSIS_CONFIG_FILE=fixtures/networks/hegota-testnet.yaml`, then: dump the
      rendered `values.env` from the genesis-generator service and confirm
      `DEPOSIT_CONTRACT_GATED`, `DEPOSIT_CONTRACT_ADMINS` and
      `DEPOSIT_CONTRACT_SETTINGS` survived the shell round-trip as valid JSON; confirm
      the generator log lines "adding admin 0x…" and "adding prefix 0x… settings";
      `eth_getCode` on both the deposit address and
      `0x00000000a11acc355c0de0000a11acc355c0de00` returns non-empty;
      `eth_chainId == 0x1fcd`; the `gating-cli status` output matches the Task 7.6
      policy. List each task and its status.

### Phase 8: Reachability, publication and interop bring-up

Why this phase: an enclave that only its own host can reach is not a testnet a third
party can join, and the seven-point verification pass in
`docs/hegota-devnet-genesis.md` predates FOCIL and the gated deposit contract.

- [ ] Task 8.1: Set the reachability block in
      `fixtures/networks/hegota-testnet.yaml`: `port_publisher.nat_exit_ip:
      "<PUBLIC_IP>"` (the literal address, not `"auto"` — `auto` resolves at run time
      via `get_public_ip` (called at `src/package_io/input_parser.star:356,385`,
      defined at `:2615`), so it cannot be
      templated into the `--nat.extip` of Task 7.1 and the two would drift), with
      `el: {enabled: true, public_port_start: 32000}`,
      `cl: {enabled: true, public_port_start: 31000}`,
      `additional_services: {enabled: true, public_port_start: 31500}`. A global
      `nat_exit_ip` propagates to every service group, which is what we want for `el`
      and `cl`. Every published port must stay below the host's ephemeral floor
      (`/proc/sys/net/ipv4/ip_local_port_range`, 32768 by default) or a fixed publish
      races kurtosis' own dynamic allocations and loses; on a production host the NAT
      and floor constraints compose, because every port an external peer must reach has
      to be both fixed and below the floor.
- [ ] Task 8.2: Write the port and firewall surface into
      `docs/hegota-testnet-joining.md`, derived from the package's stride of 7 ports per
      EL and per CL node (`shared_utils.MAX_PORTS_PER_EL_NODE = 7`,
      `MAX_PORTS_PER_CL_NODE = 7`, index 0 = discovery TCP+UDP, 1 = engine/HTTP,
      2 = metrics, and for EL index 3 = JSON-RPC):

      | Purpose | Ports | Proto | Exposure |
      |---|---|---|---|
      | EL discv4 + RLPx | 32000, 32007, 32014 | TCP **and** UDP | must be public for peering |
      | CL discv5 + libp2p | 31000, 31007, 31014 | TCP **and** UDP | must be public for peering |
      | EL JSON-RPC (node 0 only) | 32003 | TCP | public for convenience, via reverse proxy |
      | Dora explorer | 31500 | TCP | public for convenience, via reverse proxy |
      | EL engine authrpc | 32001, 32008, 32015 | TCP | must stay closed |
      | EL metrics | 32002, 32009, 32016 | TCP | must stay closed |
      | EL JSON-RPC (nodes 1, 2) | 32010, 32017 | TCP | must stay closed |
      | CL beacon REST | 31001, 31008, 31015 | TCP | must stay closed |
      | CL metrics | 31002, 31009, 31016 | TCP | must stay closed |

      State that the RPC and explorer go through the reverse proxy rather than being
      opened directly, following `scripts/eip8141-devnet/Caddyfile`, and that the
      engine ports carry the JWT-authenticated payload API and must never be reachable.
- [ ] Task 8.3: Create `scripts/hegota-testnet/publish-artifacts.sh` that extracts the
      joiner bundle from a running enclave and writes it to a directory the reverse
      proxy serves: every file under `/network-configs/` (which the generator populates
      from `/data/metadata/*` — `genesis.json`, `chainspec.json`, `besu.json`,
      `config.yaml`, `genesis.ssz`, `deposit_contract_block_hash.txt`,
      `genesis_validators_root.txt`), plus a generated `bootnodes.txt` of the three EL
      `enode://` URLs and a `bootnodes-cl.txt` of the three beacon ENRs. Source the
      enodes from each EL's `admin_nodeInfo` and the ENRs from each beacon node's
      `/eth/v1/node/identity`, and rewrite any container-internal host to `<PUBLIC_IP>`
      before writing them out. Emit a `MANIFEST.txt` with the sha256 of every file.
- [ ] Task 8.4: Patch the published chainspec for the two EIPs the generator does not
      know about. `apps/el-gen/generate_genesis.sh:922-949` at 6.1.6 emits only
      `eip7805TransitionTimestamp` and `eip8141TransitionTimestamp` for heze, so
      `publish-artifacts.sh` must add `eip8250TransitionTimestamp` and
      `eip8272TransitionTimestamp` at the same `bogotaTime` value, and the script must
      fail loudly if `chainspec.json` already carries either key (meaning the generator
      caught up and the patch is stale). Record in `docs/hegota-testnet-joining.md` that
      the authoritative key names must be confirmed with Nethermind, and that ethrex
      itself reads `bogotaTime` from `genesis.json` through the `hezeTime` / `bogotaTime`
      serde aliases on `ChainConfig::hegota_time` (`crates/common/types/genesis.rs:272-284`).
- [ ] Task 8.5: Write the joining instructions into `docs/hegota-testnet-joining.md`:
      the exact artifact list and where to fetch it; chain ID `8141`; the four fork
      timestamps a joiner must agree on (`osaka`/`fulu` at genesis, `amsterdamTime` /
      Gloas at epoch 1, `hegotaTime` / `bogotaTime` / heze at epoch 2, and that all
      four in-scope EIPs activate at that one timestamp); the deposit contract and gater
      addresses; how to start an EL (`--network genesis.json --bootnodes <enodes>
      --nat.extip <their ip>`, `cmd/ethrex/cli.rs:82` for `--bootnodes`) and a CL
      (`--boot-nodes <ENRs>`); and — stated prominently — that **joining does not
      require a validator**: the gated deposit contract permissions validator entry
      only, so anyone can sync, peer and submit transactions. That asymmetry is the
      Sepolia model and it is what makes the network simultaneously open and
      permissioned. The token-minting runbook in
      `docs/hegota-testnet-permissioning.md` is the separate step that turns a joined
      node into a validator.
- [ ] Task 8.6: Extend the verification pass in `docs/hegota-devnet-genesis.md` from
      seven checks to twelve, keeping the existing seven and adding: (8) every EL
      advertises `<PUBLIC_IP>` in its enode and an external `ethrex --bootnodes` node
      on a different host completes discovery and reaches the head; (9)
      `engine_getInclusionListV1` returns a non-empty list on a slot with a full
      mempool, and an inclusion list delivered on `engine_newPayloadV6` is honoured by
      the builder while an omitted frame transaction is excused rather than yielding
      `INCLUSION_LIST_UNSATISFIED`; (10) `eth_getCode` on `0x…8272` returns exactly the
      144-byte `RECENT_ROOT_CODE`, and a plain EOA transaction to it with 64 bytes of
      calldata writes an entry that a subsequent frame transaction successfully
      references; (11) an ungated deposit attempt from an address holding no token
      reverts with "Not enough tokens", and the same deposit succeeds after
      `gating-cli mint`; (12) the deposit produces a standard `DepositEvent` that
      appears in the block's `requestsHash` and the consensus client activates the
      validator, with `eth_getLogs` on the deposit address showing `DEPOSIT_TOPIC`
      (`0x649bbc62…38c5`) and no additional event.
- [ ] Task 8.7: Run the full twelve-check pass on the production host and record the
      observed values — genesis hash, chain ID, the three fork timestamps, the three
      enodes, the three ENRs, the deposit and gater addresses, and the measured
      recent-root write gas — in a "Deployed configuration" section of
      `docs/hegota-testnet.md`. Anything that does not match the plan is a blocker, not
      a note.
- [ ] Task 8.8: Run one real deposit-and-activate cycle end to end and record it in
      `docs/hegota-testnet-permissioning.md`: mint one token to a depositor EOA,
      generate keys and a deposit datum with execution (`0x01`) withdrawal credentials,
      send `deposit()`, observe the token burn, observe the `DepositEvent` and the
      non-empty `requestsHash`, and observe the validator activating and proposing.
      Then repeat with a `0x00` BLS credential and confirm it reverts with "Deposit type
      is blocked".
- [ ] Task 8.9: **Checkpoint: Verify Phase 8 complete.** Confirm all twelve checks pass
      on the production host, that `publish-artifacts.sh` output is fetchable from
      outside the host and its `MANIFEST.txt` sha256s match, that an external ethrex
      node joins from a different host using only the published bundle, and that Task
      8.8's two deposit outcomes are both recorded. List each task and its status.

### Phase 9: Install the testnet on its own server

Why this phase: the testnet lands on its own host, with its own keys and its own faucet.
The target server is not yet chosen, so nothing here may depend on a particular machine.
The existing Hegotá devnet is the reference implementation of the surrounding
infrastructure, and the reusable patterns below were read off it; the runbook is written
against a fresh host.

The config is validated locally under kurtosis before it reaches any server. That is the
gate for this phase: a launch that works on a laptop proves the genesis, the gated deposit
contract and the fork schedule, and leaves only reachability and DNS to prove on the host.

**Observed on a running Hegotá deployment** (read-only), and reusable:

- **The fork schedule that works is `fulu 0 / gloas 1 / heze 2`**, confirmed from the
  consensus client's own `/eth/v1/config/spec`. Gloas must be scheduled and must not be
  at genesis. An earlier reading of a stale config file suggested the generator derives
  the Amsterdam-era times from `heze` alone, making an explicit Gloas unnecessary; that
  was wrong, and the file it came from was six weeks older than the running enclave.
  Treat any on-host config file as evidence only after checking it against the live
  chain.
- **`ethpandaops/lighthouse:focil` is a published, FOCIL-capable image**, reporting
  `Lighthouse/v8.1.3-52e5197`, with the FOCIL preset in its spec endpoint
  (`INCLUSION_LIST_COMMITTEE_SIZE`, `MAX_BYTES_PER_INCLUSION_LIST`,
  `DOMAIN_INCLUSION_LIST_COMMITTEE`, `INCLUSION_LIST_DUE_BPS`,
  `MAX_REQUEST_INCLUSION_LIST`) alongside `HEZE_FORK_EPOCH`. No branch build is needed.
- `~/hegota-Caddyfile` fronts every service as a host-based vhost with automatic
  Let's Encrypt certificates: three RPC subdomains, the explorer, the faucet. Two
  hard-won details to carry over. **Never** add `header Access-Control-*` directives:
  ethrex's RPC server already emits a complete permissive CORS set, Caddy's `header`
  appends rather than replaces, and a duplicated `Access-Control-Allow-Origin` is
  hard-rejected by browsers and by MetaMask's request layer. And the explorer is proxied
  through a **stable** local port maintained by a small socat unit, because kurtosis
  reassigns the explorer's real port on every re-genesis; only that unit changes, never
  the Caddy config.
- The no-regenesis binary-swap runbook: `/usr/local/bin/ethrex` is a small wrapper that
  execs `ethrex-real "$@" --http.api=ethrex`, and upgrades replace only `ethrex-real`
  inside the running container. Consequence worth carrying: **the image tag does not
  identify the running binary**, so always read `--version` from the binary. The upgrade
  scripts record `eth_blockNumber` and the block-1000 hash before and after the restart,
  so an accidental regenesis or DB rewrite is caught immediately. Reuse that check for
  every manual intervention.

- [ ] Task 9.1: Replace the validator keys. `preregistered_validator_keys_mnemonic`
      defaults to the ethereum-package mnemonic beginning "giant issue aisle success",
      which is public and identical on every kurtosis devnet. Generate our own and set it
      explicitly. This is not cosmetic on a permissioned chain: the accounts derived from
      the default mnemonic are prefunded and spendable by anyone who has ever run the
      package.
- [ ] Task 9.2: Set `prefunded_accounts` explicitly for the accounts we control — the
      faucet, the deposit-gater admin, and a deployer — rather than relying on whatever
      the mnemonic funds. **The gater admin MUST NOT be a default-mnemonic account**: it
      can mint deposit tokens, so a public key there makes the permissioning worthless
      while appearing to work. Cross-check against `DEPOSIT_CONTRACT_ADMINS` from Phase 7,
      which is what grants that address its gater role in genesis.
- [ ] Task 9.3: Set `network_id` to the chain ID fixed in Phase 7 (`8141`), replacing the
      package default `3151908`.
- [x] Task 9.4: Pin the consensus client to `ethpandaops/lighthouse:focil`, verified
      FOCIL-capable from its own spec endpoint rather than from its tag name. A client
      without FOCIL cannot drive this chain at all: ethrex rejects `newPayloadV5` and
      `forkchoiceUpdatedV4` once Hegotá is active, so only the V6/V5 pair works, and
      there is no inert intermediate state in which a non-FOCIL client merely degrades.
- [ ] Task 9.9: Validate the config locally under kurtosis before touching a server:
      `make localnet ENCLAVE=hegota-testnet
      KURTOSIS_CONFIG_FILE=fixtures/networks/hegota-testnet.yaml`, with the REPLACE
      placeholders filled by throwaway local values. Confirm the enclave reaches the
      Hegotá boundary and keeps producing, that the gated deposit contract and its gater
      are both present with the expected storage, and that a token-holding address can
      deposit while a token-less one cannot. `--nat.extip` and DNS are the only items
      this cannot prove.
- [ ] Task 9.5: Move `port_publisher.cl` below the host's ephemeral floor. The devnet
      publishes it at `33000`, inside the default `32768`-and-up range, where a fixed
      publish can lose the race against a dynamically assigned port. It has not bitten
      there; do not carry the risk to a host that must also stay externally reachable.
- [ ] Task 9.6: Stand up our own faucet. The devnet's is a container on a local port
      behind the Caddy vhost, configured by an env file holding an RPC URL, a funding
      private key, a per-request amount, and the public RPC and explorer URLs. Build the
      testnet's own with its own key, funded from a Task 9.2 account. The key is a
      secret: it belongs in the host's env file, never in this repository or in a
      kurtosis config.
- [ ] Task 9.7: Write `scripts/hegota-testnet/INSTALL.md` covering, in order: host
      prerequisites, the docker image build and tag, `make checkout-ethereum-package`,
      the kurtosis launch with our config, extracting the artifact set from
      `/network-configs`, the Caddy vhosts and the explorer port-forward unit, the faucet
      deploy, the firewall rules from Phase 8, and the deposit-token minting runbook.
      Include the verification pass and the no-regenesis upgrade path. Written so someone
      who has never seen the devnet host can follow it.
- [ ] Task 9.8: **Checkpoint: Verify Phase 9 complete.** On the target host: the three
      ELs agree on head number and hash; every published artifact is fetchable over
      HTTPS; the faucet funds a fresh address; a deposit from a token-holding address
      succeeds and one from a token-less address reverts; no account derived from the
      default mnemonic holds a balance or the gater admin role. List each task and its
      status.

- [ ] **Final Audit**. Re-read the entire plan. For each task, verify the
      implementation exists in the codebase: the deletions of Phase 2 by grep, the
      tests of Phases 3-6 by name, the config of Phase 7 by file, the artifacts and
      recorded observations of Phase 8 by file, and the install runbook and host state of
      Phase 9. List any gaps. All gaps must be resolved before reporting completion.

## PR reconciliation snapshot

Verified via `gh` while planning. **Refresh every row in Task 1.2** — states move.
"In base" means the commit is already reachable from `origin/hegota-devnet`.

| PR | Head | Base | Review | Mergeable | In scope | Action |
|---|---|---|---|---|---|---|
| #7120 | `daniil/eip8272-recent-root-code` | `hegota-devnet` | — | MERGEABLE | yes | transplant final commit (Task 4.1) |
| #7086 | `daniil/eip8272-native-write-paths` | `hegota-devnet` | — | MERGEABLE | no | close as superseded by #7120 |
| #7084 | `daniil/eip8272-price-empty-list` | `hegota-devnet` | APPROVED | MERGEABLE | yes | check "in base"; else transplant |
| #7085 | `daniil/eip8250-first-use-default-code` | `hegota-devnet` | — | MERGEABLE | yes | check "in base"; else transplant |
| #7121 | `daniil/frame-tx-call-tracer` | `hegota-devnet` | — | MERGEABLE | yes | check "in base"; else transplant |
| #7089 | `fix/sigparam-copy-operand-order` | `main` | APPROVED | MERGEABLE | yes | in base as `c45a95df9`; merge to `main` too |
| #7048 | `daniil/eip8141-skipped-frame-status` | `main` | APPROVED | MERGEABLE | yes | check "in base"; else merge to `main` first |
| #7059 | `daniil/eip8141-intrinsic-gas-tests` | `main` | APPROVED | MERGEABLE | yes | in base as `45aa059e2` |
| #7108 | `fix/eip8141-ban-slotnum-prefix` | `main` | REVIEW_REQUIRED | MERGEABLE (draft) | yes | land; decision recorded in Task 6.3 |
| #7073 | `fix/frame-tx-fee-settlement` | `main` | REVIEW_REQUIRED | MERGEABLE | yes | consensus-visible fee settlement; land before bring-up |
| #7061 | `daniil/eip8141-verify-target-approved` | `main` | REVIEW_REQUIRED | MERGEABLE | yes | in base as `a4e49a350`; confirm |
| #7082 | `daniil/frame-receipt-storage-codec` | `main` | REVIEW_REQUIRED | MERGEABLE | yes | resolve against #7058 (Task 1.5) |
| #7058 | `fix/frame-receipt-persistence` | `main` | REVIEW_REQUIRED | CONFLICTING | yes | resolve against #7082 (Task 1.5) |
| #7081 | `daniil/eip8141-frame-receipt-to` | `main` | REVIEW_REQUIRED | MERGEABLE (draft) | yes | receipt `to` is RPC-visible; land |
| #7075 | `fix/frame-tx-mempool-rules` | `main` | REVIEW_REQUIRED | CONFLICTING | yes | in base as `62431013e`; confirm, then close |
| #7052 | `frame-tx-max-verify-gas-config` | `main` | REVIEW_REQUIRED | CONFLICTING | yes | in base as `4e842d940`; confirm, then close |
| #7038 | `feat/eip-8141-blob-frame-txs` | `main` | REVIEW_REQUIRED | CONFLICTING | yes | in base as `d5192b82c`; confirm, then close |
| #7039 | `focil` | `main` | REVIEW_REQUIRED | CONFLICTING (draft) | yes | in base via `d57568844`; confirm, then close |
| #6906 | `eip-8250` | `main` | REVIEW_REQUIRED | CONFLICTING (draft) | yes | in base; keep open only for upstreaming |
| #6907 | `eip-8272` | `main` | REVIEW_REQUIRED | CONFLICTING | yes | in base; keep open only for upstreaming |
| #7047 | `daniil/eip8141-receipt-rpc-decode` | `frames-devnet-0` | — | MERGEABLE | yes | needs full re-target to `main` |
| #6974 | `ethrex-simulate-frame-tx-rpc` | `main` | REVIEW_REQUIRED | MERGEABLE | after bring-up | in base as `321454542`; not a gate |
| #7091 | `fix/l1-rlpx-inbound-decode-drop` | `main` | REVIEW_REQUIRED | MERGEABLE | after bring-up | not interop-critical |
| #6891 | `eip-7906` | `main` | REVIEW_REQUIRED | MERGEABLE | **no** | EIP-7906 deleted from this branch |
| #6730 | `eip-8025-hegota-fork` | `main` | REVIEW_REQUIRED | CONFLICTING (draft) | **no** | EIP-8025 out of scope |
| #6625 | `feat/mempool-punish-spammer` | `main` | REVIEW_REQUIRED | CONFLICTING | **no** | unrelated |
| #6325 | `fix/eth-call-nonce-validation` | `main` | CHANGES_REQUESTED | CONFLICTING | **no** | unrelated |

## Edge Cases & Risks

- **A surviving consensus-visible divergence splits the chain when Nethermind joins.**
  Addressed by Phase 6, which gates bring-up: Task 6.7 forbids any consensus-visible
  row from ending unresolved without a fallback.
- **`SIGPARAM 0x04` operand order.** `main` still pops the pre-`4a9ad32cf` order at
  `crates/vm/levm/src/opcode_handlers/frame_tx.rs:477`. Addressed by Task 2.6, which
  asserts the merge keeps the branch side.
- **EIP-8272 write pricing.** 38 064 vs 127 196 gas. Addressed by Task 4.1 and measured
  in Task 4.5.
- **The recent-root bytecode is transcribed wrong.** A single wrong byte is a silent
  state-root divergence. Addressed by Task 4.2's length, code-hash and per-`PUSH32`
  domain assertions.
- **`RECENT_ROOT_LENGTH` changes and the `0x1fff` mask silently stops meaning `mod`.**
  Addressed by Task 4.3's compile-time assertion.
- **A frame transaction in an inclusion list makes attesters reject a valid block.**
  Addressed by the existing exclusion plus Tasks 5.1-5.3, and pinned by Task 3.2.
- **EIP-8312 or EIP-8369 accidentally activate on the published chain.** Addressed by
  Tasks 3.1-3.3, which test inertness rather than asserting it, and Task 3.5, which
  lists every field that must be absent.
- **The `extra_env` values arrive at the generator as invalid JSON** because the package
  JSON-encodes them and bash then strips the inner quotes. Addressed by Task 7.4's
  string requirement and Task 7.8's `values.env` dump.
- **A custom `deposit_contract_address` makes every deposit revert** against the
  gater's hard-coded role grant. Addressed by Task 7.5.
- **Blocking `0x00` locks out builder deposits too.** `0x03` is a distinct prefix, so
  it does not; Task 7.6 allows `0x03` explicitly and Task 8.8 exercises `0x00`
  rejection separately.
- **Top-ups without a token let an existing validator grow without permission.** Accepted
  deliberately: it does not add validators, and gating it breaks EIP-7251 consolidation.
  Recorded as a policy decision in Task 7.6, revocable with `gating-cli setConfig
  --prefix 0xffff --no-token false`.
- **The sticky genesis admin cannot be revoked.** The generator writes value `2`, which
  `SimpleAccessControl.isStickyRole` treats as permanent. Addressed by Task 7.6
  recording it, so the key is treated as a long-lived secret from day one.
- **A published port inside the ephemeral range loses a bind race to kurtosis.**
  Addressed by Task 8.1's port choices, all below 32768.
- **`nat_exit_ip: "auto"` and `--nat.extip` drift apart.** Addressed by Task 8.1
  requiring the literal address in both.
- **The engine authrpc port is exposed while opening the peering ports.** Addressed by
  Task 8.2's explicit must-stay-closed column and verified in Task 8.9.
- **Nethermind cannot enable EIP-8250/EIP-8272 from the generated chainspec.**
  Addressed by Task 8.4's patch and its stale-patch guard.
- **The consensus client is not FOCIL-aware, and swapping it is not reversible.**
  `docs/hegota-devnet.md` records that execution and consensus upgrade atomically with
  no inert intermediate state. Addressed by Task 7.2 pinning `sigp/lighthouse@focil`
  before the first genesis, so the swap never has to happen on a live chain.
- **`sigp/lighthouse@focil` cannot boot with Gloas at epoch 1 on generator 6.1.6.**
  `fixtures/networks/focil.yaml` runs `gloas_fork_epoch: 0` with lodestar, while
  `docs/hegota-devnet-genesis.md` requires Gloas at epoch ≥1 because Lighthouse rejects
  a Gloas genesis state. Fallback if Task 7.8 hits it: schedule `gloas_fork_epoch: 0`
  and `heze_fork_epoch: 1`, and verify the beacon genesis block root matches the EL
  genesis hash before proceeding — that mismatch is the failure
  `fixtures/networks/focil.yaml`'s comments describe.
- **The EIP-8250 in-atomic-batch payment-durability gap.** `docs/eip-8250.md` divergence
  #4 rejects a payment-scoped `APPROVE` inside an atomic batch rather than implementing
  durable consumption. EIPs#12109 statically disallows approval scope on atomic-batch
  frames, which is the same outcome reached differently. Addressed by Task 6.3's row;
  the fallback we ship is the current rejection, which is a strict subset of what
  #12109 permits.
- **The EIP-8272 BAL read record may be forbidden rather than required.** Addressed by
  Task 6.6, which decides it against `get_eip(7928)` and `get_eip(8272)`.
- **`lychee` fails the docs build on a dead external link.** `lychee.toml` runs over the
  whole `docs/` directory, not just the mdBook chapters. Addressed by citing upstream
  material as bare identifiers (`ethereum/EIPs#12131`, `pk910/gated-deposit-contract`)
  rather than as URLs, so the new documents contribute no links to check.

## Acceptance Criteria

- `grep -rn -iE '7906|TXTRACE|EVENTDATACOPY|TXDIFF|POST_TX|NONCEKEYLOAD' crates/ cmd/
  test/ tooling/ fixtures/` returns nothing.
- `cargo check --workspace`, `cargo clippy --workspace -- -D warnings` and
  `cargo fmt --check` are clean on `hegota-testnet`.
- `cargo test --workspace --exclude 'ethrex-l2*' --exclude ethrex-prover --exclude
  ethrex-guest-program` is green.
- `make -C tooling/ef_tests/blockchain test` and `make -C tooling/ef_tests/engine test`
  are green, the latter including the `focil-vectors` overlay from
  `tests-focil@v0.1.0`.
- `hegota_active_surface_tests` and `hegota_opcode_surface_tests` pass, proving mode 5,
  `TXPARAM(0x11)` and opcodes `0xB6`-`0xB9` are all unavailable.
- A test asserts `RECENT_ROOT_RUNTIME_BYTECODE.len() == 144` and its keccak-256 code
  hash, and `eth_getCode` on `0x…8272` on the live chain returns those exact 144 bytes.
- `test/tests/levm/eip8272_tests.rs` covers plain-EOA write, `DELEGATECALL` no-write,
  static-context revert and revert-rollback, each asserting measured gas.
- `docs/hegota-testnet-divergences.md` has one row per audited item with a non-empty
  action, and no consensus-visible row is unresolved without a stated fallback.
- The ```pins``` block in `docs/hegota-devnet.md` matches `diff_eip` head for
  `eip-8141`, `eip-8250`, `eip-8272` and `eip-7805`.
- `eth_chainId` returns `0x1fcd` (8141) on all three nodes, and all three report the
  same genesis hash and the same `amsterdamTime` / `hegotaTime` from
  `debug_chainConfig`.
- All twelve checks of the extended verification pass in
  `docs/hegota-devnet-genesis.md` pass on the production host, with the observed values
  recorded in `docs/hegota-testnet.md`.
- An ethrex node on a host outside the enclave joins using only the published bundle,
  completes discovery against the published enodes and reaches the same head hash.
- A deposit from an address holding no token reverts with "Not enough tokens"; the same
  deposit succeeds after `gating-cli mint` and the validator activates; a `0x00` BLS
  deposit reverts with "Deposit type is blocked".
- `publish-artifacts.sh` output is fetchable from outside the host and every
  `MANIFEST.txt` sha256 matches.

## Open Questions

- **EIPs#12131 is unmerged, so `RECENT_ROOT_CODE` is still `TBD` in the merged text.**
  We ship the PR's 144 bytes verbatim. If the PR changes before merge, Task 4.2's
  code-hash assertion fails loudly and the branch must re-transplant; if it is rejected
  in favour of a different encoding, the chain needs a re-genesis, because the installed
  code is part of the post-activation state. Re-check with `pending_prs_for_eip(8272)`
  immediately before genesis and again before inviting Nethermind.
- **EIPs#12041's canonical paymaster code hash is unmerged.** Task 6.4 pins it. Until it
  merges, a hash change demotes every deployed instance to non-canonical, which is an
  admission-policy change only, not a split. Confirm with Nethermind which hash they
  pin before either side deploys a paymaster.
- **Which chainspec keys does Nethermind read for EIP-8250 and EIP-8272?** Task 8.4
  guesses `eip8250TransitionTimestamp` / `eip8272TransitionTimestamp` by analogy with
  `eip8141TransitionTimestamp`. This needs a direct answer from Nethermind before the
  bundle is published, and the answer belongs upstream in
  `apps/el-gen/generate_genesis.sh` so the generator emits them.
- **Is a FOCIL-capable Lighthouse image published, or must we build it?** Task 7.2
  assumes we build `sigp/lighthouse@focil` ourselves. If a published image exists, use
  it and pin the digest.
- **Frame-transaction inclusion-list eligibility is unspecified upstream.** We ship full
  exclusion, which can only over-accept. EIPs#12110 (VOPS profiles) is the candidate
  mechanism and EIP-8369 support already exists on the branch behind
  `AA_VOPS_SLOT_COUNT`; enabling it is a follow-up that requires Nethermind to
  implement the same profile classification, so it stays off for this network.
