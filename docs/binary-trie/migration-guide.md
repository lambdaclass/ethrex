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

## Prerequisites

- A Geth snapshot **with preimages enabled**. The snapshot must have been
  produced by a Geth node running with `--cache.preimages`.
- The `geth` binary (v1.17+) installed locally.
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

## Step 3: Export snapshot state

Export the account and storage state from the Geth snapshot:

```bash
geth --datadir <geth_datadir> --<network> db export snapshot snapshot.rlp
```

For example:

```bash
geth --datadir ~/data/geth-data --hoodi db export snapshot snapshot.rlp
```

This takes ~15 minutes on Hoodi (~24 GB output). The file contains all
account states (nonce, balance, code hash) and storage slot values.

## Step 4: Run the migration

With both export files ready, run the ethrex migrate command. This creates a
fresh ethrex database and builds the binary trie from the exported state:

```bash
ethrex --network <network> migrate <preimages.rlp> <snapshot.rlp>
```

For example:

```bash
ethrex --network hoodi migrate preimages.rlp snapshot.rlp
```

The tool will:
1. Create a new ethrex database with genesis state
2. Load all preimages into memory (maps keccak hashes to original keys)
3. Stream the snapshot file, and for each account/storage entry:
   - Look up the original address/slot via the preimage map
   - Compute the BLAKE3 tree key
   - Insert into the binary trie
4. Compute and log the final state root
5. Flush the binary trie to disk

Progress is logged every 5 seconds with percentage, entry counts, and
throughput.

## Step 5: Start the node

After migration completes, start the node normally. It will resume from the
migrated state:

```bash
ethrex --network <network>
```

## Notes

- **Memory usage**: The preimage map is held in memory during migration.
  For Hoodi this is ~8-10 GB. For mainnet expect ~40-50 GB. Ensure your
  machine has enough RAM.
- **Code chunks**: The snapshot export contains code hashes but not the
  actual bytecode. Code chunks in the binary trie are populated from
  genesis contracts only. Full code migration requires a separate code
  import step (not yet implemented).
- **Disk space**: You need space for both export files plus the final ethrex
  database. Plan for ~3x the snapshot size as headroom.
- **Snapshot freshness**: The Geth snapshot corresponds to a specific block.
  After migration, the node will need to sync forward from that block to
  reach the chain head.
