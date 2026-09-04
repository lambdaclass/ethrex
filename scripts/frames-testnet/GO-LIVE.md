# Frames testnet — go-live

The chain runs and every component is verified. What remains is two infrastructure
actions that cannot be done from the host, followed by three commands on it.

Host: **ethrex-office-4**, LAN `192.168.1.3`, behind NAT at **`181.104.27.112`**.

---

## 1. DNS (Cloudflare)

Five `A` records, all to `181.104.27.112`, **proxy disabled (DNS only / grey cloud)**.

| Name | → host port | Service |
| --- | --- | --- |
| `rpc1.frames.ethrex.xyz` | 36003 | ethrex el-1 JSON-RPC |
| `rpc2.frames.ethrex.xyz` | 36010 | ethrex el-2 JSON-RPC |
| `rpc3.frames.ethrex.xyz` | 36017 | ethrex el-3 JSON-RPC |
| `dora.frames.ethrex.xyz` | 36400 | Dora explorer |
| `faucet.frames.ethrex.xyz` | 8088 | Faucet + `/artifacts/` bundle |
| `checkpoint-sync.frames.ethrex.xyz` | 36201 via Caddy | Beacon API, checkpoint sync (read-only `GET /eth/*`) |

Proxying must be off: Caddy issues its own certificates over HTTP-01, and behind
Cloudflare's proxy that challenge never reaches the origin, so the certificate never
issues.

The port column is what sits behind each name on the host, not something Cloudflare
configures. The three RPC names are proxied to the namespace guard, never to the nodes
directly — the nodes answer `admin_*` and `debug_*`, and the guard is what withholds them.

## 2. Router port forwards → `192.168.1.3`

80 and 443 are already forwarded. These are the additions:

| Purpose | Ports | Protocol |
| --- | --- | --- |
| EL discovery + RLPx | 36000, 36007, 36014 | TCP **and** UDP |
| CL libp2p | 36200, 36207, 36214 | TCP |
| CL QUIC | 36200, 36207, 36214 | UDP |
| CL discv5 | 36201, 36208, 36215 | **UDP only** |

`36201`, `36208`, `36215` are UDP-only on purpose. The same numbers on TCP are the
beacon REST APIs and must stay closed; a rule written without `-p` matches both and
publishes them.

Leave closed: `36001/36008/36015` (engine authrpc — reaching these is equivalent to
controlling the node's head), `36002/36009/36016` (EL metrics), `36003/36010/36017`
(EL RPC, reached through Caddy), `8088`/`8089`/`36400` (reached through Caddy).

**If the forwards are UPnP mappings rather than static router rules, they must be
renewed.** The office gateway (Huawei EchoLife IGD) accepts `LeaseDuration=0` and reports
the mapping as permanent, then silently drops it if nothing re-requests it — the tailscale
mapping on the same router survives only because tailscale renews it. Every frames mapping
vanished within a day of being created, with no router reboot (WAN uptime 130 days), and
an external joiner went to 0 EL peers as a result. `frames-portmap.timer` re-asserts all
sixteen mappings every 10 minutes; `AddPortMapping` is idempotent for an identical entry.
Static forwards configured by whoever administers the router remain the durable fix.

**This is not only about external joiners.** The nodes advertise `nat_exit_ip`, so
without these forwards nothing reaches them at the address they publish — including
their own siblings on this host. That is what split the deployment into one chain per
node before. `frames-cl-peers.timer` currently works around it by peering them over the
docker network; the forwards are what make the peering real.

## 3. On the host, once the records resolve

```
dig +short rpc1.frames.ethrex.xyz          # must return 181.104.27.112 first
sudo cp /etc/caddy/Caddyfile.new /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

The staged config was pre-flighted on a scratch port against the real upstreams: all
three RPC names reach distinct nodes through the guard with `admin_nodeInfo` refused,
dora and the faucet answer 200, `/api/claim` reaches the app, and
`/artifacts/genesis.json` serves `chainId 81410`.

## 3b. Other execution clients

The bundle's `chainspec.json` is already corrected for Nethermind (the generator's
own output schedules FOCIL, which this chain does not run; the published file has that key
removed in place). Nethermind additionally needs a build that
includes the execution-specs#3396 frame-transaction envelope; the public
`ethpandaops/nethermind:frames-devnet-0` at `1b9daf39` predates it and rejects every
frame transaction. See the user guide's "Join the network" note.

## 4. Verify, in this order

```
# certificates issued and the public surface answers
for h in rpc1 rpc2 rpc3 dora faucet; do
  curl -s -o /dev/null -w "$h %{http_code} tls=%{ssl_verify_result}\n" https://$h.frames.ethrex.xyz/
done

# the nodes agree with each other — a partition is invisible per-node
for p in 36003 36010 36017; do
  curl -s -X POST -H 'content-type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' http://127.0.0.1:$p
done
for p in 36201 36208 36215; do
  curl -s http://127.0.0.1:$p/eth/v1/beacon/states/head/finality_checkpoints
done

# peering is now real rather than worked around
for p in 36201 36208 36215; do curl -s http://127.0.0.1:$p/eth/v1/node/peer_count; done
```

Heights must match, finalized epochs must match and be non-zero, and `connected` must be
non-zero. Then confirm an **external** joiner peers, from a machine outside this network,
using only the published bundle — that is the check no test on this host can stand in for.

---

## What is already done

Verified on this deployment, all four with evidence rather than assertion:

- **Faucet** — serves the guide content and funds accounts; page carries the rex commands
  and the common errors, with copy buttons on every command block.
- **Explorer** — a freshly sent type-`0x06` rendered as `Frame (EIP-8141) (6)`,
  `2 frames · 1 signature`, `VERIFY / APPROVE execution+payment` then
  `SENDER / APPROVE none`, matching `rex frame send`'s own decode field for field.
- **Validator lifecycle** — deposited from a second host through the token gate,
  activated at epoch 51, attested and proposed, then exited: `active_exiting`,
  `exit_epoch 424`, `withdrawable_epoch 680`.
- **rex** — `frames-testnet` branch, `rex frame send` / `build` / `inspect` verified
  against the live chain.

Standing services: `frames-rpc-guard.service` (namespace guard on 8645),
`frames-cl-peers.timer` (beacon peering, one minute, idempotent).

The reusable checklist is `docs/workflows/testnet-readiness.md`.
