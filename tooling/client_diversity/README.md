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
| `--beacon-url` | `http://localhost:5052` | Beacon node API URL. Credentials (`https://user:pass@host`) and a query string (`?apikey=...`) are passed on to every request. |
| `--client` | `ethrex` | Execution client to chart and estimate the stake of, by name or two-letter code (`geth`/`GE`, `nethermind`/`NM`, `besu`/`BU`, `reth`/`RH`, `erigon`/`EG`, `ethrex`/`EX`, ...). |
| `--bucket` | picked from the range | Time bucket for the chart, e.g. `6h`, `1d`. |
| `--concurrency` | `16` | Parallel block requests. |
| `--no-stake` | off | Skip the stake estimate and its validator download. |

Progress goes to stderr and the report goes to stdout. Example output:

```
$ python3 tooling/client_diversity/client_diversity.py --range 30d
Range: 2026-10-18 01:20 UTC .. 2026-11-17 01:20 UTC (slots 15,455,201..15,671,200, 681 empty)
Total blocks: 215,319
Tagged blocks: 130,677 (60.7%)

Client composition of total blocks:
- Geth: 35.30%
- Nethermind: 11.45%
- Reth: 7.29%
- Besu: 6.02%
- Erigon: 0.33%
- ethrex: 0.30%
- Untagged: 39.31%

ethrex share of total blocks per 1d (first bucket 0.28%, last bucket 0.24%):
0.50% |
      |      #
      |      #               #       #
      |      #     #   #     #     # #   # #         #     #
      |#   # #   # #   # # # # #   # #   # # # #   # # # # # # #
0.25% |# # # # # # # # # # # # #   # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # #   # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
0.00% +------------------------------------------------------------
       2026-10-18                                        2026-11-17

Total active stake: 44,319,955 ETH (791,963 validators)
Estimated stake running on ethrex: 134,821 ETH (lower bound; 655 of 215,319 blocks)
  (222,148 ETH if untagged blocks have the same client mix as tagged ones)
```

## How it works

Consensus clients call the `engine_getClientVersionV1` Engine API method to
learn which execution client they are paired with, and most of them append a
tag to the graffiti of the blocks they propose. The tag uses the two-letter
client codes from the [execution-apis spec](https://github.com/ethereum/execution-apis/blob/main/src/engine/identification.md);
its layout is not part of that spec, and each client shrinks it in its own way
to fit the space left by the user graffiti:

| Form | Example | Meaning |
|------|---------|---------|
| 12 chars | `GE1df3LH57c2` | Geth commit `1df3` + Lighthouse commit `57c2` |
| 8 chars | `EX80PMd0` | ethrex commit `80` + Prysm commit `d0` |
| 4 chars | `EXLS` | ethrex + Lodestar |
| 2 chars | `... GE` | Geth only |

Lighthouse, Prysm and Teku use all four forms. Lodestar uses the 12- and
4-character forms and otherwise appends only its own code (` LS`). Grandine
appends the 12-character form followed by `Grandine/<version>`. Nimbus appends
no tag. Prysm v7.1.4 wrote `PR` instead of `PM` for itself, and local Prysm
builds write `Loca` in place of their commit; both are accepted.

The script accepts only these shapes, with known client codes and lowercase
hex commits. The tag can be a standalone word anywhere in the graffiti. Prysm
glues it to the user graffiti when there is no room for a space, and clients
only shrink it to the lone EL code when the graffiti is nearly full, so those
forms count only in graffiti of 30 bytes or more; otherwise words like
`ANGELS` (`GE` + `LS`) would match. A lone EL code must follow a space or
punctuation, except in a full 32-byte graffiti, where Prysm glues it too
(`...manuallyGE`, but not `...CHANGE`). If several candidates appear, the one
closest to the end wins, since clients append the tag after the user graffiti.

Blocks are fetched with `/eth/v1/beacon/blinded_blocks/{slot}`, or with
`/eth/v2/beacon/blocks/{slot}` when the node doesn't serve blinded blocks; a
404 is an empty slot. A slot that still fails after retries is skipped with a
warning, but if 64 or more fail in a row the node is considered down and the
script exits with an error.

The total stake is the sum of the effective balances of active validators at
the finalized state (`/eth/v1/beacon/states/finalized/validators?status=active`,
a few hundred MB on mainnet, streamed rather than parsed in memory). The stake
on a client is estimated as

```
total active stake * (blocks tagged with the client / total blocks)
```

Proposers are picked with probability proportional to effective balance, so
over a long enough range the share of blocks approximates the share of stake.
A second figure scales by tagged blocks instead of total blocks: the stake on
the client if untagged blocks had the same client mix as tagged ones.

## Caveats

- **About 40% of mainnet blocks carry no client tag.** Nimbus never appends
  one. Lighthouse appends none when graffiti is set on the beacon node, and
  falls back to `Lighthouse/<version>` when the execution client doesn't
  answer `engine_getClientVersionV1`. Prysm only tags since v7.1.4, and most
  clients can turn tagging off. Long user graffiti leaves no room for even the
  shortest form. Untagged blocks are not a random sample, so every client's
  share of total blocks, and the stake estimate, is a lower bound. A few false
  positives are possible the other way, e.g. a nearly full user graffiti that
  happens to end in ` EX`.
- **It measures proposing stake.** Offline validators miss their slots and
  never show up in the sample, so a client's share is of the stake that
  actually proposed blocks.
- **The tag names the proposer's execution client, not the block builder.**
  Most mainnet payloads are built by MEV-boost builders; the tag still
  reflects the client the validator runs.
- **Small clients are noisy.** At 0.3% of blocks, a client gets about 20
  blocks a day, and a single operator can move its share. Longer ranges
  smooth this out, but the stake estimate is an average over the whole range:
  the current stake on a client that is growing is higher than a long range
  suggests. Use the chart to see the trend.
- **The beacon node must hold the blocks of the whole range.** A
  checkpoint-synced Lighthouse only backfills about 5 months of blocks unless
  started with `--genesis-backfill`. If the range starts before the node's
  oldest block, the script warns and uses the available part only. It also
  warns about runs of 64+ empty slots inside the range, which only a node
  missing blocks or a major network outage would show.
- **Speed depends on the node.** Against a local Lighthouse it fetches about
  11,000 slots/s (30 days in ~20 s); against a remote node it is limited by
  network latency, so raise `--concurrency` or run it next to the node. Nodes
  that don't serve blinded blocks are read through `/eth/v2/beacon/blocks`,
  about 150 KB per block instead of about 12 KB, which is much slower over a
  network. The stake estimate downloads every active validator (a few hundred
  MB on mainnet); `--no-stake` skips it.

## Tests

```bash
python3 tooling/client_diversity/test_client_diversity.py
```

The tests cover the graffiti parser with mainnet samples, the streamed stake
sum, the report math, beacon URL handling, and end-to-end runs of the script
against a fake beacon node on localhost (missing history, failed slots, a node
that stops answering, the full-block fallback). They need no real beacon node.
