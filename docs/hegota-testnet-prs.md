# Hegotá testnet — open PR reconciliation

Working document. One row per open `lambdaclass/ethrex` pull request touching the
frame-transaction stack or FOCIL, with what it means for `hegota-testnet`.

`hegota-testnet` is cut from `origin/hegota-devnet`, so most fixes written against
`main` are already in the tree. The baseline that matters is this branch, never `main`:
a PR that reads "conflicting" or "unmerged" against `main` may be long since present
here.

## How presence was determined

Commit-subject matching against `git log origin/hegota-devnet` is unreliable in both
directions and was not used on its own. Branches were composed with merge commits rather
than squashes, so a merged feature PR's subject never appears; and a cherry-picked change
that later took review fixups appears while differing from the PR's final state.

Rows marked **present** were confirmed by probing for the change in the source. Rows
marked **clean** were confirmed by actually computing the merge
(`git merge-tree --write-tree HEAD FETCH_HEAD`). Where a probe was weak, the row says so
and the change must be diffed before it is taken.

Four PRs have their head branch in a **fork**, `github.com/AnkushinDaniil/ethrex`, not in
`origin`. `git fetch origin <branch>` fails for them; fetch the fork URL directly.

## Merge candidates — head in AnkushinDaniil/ethrex, all apply cleanly

| PR | Change | Base | Merge onto this branch | Action |
| --- | --- | --- | --- | --- |
| #7120 | Install the specified EIP-8272 `RECENT_ROOT_CODE` instead of a native write | `hegota-devnet` | clean | **Take.** This is the whole of Phase 4. |
| #7084 | Price the empty EIP-8272 reference list byte | `hegota-devnet` | clean | Take. Approved upstream of us. |
| #7085 | Charge the EIP-8250 first-use surcharge on the default-code approval | `hegota-devnet` | clean | Take. |
| #7121 | Trace every frame of a frame transaction | `hegota-devnet` | clean | Take. Tooling only, no consensus surface. |
| #7086 | Perform the EIP-8272 native write on every path into the predeploy | `hegota-devnet` | clean | **Close as superseded.** #7120 is stacked on it and deletes the native write wholesale, so its additions are dead on arrival. Taking #7086 alone would leave the divergent flat-gas pricing in place. |

## Already in the tree — no action

Confirmed by source probe unless noted.

| PR | Change | Evidence |
| --- | --- | --- |
| #7089 | `SIGPARAM` copy operands aligned with `CALLDATACOPY` | `opcode_handlers/frame_tx.rs` pops `[mem_offset, data_offset, length]`, the post-`4a9ad32cf` order |
| #6974 | `ethrex_simulateFrameTransaction` RPC | `simulateFrameTransaction` present in `crates/networking/rpc/` |
| #7048 | Status 2 for skipped frames, rolled-back batch frames keep their gas | present in `crates/common/types/receipt.rs` |
| #7073 | Fee settlement on `max_gas` and the base blob rate | present in `crates/vm/levm/src/vm.rs` |
| #7082 | Frame receipts flushed with the storage codec | `encode_storage` present in `crates/common/types/receipt.rs` |
| #7047 | Stored frame receipts decode without `MalformedBoolean` | fix present; note its base is `frames-devnet-0`, older than either branch |
| #7059 | Intrinsic-gas accounting test coverage | subject match only — confirm before assuming the final version landed |
| #7061 | VERIFY frame may target an approved non-sender account | subject match only |
| #7075 | Post-prefix and replacement/eviction mempool rules | subject match only |
| #7052 | Configurable mempool verify-gas budget | subject match only |
| #7038 | Blob-carrying frame transactions propagate and build | subject match only |
| #6906 | EIP-8250 keyed nonces | merged into `hegota-devnet` as a branch merge; this PR is the `main`-ward upstreaming path, not a testnet item |
| #6907 | EIP-8272 recent roots (core) | as above |
| #7039 | EIP-7805 FOCIL inclusion lists | as above; `origin/focil` is an ancestor of `origin/hegota-devnet` |

## Absent and wanted

| PR | Change | Action |
| --- | --- | --- |
| #7108 | Ban `SLOTNUM` in the EIP-8141 validation prefix | **Take.** Adopted per the decision in `hegota-testnet.md`: EIP-8272 makes the beacon slot load-bearing, so `SLOTNUM` has exactly the property the banned-opcode list exists to exclude, and through EIP-8369 Profile 2 the ban decides an omission verdict. Base is `main`, so it needs re-targeting onto this branch. |

## Absent, out of scope for bring-up

| PR | Change | Why not now |
| --- | --- | --- |
| #7081 | Stop reporting the sender as a frame transaction's receipt `to` | RPC presentation, no consensus surface. Probe was weak; diff before taking. Take after bring-up. |
| #7091 | Drop peers on undecodable RLPx inbound frames | Networking hardening, unrelated to the EIP set. Probe was weak. Take after bring-up. |
| #6891 | EIP-7906 transaction assertions | **Excluded by scope.** EIP-7906 is not on this chain and its `Fork::Hegota`-gated surface is deleted in Phase 2. |

## Consequences for the plan

- Phase 4 is a merge, not an implementation. #7120 already carries the predeploy install,
  deletes the four native-write interception points, and removes the flat
  `RECENT_ROOT_WRITE_GAS`.
- The only conformance change that has to be written rather than merged is #7108's
  re-target, plus the EIP-8369 enforcement work in Phase 3, which no open PR covers.
- Nothing in the queue conflicts with this branch. Every merge candidate was tested, not
  assumed.
