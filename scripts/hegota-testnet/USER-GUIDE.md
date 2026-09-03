# Hegotá testnet — user guide

A public test network running the frame-transaction family of EIPs on
[ethrex](https://github.com/lambdaclass/ethrex). Test ETH has no value.

Anyone may sync, peer and transact without permission. Only **validator entry** is
gated; see "Become a validator" below.

> The zone is `privacy.ethrex.xyz`. Five names exist: `rpc1`, `rpc2`, `rpc3`, `dora` and
> `faucet`. There is no `artifacts` name and the apex has no address record, so the
> artifact bundle is served from a path under the faucet host rather than a host of its
> own. Add an `artifacts` record and it can move, with the path left as a redirect.

## Network details

| | |
| --- | --- |
| Chain ID / network ID | `8141` |
| Seconds per slot | 6 |
| Block gas limit | 200,000,000 |
| Execution client | ethrex, Hegotá testnet build |
| Consensus client | Lighthouse (`ethpandaops/lighthouse:focil`) |
| RPC | `https://rpc1.privacy.ethrex.xyz` (also `rpc2`, `rpc3`) |
| Explorer | `https://dora.privacy.ethrex.xyz` |
| Faucet | `https://faucet.privacy.ethrex.xyz` |
| Artifact bundle | `https://faucet.privacy.ethrex.xyz/artifacts` |
| Bootnodes | `https://faucet.privacy.ethrex.xyz/bootnodes` |
| Deposit contract | `0x00000000219ab540356cBB839Cbe05303d7705Fa` |
| Deposit gater | `0x00000000a11acc355c0de0000a11acc355c0de00` |

Chain ID `8141` is unclaimed in the `chainid.network` registry and is deliberately not
the kurtosis default `3151908`, which every local devnet collides on.

### Fork schedule

| Fork | Epoch | Offset from genesis |
| --- | --- | --- |
| Fulu / Osaka | 0 | genesis |
| Gloas / Amsterdam | 1 | +192s |
| Heze / Hegotá | 2 | +384s |

The consensus layer names the last fork `heze`; the execution genesis names the same
timestamp `bogotaTime`. ethrex reads `hegotaTime`, `hezeTime` and `bogotaTime` as
aliases for one field.

### Predeploys

| Address | Contents |
| --- | --- |
| `0x…8141` | `EXPIRY_VERIFIER` |
| `0x…8250` | `NONCE_MANAGER` |
| `0x…8272` | `RECENT_ROOT_ADDRESS`, 144 bytes |

All three are installed by the client at the Hegotá boundary and need no genesis entry.

## Connect a wallet

Add a custom network with the chain ID and RPC URL above. The currency symbol is ETH
and it is worthless.

The three RPC hostnames are three different nodes, so comparing a block *hash* between
`rpc1` and `rpc2` is a real agreement check and not the same node answering twice.

They serve `eth`, `net`, `web3`, `txpool` and `ethrex` — the last of which is
`ethrex_simulateFrameTransaction`, the one way to dry-run a type-`0x06` envelope without
submitting it. `debug` and `admin` are not served here; run your own node from the bundle
if you need them, where they are yours to enable.

**Budget more gas than you expect.** Under EIP-8037 a plain transfer that *creates* an
account costs far more than the historical 21,000 — closer to 210,000 — because account
creation is charged as state growth. Estimate gas rather than hardcoding it; tooling
with a baked-in 21,000 fails here, and it fails specifically when paying someone new.
A transfer to an account that already exists still costs 21,000.

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
mkdir hegota-testnet && cd hegota-testnet
for f in genesis.json genesis.ssz config.yaml chainspec.json besu.json \
         bootnodes.txt bootnodes-enr.txt bootnodes-cl.txt \
         deposit_contract.txt deposit_contract_block.txt \
         deposit_contract_block_hash.txt genesis_validators_root.txt MANIFEST.txt; do
  curl -fsSLO "https://faucet.privacy.ethrex.xyz/artifacts/$f"
done
sha256sum -c MANIFEST.txt
```

`MANIFEST.txt` carries a sha256 for every published file, `chainspec.json` and `besu.json`
included, so fetching only the files a node reads leaves the check reporting two missing
files. Fetch the whole set and check it; the two extra files are generator output nobody
maintains, and ethrex ignores them.

You also need an ethrex build of this chain's branch. No published release or image carries
it, and a stock release does not implement the rule set:

```
git clone --branch hegota-testnet --depth 1 https://github.com/lambdaclass/ethrex
cd ethrex && make build-image TAG=hegota-testnet     # or: cargo build --release
docker run --rm ethrex:hegota-testnet --version       # names the branch and commit
```

The three bootnode files are also served live as JSON, so peers can be checked or
re-fetched without pulling the whole bundle:

```
curl -s https://faucet.privacy.ethrex.xyz/bootnodes
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

Consensus layer, pointed at the execution client's engine port, synced from genesis:
```
lighthouse beacon_node \
  --testnet-dir=. \
  --execution-endpoint=http://127.0.0.1:8551 \
  --execution-jwt=<path to the jwtsecret your EL uses> \
  --boot-nodes="$(paste -sd, bootnodes-cl.txt)" \
  --allow-insecure-genesis-sync
```

**Sync from genesis, not from the checkpoint endpoint, for now.** Genesis sync imports this
chain at roughly 20 slots per second (1,900 slots in 90 seconds on 2026-09-03), so it is
minutes, not hours, and the execution client follows a few blocks behind.

A checkpoint endpoint exists, `https://checkpoint-sync.privacy.ethrex.xyz`, a read-only view
of one of the network's beacon nodes (`GET` on `/eth/*` only, everything else refused), and
it serves a correct finalized state, block and execution payload envelope. But the
Lighthouse build this chain requires (`ethpandaops/lighthouse:focil`, v8.1.3-52e5197) does
not use it correctly on a Gloas chain: it downloads the checkpoint block and state and never
asks for the block's execution payload envelope, so the anchor sits in fork choice with no
payload, the first block after it fails with `Block has an unknown parent: <checkpoint
root>`, range sync blacklists every peer, and the node stays at the checkpoint slot for
good. Verified on 2026-09-03 through the endpoint and directly against the beacon node, at
checkpoints from slot 96 to slot 1,888; `--reset-payload-statuses` does not help. The
endpoint stays up for when the client is fixed. If you try it anyway, the symptom above is
what a failure looks like; nothing is wrong with your setup.

One `ERROR Could not add peer to the local routing table … "Failed bucket filter"` at
startup is expected: the three beacon nodes share an IP and discv5 admits two per /24 into a
bucket. The node connects to all three regardless.

If your execution client already followed an earlier checkpoint attempt, expect ethrex to
log `Too deep reorg` while the consensus client replays from genesis; it clears once the
replay passes the execution client's head.

The consensus client must be FOCIL-aware. ethrex rejects `engine_newPayloadV5` and
`engine_forkchoiceUpdatedV4` from Hegotá on, because only the V6/V5 pair carries
`inclusionListTransactions`, so a client speaking only the older pair halts at the fork
boundary with no inert intermediate state. `ethpandaops/lighthouse:focil` works; a
stock release generally does not.

Confirm you are actually following:

```
cast block-number --rpc-url http://127.0.0.1:8545     # advances
cast rpc net_peerCount --rpc-url http://127.0.0.1:8545
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
cast send --rpc-url https://rpc1.privacy.ethrex.xyz \
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

Five EIPs activate together at one timestamp. There is no per-EIP activation and no
intermediate state in which some are live and others are not.

| EIP | Title |
| --- | --- |
| 8141 | Frame Transaction |
| 8250 | Keyed Nonces |
| 8272 | Recent Roots |
| 7805 | FOCIL (fork-choice enforced inclusion lists) |
| 8369 | VOPS Profiles for FOCIL Eligibility |

**Do not implement this fork from its name.** EIP-8081 (*Hardfork Meta - Hegotá*)
schedules EIP-7805 alone under that name; this chain activates all five. A client that
implements "Hegotá" from the meta EIP implements FOCIL only and rejects every frame
transaction on the chain.

Inherited from Amsterdam, and live: EIP-7928 block-level access lists, EIP-8037
two-dimensional gas, EIP-8038 repricing, EIP-7843 slot numbers, EIP-8282 builder
deposits and exits.

Two EIPs are present in the ethrex binary but **not** part of this chain's rule set:
EIP-8312 (UTXO frames) never activates because `utxoFramesTime` is unset, and EIP-7906
(transaction assertions) is not in the build at all. Frame mode 3, which EIP-7906 would
have used, is unassigned here.

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

Dry-run before sending:

```
curl -s -X POST -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","method":"ethrex_simulateFrameTransaction","params":["<RAW_TX>"],"id":1}' \
  https://rpc1.privacy.ethrex.xyz
```

This reports validity, the resolved payer, the prefix shape and per-frame gas before
anything is broadcast. It is worth doing for any transaction that consumes keyed
nonces: a transaction that mines with a reverting `SENDER` frame still consumes its
nonces, so the effects are not recoverable by retrying.

### EIP-8250 keyed nonces

A transaction picks its own nonce domains instead of a single sequential counter, so one
shared sender address stops being a throughput bottleneck. Useful for privacy pools and
relayers.

### EIP-8272 recent roots

A transaction declares verified `(source_id, slot, root)` commitments in its signed
envelope, readable from execution without touching another account's mutable storage.

Write a root by calling `0x…8272` directly with exactly 64 bytes of calldata — 32-byte
salt followed by 32-byte root — and zero value. Any other length, or a non-zero value,
reverts. A write costs about **127,256 gas** with 64 non-zero calldata bytes; the figure
is emergent from the EVM rather than a constant, and moves with how many zero bytes the
payload carries.

A root written during slot `S` becomes referenceable from slot `S + 1`, and stays
referenceable for 8,191 slots.

### Consensus inputs that are not in the genesis file

Four values decide consensus outcomes and appear in no configuration file. A client that
guesses them will disagree about which blocks are attestable, and that disagreement
produces no state-root mismatch to make it visible. They are stated in
`docs/hegota-testnet-joining.md` and nowhere else. The most load-bearing:
`AA_VOPS_SLOT_COUNT = 4`, where **absent means 4, not disabled** — there is no way to
turn Profile 2 off.

## Divergences from the drafts

Every place this implementation departs from the published drafts is recorded in
`docs/hegota-testnet-divergences.md`, with whether it is consensus-visible. The
published specification a joining client should read first is
`docs/hegota-testnet-joining.md`.
