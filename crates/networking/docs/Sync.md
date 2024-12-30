# Syncing

## Snap Sync

A snap sync cycle begins by fetching all the block headers (via eth p2p) between the current head (latest canonical block) and the sync head (block hash sent by a forkChoiceUpdate).

We will then fetch the block bodies from each header and at the same time select a pivot block (sync head - 64) and start rebuilding its state via snap p2p requests, if the pivot were to become stale during this rebuild we will select a newer pivot (sync head) and restart it.

After we fully rebuilt the pivot state and fetched all the block bodies we will fetch and store the receipts for the range between the current head and the pivot (including it), and at the same time store all blocks in the same range and execute all blocks after the pivot (like in full sync).

This diagram illustrates the process described above:

![snap_sync](/crates/networking/docs/diagrams/snap_sync.jpg).

### Snap State Rebuild

During snap sync we need to fully rebuild the pivot block's state. We can divide this process into the initial sync and the healing phase.
For the first phase we will spawn two processes, the `bytecode_fetcher` and the `storage_fetcher` which will both remain active and listening for requests from the main rebuild process which they will then queue and process in fixed size batches (more on this later). It will then request the full extent of accounts from the pivot block's state trie via p2p snap requests. For each obtained range we will send the account's code hash and storage root to the `bytecode_fetcher` and `storage_fetcher` respectively for fetching. Once we fetch all accounts (or the account state is no longer available), we will signal the `storage_fetcher` to finish all pending requests and move on to the next phase, while keeping the `bytecode_fetcher` active.

In the healing phase we will spawn another queue-like process called `storage_healer`, and we will begin requesting state trie nodes. We will begin by requesting the pivot block's state's root node proceed by requesting the current node's children (if they are not already part of the state) until we have the full trie stored (aka all child nodes are known). For each fetched leaf node we will send its code hash to the `bytecode_fetcher` and account hash to the `storage_healer`.

The `storage_healer` will contain a list of pending account hashes and paths. And will add new entries by either adding the root node of an account's storage trie when receiving an account hash from the main process or by adding the unknown children of nodes returned by peers.

This diagram illustrates the process described above:

![rebuild_state](/crates/networking/docs/diagrams/rebuild_state_trie.jpg).

To exemplify how queue-like processes work we will explain how the `bytecode_fetcher` works:

The `bytecode_fetcher` has its own channel where it receives code hashes from an active `rebuild_state_trie` process. Once a code hash is received, it is added to a pending queue. When the queue has enough messages for a full batch it will request a batch of bytecodes via snap p2p and store them. If a bytecode could not be fetched by the request (aka, we reached the response limit) it is added back to the pending queue. After the whole state is synced `fetch_snap_state` will send an empty list to the `bytecode_fetcher` to signal the end of the requests so it can request the last (incomplete) bytecode batch and end gracefully.

This diagram illustrates the process described above:

![snap_sync](/crates/networking/docs/diagrams/bytecode_fetcher.jpg)
