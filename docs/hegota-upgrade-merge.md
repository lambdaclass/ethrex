# Hegotá upgrade — merging `frames-devnet-0` into the testnet branch

Working notes for the merge of `origin/frames-devnet-0` (`d587cf9ff`, EIP-8141 at
`b75cbe6115`, `tests-frames-devnet@v0.3.0`) into `hegota-testnet` (`c575e4481`), done on the
branch `hegota-upgrade`. The two lines had implemented the same EIP-8141 revision
independently since their merge base `a67b275b6` (2026-08-11): 325 commits on one side, 79 on
the other, 197 shared files, 25 in conflict (218 hunks).

## Why a merge and not a rebase

A rebase replays 325 commits over 197 shared files: every commit that touched the frame
transaction core conflicts, most of them several times, for no gain in the result. A merge is
one conflict pass, and later `frames-devnet-0` work is inherited the same way, by merging
again.

## The rule applied to every hunk

1. **EIP-8141 mechanics take `frames-devnet-0`'s side.** It is the line the spec fixtures
   are filled against, and it is what future frame-transaction work will land on. Concretely
   this branch now uses its shapes:
   - frame results are the tuple `(status, gas_used, state_gas_used, logs)`, not a struct;
   - `Frame::state_gas_limit` (was `state_limit`), `FrameTxContext::max_gas` (was
     `total_gas_limit`), `FrameTransaction::max_gas()` / `total_frame_gas()` /
     `total_frame_execution_gas()` / `value_transfer_gas()`;
   - fees are `U256` (`max_fee_per_gas`, `max_priority_fee_per_gas`), decoded field by field;
   - the state dimension inside a frame is the isolated EIP-8037 reservoir
     (`state_gas_isolated`, `outstanding_charge_owners` on the context, `credit_frame_state_gas_refill`),
     replacing the per-frame `FrameStatePool` / per-slot charge-owner / refill bookkeeping;
   - frame entry charges the target access and any 7702 delegation access up front and each
     dispatch branch reports `frame_entry_gas + gas_used`; the value-transfer NEW_ACCOUNT
     charge is taken at entry from the frame's state budget;
   - settlement: refund over execution+state, payer floored after the refund, block figure
     before it (EIP-7778), both floored against `calldata_floor_total()`;
   - EIP-4844 blob rules for frame transactions (`validate_frame_tx_blobs`), the EIP-7825 cap
     on the execution dimension, the `GASLIMIT_PRICE_PRODUCT_OVERFLOW` pre-check;
   - `ORIGIN`, `BLOBHASH`, `TLOAD`, `TSTORE` no longer banned in the validation prefix
     (spec `060351456`);
   - `SIGDATACOPY` handler, `FRAMEPARAM` 0x09–0x0B, TXPARAM 0x0C from the reservoir;
   - glamsterdam fixtures `v8.1.4`, engine runner accepts "Heze", frames fixture overlay.
2. **Hegotá's additions are kept and adapted to those shapes:**
   - EIP-8250: `nonce_keys` / `nonce_seq` envelope, `consume_keyed_nonces` in the payment
     `APPROVE`, `legacy_sender_nonce` in the context, TXPARAM 0x0D–0x10, the mempool observer
     hook for legacy-nonce reads, keyed concurrency;
   - EIP-7805 / EIP-8369: inclusion lists, Profile 2 replay (`profile_2` threaded through the
     backend), the two-endpoint evaluation, the code-byte budget;
   - the pinned canonical paymaster hash (frames' constant is still zero, "OQ1");
   - `ethrex_simulateFrameTransaction`, `debug_getRawTransaction`, the reason-carrying
     `TxValidationError::InvalidFrameTransaction(String)` alongside frames'
     `InvalidFrameTransactionFormat(String)` / `InvalidFrameSignature`; the payload builder's
     eviction matcher follows the new messages;
   - the `--mempool.private` flag and `private_mempool` option;
   - Hegotá-only test files (eip8250, eip8272, focil, inclusion lists, simulate RPC, ...).
3. **Dropped here, deliberately:**
   - the EIP-8272 envelope model (`recent_root_references`, its RLP, intrinsic gas, the
     pre-execution reference pass, `TXPARAM 0x11`, `RECENTROOTREFLOAD`, the mempool head-change
     recheck). It is replaced by the canonical-frame design of `824cbc0b0e` in the next commit
     series, so nothing of it was worth porting. The predeploy install stays.
   - EIP-8312 (UTXO frames). It was never part of the chain's rule set and it touched every
     conflicted core file; what still compiles is left in place for this merge commit and
     removed in a follow-up. Until then `check_utxo_admission` reads the EIP's constant
     instead of the dropped node option, and `UtxoNotYetSpendable` stays as an error variant.

## Hunks resolved by hand (everything else was a clean side pick)

| File | What |
| --- | --- |
| `crates/vm/levm/src/vm.rs` | context keeps `max_gas` **and** `legacy_sender_nonce`; the `nonce == u64::MAX` guard became `nonce_seq == u64::MAX`; Hegotá's post-dispatch entry-charge add-back removed (each branch already includes `frame_entry_gas`); its duplicate frame-exit state accounting removed (frames' version follows); the prefix simulation seeds `state_gas_reservoir = frame.state_gas_limit` and toggles `state_gas_isolated` like the execution path |
| `crates/vm/levm/src/errors.rs` | both error families kept, see rule 2 |
| `crates/vm/levm/src/opcode_handlers/frame_tx.rs` | TXPARAM handler: 0x0C from the reservoir plus the 8250 observer hook and the resolved-payer knob; TXPARAM ids 0x0D–0x10 kept, 0x11 (old 8272 count) dropped |
| `crates/vm/backends/levm/mod.rs` | `FrameValidationOutcome` keeps `max_cost`, `read_legacy_nonce`, `code_budget`; `..observed` on the success path; `frame_tx_max_cost` delegates to the restored `FrameTransaction::max_cost` |
| `crates/blockchain/mempool.rs` | one copy of `is_private` / `get_txs_for_new_peer_dump` (both sides had them); `fee_is_bumped` on `U256` with the same strict-greater-and-percentage rule |
| `crates/blockchain/{focil_eligibility,focil_profile2,inclusion_list_builder}.rs` | fee comparisons in `U256`; `max_gas()` |
| `crates/networking/rpc/engine/payload.rs` | the V6 handler decodes the BAL like V5: undecodable bytes are an INVALID payload |
| `crates/networking/rpc/types/receipt.rs` | both new tests kept |
| `tooling/ef_tests/engine/Makefile` | both fixture overlays (`frames-vectors`, `focil-vectors`) wired into every target |
| `test/tests/{levm/eip8141_tests,common/frame_tx_validation_tests}.rs`, `docs/eip-8141.md` | frames' side; Hegotá's 8141 coverage is superseded by the `v0.3.0` fixtures |
| `test/tests/blockchain/mempool_tests.rs` | frames' side for the type renames; Hegotá's paymaster and keyed-concurrency tests kept |

## Also in this merge

- The EIP-8312 test files (`eip8312_tests.rs`, `eip8312_mempool_tests.rs`,
  `eip8312_cross_path_tests.rs`) and the two UTXO tests in `mempool_tests.rs` are deleted,
  and the old EIP-8272 test files (`eip8272_tests.rs`, `eip8272_bal_tests.rs`) with them:
  both test the models rule 3 drops, and adapting them only to delete them later was waste.
- `scripts/hegota-testnet/gen-deployment-keys.sh` used `sed -n '/^Phrase:/{n;p}'`, which BSD
  sed rejects; the portable `{n;p;}` works on both. Found when generating keys for the local
  devnet on macOS.

## Two things the frames line does that this branch does not inherit

- **Its mempool simulation skips the frame-entry access charge.** `frames-devnet-0`'s
  `run_frame_validation_prefix` hands each prefix frame its whole `limits.execution`, while
  block execution charges the target's warm/cold access (and any 7702 delegation access) at
  entry. A frame that can pay for its code but not for its entry is therefore admitted there
  and fails in the block. Hegotá's simulation mirrors execution and is kept; the merged test
  fixtures that relied on the lenient path (100-gas self-verify frames against a coded
  sender: 100 warm access + 9 gas of `APPROVE` code) now declare 200. Worth raising on the
  frames line.
- **Its spec fixtures assume a chain with only EIP-8141 on top of Amsterdam.** Two things
  stop `tests-frames-devnet@v0.3.0` from running here. Its blocks carry frame transactions
  with a scalar `nonce`, where this chain's envelope has EIP-8250's `nonce_keys` /
  `nonce_seq`, so those blocks do not decode (`InvalidBody`). And from Hegota activation
  `prepare_block` installs the EIP-8250 nonce manager and the EIP-8272 recent-root
  predeploy, two accounts absent from the fixtures' pre-state, so the first block of every
  fixture, frame transactions or not, carries two account changes its block access list
  lacks (`BlockAccessListHashMismatch`, the bulk of the overlay's 3,127 failures). The
  `frames-vectors` targets stay in both fixture Makefiles but are no longer prerequisites
  of the test targets. EIP-8141 conformance is inherited by code from the merge, not
  re-verified by fixtures; the Amsterdam, legacy and FOCIL suites still run. The fixture
  runner maps a fixture `nonce` onto key `[0]` so it compiles.

## Follow-ups this merge leaves

- Remove EIP-8312 (`crates/common/types/utxo.rs`, the vault paths in the VM, mempool and
  builder, its tests, the two UTXO mempool tests kept in `mempool_tests.rs`).
- Implement EIP-8272 at `824cbc0b0e` (canonical frame) — replaces what rule 3 dropped.
- Implement EIP-8250 at `94f5a3e3c1` (state-gas first use) inside `consume_keyed_nonces`.
- Re-pin EIP-8369 to `51dc7b939a`; verify `codeFlag` treats a 7702 delegation as code.
- Regenerate the golden envelope vector (`frametx.py` and the Rust test) for the 8250
  envelope on frames' encoding.
