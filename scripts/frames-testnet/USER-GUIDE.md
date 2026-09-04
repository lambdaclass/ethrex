# Frames testnet — user guide

A public test network running **EIP-8141 frame transactions and nothing else** on
[ethrex](https://github.com/lambdaclass/ethrex). Test ETH has no value.

This is deliberately narrower than the Hegotá testnet, which runs EIP-8141 alongside
EIP-8250, EIP-8272 and FOCIL. Here a failure can only be a frame-transaction failure,
which is what makes the chain useful for implementers.

Anyone may sync, peer and transact without permission. Only **validator entry** is
gated; see "Become a validator" below.

> The zone is `frames.ethrex.xyz`. Five names exist: `rpc1`, `rpc2`, `rpc3`, `dora` and
> `faucet`. There is no `artifacts` name and the apex has no address record, so the
> artifact bundle is served from a path under the faucet host rather than a host of its
> own. Add an `artifacts` record and it can move, with the path left as a redirect.

## Network details

| | |
| --- | --- |
| Chain ID / network ID | `81410` |
| Seconds per slot | 6 |
| Block gas limit | 200,000,000 |
| Execution client | ethrex, `frames-devnet-0` build |
| Consensus client | prysm (`ethpandaops/prysm-beacon-chain:glamsterdam-devnet-8`) |
| RPC | `https://rpc1.frames.ethrex.xyz` (also `rpc2`, `rpc3`) |
| Explorer | `https://dora.frames.ethrex.xyz` |
| Faucet | `https://faucet.frames.ethrex.xyz` |
| Artifact bundle | `https://faucet.frames.ethrex.xyz/artifacts` — every file below, plus `MANIFEST.txt` with a sha256 per file |
| Bootnodes | `https://faucet.frames.ethrex.xyz/bootnodes` |
| Deposit contract | `0x00000000219ab540356cBB839Cbe05303d7705Fa` |
| Deposit gater | `0x00000000a11acc355c0de0000a11acc355c0de00` |

Chain ID `81410` is unclaimed in the `chainid.network` registry, is deliberately not the
kurtosis default `3151908` that every local devnet collides on, and is deliberately not
`8141` — the Hegotá testnet already runs on that, and the two are hosted together, so a
shared id would let a node built for one dial and be dialed by the other.

### Fork schedule

| Fork | Epoch | Offset from genesis |
| --- | --- | --- |
| Fulu / Osaka | 0 | genesis |
| Gloas / Amsterdam | 1 | +192s |
| Heze / frames (EIP-8141) | 2 | +384s |

The consensus layer names the last fork `heze`; the execution genesis names the same
timestamp `bogotaTime`. ethrex reads `hegotaTime`, `hezeTime` and `bogotaTime` as
aliases for one field. Only EIP-8141 activates at it on this chain — the name is
inherited from the generator's fork table, not a statement that the Hegotá bundle is
present.

### Predeploys

| Address | Contents |
| --- | --- |
| `0x0000000000000000000000000000000000008141` | `EXPIRY_VERIFIER`, 26 bytes |

Installed by the client at the Hegotá boundary; it needs no genesis entry.

The Hegotá testnet also carries `NONCE_MANAGER` at `0x…8250` and `RECENT_ROOT_ADDRESS`
at `0x…8272`. **Neither exists here** — `eth_getCode` returns empty at both, because
EIP-8250 and EIP-8272 are not active on this chain.

## Connect a wallet

Add a custom network with the chain ID and RPC URL above. The currency symbol is ETH
and it is worthless.

The three RPC hostnames are three different nodes, so comparing a block *hash* between
`rpc1` and `rpc2` is a real agreement check and not the same node answering twice.

They serve `eth`, `net`, `web3` and `txpool`. `debug` and `admin` are not served here —
the nodes themselves answer both, and the guard in front of them is what withholds them,
so run your own node from the bundle if you need them, where they are yours to enable.

There is **no `ethrex` namespace on this deployment**, so there is no way to dry-run a
frame transaction against a public node: this build rejects `--http.api=ethrex` as an
unknown namespace, and `ethrex_simulateFrameTransaction` returns *Method not found*.
Simulate against a local node instead.

**Estimate gas. Never hardcode 21,000.** Under EIP-8037 state growth is charged
alongside execution, out of the same limit an ordinary transaction carries, so the
historical 21,000 is never enough here. Measured on this chain:

| Transfer | Gas |
| --- | --- |
| to an account that already exists | 21,165 |
| to an address that does not exist yet | 204,600 (21,000 execution + 183,600 `NEW_ACCOUNT`) |

The failure is easy to misread. The transaction is mined, the receipt says `status: 0`,
and `gasUsed` equals the limit exactly — that is the out-of-gas, not a revert in the
recipient. The same applies to any call that writes new state: an EIP-7002 exit request
costs about 282,000 gas on this chain, and 200,000 is not enough.

## Get test ETH

Use the faucet. It rate-limits per IP and per recipient, and refuses recipients that
are already rich.

## Join the network

You need both an execution client and a consensus client. **An execution client alone
is not enough**: with nothing driving fork choice it will peer and then sit at genesis,
logging `No messages from the consensus layer`.

Fetch the artifact bundle first. A consensus client consumes the directory as a whole,
so a missing file fails at startup rather than at first use.

```
mkdir frames-testnet && cd frames-testnet
for f in genesis.json genesis.ssz config.yaml \
         bootnodes.txt bootnodes-enr.txt bootnodes-cl.txt \
         deposit_contract.txt deposit_contract_block.txt \
         deposit_contract_block_hash.txt genesis_validators_root.txt; do
  curl -fsSLO "https://faucet.frames.ethrex.xyz/artifacts/$f"
done
```

**Nethermind:** `chainspec.json` in the bundle is ready to use. It is the genesis
generator's output with one key removed: the generator writes `eip7805TransitionTimestamp`
(FOCIL) into every Heze chainspec alongside `eip8141TransitionTimestamp`, and this chain
does not run FOCIL. Fed the generator's file, Nethermind schedules FOCIL at the fork block,
expects inclusion-list engine methods and FOCIL-shaped payloads from then on, and stalls at
exactly that block (block 45) — headers arrive, bodies and state never validate. Genesis and
accounts are untouched, so the genesis hash is the same one every other client computes.

```
curl -fsSLO https://faucet.frames.ethrex.xyz/artifacts/chainspec.json
nethermind --Init.ChainSpecPath=chainspec.json ...
```

That removes the first blocker; a second one lives in the client build. This chain's
frame transactions use the EIP-8141 envelope as of execution-specs#3396 (2026-08-20): seven
top-level fields with the three fees nested in one `fees` list, and each frame's gas as a
two-element `[execution, state]` list. A Nethermind built before that change decodes three
flat fee fields and a scalar frame gas limit, and rejects every payload carrying a frame
transaction with `Unexpected length of integer value` — after headers arrive, before any
body validates. `ethpandaops/nethermind:frames-devnet-0` at `1.40.0-unstable+1b9daf39` is
such a build. Use a build that includes the #3396 envelope, or expect to stall at the first
frame transaction.

The three bootnode files are also served live as JSON, so peers can be checked or
re-fetched without pulling the whole bundle:

```
curl -s https://faucet.frames.ethrex.xyz/bootnodes
{"el": ["enode://…"], "el_enr": ["enr:…"], "cl": ["enr:…"]}
```

`el` and `el_enr` name the same three execution nodes. Pass `el` to `--bootnodes`;
`el_enr` is for a client that seeds discv5 from a record, and it is the stricter of the
two, because an ENR is signed by the node over the address it advertises while an enode
is just an address this deployment writes by hand.

Execution layer:

```
ethrex --network genesis.json \
       --bootnodes "$(paste -sd, bootnodes.txt)" \
       --nat.extip <your public IP> \
       --syncmode full
```

`--nat.extip` is what the node advertises in discovery and in its ENR. `--p2p.addr` is
the bind address and is **not** a substitute: a node that omits `--nat.extip` advertises
whatever local address it found, and no external peer can dial back.

Consensus layer, pointed at the execution client's engine port. **Use checkpoint sync**
— see below for why:

```
prysm-beacon-chain \
  --datadir=./beacon \
  --chain-config-file=config.yaml \
  --genesis-state=genesis.ssz \
  --checkpoint-sync-url=https://checkpoint-sync.frames.ethrex.xyz \
  --execution-endpoint=http://127.0.0.1:8551 \
  --jwt-secret=<path to the jwtsecret your EL generated> \
  --bootstrap-node="$(paste -sd, bootnodes-cl.txt)" \
  --contract-deployment-block=0 --accept-terms-of-use
```

The image is `ethpandaops/prysm-beacon-chain:glamsterdam-devnet-8`, and the requirement
is the opposite of what you would guess: **the consensus client must not implement
Heze.** ethrex on this branch serves `forkchoiceUpdated` V1–V4 and `newPayload` V1–V5.
A Heze-aware client fails before it ever reaches the engine API: it computes the Heze
fork digest from the config's `HEZE_FORK_VERSION` while the network's peers are still on
the Gloas digest (`0x20AED5CC`), so it is dropped at the STATUS handshake and sits at
**zero peers** — the symptom reads as a networking problem, not a fork-choice one. If it
did peer, it would switch to `forkchoiceUpdatedV5` / `newPayloadV6` at the boundary,
which this branch does not serve, and halt the node with
`RequiredMethodUnsupported`. This prysm build is Gloas-only — it ignores
`HEZE_FORK_EPOCH` entirely — so it keeps driving the pair ethrex serves, while the
execution layer activates frames on its own timestamp schedule.

You will see `field HEZE_FORK_VERSION not found in type` in the log. That is the client
telling you it does not know Heze, which is exactly what this chain needs.

**Genesis sync does not work; checkpoint sync does.** Started from genesis, this build
stalls in payload-envelope backfill with `beacon block root ... not found in
forkchoice`, crawls at a few slots a minute, and leaves the execution client stranded a
handful of blocks in. With `--checkpoint-sync-url` the same pair reaches head normally.

The public checkpoint endpoint is `https://checkpoint-sync.frames.ethrex.xyz` — a
read-only proxy of a beacon node's `/eth/*` API. It is a **hostname of its own** rather
than a path under another one because prysm's checkpoint and genesis providers strip the
path from the base URL they are given; for the same reason, pass
`--genesis-state=genesis.ssz` from the bundle instead of `--genesis-beacon-api-url`.

Confirm you are actually following. Check the execution client independently of the
consensus client — a beacon node can sit at head while its execution client is still at
genesis, and only the block number tells you which:

```
cast block-number --rpc-url http://127.0.0.1:8545     # advances, and matches rpc1
cast rpc net_peerCount --rpc-url http://127.0.0.1:8545
curl -s http://127.0.0.1:3500/eth/v1/node/syncing     # want sync_distance 0,
                                                      # is_optimistic false
```

Compare a block hash against `rpc1` at the same height. Matching hashes, not just a
matching height, is what proves agreement.

### Firewall

Discovery needs **both** TCP and UDP. A rule that opens only TCP produces a node that
accepts connections from peers who already know it and is discovered by nobody, which
reads as slow peering rather than as a firewall mistake.

Keep your engine port closed to the internet. Reaching it is equivalent to controlling
your node's head, and the JWT does not protect you if the secret came from a shared
kurtosis deployment.

## Become a validator

Anyone may sync and transact. Validator entry is permissioned by a token that the
**depositing address** must hold.

`<DEPOSITOR>` is the address that *sends* the deposit transaction — not the withdrawal
address and not the validator public key. This is the most common mistake.

### 1. Ask for a token

Contact the network operators with the address you will deposit from. They mint a token
to it. Without one, the deposit reverts with `Not enough tokens`.

### 2. Choose withdrawal credentials

BLS credentials are **blocked outright**, so a deposit must use an execution-layer form:

| Prefix | Meaning | Allowed |
| --- | --- | --- |
| `0x00` | BLS | **blocked** — the deposit reverts |
| `0x01` | execution-layer withdrawal address | yes, token required |
| `0x02` | compounding | yes, token required |
| `0x03` | builder (the chain runs Gloas) | yes, token required |
| `0xffff` | top-up to an existing validator | yes, **no token required** |

Top-ups are never gated; only new validator entry consumes a token.

### 3. Generate deposit data

Sign against this chain's genesis fork version, `0x10000038`, or the deposit is
rejected by the consensus layer even though the execution transaction succeeds. With
[ethdo](https://github.com/wealdtech/ethdo):

```
ethdo validator depositdata \
  --validatoraccount=<wallet>/<account> \
  --withdrawaladdress=<your 0x01 address> \
  --depositvalue="32 Ether" \
  --forkversion=0x10000038 \
  --raw
```

`--raw` prints the calldata for the deposit contract.

### 4. Deposit

```
cast send --rpc-url https://rpc1.frames.ethrex.xyz \
  --private-key <DEPOSITOR_KEY> --value 32ether \
  0x00000000219ab540356cBB839Cbe05303d7705Fa <CALLDATA>
```

A successful deposit consumes exactly one token from the depositing address and emits
the standard `DepositEvent` at the deposit contract, with topic
`0x649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5` and no additional
event at that address. The gater emits its own ERC-20 `Transfer` at the gater address
for the burned token; that is a different address and does not interfere with clients
matching deposit logs by address and topic.

### 5. Wait for activation

The deposit enters `pending_deposits`, is processed at an epoch transition, then joins
the activation queue. Check progress:

```
curl -s <BEACON>/eth/v1/beacon/states/head/validators/<PUBKEY> | jq .data.status
```

Statuses run `pending_initialized` → `pending_queued` → `active_ongoing`.

**Run a validator client for it.** A deposited validator with nothing signing on its
behalf accrues missed-attestation penalties from the moment it activates.

### Exiting

Either a consensus-layer voluntary exit signed with the validator key, or an
execution-layer withdrawal request through the EIP-7002 predeploy at
`0x00000961Ef480Eb55e80D19ad83579A64c007002` sent from the address in your `0x01`
credentials.

Both require the validator to have been **active for `SHARD_COMMITTEE_PERIOD` = 256
epochs** first, which is about 13.7 hours at 6-second slots. A request submitted before
that is accepted and paid for by the execution layer and then silently skipped by the
consensus layer — the execution transaction succeeding is not evidence that the exit
took. After the exit, `MIN_VALIDATOR_WITHDRAWABILITY_DELAY` adds another 256 epochs
before the balance is withdrawable.

The EIP-7002 request is a 56-byte payload — 48-byte pubkey followed by an 8-byte amount,
where `0` means a full exit — plus the current fee as value:

```
FEE=$(cast call --rpc-url <RPC> 0x00000961Ef480Eb55e80D19ad83579A64c007002)
cast send --rpc-url <RPC> --private-key <WITHDRAWAL_ADDR_KEY> \
  --value $(cast to-dec $FEE)wei \
  0x00000961Ef480Eb55e80D19ad83579A64c007002 "0x<PUBKEY><00000000 00000000>"
```

## What this network runs

**One EIP activates at the fork timestamp: EIP-8141, frame transactions.** That is the
whole point of this chain — a failure here can only be a frame-transaction failure.
Frames are live; see the fork schedule above for the epoch.

Inherited from Amsterdam, and live: EIP-7928 block-level access lists, EIP-8037
two-dimensional gas, EIP-8038 repricing, EIP-7843 slot numbers, EIP-8282 builder
deposits and exits. EIP-8037 is the one you will notice first, because it is why
hardcoding 21,000 gas fails — see "Connect a wallet" above.

**Do not implement this fork from its name.** EIP-8081 (*Hardfork Meta - Hegotá*)
schedules EIP-7805 alone under that name, and the generator's fork table calls this
boundary `heze`/`hegota` regardless of what activates at it. On this chain the name
means EIP-8141 and nothing else. A client that implements "Hegotá" from the meta EIP
implements FOCIL, which this chain does not run, and still rejects every frame
transaction on it.

### Not on this chain

EIP-8250 keyed nonces, EIP-8272 recent roots, EIP-7906 assertions, and EIP-7805 /
EIP-8369 FOCIL are **not** active here. A transaction that depends on any of them is
invalid on this chain. They run on the Hegotá testnet instead.

Two more are in the ethrex binary but outside this chain's rule set: EIP-8312 (UTXO
frames) never activates because `utxoFramesTime` is unset, and EIP-7906 is not in the
build at all. Frame mode 3, which EIP-7906 would have used, is unassigned here.

### Frame transactions

Type `0x06`. The payload is a list of *frames*, each with its own target, gas limit and
mode, with payment authorised by an `APPROVE` in a validation prefix.

| Mode | Meaning |
| --- | --- |
| 0 | `DEFAULT` |
| 1 | `VERIFY`, static; where authorisation happens |
| 2 | `SENDER`, executes with `tx.sender` as the caller |
| 3 | unassigned on this chain |

The smallest useful transaction is two frames: a `VERIFY` frame targeting the sender
with `flags = 0x03`, then a `SENDER` frame carrying the call. Without an `APPROVE` the
transaction has no payer and is invalid.

Submitters live in this directory: `frametx.py`, `frametx_submit.py` and
`frametx_sponsor_submit.py`, with `contracts/OpenSponsor.yul` for the sponsored form.

Dry-run before sending — **against your own node, not the public RPC**, which does not
serve this namespace:

```
curl -s -X POST -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","method":"ethrex_simulateFrameTransaction","params":["<RAW_TX>"],"id":1}' \
  http://127.0.0.1:8545
```

This reports validity, the resolved payer, the prefix shape and per-frame gas before
anything is broadcast. It is worth doing for any transaction that consumes keyed
nonces: a transaction that mines with a reverting `SENDER` frame still consumes its
nonces, so the effects are not recoverable by retrying.



### Consensus inputs that are not in the genesis file

One value on this chain is not in any configuration file and still changes what your
node does: the **VERIFY gas budget**, which these nodes run at `500000` rather than the
100,000 the draft specifies, via `--mempool.max-verify-gas=500000`.

It is a mempool admission rule, not a consensus rule, so a node left at the default does
not fork — it silently *drops* frame transactions whose validation prefix exceeds
100,000 gas as they propagate, and then builds blocks without them. The symptom is a
transaction that your node accepts and the network appears to ignore. Match the budget
if you intend to relay or build.

(The Hegotá testnet has several more such values, including `AA_VOPS_SLOT_COUNT`. None
of them apply here, because none of those EIPs are active on this chain.)

## Divergences from the drafts

This chain tracks the EIP-8141 draft as implemented on ethrex's `frames-devnet-0`
branch; `docs/eip-8141.md` in that repository records where the implementation departs
from the published draft and whether the departure is consensus-visible. For joining a
node, this document is the reference.
