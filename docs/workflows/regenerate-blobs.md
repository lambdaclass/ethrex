# Blob Regeneration Workflow

## Overview

The state reconstruction test (`crates/l2/tests/state_reconstruct.rs`) replays a fixed set of blobs to verify L2 state can be correctly reconstructed. These blob fixtures must be regenerated whenever the L2 genesis file (`fixtures/genesis/l2.json`) changes, because a new genesis alters the hash of the first block and all descendant blocks. The stored blobs encode parent pointers, so stale hashes make the fixtures unusable.


## Prerequisites

- Rust toolchain installed
- `solc` available in PATH
- Docker installed and running

## Workflow

### 1. Check Prerequisites

Before starting, verify all prerequisites are met:

```bash
# Check Rust toolchain
rustc --version

# Check solc is available
solc --version

# Check docker is running
docker info > /dev/null 2>&1 && echo "Docker is running"
```

If any command fails, stop and ask the user to install the missing dependency before proceeding.

### 2. Apply Temporary Code Changes

The agent must apply two patches to the sequencer code:

#### 2.1 Cap Block Payloads at 10 Transactions

Edit `crates/l2/sequencer/block_producer/payload_builder.rs`. Find the `fill_transactions` function and locate the main `loop` block. Add the following early exit at the **beginning of the loop body**, before the gas and blob space checks:

```rust
if context.payload.body.transactions.len() >= 10 {
    println!("Reached max transactions per block limit");
    break;
}
```

#### 2.2 Persist Blobs When Committer Sends a Batch

Edit `crates/l2/sequencer/l1_committer.rs`:

**First, add the required imports at the top of the file:**

```rust
use ethrex_common::types::Blob;
use std::fs;
```

**Then add this helper function** (place it at the end of the file, outside of any impl block):

```rust
fn store_blobs(blobs: Vec<Blob>, current_blob: u64) {
    let blob = blobs.first().unwrap();
    fs::write(format!("{current_blob}-1.blob"), blob).unwrap();
}
```

**Add the store call at the end of `send_commitment`** (after logging the transaction hash):

```rust
// ... existing code ...
info!("Commitment sent: {commit_tx_hash:#x}");
store_blobs(batch.blobs_bundle.blobs.clone(), batch.number);
Ok(commit_tx_hash)
```

### 3. Clean Previous State

```bash
cd crates/l2
make rm-db-l1 2>/dev/null || true
make rm-db-l2 2>/dev/null || true
rm -f *.blob
```

### 4. Start the L2 Stack

**Important:** Start the prover first, then the sequencer. This prevents the committer from getting stuck waiting for deposits to be verified.

**Note:** The prover is stateless. It doesn't need to be restarted if the sequencer needs to be restarted during this workflow. However, if the prover was already running before this workflow, it must be restarted so it runs on the same commit as the sequencer.

**Terminal 1 - Start prover (exec mode):**

```bash
cd crates/l2
make init-prover-exec
```

**Terminal 2 - Start L1 + L2 sequencer:**

```bash
cd crates/l2
ETHREX_NO_MONITOR=true make init-l2-dev
```

### 5. Wait for 6 Blobs

Monitor Terminal 2 (sequencer) for commitment messages:
```
INFO Commitment sent: 0x...
```

**Wait until 6 commitment messages appear.** This creates files `1-1.blob` through `6-1.blob` in `crates/l2/`.

### 6. Stop Processes and Copy Blobs

Stop the sequencer (Ctrl+C), then:

```bash
cd crates/l2

# Verify 6 blobs exist
ls -la *.blob

# Remove old blobs and move new ones to fixtures
rm -f ../../fixtures/blobs/*.blob
mv *.blob ../../fixtures/blobs/
```

### 7. Revert Code Changes

```bash
git checkout crates/l2/sequencer/block_producer/payload_builder.rs
git checkout crates/l2/sequencer/l1_committer.rs
```

### 8. Clean Up Databases

```bash
cd crates/l2
make rm-db-l1 2>/dev/null || true
make rm-db-l2 2>/dev/null || true
```

### 9. Verify Regeneration

```bash
cd crates/l2

# Quick validation (fast)
make validate-blobs

# Full state reconstruction test (requires docker)
make state-diff-test
```

**Both tests must pass.**

---

## Error Handling Protocol

If verification tests fail, the agent must:

1. **Analyze the error** - Read the test output carefully to understand what failed
2. **Check the Troubleshooting table** - Look for known issues and solutions
3. **Attempt a fix** - Apply the appropriate solution from the table
4. **Retry the workflow** - Restart from the appropriate step (usually Step 3)

**After 3 failed attempts**, the agent must:
- Stop retrying
- Summarize all errors encountered
- Present the errors to the user and ask how to proceed

Example prompt after 3 failures:
```
I've attempted blob regeneration 3 times without success. Here are the errors encountered:

Attempt 1: [error description]
Attempt 2: [error description]
Attempt 3: [error description]

How would you like me to proceed?
```

---

## Troubleshooting

### Common Issues

| Issue | Solution |
|-------|----------|
| No `.blob` files generated | Verify `store_blobs` patch was applied correctly |
| Less than 6 blobs | Wait longer; commit interval is 20 seconds |
| `validate-blobs` fails after regeneration | Genesis may have changed during blob generation; restart |
| `state-diff-test` fails | Ensure docker is running; verify blobs were generated from a clean state |
| Compilation errors | Ensure `solc` is in PATH. If compilation fails with undefined types or modules, verify that all required imports (`Blob` and `fs`) have been added at the top of the modified files. |

### Verification Commands

```bash
# Check blob files exist (should be 131072 bytes each)
ls -la fixtures/blobs/

# Quick validation
cd crates/l2 && make validate-blobs

# Full test
cd crates/l2 && make state-diff-test
```

---

## Results Template

### Regeneration Session Info

| Field | Value |
|-------|-------|
| **Date** | YYYY-MM-DD |
| **Blobs Generated** | 6 |
| **validate-blobs** | PASS / FAIL |
| **state-diff-test** | PASS / FAIL |
| **Attempts** | 1 |

### Observations

(Note any issues encountered during regeneration)

### Conclusion

(Confirm blobs were successfully regenerated and tests pass)
