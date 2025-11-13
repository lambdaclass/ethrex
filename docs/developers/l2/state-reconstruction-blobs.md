# Generate blobs for the state reconstruction test

The test in `crates/l2/tests/state_reconstruct.rs` replays a fixed set of blobs to reconstruct the state. If you ever need to regenerate those fixtures, you need to change the files `payload_builder.rs` and `l1_committer.rs`.

## 1. Cap block payloads at 10 transactions

Edit `crates/l2/sequencer/block_producer/payload_builder.rs` and, inside the `fill_transactions` loop, add the early exit that forces every L2 block to contain at most ten transactions:

```rust
if context.payload.body.transactions.len() >= 10 {
    println!("Reached max transactions per block limit");
    break;
}
```

That guarantees we have transactions in each block for at least 6 batches.

## 2. Persist every blob locally when the committer sends a batch

Still in the sequencer, open `crates/l2/sequencer/l1_committer.rs`:

- At the end of `send_commitment` (after logging the transaction hash) dump the blob that was just submitted:

  ```rust
  // Rest of the code ...
  info!("Commitment sent: {commit_tx_hash:#x}");
  store_blobs(batch.blobs_bundle.blobs.clone(), batch.number);
  Ok(commit_tx_hash)
  ```

- Add this helper function:

  ```rust
  fn store_blobs(blobs: Vec<Blob>, current_blob: u64) {
      let blob = blobs.first().unwrap();
      fs::write(format!("{current_blob}-1.blob"), blob).unwrap();
  }
  ```

Running the node with the deposits of the rich accounts will create `N-1.blob` files (you can move them into `fixtures/blobs/` afterwards).

## 3. Run the L2 and capture six blobs

Start the local L2 with a 20 seconds per commit so we have at least 6 batches with transactions:

```sh
ethrex l2 --dev --no-monitor --committer.commit-time 20000
```

Once the sequencer has produced six batches you will see six files named `1-1.blob` through `6-1.blob`. Copy them into `fixtures/blobs/` (overwriting the existing files).
