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

Generalized Flowchart of snapsync

![Flow - Snap Sync](snap_sync/Flow%20-%20Snap%20Sync.png)

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

#### API

When downloading the account values, we use the snap function [`GetAccountRange`](https://github.com/ethereum/devp2p/blob/master/caps/snap.md#getaccountrange-0x00). This requests receives:

- rootHash: state_root of the block we're trying to download
- startingHash: Account hash[^4] of the first to retrieve
- limitHash: Account hash after which to stop serving data
- responseBytes: Soft limit at which to stop returning data

[^4]: All accounts and storages are sent and found throught the hash of their address. Example: the account with address 0xf003 would be found through the 0x26c2...38c1 hash, and would be found before the account with adress 0x0001 whose hash would be 0x49d0...49d5

This method returns the following

- accounts: List of consecutive accounts from the trie
    - accHash: Hash of the account address (trie path)
    - accBody: Account body in slim format
- proof: List of trie nodes proving the account range

The proof is a merkle proof of the accounts provided, and the root of that merkle must equal to the rootHash.
In ethrex this is checked by the `verify_range` function.

```rust
/// Verifies that the key value range belongs to the trie with the given root given the edge proofs for the range
/// Also returns true if there is more state to be fetched (aka if there are more keys to the right of the given range)
pub fn verify_range(
    root: H256,
    left_bound: &H256,
    keys: &[H256],
    values: &[ValueRLP],
    proof: &[Vec<u8>],
) -> Result<bool, TrieError>
```

We know we have finished a range if the last of the accounts downloaded is to the right of the bound we have set to the request, or if the `verify_range` returns true.

#### Dump to file

To avoid having all of the accounts in memory, when their size in memory exceeds 64MiB we dump them to a new file. 
These files are a subfolder of the datadir folder called `"account_state_snapshots"`.
For an optimization for faster insertion, these are stored ordered in the RocksDB sst file format.

#### Flowchart

![request_account_range flowchart](snap_sync/Flow%20-%20Download%20Accounts.png)

### Insertion of Accounts

The sst files in the `"account_state_snapshots"` subfolder are ingested into a RocksDB database. This provides an ordered array that is used for insertion.

[More detailed documentation foun in sorted_trie_insert.md](../../internal/l1/sorted_trie_insert.md).

### Downloading Storage Slots

The download of the storage slots is conceptually similar to the download of accounts, but very different in implementation. The method uses the snap function [`GetStorageRanges`](https://github.com/ethereum/devp2p/blob/master/caps/snap.md#getstorageranges-0x02). This requests has the following parameters:

- rootHash: state_root of the block we're trying to download
- accountHashes: List of all the account address hashes of the storage tries to serve
- startingHash: Storage slot hash of the first to retrieve
- limitHash: Storage slot hash after which to stop serving
- responseBytes: Soft limit at which to stop returning data

The parameters `startingHash` and `limitHash` are only read when `accountHashes` is a single account.

The return is similar to the one from `GetAccountRange`, but with multiple results, one for each account provided, with the following parameters:

- slots: List of list of consecutive slots from the trie (one list per account)
    - slotHash: Hash of the storage slot key (trie path)
    - slotData: Data content of the slot
- proof: List of trie nodes proving the slot range

From these parameters, there is a couple of difficulties that pop up.

- We need to know which accounts have storage that needs to be downloaded
- We need to know what storage root each account has to be able to verify it

To solve these issues we take two action:
- Before we download the storage slots we ensure that the state trie is in a consistent complete state. This is accomplished by doing the insertion of accounts step first and then healing the trie. If during the storage slot download the pivot becomes stale, we heal the trie again with the new pivot, to keep the trie up to date.
- When inserting the accounts, we grab a list of all the accounts with their storage root. If the account is healed, we marked the storage root as `None`, to indicate we should check in the DB what is the state of the storage root.

#### The time traveling problem

During the development of snap sync we found a recurring problem, the time traveling problem. In hash based, when a node returned to a previous state, those accounts wouldn't be redownloaded, as it would just return to the previous hash which was present in the database.
 **Any algorithm that depended on updating accounts during healing would fail**.
The alternative is to always go to disk if there is to mark healed accounts as unreliable and go to disk to get the datum.
This may not be a problem in path based again, but should be studied.

#### Repeated Storage Roots

A large amount of the accounts with storage have exactly the same storage as other accounts.[^5] As such, when we are creating tasks for download, it's important to group the tasks by storage root and not download them twice.

[^5]: This may be for a variety of reasons, but the most likely is ERC20 tokens that were deployed and never used.

#### Big Accounts

The storage trie is very unneven distribution of the accounts sizes. Between accounts with a single or two storage slots are around 70% of all accounts with storage trie. And large accounts have more storage slots than accounts slots are present in the state accounts. As such they need to be downloaded with special consideration.

At the beginning of the algorithm, we divide the accounts into chunks of 300 storage roots and their corresponding accounts. We start downloading the storage slots, until we find an account whose storage doesn't fit into a single requests. This will be indicated by the proof field having the data indicating that there are still more nodes to download in that account.

![proofs for missing slots](snap_sync/Snap%20Sync%20Downloading%20Storages%20-%201.png)

When we reach that situation, we chunk the big account based on the "density"[^7] of storage slots we downloaded, following this code to get chunks of 10,000 slots[^6]. We create the tasks to download those intervals, and store all of the intervals in a struct to check when everything for that account was properly download.

[^6]: 10_000 slots is a number chosen without hard data, we should review that number.

[^7]: actually [specific volume](https://en.wikipedia.org/wiki/Specific_volume) (maximum possible slots/actual slots downloaded)

```rust
    // start_hash_u256 is the hash of the address of the last slot
    // slot_count is the amount of slots we have downloaded
    // The division gives us the density (maximum possible slots/actual slots downloaded) 
    // we want chunks of 10.000 slots, so we multiply those two numbers
    let storage_density = start_hash_u256 / slot_count;
    let slots_per_chunk = U256::from(10000);
    let chunk_size = storage_density
        .checked_mul(slots_per_chunk)
        .unwrap_or(U256::MAX);
```

#### Tasks API

```rust
struct StorageTask {
    // Index of the first storage account we want to download
    start_index: usize,
    // Index of the last storage account we want to download (not inclusive)
    end_index: usize,
    // startingHash, used when the task is downloading a single account
    start_hash: H256,
    // end_hash is Some if the task is to download a big task
    end_hash: Option<H256>,
}

struct StorageTaskResult {
    // Index of the first storage account we want to download
    start_index: usize,
    // Slots we have succesfuly downloaded with the hash of the slot + value
    account_storages: Vec<Vec<(H256, U256)>>,
    // Which peer answered the task, used for scoring
    peer_id: H256,
    // Index of the first storage account we still need to download
    remaining_start: usize,
    // Index of the last storage account we still need to download
    remaining_end: usize,
    // remaining_hash_range[0] is the hash of the last slot we downloaded (so we need to download starting from there)
    // remaining_hash_range[1] is the end_hash from the original StorageTask
    remaining_hash_range: (H256, Option<H256>),
}
```

#### Big Accounts Flow

![Big Account logic](snap_sync/Flow%20-%20Big%20Account%20Logic.png)

#### Retry Limit

Currently, if ethrex has been downloading storages for more than 2 pivots, the node will stop trying to download storage, and fallback to heal (fast sync) all the storage accounts that were still missing downloads. This stops ethrex hanging due to a problem but it indicates that we still have bugs in our storage slots download.

### Downloading Bytecodes

Whenever an account is download or healed we check if the code is not empty. If it isn't, we store it for future download. This is added to a list, and when the list grows beyond a certain size it is written to disk. After the healing is done and we have a complete state and storage tree, we start with the download of bytecodes, chunking them to avoid memory overflow.

### Forkchoice update

Once the entire files are downloaded, we change the snap sync mode to full, and we do an `apply_forkchoice` to mark that as the last pivot as the last block.
