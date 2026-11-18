# Client diversity from block graffiti

`client_diversity.py` estimates which execution clients are behind the blocks
proposed in a time range. It reads every block in the range from a beacon
node, parses the client tag that most consensus clients append to the
graffiti, and reports:

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
| `--range` | `30d` | Time period ending at the latest finalized slot: a number and `s`, `m` (minutes), `h`, `d` or `w`, e.g. `12h`, `2w`. |
| `--beacon-url` | `http://localhost:5052` | Beacon node API URL. Credentials (`https://user:pass@host`) and a query string (`?apikey=...`) are passed on to every request. |
| `--client` | `ethrex` | Execution client to chart and estimate the stake of, by name or two-letter code (`geth`/`GE`, `nethermind`/`NM`, `besu`/`BU`, `reth`/`RH`, `erigon`/`EG`, `ethrex`/`EX`, ...). |
| `--bucket` | picked from the range | Time bucket for the chart, e.g. `6h`, `1d`. |
| `--concurrency` | `16` | Parallel block requests. |
| `--no-stake` | off | Skip the stake estimate and its validator download. |

Progress goes to stderr. Example report (stdout) from a mainnet run on
2026-11-18:

```
$ python3 tooling/client_diversity/client_diversity.py --range 30d
Range: 2026-10-19 16:03 UTC .. 2026-11-18 16:03 UTC (slots 15,466,817..15,682,816, 674 empty)
Total blocks: 215,326
Tagged blocks: 130,612 (60.7%)

Client composition of total blocks:
- Geth: 35.46%
- Nethermind: 11.22%
- Reth: 7.29%
- Besu: 6.05%
- Erigon: 0.33%
- ethrex: 0.31%
- Untagged: 39.34%

ethrex share of total blocks per 1d (first bucket 0.32%; last bucket 0.29%):
0.50% |
      |                                          #
      |                          #     #         #
      |  # #   #   #     # #     # #   #   #     #   #   #     #
      |# # #   # # # #   # #     # #   #   #     #   # # #   # # #
0.25% |# # #   # # # # # # #   # # #   # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
      |# # # # # # # # # # # # # # # # # # # # # # # # # # # # # #
0.00% +------------------------------------------------------------
       2026-10-19                                        2026-11-18

Total active stake: 44,343,568 ETH (786,756 validators)
Estimated stake running on ethrex: 136,948 ETH (tagged blocks only, so likely an underestimate; 665 of 215,326 blocks)
  (225,772 ETH if untagged blocks have the same client mix as tagged ones)
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
appends the 12-character form only when it fits, then `Grandine/<version>` if
there is room. Prysm v7.1.4 wrote `PR` instead of `PM` for itself, and local
Prysm builds write `Loca` (`Lo` in the 8-character form) in place of their
commit; both are accepted.

The script accepts only these shapes, with known client codes and lowercase
hex commits:

- A tag separated from the rest of the graffiti counts anywhere in it.
- A tag glued to the preceding text counts only if it ends a full 32-byte
  graffiti: Prysm and Teku glue it on when it exactly fills the remaining space.
- A lone EL code counts only at the end of a graffiti of 31 bytes or more,
  after a space or punctuation. In a full 32-byte graffiti it may also be
  glued, but not after a capital letter (`...manuallyGE` counts, `...CHANGE`
  does not).
- If several candidates appear, the one closest to the end wins, since clients
  append the tag after the user graffiti.

Requiring a (nearly) full graffiti for the short and glued forms keeps words
like `ANGELS` (`GE` + `LS`) from counting.

Blocks are fetched with `/eth/v1/beacon/blinded_blocks/{slot}`, or with
`/eth/v2/beacon/blocks/{slot}` when the node doesn't serve blinded blocks; a
404 is an empty slot. A slot that still fails after retries is skipped with a
warning. If 64 slots in a row fail (4 × `--concurrency` when that is larger)
and nothing has succeeded for 3 minutes, the node is considered down and the
script exits with an error; a shorter outage, like a node restart, only skips
slots.

The total stake is the sum of the effective balances of active validators at
the finalized state (`/eth/v1/beacon/states/finalized/validators?status=active`,
about 400 MB on mainnet, streamed rather than parsed in memory; `--no-stake`
skips it). The stake on a client is estimated as

```
total active stake * (blocks tagged with the client / total blocks)
```

Proposers are picked with probability proportional to effective balance, so
over a long enough range the share of blocks approximates the share of stake.
A second figure scales by tagged blocks instead of total blocks: the stake on
the client if untagged blocks had the same client mix as tagged ones.

## Caveats

- **About 40% of mainnet blocks carry no client tag.** Nimbus never appends
  one. Lighthouse appends none when graffiti is set on the beacon node, or
  when the execution client doesn't answer `engine_getClientVersionV1` (it
  then uses the validator's graffiti, or `Lighthouse/<version>` if there is
  none). Prysm only tags since v7.1.4, and every client that tags can turn it
  off. Long user graffiti leaves no room for even the shortest form.
- **Shares are underestimates.** Untagged blocks count toward no client, so
  every client's share of total blocks, and the stake estimate, is a lower
  bound (up to sampling noise and the rare false positive, e.g. a nearly full
  user graffiti that happens to end in ` EX`). Untagged blocks are not a
  random sample either, so the second figure, which assumes they have the same
  client mix as tagged ones, is a rough guess, not an upper bound.
- **It measures proposing stake.** Offline validators miss their slots and
  never show up in the sample, so a client's share is of the stake that
  actually proposed blocks; the ETH estimate applies that share to all active
  stake.
- **The tag names the proposer's execution client, not the block builder.**
  Most mainnet payloads are built by MEV-boost builders; the tag still
  reflects the client the validator runs.
- **Small clients are noisy.** At 0.3% of blocks, a client gets about 20
  blocks a day, and a single operator can move its share. Longer ranges
  smooth this out, but the stake estimate is an average over the whole range:
  the current stake on a client that is growing is higher than a long range
  suggests. Use the chart to see the trend. The share is also applied to
  today's total stake, so changes in total stake during a long range add
  some error.
- **The beacon node must hold the blocks of the whole range.** A
  checkpoint-synced Lighthouse only backfills about 5 months of blocks unless
  started with `--genesis-backfill`. If the range starts before the node's
  oldest block, the script warns and uses the available part only. It also
  warns about runs of 64+ empty slots inside the range, which only a node
  missing blocks or a major network outage would show.
- **Speed depends on the node.** Against a local Lighthouse it fetched about
  6,000 slots/s (30 days in under a minute, including the validator
  download, on a 12-core host). Against a remote node it is limited by network latency, so raise `--concurrency` or run it next to the
  node. Full blocks, used when the node doesn't serve blinded ones, are about
  150 KB each instead of about 12 KB, which is much slower over a network.

## Tests

```bash
python3 tooling/client_diversity/test_client_diversity.py
```

The tests cover the graffiti parser with mainnet samples, the streamed stake
sum, the report math, beacon URL handling, and end-to-end runs of the script
against a fake beacon node on localhost (missing history, failed slots, a node
that stops answering, the full-block fallback). They need no real beacon node.
