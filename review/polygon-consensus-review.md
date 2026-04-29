# Polygon Consensus Engine Code Review

**Reviewer:** polygon-consensus-reviewer
**Branch:** `implement-polygon` vs `main`
**Date:** 2026-03-31
**Reference:** Bor source at `/tmp/bor/` used as spec authority

## Scope

Files reviewed:
- `crates/polygon/src/consensus/engine.rs`
- `crates/polygon/src/consensus/extra_data.rs`
- `crates/polygon/src/consensus/mod.rs`
- `crates/polygon/src/consensus/seal.rs`
- `crates/polygon/src/consensus/snapshot.rs`
- `crates/polygon/src/bor_config.rs`
- `crates/polygon/src/fork_id.rs`
- `crates/polygon/src/genesis.rs`
- `crates/polygon/src/validation.rs`
- `crates/polygon/src/lib.rs`
- `crates/polygon/Cargo.toml`
- `crates/polygon/allocs/amoy.json`
- `crates/polygon/allocs/bor_mainnet.json`
- `crates/polygon/tests/integration.rs`
- `fixtures/networks/polygon.yaml`

---

## Critical Findings (Consensus-Breaking)

### C1. Post-Rio span commits are NOT skipped in `get_system_calls()`

**Severity:** Critical (consensus-breaking post-Rio)
**File:** `crates/polygon/src/consensus/engine.rs:240`

Bor's `Finalize()` explicitly skips span commits post-Rio:
```go
// bor/consensus/bor/bor.go:1194
if !c.config.IsRio(header.Number) {
    if err := c.checkAndCommitSpan(wrappedState, header, cx); err != nil {
```

ethrex's `get_system_calls()` has NO Rio check:
```rust
// engine.rs:240
if self.need_to_commit_span(block_number) {
    let span_call = self.build_span_commit_call(block_number).await?;
    calls.push(span_call);
}
```

This means post-Rio blocks would incorrectly include `commitSpan` system calls, producing wrong state roots and causing consensus failures.

**Fix:** Add `!self.config.is_rio_active(block_number)` guard around the span commit.

---

### C2. Post-Rio snapshot uses wrong validator source (`validators` instead of `selected_producers`)

**Severity:** Critical (consensus-breaking post-Rio)
**File:** `crates/polygon/src/consensus/engine.rs:543-563`

Bor's `getVeBlopSnapshot()` (post-Rio snapshot construction) uses `span.SelectedProducers` sorted by address:
```go
// bor/consensus/bor/bor.go:821-832
producers := make([]*valset.Validator, len(span.SelectedProducers))
for i, validator := range span.SelectedProducers {
    producers[i] = &valset.Validator{
        Address:     common.HexToAddress(validator.Signer),
        VotingPower: validator.VotingPower,
    }
}
sortedProducers := valset.ValidatorsByAddress(producers)
sort.Sort(sortedProducers)
snap := newSnapshot(..., sortedProducers)
```

ethrex's `span_to_validator_set()` always uses `span.validators`:
```rust
// engine.rs:550-558
let mut set: Vec<super::snapshot::ValidatorInfo> = span
    .validators
    .iter()
    .map(|v| super::snapshot::ValidatorInfo {
        address: v.signer,
        voting_power: v.voting_power,
        proposer_priority: v.proposer_priority,
    })
    .collect();
if is_rio {
    set.sort_by_key(|v| v.address);
}
```

The comment at line 547-548 even acknowledges this: "Always use the full validator set for signer authorization." But post-Rio, Bor uses `selected_producers` for the snapshot, NOT the full validator set. The full set has ~25 validators; `selected_producers` has a smaller subset. This difference affects:
- Signer authorization (wrong set of authorized signers)
- Difficulty calculation (wrong succession ring)
- Proposer rotation

**Fix:** Post-Rio, use `span.selected_producers` (sorted by address) to build the snapshot.

---

### C3. Validator set update from header extra data at sprint-end is missing

**Severity:** Critical (validator set drift)
**File:** `crates/polygon/src/consensus/snapshot.rs:73-96` vs Bor `snapshot.go:150-173`

Bor's `snapshot.apply()` updates the validator set from the header's extra data at sprint-end blocks:
```go
// snapshot.go:151-173
if number > 0 && (number+1)%sprint == 0 {
    validatorBytes := header.GetValidatorBytes(s.chainConfig)
    newVals, _ := valset.ParseValidators(validatorBytes)
    v := getUpdatedValidatorSet(snap.ValidatorSet.Copy(), newVals)
    v.IncrementProposerPriority(1)
    snap.ValidatorSet = v
}
```

ethrex's `apply_header()` does NOT update the validator set — it only records the signer and prunes recents. The proposer rotation in `verify_header()` (engine.rs:206-208) increments priority but never reads new validators from the header.

This means the snapshot's validator set becomes stale after the first sprint, diverging from Bor at every subsequent sprint-end. This causes cascading consensus failures (wrong difficulty, wrong authorization).

**Fix:** At sprint-end blocks, parse validator bytes from the header's extra data and update the snapshot's validator set, then increment proposer priority. This requires the `parse_extra_data()` + `parse_validators()` pipeline.

---

## High Findings

### H1. Missing gas limit cap validation

**Severity:** High
**File:** `crates/polygon/src/validation.rs`

Bor checks that gas limit fits in int64:
```go
// bor.go:512-515
gasCap := uint64(0x7fffffffffffffff)
if header.GasLimit > gasCap {
    return fmt.Errorf("invalid gasLimit: have %v, max %v", header.GasLimit, gasCap)
}
```

ethrex only checks `gas_used <= gas_limit` but has no gas limit cap. A malicious header with `gas_limit > 2^63-1` would pass validation.

### H2. Missing mix digest (prev_randao) validation

**Severity:** High
**File:** `crates/polygon/src/validation.rs`

Bor requires mix digest to be zero:
```go
// bor.go:496-498
if header.MixDigest != (common.Hash{}) {
    return errInvalidMixDigest
}
```

ethrex does not validate this. A header with non-zero `prev_randao` would pass validation, which could cause issues with state sync or P2P.

### H3. Missing validator bytes verification at sprint-end blocks

**Severity:** High
**File:** `crates/polygon/src/consensus/engine.rs`

Bor's `verifyCascadingFields()` (bor.go:614-656) validates that the validator bytes embedded in sprint-end block headers match the Heimdall span data. This prevents a malicious block producer from inserting an unauthorized validator set.

ethrex has no equivalent check. Sprint-end blocks with incorrect validator bytes would pass verification.

### H4. Missing Giugliano extra data field validation

**Severity:** High
**File:** `crates/polygon/src/validation.rs`

Bor checks that post-Giugliano blocks contain `gas_target` and `base_fee_change_denominator` in the BlockExtraData:
```go
// bor.go:488-493
if c.config.IsGiugliano(header.Number) {
    gasTarget, bfcd := header.GetBaseFeeParams(c.chainConfig)
    if gasTarget == nil || bfcd == nil {
        return errMissingGiuglianoFields
    }
}
```

ethrex does not validate Giugliano extra fields. Post-Giugliano blocks missing these fields would pass validation.

---

## Medium Findings

### M1. Missing block early / producer delay check

**Severity:** Medium
**File:** `crates/polygon/src/consensus/engine.rs`

Bor's `verifySeal()` (bor.go:951-961) rejects blocks from non-primary producers announced before their expected time slot:
```go
if c.config.IsBhilai(header.Number) && succession != 0 {
    if header.Time > now {
        return consensus.ErrFutureBlock
    }
}
if IsBlockEarly(parent, header, number, succession, c.config) {
    return &BlockTooSoonError{number, succession}
}
```

ethrex does not check block timing relative to producer succession. This means out-of-turn producers could submit blocks early, which Bor nodes would reject.

### M2. Missing future block timestamp check

**Severity:** Medium
**File:** `crates/polygon/src/validation.rs`

Bor has extensive time-based header checks (bor.go:422-462) varying by fork (Giugliano, Bhilai, pre-Bhilai). ethrex only checks `timestamp > parent.timestamp`.

While this is acceptable during sync (processing already-mined blocks), it means the node would accept blocks with timestamps far in the future during live sync, which could cause issues.

### M3. Missing timestamp gap validation (`parent.Time + period`)

**Severity:** Medium
**File:** `crates/polygon/src/validation.rs`

Bor checks minimum timestamp gap:
```go
// bor.go:610
if parent.Time+c.config.CalculatePeriod(number) > header.Time {
    return ErrInvalidTimestamp
}
```

ethrex only checks `timestamp > parent.timestamp` without enforcing the minimum period gap. This is a weaker check than Bor's.

### M4. Seal hash `base_fee` conditional on Jaipur (historical correctness)

**Severity:** Medium (not currently triggerable)
**File:** `crates/polygon/src/consensus/seal.rs:55`

Bor conditionally includes `base_fee` in seal hash only when Jaipur is active:
```go
// bor.go:197-200
if c.IsJaipur(header.Number) {
    if header.BaseFee != nil {
        enc = append(enc, header.BaseFee)
    }
}
```

ethrex always includes `base_fee_per_gas` if present (via `encode_optional_field`). Since `seal_hash()` doesn't receive a BorConfig, it can't check the Jaipur fork status. Pre-Jaipur blocks that somehow have a base_fee would produce the wrong seal hash.

**Risk:** Low in practice since all current Polygon blocks are post-Jaipur, but historically incorrect.

---

## Low Findings

### L1. `is_multiple_of` usage for sprint check

**File:** `crates/polygon/src/bor_config.rs:243`

```rust
block_number.is_multiple_of(sprint)
```

Bor uses `%` operator. Rust's `is_multiple_of()` is equivalent for non-zero values, but if `sprint == 0` the behavior differs. Currently protected by the `sprint == 0` early return, so this is safe.

### L2. Genesis `base_fee_per_gas` for mainnet pre-London blocks

**File:** `crates/polygon/src/genesis.rs:155-156`

Mainnet genesis sets `base_fee_per_gas: None`, which is correct because London activates at block 23,850,000 (not genesis). The `polygon_genesis_block()` function correctly checks `is_london_activated(0)`. This is well-handled.

### L3. Fork ID only includes EVM-level forks (correct)

**File:** `crates/polygon/src/fork_id.rs:16-28`

The comment correctly notes that Bor's `gatherForks()` uses reflection on `ChainConfig` and does NOT descend into the nested `*BorConfig` struct. Only EVM-level forks are included. This matches the Bor reference.

### L4. Snapshot persist interval matches Bor

`SNAPSHOT_PERSIST_INTERVAL = 1024` in `snapshot.rs:14` matches Bor's `checkpointInterval = 1024` (bor.go:57). Correct.

### L5. Snapshot cache size matches Bor

`SNAPSHOT_CACHE_SIZE = 128` in `snapshot.rs:13` matches Bor's `inmemorySnapshots = 128` (bor.go:58). Correct.

---

## Positive Observations

1. **Signer recovery correctness:** The `seal_hash()` and `recover_signer()` implementation matches Bor's `encodeSigHeader()` field ordering and uses proper ecrecover. Cross-validated against real block 83,838,496 in tests.

2. **Difficulty calculation:** The `expected_difficulty()` succession ring formula matches Bor's `Difficulty()` function exactly. Well-tested.

3. **Proposer rotation:** The Tendermint-based `increment_proposer_priority()` with rescale/center/increment/select/reduce matches Bor's `valset.IncrementProposerPriority()`. Extensive unit tests.

4. **Extra data parsing:** Both pre-Lisovo (raw validator bytes) and post-Lisovo (RLP BlockExtraData) formats are handled. Cross-validated against real mainnet blocks.

5. **Fork schedule:** All 12 Polygon forks (Jaipur through Giugliano) are correctly mapped with activation checks. Fork ID computation correctly excludes Bor-specific forks per Bor's reflection-based approach.

6. **Genesis construction:** Custom `polygon_genesis_block()` correctly omits post-merge header fields that `Genesis::get_block()` would incorrectly add. Genesis hashes are documented.

7. **System call encoding:** `encode_commit_span()`, `encode_commit_state()`, `encode_last_state_id()`, `encode_get_current_span()` all have correct ABI selectors verified by hex constants in tests.

8. **Milestone reorg protection:** The `is_reorg_allowed()` logic correctly uses strictly-greater comparison, matching Bor's milestone protection semantics.

9. **Test coverage:** Comprehensive integration tests covering chain validation, sprint/span detection, difficulty calculation, system call encoding, fork choice, genesis construction, milestone protection, and snapshot cache.

---

## Summary

| Severity | Count | Description |
|----------|-------|-------------|
| Critical | 3 | Post-Rio span commit skip missing (C1), wrong validator source for post-Rio snapshot (C2), validator set update at sprint-end missing (C3) |
| High | 4 | Gas limit cap (H1), mix digest check (H2), validator bytes verification (H3), Giugliano fields (H4) |
| Medium | 4 | Block early check (M1), future timestamp (M2), timestamp gap (M3), seal hash Jaipur conditional (M4) |
| Low | 5 | Minor issues, all well-handled or non-triggerable |

The critical findings (C1-C3) are all consensus-breaking in post-Rio blocks and must be fixed before Polygon mainnet sync can succeed past block 77,414,656 (Rio activation). The high findings (H1-H4) represent validation gaps where malformed headers could pass, potentially causing P2P compatibility issues with Bor nodes.

The implementation foundation is solid — signer recovery, difficulty calculation, proposer rotation, extra data parsing, and fork schedule are all spec-compliant. The main gaps are in the `verify_header` pipeline (missing checks) and the post-Rio behavioral changes.
