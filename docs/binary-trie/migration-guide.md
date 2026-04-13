# Migrating to Binary Trie from a Geth Snapshot

This guide explains how to bootstrap an ethrex binary trie node from a Geth
snapshot, avoiding a full chain replay from genesis.

## Overview

The binary trie (EIP-7864) uses BLAKE3-based keys, while Ethereum's state is
indexed by keccak256 hashes. To build the binary trie we need:

1. **The actual state** (account balances, nonces, storage values) -- from a
   Geth snapshot export.
2. **Keccak preimages** (mapping keccak256(x) back to x) -- from a Geth
   preimage export. These let us recover original addresses and storage keys
   so we can compute the BLAKE3 tree keys.

Both exports use Geth's `gethdbdump` format and are produced with the
`geth db export` command.

### Why preimages alone are not enough

The preimage file only contains keccak hash mappings (keccak(x) -> x). It
tells you the original address or storage key behind each hash, but it does
not contain the actual state values (balances, nonces, code hashes, storage
slot values). Without the snapshot export there is nothing to insert into the
binary trie -- you would know *where* each account lives but not *what* it
contains.

Conversely, the snapshot alone is not enough either. The snapshot keys are
keccak hashes, and the binary trie needs the original addresses to compute
BLAKE3 tree keys. Keccak is a one-way function, so you cannot reverse the
hash without the preimage database.

## Prerequisites

- A Geth snapshot **with preimages enabled**. The snapshot must have been
  produced by a Geth node running with `--cache.preimages`.
- A `geth` binary with the `code` exporter. Upstream Geth (v1.17+) supports
  `preimage` and `snapshot` exports but not `code`. Use the patched fork at
  https://github.com/edg-l/go-ethereum/tree/feat/export-code which adds
  `geth db export code`.
- Enough disk space for the exports (~10 GB for preimages, ~25 GB for
  snapshot on Hoodi; mainnet will be larger).

## Step 1: Obtain a Geth snapshot

Pre-built Geth snapshots with `--cache.preimages` enabled are available from
ethPandaOps:

**https://ethpandaops.io/data/snapshots/**

Download the snapshot for your target network (mainnet, hoodi, etc.). Look
for a Geth snapshot whose **Extra Args** include `--cache.preimages`. The
download is a compressed tar archive:

```bash
wget https://snapshots.ethpandaops.io/<network>/geth/<block>/snapshot.tar.zst
```

Extract it:

```bash
tar --zstd -xf snapshot.tar.zst
```

This gives you a Geth datadir (typically containing `chaindata/`, `triedb/`,
`nodes/`, etc.).

## Step 2: Export preimages

Export the keccak preimage database from the Geth datadir:

```bash
geth --datadir <geth_datadir> --<network> db export preimage preimages.rlp
```

For example, on Hoodi:

```bash
geth --datadir ~/data/geth-data --hoodi db export preimage preimages.rlp
```

This takes ~7 minutes on Hoodi (~10 GB output, ~138M preimages).

## Step 3: Export contract code

Export the contract bytecode from the Geth database:

```bash
geth --datadir <geth_datadir> --<network> db export code code.rlp
```

For example:

```bash
geth --datadir ~/data/geth-data --hoodi db export code code.rlp
```

Each entry is keyed by `"c" + keccak256(code)` with the raw bytecode as the
value. This is needed to populate code chunks in the binary trie (the
snapshot export only contains code hashes, not the actual bytecode).

## Step 4: Export snapshot state

Export the account and storage state from the Geth snapshot (unchanged from
upstream Geth):

```bash
geth --datadir <geth_datadir> --<network> db export snapshot snapshot.rlp
```

For example:

```bash
geth --datadir ~/data/geth-data --hoodi db export snapshot snapshot.rlp
```

This takes ~15 minutes on Hoodi (~24 GB output). The file contains all
account states (nonce, balance, code hash) and storage slot values.

## Step 5: Run the migration

With all three export files ready, run the ethrex migrate command. You must
specify `--at-block` with the block number the snapshot state corresponds to
(the state is the result of executing that block). This is typically shown
on the ethPandaOps snapshot page or in the Geth log when the snapshot was
taken.

```bash
ethrex --network <network> migrate <preimages.rlp> <snapshot.rlp> --code <code.rlp> --at-block <BLOCK_NUMBER>
```

For example:

```bash
ethrex --network hoodi migrate preimages.rlp snapshot.rlp --code code.rlp --at-block 3456789
```

The tool runs in two phases:

**Phase 1 - Collection**: Streams the snapshot file, decodes entries in
parallel (preimage lookups, RLP decode, BLAKE3 key computation), and writes
all `(tree_key, value)` pairs to a temporary RocksDB column family. No trie
is built during this phase.

**Phase 2 - Build**: Iterates the collected entries in sorted key order and
constructs the binary trie in a single left-to-right pass. Only a small
"right spine" of internal nodes is kept in memory (~25 KB). Completed
subtrees are flushed to disk immediately.

After both phases, the block number is recorded so the node knows where to
resume syncing from.

Progress is logged every 5 seconds with percentage, entry counts, and
throughput.

## Step 6: Start the node

After migration completes, start the node normally. It will resume from the
migrated state:

```bash
ethrex --network <network>
```

## Notes

- **Memory usage**: The tool auto-tunes based on available RAM
  (`/proc/meminfo`). If enough memory is available it loads preimages into
  HashMaps for maximum throughput. Otherwise it falls back to memory-mapped
  sorted files with binary search (constant RAM, slower lookups).
  Use `--fast` to force in-memory mode regardless of available RAM.
  The build phase uses very little memory (only the open trie spine).
- **Code chunks**: The snapshot export contains code hashes but not the
  actual bytecode. A separate `geth db export code` step (using the
  patched Geth fork) provides the raw bytecode needed to populate code
  chunks and code_size in the binary trie.
- **Disk space**: You need space for the export files, the temporary
  collection CF (~same size as snapshot), plus the final ethrex database.
  Plan for ~3x the snapshot size as headroom.
- **Snapshot freshness**: The Geth snapshot corresponds to a specific block
  (provided via `--at-block`). After migration, the node will need to sync
  forward from that block to reach the chain head.
- **Performance**: On Hoodi (~33M accounts, ~268M storage slots), the
  collection phase runs at ~30 MB/s and the build phase processes ~5M
  entries/sec. Total migration time is ~30 minutes on a server with 32 GB
  RAM and SSD storage.
