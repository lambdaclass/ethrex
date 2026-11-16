# Client diversity from block graffiti

`client_diversity.py` estimates which execution clients are behind the blocks
proposed in a time range. It reads every block in the range from a beacon
node, parses the client tag that consensus clients append to the graffiti, and
reports:

- the share of blocks per execution client,
- a chart of how one client's share (ethrex by default) changed over the range,
- the stake estimated to run on that client.

It only needs Python 3.9+ (stdlib) and a beacon node API.

## Usage

```bash
python3 tooling/client_diversity/client_diversity.py --range 30d
```

| Flag | Default | Description |
|------|---------|-------------|
| `--range` | `30d` | Time period ending at the latest finalized slot (`s`, `m`, `h`, `d`, `w` units, e.g. `12h`, `2w`). |
| `--beacon-url` | `http://localhost:5052` | Beacon node API URL. |
| `--client` | `ethrex` | Execution client to chart and estimate the stake of (`geth`, `nethermind`, `besu`, `reth`, `erigon`, `ethrex`, ...). |
| `--bucket` | picked from the range | Time bucket for the chart, e.g. `6h`, `1d`. |
| `--concurrency` | `16` | Parallel block requests. |

Progress goes to stderr and the report goes to stdout. Example output:

```
$ python3 tooling/client_diversity/client_diversity.py --range 30d
Range: 2026-10-17 23:06 UTC .. 2026-11-16 23:05 UTC (slots 15454529..15670528, 682 empty)
Total blocks: 215,318
Tagged blocks: 130,650 (60.7%)

Client composition of total blocks:
- Geth: 35.29%
- Nethermind: 11.45%
- Reth: 7.29%
- Besu: 6.01%
- Erigon: 0.33%
- ethrex: 0.30%
- Untagged: 39.32%

ethrex share of total blocks per 1d (first bucket 0.28%, last bucket 0.24%):
0.50% |
      |
      |      #         #                             #
      |    # #         #     # #   # #               #     # #
      |#   # # #   # # #   # # #   # # # # # # #     #   # # #
0.25% |#   # # # # # # # # # # #   # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # #   # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
0.00% +------------------------------------------------------------
       2026-10-17                                        2026-11-16

Total active stake: 44,320,870 ETH (792,393 validators)
Estimated stake running on ethrex: 134,825 ETH
```

## How it works

Consensus clients call the `engine_getClientVersionV1` Engine API method to
learn which execution client they are paired with, and append a tag to the
graffiti of the blocks they propose using the two-letter client codes from the
execution-apis spec. The tag shrinks to fit the space left by the user
graffiti:

| Form | Example | Meaning |
|------|---------|---------|
| 12 chars | `GE1df3LH57c2` | Geth commit `1df3` + Lighthouse commit `57c2` |
| 8 chars | `EX80PMd0` | ethrex commit `80` + Prysm commit `d0` |
| 4 chars | `EXLS` | ethrex + Lodestar |
| 2 chars | `... GE` | Geth only |

The script recognizes the tag as a standalone word anywhere in the graffiti,
glued to the end of the user graffiti with no separator (seen on mainnet), or
as a lone execution client code at the end. Blocks are fetched with
`/eth/v1/beacon/blinded_blocks/{slot}` (falling back to
`/eth/v2/beacon/blocks/{slot}`); a 404 is an empty slot.

The total stake is the sum of the effective balances of active validators at
the finalized state (`/eth/v1/beacon/states/finalized/validators?status=active`,
a few hundred MB on mainnet, streamed rather than parsed in memory). The stake
on a client is estimated as

```
total active stake * (blocks tagged with the client / total blocks)
```

Proposers are picked with probability proportional to effective balance, so
over a long enough range the share of blocks approximates the share of stake.

## Caveats

- **About 40% of mainnet blocks carry no client tag.** Empty graffiti,
  Nimbus' and Lighthouse's version-only defaults, and custom graffiti that
  fills all 32 bytes leave no room for it. Untagged blocks are not a random
  sample, so every client's share of total blocks, and the stake estimate,
  is a lower bound.
- **The tag names the proposer's execution client, not the block builder.**
  Most mainnet payloads are built by MEV-boost builders; the tag still
  reflects the client the validator runs.
- **Small clients are noisy.** At 0.3% of blocks, a client gets about 20
  blocks a day, and a single operator can move its share. Longer ranges
  smooth this out, but the stake estimate is an average over the whole range:
  the current stake on a client that is growing is higher than a long range
  suggests. Use the chart to see the trend.
- **The beacon node must hold the blocks of the whole range.** A
  checkpoint-synced Lighthouse only backfills about 5 months of blocks by
  default. If the range starts before the node's oldest block, the script
  warns and uses the available part only.
- **Speed depends on the node.** Against a local Lighthouse it fetches about
  11,000 slots/s (30 days in ~20 s); against a remote node it is limited by
  network latency, so raise `--concurrency` or run it next to the node.
