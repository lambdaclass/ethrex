# VM/LEVM Polygon Review

**Scope:** VM hooks, opcode handlers, gas cost, precompiles, execution, constants, environment, backends
**Reviewer:** vm-reviewer
**Branch:** implement-polygon vs main
**Reference:** Bor at /tmp/bor

---

## High Severity

### H-1: Missing substate backup revert for failed Polygon transactions

**Files:** `crates/vm/levm/src/vm.rs:569-591`, `crates/vm/levm/src/hooks/hook.rs:37-42`

**Description:** The Polygon hook list `[PolygonHook]` does not include a `BackupHook`. For L1, the hook chain is `[DefaultHook, BackupHook]` where `BackupHook.finalize_execution()` stores the call frame backup for transaction undo. For Polygon, this backup management is entirely absent.

More critically, the substate `push_backup()` at `vm.rs:569` is never committed or reverted for Polygon transactions. The flow is:

1. `push_backup()` creates a checkpoint (line 569)
2. Polygon LogTransfer is added to the current (post-backup) substate (line 583-590)
3. Execution runs, adding more logs from nested calls
4. On failure, `PolygonHook.finalize_execution()` calls `undo_value_transfer()` but never calls `revert_backup()` or `commit_backup()`
5. `extract_logs()` walks the entire parent chain and collects ALL logs (line 732-733)

**Impact:** On a failed Polygon transaction, the receipt will contain:
- The initial LogTransfer (should be reverted, as Bor reverts it via snapshot)
- All committed nested call logs (should be reverted)
- The LogFeeTransfer (correct, should survive)

In Bor, a failed transaction only produces the LogFeeTransfer in the receipt. The Transfer log and all execution logs are inside `evm.Call()`'s snapshot and get reverted.

**Bor reference:** `core/evm.go:267-302` — Transfer() is called inside Call() which uses `StateDB.Snapshot()`/`RevertToSnapshot()`. Fee logs are added in `state_transition.go:636` outside any snapshot.

**Suggested fix:** The PolygonHook needs to revert the substate backup before adding the LogFeeTransfer on failed transactions, or the log extraction logic needs to be aware of the pending backup. A possible approach:

```rust
// In PolygonHook.finalize_execution, on failure:
if !ctx_result.is_success() {
    vm.substate.revert_backup();
    undo_value_transfer(vm)?;
}
// ... compute fees, add LogFeeTransfer ...
```

The `BackupHook` equivalent (storing `tx_backup` for `undo_last_transaction`) is also missing for Polygon, which means `stateless_execute` and other callers that rely on `tx_backup` may not work correctly.

---

### H-2: POLYGON_INIT_CODE_MAX_SIZE is wrong (65536 vs Bor's 49152)

**Files:** `crates/vm/levm/src/constants.rs:33`, `crates/vm/levm/src/hooks/default_hook.rs:374`, `crates/vm/levm/src/opcode_handlers/system.rs:648`

**Description:** ethrex defines `POLYGON_INIT_CODE_MAX_SIZE = 65536` (derived as `2 * POLYGON_MAX_CODE_SIZE`). But Bor uses `MaxInitCodeSize = 2 * MaxCodeSize = 2 * 24576 = 49152` for ALL forks, including post-Ahmedabad. The Ahmedabad upgrade only increased the deployed code size limit (`MaxCodeSizePostAhmedabad = 32768`), NOT the init code size limit.

**Bor references:**
- `params/protocol_params.go:148`: `MaxInitCodeSize = 2 * MaxCodeSize` (always 49152)
- `core/vm/gas_table.go:342`: `if size > params.MaxInitCodeSize` (CREATE/CREATE2 opcode)
- `core/state_transition.go:529`: `len(msg.Data) > params.MaxInitCodeSize` (tx validation)

**Impact:** ethrex will accept init code between 49152-65536 bytes on Polygon that Bor rejects. This causes consensus divergence — blocks containing such transactions would be valid on ethrex but invalid on Bor.

**Fix:** Change `POLYGON_INIT_CODE_MAX_SIZE` to `49152` (same as `INIT_CODE_MAX_SIZE`), or better yet, just use `INIT_CODE_MAX_SIZE` for Polygon since they're the same value.

---

## Medium Severity

### M-1: `is_precompile()` hardcodes Prague set for all Polygon forks

**File:** `crates/vm/levm/src/precompiles.rs:262-266`

```rust
if matches!(vm_type, VMType::Polygon(_)) {
    let addr_low = address.to_low_u64_be();
    return (1..=SIZE_PRECOMPILES_PRAGUE).contains(&addr_low) || *address == P256VERIFY.address;
}
```

**Description:** This returns `true` for all addresses 1-17 plus P256Verify regardless of the actual Polygon fork level. BLS precompiles (11-17) were added at Prague-equivalent forks. If a Polygon node is syncing from genesis through earlier forks (pre-Ahmedabad), BLS precompile addresses would incorrectly be treated as "warm" for gas accounting purposes, leading to incorrect gas charges for CALLs to those addresses.

**Impact:** Incorrect warm/cold gas costs for BLS precompile addresses on pre-Prague Polygon blocks. This affects gas calculation and could cause consensus issues when replaying historical blocks.

**Suggestion:** Make the precompile range fork-dependent, similar to how `Substate::initialize` handles it for L1.

---

### M-2: SLOTNUM opcode enabled on Polygon (possibly incorrect)

**File:** `crates/vm/levm/src/opcodes.rs:1161-1177`

**Description:** The Polygon opcode table starts from the Amsterdam table (`build_opcode_table_amsterdam`), which includes `SLOTNUM` (opcode 0x49 area — actually, SLOTNUM uses a different slot). Polygon has no concept of beacon chain slots. While the code sets `slot_number` to `U256::zero()` for Polygon (so SLOTNUM returns 0), it's unclear whether SLOTNUM should be a valid opcode at all on Polygon.

**Impact:** If Bor treats SLOTNUM as an invalid opcode, contracts using it would revert on Bor but succeed (returning 0) on ethrex. Need to verify against Bor whether Amsterdam-era opcodes are activated.

**Recommendation:** Verify with the Bor reference whether SLOTNUM, EXTCALL, EXTDELEGATECALL, EXTSTATICCALL, RETURNDATALOAD, and other Amsterdam opcodes are valid on Polygon.

---

### M-3: Dead code in `build_opcode_table` for Polygon detection

**File:** `crates/vm/levm/src/opcodes.rs:408-410`

```rust
pub(crate) fn build_opcode_table(fork: Fork) -> [OpCodeFn; 256] {
    if fork.is_polygon() {
        return Self::build_opcode_table_polygon(fork);
    }
```

**Description:** The code comment at `vm.rs:348` says "env.config.fork is Prague (not a Polygon-specific fork), so fork.is_polygon() returns false." This means the `fork.is_polygon()` branch in `build_opcode_table` is dead code. The actual Polygon dispatch happens via `vm_type` in `VM::new()` at line 504-506.

**Impact:** No runtime impact (the vm_type check handles it correctly), but the dead code is confusing and could mask bugs if someone adds a new call to `build_opcode_table` expecting it to handle Polygon forks.

**Fix:** Remove the dead branch from `build_opcode_table` or add a comment explaining it's unreachable.

---

## Low Severity

### L-1: Debug logging should be removed or downgraded before merge

**File:** `crates/vm/backends/levm/mod.rs`

Two `tracing::debug!` blocks were added:
1. **Lines 153-161:** `TX_GAS` per-transaction gas logging
2. **Lines 172-179:** `BLOCK_GAS_TOTAL` per-block gas summary

These are useful for development debugging but should be removed or downgraded to `trace!` before merge. The codebase convention (per recent commit `479ade087`) is to use `trace` for verbose per-request logging.

---

### L-2: `polygon_system_call_levm` duplicates `generic_system_contract_levm`

**File:** `crates/vm/backends/levm/mod.rs:1802-1874`

**Description:** `polygon_system_call_levm` is nearly identical to `generic_system_contract_levm` — same structure for environment setup, transaction creation, execution, and state restoration. The only differences are: configurable gas limit and skipping the Prague empty-code check.

**Impact:** Code duplication increases maintenance burden. Consider refactoring to share the common logic.

---

### L-3: KZG point evaluation removed at LisovoPro

**File:** `crates/vm/levm/src/precompiles.rs:194-196`

```rust
if fork.is_polygon() && precompile.address == POINT_EVALUATION.address {
    return fork >= Fork::Lisovo && fork < Fork::LisovoPro;
}
```

**Description:** KZG point evaluation (precompile 0x0A) is activated at Lisovo and then removed at LisovoPro. This is an unusual pattern — precompiles are typically only added, never removed. Verify against the Bor spec that this lifecycle is correct.

---

## Correct Implementations

The following were verified against the Bor reference and found to be correct:

| Area | Status | Notes |
|------|--------|-------|
| **COINBASE override** | Correct | `VMType::Polygon(pfc) => pfc.coinbase` used in all 4 Environment construction sites. Matches Bor's `NewEVMBlockContext` setting `Coinbase = CalculateCoinbase()`. |
| **LogTransfer emission (CALL)** | Correct | Emitted after `self.transfer()` in both pre- and post-backup paths. Matches Bor's `Transfer()` in `evm.Call()`. |
| **LogTransfer emission (CREATE)** | Correct | Emitted in both `execution_handlers.rs` (top-level CREATE tx) and `system.rs` (CREATE/CREATE2 opcodes). Matches Bor's `Transfer()` in `evm.create()`. |
| **No LogTransfer for SELFDESTRUCT** | Correct | Bor's `opSelfdestruct` does not call `Transfer()` — it directly manipulates balances. No LogTransfer is expected. |
| **LogFeeTransfer format** | Correct | Topic hash `0x4dfe1bbb...` matches `transferFeeLogSig` in `bor_fee_log.go`. Data layout (amount, input1, input2, output1, output2) matches. Fee contract address `0x...1010` matches. |
| **LogTransfer format** | Correct | Topic hash `0xe6497e3e...` matches `transferLogSig` in `bor_fee_log.go`. |
| **Fee distribution** | Correct | Base fee to burnt contract, tip to coinbase. Matches `state_transition.go:614-624`. |
| **LogFeeTransfer balance capture timing** | Correct | `sender_balance_before` captured before `DefaultHook.prepare_execution` (gas deduction). Matches Bor's `execute()` lines 458-476 where `input1` is read before `preCheck()`/`buyGas()`. |
| **MODEXP gas (EIP-7883)** | Correct | `eip7883` flag threaded through all modexp gas paths. Polygon activates at Lisovo via force-enable in `backends/levm/mod.rs:1391`. |
| **POLYGON_MAX_CODE_SIZE** | Correct | `0x8000` (32768) matches `MaxCodeSizePostAhmedabad` in Bor. |
| **POLYGON_MAX_TX_GAS** | Correct | `1 << 25` (33,554,432) matches `MaxTxGas` in Bor. |
| **DIFFICULTY opcode (0x44)** | Correct | Returns `env.difficulty` instead of PREVRANDAO. Polygon has no beacon chain. |
| **BLOBHASH/BLOBBASEFEE disabled** | Correct | Set to `OpInvalidHandler`. Polygon has no blob transactions. |
| **StateSyncTransaction skip** | Correct | `matches!(tx, Transaction::StateSyncTransaction(_))` skips these in block execution. |
| **EIP-2935 (block hash history)** | Correct | Enabled for Polygon at Prague, matches fork behavior. |
| **EIP-7778 skip for Polygon** | Correct | Amsterdam separate gas accounting skipped — Polygon doesn't have this. |
| **P256Verify warm set** | Correct | Added to warm set for Polygon in `VM::new` since fork-based check doesn't work. |
| **CLZ conditional activation** | Correct | Only enabled for `>= Fork::Lisovo`. |

---

## Summary

| Severity | Count | Key Issues |
|----------|-------|------------|
| High | 2 | Missing backup revert on failed txs; wrong init code size limit |
| Medium | 3 | Hardcoded precompile set; SLOTNUM possibly invalid; dead code |
| Low | 3 | Debug logging; code duplication; KZG lifecycle |

The overall implementation quality is good — fee distribution, log format, COINBASE override, and most opcode changes closely match the Bor reference. The two high-severity issues (backup management and init code size) should be addressed before merge as they can cause consensus divergence.
