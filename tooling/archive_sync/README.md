# Archive Sync

Archive sync can be used to download the whole state of a particular block from an active archive node.

Note that this involves using IPC to communicate with the archive node so both the archive node and ethrex must be on the same computer/server.

We also rely on geth's debug api for this, so it is not guaranteed to work for other non-geth-compatible implementations.

## Usage

### Step 1: Launch an archive node

You can do so by running:

```bash
    geth --NETWORK --http.addr 0.0.0.0 --ipcpath IPC_PATH --syncmode full --gcmode archive --state.scheme=hash
```

You may get the following error if you have been using geth as non-archive before:

```bash
Failed to register the Ethereum service: incompatible state scheme, stored: hash, provided: path
```

This should be solved by either clearing the DB or fully uninstalling and reinstalling geth

You should also launch a consensus node. You can read more about this on the main [README](../../README.md)

You may skip this step if you already have an archive node up and running.

### Step 2: Run the `archive_sync` executable

This executable takes 3 arguments:

* The IPC path of the archive node:

    This will be the `IPC_PATH` we used when launching geth + the path to geth's data. For example, in a geth installed by brew on mac, the ipc file will be located at: /Users/USERNAME/Library/Ethereum/NETWORK/IPC_PATH. This can vary depending on your system so I recommend looking for this kind of log line when you startup geth which will give you the full path:

    ```bash
    INFO [06-19|11:37:09.316] IPC endpoint opened                      url=/Users/USER/Library/Ethereum/IPC_PATH
    ```

* The number of the block you want to sync to:

     Our most common use case for this tool will be to do fullsync on networks like mainnet and sepolia, but starting from the first post-merge blocks, so we will commonly be using the blocks right after the merge transition blocks for this tool which should be:

  - Sepolia Next Block After Merge: 1450410
  - Mainnet Next Block After Merge: 15537395

* (Optional with flag --datadir):

    The path to the DB directory, if none is set the default will be used

With these arguments you can run the following command from this directory:

```bash
 cargo run --release  BLOCK_NUMBER --ipc_path IPC_PATH
```

And adding `--datadir DATADIR` if you want to use a custom directory

While archive sync is faster than the alternatives (snap, full) it can still take a long time on later blocks of large chains

## Usage without an active archive node connection

We can avoid relying on an active archive node connection once we have already performed the first sync by writing the state dump to a directory. Note that this will still require an active archive node for the first step.

### Step 1: Run the `archive_sync` executable as usual with `--output_dir` flag

```bash
 cargo run --release BLOCK_NUMBER --ipc_path IPC_PATH --output_dir STATE_DUMP_DIR
```

If we don't need the node to be synced (for example if we plan to move the state dump to another server after the sync) we can also add the flag `--no_sync` to skip the state sync and only write the state data to files.

### Step 2: Run the `archive_sync` executable with `--input_dir` instead of `--ipc-path`

```bash
 cargo run --release BLOCK_NUMBER --input_dir STATE_DUMP_DIR
```
