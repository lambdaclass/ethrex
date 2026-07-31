# EIP-8312 UTXO Frames — PoC devnet

Reproduction for the EIP-8312 devnet described in
[`docs/eip-8312.md`](../../docs/eip-8312.md): a three-EL kurtosis network on which the full
deposit → spend cycle runs, plus a live integration suite.

Three ethrex ELs means agreement between them exercises the payload-build path against the import
paths. It is **not** cross-client interop — no other client implements EIP-8312.

## Prerequisites

- Docker and [kurtosis](https://docs.kurtosis.com/install) (1.20.0 or newer).
- A checkout of the branch that carries EIP-8312.
- Python 3 with `eth-hash` and `eth-keys`, for the test harness.

## 1. Build the EL image

The image tag in `kurtosis.yaml` is `ethrex:eip8312-poc`.

```bash
docker build -t ethrex:eip8312-poc .
```

## 2. Start the network

```bash
kurtosis run --enclave eip8312 github.com/ethpandaops/ethereum-package \
  --args-file scripts/eip8312-devnet/kurtosis.yaml
```

The fork schedule must include gloas — see the comments in `kurtosis.yaml` for why skipping it
produces a chain that charges Amsterdam's repriced costs without Amsterdam's state-gas reservoir.

Note the published EL RPC ports:

```bash
kurtosis enclave inspect eip8312 | grep -A2 "el-.*ethrex"
```

## 3. Activate EIP-8312

EIP-8312 has its own chain-config timestamp, `utxoFramesTime`, which the genesis generator cannot
emit. Patch it into each EL's chain config and restart that EL:

```bash
ACTIVATE=$(( $(date +%s) + 120 ))    # a timestamp comfortably in the future

for c in $(docker ps --filter label=com.kurtosistech.enclave-name=eip8312 \
                     --format '{{.Names}}' | grep '^el-'); do
  docker cp "$c":/network-configs/genesis.json /tmp/g.json
  ACTIVATE=$ACTIVATE python3 -c '
import json, os
g = json.load(open("/tmp/g.json"))
g["config"]["utxoFramesTime"] = int(os.environ["ACTIVATE"])
json.dump(g, open("/tmp/g.json", "w"), indent=2)'
  docker cp /tmp/g.json "$c":/network-configs/genesis.json
  docker restart "$c"
done
```

Patch the JSON on the host, not with `sed` inside the container: the containers have no Python, and
the obvious `sed` anchor does not exist. The EL chain config names this fork **`bogotaTime`**, not
`hezeTime` — `heze` is the consensus-layer name for the same fork, and anchoring on it silently
matches nothing, leaving the field unset while the command reports success.

Filtering containers by the `com.kurtosistech.enclave-name` label also matters: kurtosis container
names share the `el-N-<el>-<cl>` prefix across enclaves and differ only by a UUID suffix, so a
name-prefix filter will happily patch a different network's ELs. That failure presents as the
feature simply not working.

Two things about this procedure are the point rather than a workaround:

- **It is state-preserving.** With a future timestamp every already-produced block re-executes
  byte-identically, so a running chain adopts EIP-8312 with no new genesis. This is the same path a
  production upgrade takes.
- **It must be applied to every EL before the timestamp arrives.** `utxoFramesTime` is invisible to
  ForkId, so there is no peer-level protection at the boundary. An EL left un-patched degrades by
  rejecting UTXO transactions — it never rewrites history — but it will not build UTXO blocks.

Confirm activation — the vault predeploy appears at `0x8312` once the timestamp passes:

```bash
curl -s -X POST http://127.0.0.1:31403 -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"eth_getCode",
           "params":["0x0000000000000000000000000000000000008312","latest"]}'
# → 76 bytes of runtime code
```

## 4. Run the integration suite

```bash
./utxo_itest.py \
  --rpc   http://127.0.0.1:31403 \
  --peers http://127.0.0.1:31410,http://127.0.0.1:31417 \
  --funder-key <prefunded devnet key>
```

`--funder-key` is one of the ethereum-package's publicly documented prefunded dev keys. `--peers`
enables the cross-EL agreement check. `--only NAME` runs a single case.

Each case needs a real chain — the block-end openings root, the ring proof, the spent bit,
settlement and multi-EL agreement do not exist until blocks are produced. Cases already covered by
unit tests are deliberately not repeated:

| Case | What would break it |
|------|--------------------|
| Deposit creates a UTXO and the block-end root commits to it | The harness recomputes the openings root **from the EIP text**, independently of the client, so a match is agreement rather than a tautology |
| A recipient holding zero ETH spends its UTXO | The headline claim: conservation must fund the fee from the input before any balance check |
| The spent bit is set and the spend cannot be replayed | Durability of the bit across the transaction boundary |
| A forged proof cannot spend | Admission-time proof verification against head state |
| A UTXO is not spendable in its creation block | The root for a block does not exist until that block ends; must stay pooled, not evicted |
| A UTXO cycle vs a plain transfer to a fresh account | The state claim: the UTXO payee has no account leaf; the plain transfer's recipient does |
| All three ELs agree after UTXO activity | Build path and import paths must produce identical state |

The suite is re-runnable against a chain that already has UTXO history: throwaway accounts are
derived per run. Fixed seeds would pass once and then fail on a chain where those addresses already
exist, which is indistinguishable from a real regression.

## Not covered here

- **Batch-proof spends.** The batch path is implemented but only reachable on a chain older than
  `BATCH_SIZE` (8,192) blocks, so it needs a long-running network.
- **Cross-client interop.** Requires a second client implementing EIP-8312; none exists.
- **Token-carrying openings.** Deferred by the draft.

## Related

- [`docs/eip-8312.md`](../../docs/eip-8312.md) — implementation notes and divergences from the spec.
- [`docs/eip-8312-use-cases.md`](../../docs/eip-8312-use-cases.md) — where this feature earns its keep.
- [`../hegota-devnet/NOTES-FOR-8312-AUTHOR.md`](../hegota-devnet/NOTES-FOR-8312-AUTHOR.md) — spec
  feedback for the EIP authors.
- [`../hegota-devnet/frametx.py`](../hegota-devnet/frametx.py) — the frame-transaction encoder the
  harness imports.
