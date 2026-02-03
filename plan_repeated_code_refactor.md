# Plan: Refactor Repeated Snapshot Dumping Code

## Problem

In `snap/client.rs`, there are two patterns of repeated code:

### Pattern 1: Account State Snapshot Dumping

**Location A** (lines 155-181) - In loop, when buffer is full:
```rust
let current_account_hashes = std::mem::take(&mut all_account_hashes);
let current_account_states = std::mem::take(&mut all_accounts_state);

let account_state_chunk = current_account_hashes
    .into_iter()
    .zip(current_account_states)
    .collect::<Vec<(H256, AccountState)>>();

if !std::fs::exists(account_state_snapshots_dir)...
    std::fs::create_dir_all(account_state_snapshots_dir)...

let path = get_account_state_snapshot_file(&dir, chunk_file);
dump_accounts_to_file(&path, account_state_chunk)
```

**Location B** (lines 283-309) - After loop, for remaining data:
```rust
// Same pattern, almost identical code
```

### Pattern 2: Storage Snapshot Dumping

**Location A** (lines 615-636) - In loop
**Location B** (lines 996-1009) - After loop

Same pattern with `dump_storages_to_file`.

---

## Proposed Solution

### Option 1: Helper Functions (Recommended)

Create two helper functions that encapsulate the snapshot dumping logic:

```rust
/// Ensures directory exists and dumps account state snapshot to file
fn dump_account_state_snapshot(
    dir: &Path,
    chunk_index: usize,
    account_hashes: Vec<H256>,
    account_states: Vec<AccountState>,
) -> Result<(), SnapError> {
    let chunk = account_hashes
        .into_iter()
        .zip(account_states)
        .collect::<Vec<(H256, AccountState)>>();

    ensure_dir_exists(dir)?;

    let path = get_account_state_snapshot_file(dir, chunk_index);
    dump_accounts_to_file(&path, chunk)
        .map_err(|e| SnapError::write_failed(path, e.error))
}

/// Ensures directory exists and dumps storage snapshot to file
fn dump_storage_snapshot(
    dir: &Path,
    chunk_index: usize,
    storages: Vec<AccountsWithStorage>,
) -> Result<(), SnapError> {
    ensure_dir_exists(dir)?;

    let path = get_account_storages_snapshot_file(dir, chunk_index);
    dump_storages_to_file(&path, storages)
        .map_err(|e| SnapError::write_failed(path, e.error))
}

/// Ensures a directory exists, creating it if necessary
fn ensure_dir_exists(dir: &Path) -> Result<(), SnapError> {
    if !std::fs::exists(dir).map_err(|_| SnapError::dir_not_exists(dir.to_path_buf()))? {
        std::fs::create_dir_all(dir).map_err(|_| SnapError::dir_create_failed(dir.to_path_buf()))?;
    }
    Ok(())
}
```

### Option 2: Trait-Based Approach (Over-engineered)

Create a `SnapshotDumper` trait. Not recommended - adds complexity for little benefit.

---

## Implementation Steps

### Step 1: Add helper function `ensure_dir_exists`

Location: `snap/client.rs` (near the top, after imports)

```rust
/// Ensures a directory exists, creating it if necessary
fn ensure_dir_exists(dir: &Path) -> Result<(), SnapError> {
    if !dir.exists() {
        std::fs::create_dir_all(dir)
            .map_err(|_| SnapError::dir_create_failed(dir.to_path_buf()))?;
    }
    Ok(())
}
```

### Step 2: Add helper function `dump_account_state_snapshot`

```rust
/// Prepares and dumps account state snapshot to file
fn dump_account_state_snapshot(
    dir: &Path,
    chunk_index: usize,
    account_hashes: Vec<H256>,
    account_states: Vec<AccountState>,
) -> Result<(), DumpError> {
    let chunk: Vec<(H256, AccountState)> = account_hashes
        .into_iter()
        .zip(account_states)
        .collect();

    let path = get_account_state_snapshot_file(dir, chunk_index);
    dump_accounts_to_file(&path, chunk)
}
```

### Step 3: Add helper function `dump_storage_snapshot`

```rust
/// Prepares and dumps storage snapshot to file
fn dump_storage_snapshot(
    dir: &Path,
    chunk_index: usize,
    storages: Vec<AccountsWithStorage>,
) -> Result<(), DumpError> {
    let path = get_account_storages_snapshot_file(dir, chunk_index);
    dump_storages_to_file(&path, storages)
}
```

### Step 4: Refactor `request_account_range` (Location A - in loop)

Before:
```rust
if all_accounts_state.len() * size_of::<AccountState>() >= RANGE_FILE_CHUNK_SIZE {
    let current_account_hashes = std::mem::take(&mut all_account_hashes);
    let current_account_states = std::mem::take(&mut all_accounts_state);

    let account_state_chunk = current_account_hashes
        .into_iter()
        .zip(current_account_states)
        .collect::<Vec<(H256, AccountState)>>();

    if !std::fs::exists(account_state_snapshots_dir)...

    let account_state_snapshots_dir_cloned = account_state_snapshots_dir.to_path_buf();
    write_set.spawn(async move {
        let path = get_account_state_snapshot_file(...);
        dump_accounts_to_file(&path, account_state_chunk)
    });

    chunk_file += 1;
}
```

After:
```rust
if all_accounts_state.len() * size_of::<AccountState>() >= RANGE_FILE_CHUNK_SIZE {
    let hashes = std::mem::take(&mut all_account_hashes);
    let states = std::mem::take(&mut all_accounts_state);

    ensure_dir_exists(account_state_snapshots_dir)?;

    let dir = account_state_snapshots_dir.to_path_buf();
    let idx = chunk_file;
    write_set.spawn(async move {
        dump_account_state_snapshot(&dir, idx, hashes, states)
    });

    chunk_file += 1;
}
```

### Step 5: Refactor `request_account_range` (Location B - after loop)

Before:
```rust
// TODO: This is repeated code, consider refactoring
{
    let current_account_hashes = std::mem::take(&mut all_account_hashes);
    let current_account_states = std::mem::take(&mut all_accounts_state);

    let account_state_chunk = current_account_hashes
        .into_iter()
        .zip(current_account_states)
        .collect::<Vec<(H256, AccountState)>>();

    if !std::fs::exists(account_state_snapshots_dir)...

    let path = get_account_state_snapshot_file(account_state_snapshots_dir, chunk_file);
    dump_accounts_to_file(&path, account_state_chunk)
        .inspect_err(|err| error!(...))
        .map_err(|_| SnapError::SnapshotDir(...))?;
}
```

After:
```rust
// Dump remaining accounts
{
    let hashes = std::mem::take(&mut all_account_hashes);
    let states = std::mem::take(&mut all_accounts_state);

    if !hashes.is_empty() {
        ensure_dir_exists(account_state_snapshots_dir)?;
        dump_account_state_snapshot(account_state_snapshots_dir, chunk_file, hashes, states)
            .inspect_err(|err| error!("Error dumping accounts to disk: {}", err.error))
            .map_err(SnapError::from)?;
    }
}
```

### Step 6: Apply same pattern to storage dumping

Refactor lines 615-636 and 996-1009 using `dump_storage_snapshot`.

### Step 7: Remove the TODO comment

---

## Benefits

1. **DRY**: Each pattern appears once
2. **Testable**: Helper functions can be unit tested
3. **Readable**: Intent is clear from function names
4. **Maintainable**: Changes to dumping logic happen in one place

## Risks

- Low risk: Pure refactoring with no logic changes
- All existing tests should pass

## Testing

1. Run `cargo clippy -p ethrex-p2p` - no warnings
2. Run `cargo test -p ethrex-p2p` - all tests pass
3. Manual test: Run snap sync on testnet to verify snapshots are created correctly

---

## Estimated Changes

- ~3 new helper functions (~30 lines)
- ~4 call sites refactored
- Net reduction: ~40 lines
