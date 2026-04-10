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

With all three export files ready, run the ethrex migrate command. This
creates a fresh ethrex database and builds the binary trie from the exported
state:

```bash
ethrex --network <network> migrate <preimages.rlp> <snapshot.rlp>
```

For example:

```bash
ethrex --network hoodi migrate preimages.rlp snapshot.rlp
```

The tool will:
1. Create a new ethrex database with genesis state
2. Auto-tune memory mode and flush interval based on available RAM
3. Parse preimages (into HashMaps or sorted flat files depending on mode)
4. Stream the snapshot file in batches, processing entries in parallel:
   - Look up the original address/slot via the preimage map
   - Compute the BLAKE3 tree key
   - Insert into the binary trie
5. Periodically flush the trie to disk and release cached nodes
6. Compute and log the final state root

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
  sorted files with binary search (constant RAM, slower lookups). The trie
  node cache is cleared after each periodic flush to keep memory bounded.
  Use `--fast` to force in-memory mode regardless of available RAM.
- **Code chunks**: The snapshot export contains code hashes but not the
  actual bytecode. A separate `geth db export code` step (using the
  patched Geth fork) provides the raw bytecode needed to populate code
  chunks in the binary trie. Code chunk import is not yet integrated
  into the migration tool.
- **Disk space**: You need space for both export files plus the final ethrex
  database. Plan for ~3x the snapshot size as headroom.
- **Snapshot freshness**: The Geth snapshot corresponds to a specific block.
  After migration, the node will need to sync forward from that block to
  reach the chain head.
- **Flush interval**: The tool auto-tunes how often it flushes the trie to
  disk (between 2M and 20M inserts) based on available RAM. More memory
  allows larger batches, reducing RocksDB write amplification.
