# Prune Blocks

This tool can be used to reduce the DB size of the node by removing old blocks and their asociated data. Note that this is counter-spec and will hinder the node's ability to provide data to other nodes. It also does not performe state prunning.

## Usage

The tool takes two optional arguments:
    *`datadir`: The path to the DB location, will use the default one if not provided
    *`blocks_to_keep`: The amount of latest blocks that will be kept in the DB. This value must be at least 128 and lower than the current amount of blocks in the chain.

And should be ran like this:

```bash
cargo run --release -- --datadir DATADIR --blocks-to-keep BLOCKS_TO_KEEP
```
