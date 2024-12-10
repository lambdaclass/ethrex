# Syncing

## Snap Sync

A snap sync cycle begins by fetching all the block headers (via p2p) between the current head (latest canonical block) and the sync head (block hash sent by a forkChoiceUpdate).
The next two steps are performed in parallel:
On one side, blocks and receipts for all fetched headers are fetched via p2p and stored.

On the other side, the state is reconstructed via p2p snap requests. Our current implementation of this works as follows:
We will spawn two processes, the `bytecode_fetcher` which will remain active and process bytecode requests in batches by requesting bytecode batches from peers and storing them, and the `fetch_snap_state` process, which will iterate over the fetched headers and rebuild the block's state via `rebuild_state_trie`.

`rebuild_state_trie` will spawn a `storage_fetcher` process (which works similarly to the `bytecode_fetcher` and is kept alive for the duration of the rebuild process), it will open a new state trie and will fetch the block's accounts in batches and for each account it will: send the account's code hash to the `bytecode_fetcher` (if not empty), send the account's address and storage root to the `storage_fetcher` (if not empty), and add the account to the state trie. Once all accounts are processed, the final state root will be checked and committed.

(Not implemented yet) When `fetch_snap_state` runs out of available state (aka, the state we need to fetch is older than 128 blocks and peers don't provide it), it will begin the `state_healing` process.
This diagram illustrates the process described above:

![snap_sync](/crates/networking/docs/diagrams/snap_sync.jpg)

The `bytecode_fetcher` has its own channel where it receives code hashes from active `rebuild_state_trie` processes. Once a code hash is received, it is added to a pending queue. When the queue has enough messages for a full batch it will request a batch of bytecodes via snap p2p and store them. If a bytecode could not be fetched by the request (aka, we reached the response limit) it is added back to the pending queue. After the whole state is synced `fetch_snap_state` will send an empty list to the `bytecode_fetcher` to signal the end of the requests so it can request the last (incomplete) bytecode batch and end gracefully.
This diagram illustrates the process described above:

![snap_sync](/crates/networking/docs/diagrams/bytecode_fetcher.jpg)

The `storage_fetcher` works almost alike, but one will be spawned for each `rebuild_state_trie` process as we can't fetch storages from different blocks in the same request.
