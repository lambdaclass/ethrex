# Snap Sync

API reference: https://github.com/ethereum/devp2p/blob/master/caps/snap.md

Terminology: 
 - Peers: other ethereum execution clients we are connected which can respond to snap requests.
 - Pivot: The block we have chosen to snap sync to.


## Concept
### What

Executing all blocks to rebuild the state is slow. It's also not possible on ethrex, because we don't support pre merge execution. So we need to download the state from our peers as it currently exists.

The largest challenge in snapsync concerns the download from the state (account state trie and storage state tries). Secondary concerns are the downloading of headers and bytecods. Fast-sync and snap-sync work the same for both the account trie and storage tries.

### First solution: Fast-Sync

Fast sync is a method to download a patricia merkle trie, and the first one implemented in Ethereum. The idea is to download the trie top to bottom, starting at the root, and then downloading the child nodes recursively until all nodes on the trie are downloaded.

![Initial state of simple fast sync](snap_sync/Fast%20Sync%20-%201.png)
![Root download of simple fast sync](snap_sync/Fast%20Sync%20-%202.png)
![Branches download of simple fast sync](snap_sync/Fast%20Sync%20-%203.png)
![Leaves download of simple fast sync](snap_sync/Fast%20Sync%20-%204.png)

There are two problems with this:

- Peers will stop responding to node requests at a certain point. When you ask for a trie node, you speficy **at which state root** you want it. If the root is 128 or more blocks behind, nodes will not satisfy your request.
- Going through the entire trie to find the nodes you do not have is very slow.

For the first problem, once the nodes would no longer respond to the requests[^1], we stop the process of fast sync, update the pivot, and restart the process. The naive solution would be to download the new root, and for each child recursively download them first checking if they are already present in the db.

[^1]: We update pivots based only on the timestamp, not the response of the nodes. According to the specification, the nodes return empty response when the state root is stale, but due to byzantine nodes, a node may return empty whenever for any reason. As such we rely on the spec rule that nodes have to keep at least 128 blocks, and once the `time > timestamp + 128 * 12` mark the pivot as stale.

Example of a possible state after stoping fast sync due to staleness.

![Fast Sync Retaking Example - 1](snap_sync/Fast%20Sync%20Retaking%20Example%20-%201.png)

In this example, when we find that the node { hash: 0x317f, path:0 } is correct, we still need to check all it's children to see if they are present in the db (in this case they're all missing).

To solve the second problem, we introduce an optimization, which is called the "Membatch"[^2]. This structure is helped to create a new invariant, which is that we make sure that if a node is present in the db, it and all it's children are present.

[^2]: This structure should be renamed to "PendingNodes" as it's a more descriptive name.

This makes the second problem disappear: while going down the tree, if a node is in the database, the entire subtree does not need to be followed.

To maintain this invariant, we do the following:

- When we get a new node, we don't immediately store it in the database. We keep track of the amount of every node's children that are not yet in the database. As long as it's not zero, we keep it in a separate in-memory structure instead of on the db.
- When a node has all of its children in the db, we commit it and recursively go up the tree to see if its parent needs to be commited, etc.

Example of a possible state after stoping fast sync due to staleness with membatch.

![Fast Sync Membatch Example - 1](snap_sync/Fast%20Sync%20Membatch%20Example%20-%201.png)

### Speeding up: Snap-Sync

Fast-Sync as a process is quite slow (current ethrex speed for fast-sync hoodi is ~45 minutes, 4.5x times slower than snap sync). To speed it up, we can use a property from fast-sync, which is that it can take any trie that respects the invariant and update it, even if the trie as presented isn't consistent with any single state. The idea would be to download the leaves of the values from any given state, build a trie from those values and apply fast-sync to the resulting trie to "heal" it to a consistent state. 

For our code, we call the fast-sync the healing step.

Example run:

\- We download the 4 accounts in the state trie from two different blocks

![Snap Sync Leaves - 1](snap_sync/Snap%20Sync%20Leaves%20-%201.png)

\- We rebuild the trie

![Snap Sync Healing - 1](snap_sync/Snap%20Sync%20Healing%20-%201.png)

\- We run the healing algorithm and get a correct tree at the end of it

This method alone provides up to a 4-5 times boost in performance, as computing the trie is way faster than downloading it.

## Implementation

### Flags

When testing snap sync there are flags to take into account:

- If the `SKIP_START_SNAP_SYNC` environment variable is set and isn't empty, it will skip the step of downloading the leaves and will immediatly begin healing. This simulates the behaviour of fast-sync.

- If debug assertions are on, the program will validate that the entire state and storage tries are valid by traversing the entire trie and recomputing the roots. If any is found to be wrong, it will print an error and exit the program.

- `--snycmode [full, default:snap]` which defines what kind of sync we use. Full is executing each block, and isn't possible for mainnet and sepolia.

### File Structure

The sync module is a component of the `ethrex-p2p` crate, found in `crates/networking/p2p` folder. The main sync functions are found in: 
- `crates/networking/p2p/sync.rs`
- `crates/networking/p2p/peer_handler.rs`
- `crates/networking/p2p/sync/state_healing.rs`
- `crates/networking/p2p/sync/storage_healing.rs` 
- `crates/networking/p2p/sync/code_collector.rs`

### Syncer and Sync Modes

The struct that handles the needed handles for syncing is the `Syncer`, and it has a variable to indicate if the snap mode is enabled. The Sync Modes are defined in `sync.rs` as follows.

```rust

/// Manager in charge the sync process
#[derive(Debug)]
pub struct Syncer {
    /// This is also held by the SyncManager allowing it to track the latest syncmode, without modifying it
    /// No outside process should modify this value, only being modified by the sync cycle
    snap_enabled: Arc<AtomicBool>,
    peers: PeerHandler,
    // Used for cancelling long-living tasks upon shutdown
    cancel_token: CancellationToken,
    blockchain: Arc<Blockchain>,
    /// This string indicates a folder where the snap algorithm will store temporary files that are
    /// used during the syncing process
    datadir: PathBuf,
}

pub enum SyncMode {
    #[default]
    Full,
    Snap,
}
```

The flow of the program in default mode is to start by doing snap sync, then we switch to fullsync at the end and continue catching up by executing blocks.

### Downloading Headers

The first step is downloading all the headers, through the `request_block_headers` function. This function does the following steps:

- Request from peers the number of the sync_head that we received from the consensus client
- Divide the headers into descrete "chunks" to ask our peers
    - Currently, the headers are divided into 800 chunks[^3]
- Queue those chunks as tasks into a channel
    - These tasks ask the peers for their data, and respond through a channel
- Read from the channel to get a task
- Finds the best free peers
- Spawn a new async job to ask the peer for the task
- If the channel for new is empty, check if everything is downloaded
- Read from the channel of responses
- Store the read result

[^3]: This currently isn't a named constant, we should change that

![request_block_header flowchart](snap_sync/Flow%20-%20Download%20Headers.png)

### Downloading Account Values

When downloading the account values, we use the snap function [`GetAccountRange`](https://github.com/ethereum/devp2p/blob/master/caps/snap.md#getaccountrange-0x00). This requests receives:

- rootHash: state_root of the block we're trying to download
- startingHash: Account hash[^4] of the first to retrieve
- limitHash: Account hash after which to stop serving data
- responseBytes: Soft limit at which to stop returning data

This method returns the following

- accounts: List of consecutive accounts from the trie
    - accHash: Hash of the account address (trie path)
    - accBody: Account body in slim format
- proof: List of trie nodes proving the account range

[^4]: All accouns and storages are sent and found throught the hash of their address. Example: the account with address 0xf003 would be found through the 0x26c2...38c1 hash, and would be found before the account with adress 0x0001 whose hash would be 0x49d0...49d5

![request_account_range flowchart](snap_sync/Flow%20-%20Download%20Accounts.png)
