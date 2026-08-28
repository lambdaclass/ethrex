# Frames testnet — installing on a fresh host

Written for someone who has never seen the Hegotá devnet. Follow it top to bottom on a
host that has nothing on it yet.

What you end up with: three ethrex + Lighthouse pairs under kurtosis, a gated deposit
contract, a block explorer and a faucet behind HTTPS, and a published artifact bundle
another client can join with.

Companion documents: `scripts/frames-testnet/USER-GUIDE.md` (what joiners consume, and the
firewall surface), `docs/frames-testnet-permissioning.md` (validator gating),
`docs/frames-testnet-upgrading.md` (changing it later),
`docs/frames-testnet-verification.md` (the twelve verification checks).

## 0. Before you start

You need a host with **a real public IP** and inbound UDP as well as TCP. Everything
about peering depends on that; a host behind NAT that cannot forward UDP will look
healthy from inside and be unreachable from outside.

Install:

- **docker**, with the daemon running and your user able to reach it
- **kurtosis 1.20.0 or newer** — see below, do not use apt
- `jq`, `curl`, `sha256sum`, `gh` (or fetch release assets by hand)
- **foundry** (`cast`), used by `gen-deployment-keys.sh` and every verification step

Budget ~20 GB of disk for images, the enclave and its databases, and more for chain
growth.

Check the host's ephemeral port floor before anything else:

```
cat /proc/sys/net/ipv4/ip_local_port_range     # expect 32768 60999
```

Every port this chain publishes (31000-31500, 32000-32017) must stay **below** the first
number. Kurtosis hands dynamic host ports to unpublished services out of that range, so a
fixed publish inside it races them and can lose with `failed to bind host port … address
already in use`. If your floor is lower than 32768, either raise it or move
`port_publisher` down; do not leave the overlap.

### Installing kurtosis

**Not from apt.** The `apt.fury.io/kurtosis-tech` channel is abandoned at 1.15.2, so a
package install silently pins you to an old CLI. Releases are published as assets on a
separate repository:

```
gh release download 1.20.0 --repo kurtosis-tech/kurtosis-cli-release-artifacts \
  --pattern 'kurtosis-cli_1.20.0_linux_amd64.deb'
sudo dpkg -i kurtosis-cli_1.20.0_linux_amd64.deb
kurtosis version
```

Without `gh`, the same asset is a direct download:

```
curl -sSL -o kurtosis.deb https://github.com/kurtosis-tech/kurtosis-cli-release-artifacts/\
releases/download/1.20.0/kurtosis-cli_1.20.0_linux_amd64.deb
```

Upgrading over an existing install is a plain `dpkg -i`; stop the old engine first
(`kurtosis engine stop`), because the CLI refuses to talk to a mismatched engine and
restarts it at the new version on next use.

The CLI and the engine must be the same version; the CLI refuses to talk to a mismatched
engine. On a fresh host the engine starts on first use and will match.

## 1. Build the execution client image

The config names `ethrex:frames-testnet` explicitly, not `ethrex:local`, so the tag
matters:

```
git clone https://github.com/lambdaclass/ethrex && cd ethrex
git checkout frames-devnet-0
make build-image TAG=frames-testnet
docker run --rm ethrex:frames-testnet --version
```

Confirm the version string names the branch and the commit you meant to deploy. The image
tag is not evidence of what a *running* container executes — see the upgrading guide.

## 2. Pin the ethereum-package

```
make checkout-ethereum-package
```

This checks out the exact revision `ETHEREUM_PACKAGE_REVISION` names in the `Makefile`.
The pin is load-bearing: it selects a genesis generator new enough to deploy the EIP-8282
builder predeploys, without which the chain stops producing blocks at the Amsterdam
boundary.

## 3. Generate the deployment's keys

```
umask 077
./scripts/frames-testnet/gen-deployment-keys.sh > ~/frames-testnet-keys.env
chmod 600 ~/frames-testnet-keys.env
```

Keep the keys and the filled config in the home directory of **the account that runs
kurtosis**, not under `/root`. `kurtosis run` reads the args file as the invoking user,
so a root-owned `0600` file is unreadable to it and the launch fails on a path that
looks present.

This writes two independent BIP-39 mnemonics and thirteen addresses with their private
keys: a faucet, the deposit-gater admin, a deployer, and ten funded accounts. It also
prints the config block to paste.

**None of this may be committed.** The kurtosis default mnemonic is public and identical
on every deployment anyone has ever run; an address derived from it is spendable by
strangers. It matters most for the gater admin, because a genesis admin is granted a
*sticky* role that `revokeRole` refuses to revoke — a leaked genesis admin is an
unrevokable public mint for validator slots, and the only remedies are the per-prefix kill
switch or a new genesis. Keep that key offline and delegate a runtime admin for daily use.

## 4. Fill the config

Copy the config somewhere outside the repository so a filled copy can never be committed:

```
cp fixtures/networks/frames-testnet.yaml ~/frames-testnet.yaml
chmod 600 ~/frames-testnet.yaml
```

It holds the validator mnemonic once filled, so it is as sensitive as the key file.

Replace every `REPLACE_WITH_*` marker in `~/frames-testnet.yaml`:

| Marker | Value |
| --- | --- |
| `REPLACE_WITH_A_FRESHLY_GENERATED_MNEMONIC` | `VALIDATOR_MNEMONIC` |
| `REPLACE_WITH_FAUCET_ADDRESS` | `FAUCET_ADDR` |
| `REPLACE_WITH_GATER_ADMIN_ADDRESS` | `GATER_ADMIN_ADDR` (appears **twice**) |
| `REPLACE_WITH_DEPLOYER_ADDRESS` | `DEPLOYER_ADDR` |
| `REPLACE_WITH_RICH_ADDRESS_01` … `_10` | `RICH_01_ADDR` … `RICH_10_ADDR` |
| `REPLACE_WITH_HOST_PUBLIC_IP` | the host's public IP, a literal |

Then confirm nothing was missed:

```
grep -n 'REPLACE_WITH' ~/frames-testnet.yaml     # expect no output
```

The gater admin address appears in both `prefunded_accounts` and
`DEPOSIT_CONTRACT_ADMINS`, and they must be the same address, or the account that can mint
deposit tokens is not the one holding gas to do it with.

`nat_exit_ip` must be the literal public IP, not `auto` and never `127.0.0.1`. It is the
single source of the address every node advertises: the package hands it to each launcher,
which is how it reaches the EL's `--nat.extip`, the CL's advertised address, and every
enode and ENR you publish. Do not add `--nat.extip` to `el_extra_params` — the launcher
already emits it, and a second copy is a value nothing checks against the first.

## 4a. Two things this deployment proved

Both were found by launching, not by reading, and both are already in the committed
config. They are recorded here because each one fails in a way that points somewhere
else.

**Hand kurtosis the package without its metadata directories.** `kurtosis run` against
a plain `git clone` reports `No 'kurtosis.yml' file was found in the package root` while
the file is plainly there. Copy the tree first:

```
mkdir -p /tmp/ep-clean
(cd ethereum-package && tar cf - --exclude=.git --exclude=.claude --exclude=.agents .) \
  | (cd /tmp/ep-clean && tar xf -)
kurtosis run --enclave frames-testnet /tmp/ep-clean --args-file ~/frames-testnet.yaml
```

**A client that exits at startup surfaces as a gRPC error, not as its own message.**
When a service crashes, kurtosis fails to marshal the crash report and prints
`rpc error: code = Internal desc = grpc: error while marshaling: string field contains
invalid UTF-8`. That string says nothing about the cause. Read the exited container
instead:

```
docker ps -a --format '{{.Names}}\t{{.Status}}' | grep el-1-ethrex   # find the Exited one
docker logs <that container>
```

On this deployment it was `unknown RPC namespace "ethrex"` — the `--http.api=ethrex`
flag carried over from the Hegotá config, whose namespace this branch does not serve.

## 5. Launch

Install the firewall **before** this, not after. See section 10: the moment the enclave
starts, every port it publishes is reachable, engine authrpc included.

```
kurtosis run --enclave frames-testnet ./ethereum-package --args-file ~/frames-testnet.yaml
kurtosis enclave inspect frames-testnet
```

Expect six client services (`el-1..3`, `cl-1..3`) plus `dora`, all `RUNNING`, alongside
three validator clients and the keystore generator. The chain reaches Amsterdam at
genesis+192s and Hegotá at genesis+384s, so give it ~7 minutes before judging it.

A run issued while the engine is still coming up — the first one after a CLI upgrade, in
particular — can fail with:

```
No 'kurtosis.yml' file was found in the package root so fell back to Docker Compose package
```

This says nothing about the package, which is fine. Confirm `kurtosis engine status`
reports the expected version, `kurtosis enclave rm -f frames-testnet` the empty enclave it
left behind, and run it again with an absolute path to the package directory.

## 6. Verify

Run the twelve checks in `docs/frames-testnet-verification.md`. Do not skip them: most of the
failure modes are silent until a specific fork boundary, and a chain that starts is not a
chain that works.

**Check 8 is the one this host exists to prove.** It needs an `ethrex --bootnodes` node on
a *different* machine completing discovery and reaching the head. Reachability cannot be
established from the host itself: a firewall rule that opens only TCP, or a node
advertising an address nobody outside can route, both look perfectly healthy from inside.
Eleven of the twelve have already been verified on a local enclave; check 8 has not been
exercised anywhere.

## 7. Publish the artifact bundle

```
PUBLIC_IP=<host public ip> ENCLAVE=frames-testnet \
  OUT_DIR=/srv/frames-testnet/artifacts \
  ./scripts/frames-testnet/publish-artifacts.sh
```

Then check by hand, before anything is served:

```
ls /srv/frames-testnet/artifacts        # must NOT contain mnemonics.yaml
```

The script publishes an explicit allowlist for this reason: the generator's
`/network-configs` also holds `mnemonics.yaml`, the phrase every genesis validator key
derives from, and `OUT_DIR` is public. Never replace the allowlist with a directory copy.

Fetch the bundle from another machine and start a node from it before announcing the
network. A consensus client consumes the directory as a whole, so a missing file fails at
startup rather than at first use.

## 8. Reverse proxy

Serve the RPC, the explorer, the faucet and the artifact bundle over HTTPS. Caddy gets
certificates automatically from the hostnames you declare, which means **every name in
the Caddyfile must already resolve to this host**: a name without an address record fails
its ACME challenge and spends the Let's Encrypt failed-validation budget, five per
hostname per hour, for the whole deployment.

The zone is `privacy.ethrex.xyz` and it has five names — `rpc1`, `rpc2`, `rpc3`, `dora`,
`faucet`. There is no `artifacts` name and the apex carries no address record, so the
bundle is served from a path under the faucet host:

```
rpc1.privacy.ethrex.xyz { reverse_proxy localhost:8645 }
rpc2.privacy.ethrex.xyz { reverse_proxy localhost:8645 }
rpc3.privacy.ethrex.xyz { reverse_proxy localhost:8645 }

dora.privacy.ethrex.xyz { reverse_proxy localhost:31500 }

faucet.privacy.ethrex.xyz {
    handle_path /artifacts/* {
        root * /srv/frames-testnet/artifacts
        file_server browse
    }
    reverse_proxy localhost:8080
}
```

`handle_path`, not `handle`: the prefix has to be stripped before the request reaches the
filesystem root, or `/artifacts/genesis.json` is looked up at
`/srv/frames-testnet/artifacts/artifacts/genesis.json`.

**Do not add `header Access-Control-*` directives.** ethrex's RPC server already sends a
complete permissive CORS set (`CorsLayer::permissive()`), Caddy's `header` *appends*
rather than replaces, and a duplicated `Access-Control-Allow-Origin` is hard-rejected by
browsers and by MetaMask's request layer. The Caddyfile at
`scripts/eip8141-devnet/Caddyfile` does add them; do not copy that part of it.

One RPC hostname per execution node, rather than one hostname for node 0. Three-node
agreement on head *hash* is the check this chain asks users to run, and they cannot run it
against a single endpoint. The nodes' RPC ports stay closed at the firewall; reaching them
goes through this proxy, which is what the operator watches.

All three RPC hostnames proxy to **8645, the namespace guard**, not to a node port
directly. See section 8a: publishing a node port as it comes out of `ethereum-package`
publishes `debug_setHead` and `admin_addPeer` to the internet. The guard routes by `Host`,
which is why the three vhosts share one port.

## 8a. RPC namespace guard

`ethereum-package` launches every execution client with its whole API surface — for ethrex
`--http.api=eth,net,web3,debug,admin,txpool` in `src/el/ethrex/ethrex_launcher.star` —
because the package is written for private devnets where the port is unreachable. A public
reverse proxy in front of that port hands the internet `debug_setHead`, `debug_trace*`,
`admin_addPeer` and `admin_setLogLevel`.

That cannot be fixed at the launch command. Repeated `--http.api` flags take the **union**
rather than replacing — `cmd/ethrex/cli.rs` pins this in
`http_api_repeated_flags_accumulate` — so a narrower second flag only ever adds. And the
node cannot simply drop `admin`, because the package's own enode discovery calls
`admin_nodeInfo`, as does `publish-artifacts.sh`.

So the split is made by reachability. `scripts/frames-testnet/rpc-guard.py` sits between
Caddy and the nodes and forwards only the allowed namespaces; anything on the host that
talks to a node port directly still has the full API.

```
[Unit]
Description=Hegota testnet JSON-RPC namespace guard
After=network-online.target

[Service]
Environment=UPSTREAMS=rpc1.privacy.ethrex.xyz=127.0.0.1:32003,rpc2.privacy.ethrex.xyz=127.0.0.1:32010,rpc3.privacy.ethrex.xyz=127.0.0.1:32017
ExecStart=/usr/bin/python3 /usr/local/bin/rpc-guard.py
Restart=always
DynamicUser=yes

[Install]
WantedBy=multi-user.target
```

The default allowed set is `eth,net,web3,txpool,ethrex`. **`ethrex` belongs there**: it
holds one read-only method, `ethrex_simulateFrameTransaction`, and it is a separate
namespace precisely so simulating a type-0x06 envelope can be public without enabling
`debug_`. On a chain where no wallet can build a frame transaction, that call is the point
of the endpoint.

The guard adds no CORS headers of its own and relays the node's, so the single
`Access-Control-Allow-Origin` rule above still holds. It refuses a batch as a whole if any
member is denied, caps the batch at 100 calls and the body at 1 MiB, and fails closed on a
body it cannot parse. An unknown `Host` is refused rather than sent to a default node, so
Caddy must pass the original `Host` through — which it does by default, unlike nginx.

The bundle path is not the only route to the bootnode lists: the faucet serves the same
three lists as JSON at `/bootnodes`, read from the same directory. That is deliberate
redundancy — the lists are the one part of the bundle a joiner needs after it is already
running, when a peer set has gone stale.

The explorer sits at a fixed 31500 because `port_publisher.additional_services` pins it.
The Hegotá devnet needed a socat unit to give the explorer a stable local port; this chain
does not, and you should not add one.

## 9. Faucet

Run the faucet with **its own key**, funded from the `FAUCET_ADDR` account, and keep that
key in a host env file — never in the repository, never in the kurtosis config. Point it
at `http://localhost:32003` and give it the public RPC, explorer and bundle URLs for its
own links — `PUBLIC_RPC_URL`, `EXPLORER_URL`, `ARTIFACTS_URL`; each row is simply absent
from the page when its variable is unset. `scripts/frames-testnet/faucet/` is a working
reference implementation.

Give it the artifact bundle read-only, so it can serve the bootnode lists on its landing
page and at `GET /bootnodes`:

```
docker run … \
  -v /srv/frames-testnet/artifacts:/srv/frames-testnet/artifacts:ro \
  -e BUNDLE_DIR=/srv/frames-testnet/artifacts …
```

Read-only is the whole of the protection here, so keep the flag: the faucet has no reason
to write the bundle and the bundle is what every joiner trusts. The lists are re-read
whenever the files change, so section 7 can be re-run without restarting the faucet, and a
faucet started before the first publish picks them up on its own. With no bundle mounted
the peering section is simply absent from the page and `/bootnodes` answers `503` —
never an empty list, which a joiner would read as a chain with no peers.

Fund it from the faucet account rather than a rich account, so a drained faucet cannot
touch the accounts you hold for testing.

Two things about running it as a container. `--env-file` is resolved at creation, so
`docker inspect` prints `PRIVATE_KEY` in `Config.Env` rather than a reference to the file:
anyone who can run docker here can already read the key from the host, so this grants
nothing new, but it does mean inspect output is as sensitive as the env file and must not
be pasted into a ticket or a chat. And `page.html` is baked into the image by `COPY`, so
editing the page or `faucet.py` in a clone changes nothing that is being served — the
image has to be rebuilt and the container recreated. Tag the outgoing image first
(`docker tag hegota-faucet:latest hegota-faucet:pre-<change>`) so a bad page is one
`docker run` from being reverted.

## 10. Firewall

The full table is in `scripts/frames-testnet/USER-GUIDE.md`. The shape of it:

| Purpose | Ports | Proto | Exposure |
| --- | --- | --- | --- |
| EL discovery + RLPx | 32000, 32007, 32014 | TCP **and** UDP | open |
| CL discovery + libp2p | 31000, 31007, 31014 | TCP **and** UDP | open |
| CL QUIC | 31003, 31010, 31017 | UDP | open |
| EL engine authrpc | 32001, 32008, 32015 | TCP | **closed** |
| EL metrics | 32002, 32009, 32016 | TCP | **closed** |
| EL RPC nodes 1-2 | 32010, 32017 | TCP | **closed** |
| CL beacon REST | 31001, 31008, 31015 | TCP | **closed** |
| CL metrics | 31002, 31009, 31016 | TCP | **closed** |

Everything public is reached through the reverse proxy, not opened directly.

Discovery needs **both** TCP and UDP. A rule that opens only TCP produces a node that
accepts inbound connections from peers who already know it and is discovered by nobody,
which reads as slow peering rather than as a firewall mistake.

**The engine ports are the ones that matter.** Reaching them is equivalent to controlling
the node's head, and the JWT does not protect you: `ethereum-package` ships one committed
`jwtsecret` static file and mounts that same value into every client, so the token is
identical on every kurtosis deployment in existence. The firewall is the whole of the
control. Generate a fresh secret if those ports ever leave loopback.

### Where the rules go

**In the `DOCKER-USER` chain, not `ufw` and not `INPUT`.** Docker publishes a port by
DNAT'ing it in `nat/PREROUTING`, so the packet is *forwarded* to the container and never
traverses `INPUT`. A `ufw deny` on the engine port therefore reports itself as active and
blocks nothing. `DOCKER-USER` is the one chain docker guarantees it will not rewrite, and
it is consulted from `FORWARD` before docker's own accept rules.

Match on the in-interface, since the packet's destination address is already the
container's by the time the chain sees it:

```
EXT=$(ip -4 route show default | awk '{print $5; exit}')
iptables -A DOCKER-USER -m conntrack --ctstate RELATED,ESTABLISHED -j RETURN
iptables -A DOCKER-USER -i tailscale0 -j RETURN
iptables -A DOCKER-USER -i lo -j RETURN
iptables -A DOCKER-USER ! -i "$EXT" -j RETURN
iptables -A DOCKER-USER -i "$EXT" -p tcp -m multiport --dports 32000,32007,32014,31000,31007,31014 -j RETURN
iptables -A DOCKER-USER -i "$EXT" -p udp -m multiport --dports 32000,32007,32014,31000,31007,31014 -j RETURN
iptables -A DOCKER-USER -i "$EXT" -p udp -m multiport --dports 31003,31010,31017 -j RETURN
iptables -A DOCKER-USER -i "$EXT" -j DROP
```

The trailing `DROP` is what closes engine, metrics and beacon REST; the table above is
then a description of what the preceding `RETURN`s allow rather than a separate list to
maintain. Nothing here touches `INPUT`, so ssh cannot be locked out by a mistake in it.

Persist with `apt install iptables-persistent && netfilter-persistent save`, and check the
default `INPUT` policy while you are there — a host that ships `-P INPUT ACCEPT` with no
rules is relying entirely on this chain.

## 11. Granting validator slots

Anyone may sync, peer and transact. Only validator entry is gated, by a token the
depositing address must hold. Confirm the deployment first:

```
docker run --rm -it pk910/gated-deposit-contract-cli -k $ADMIN_KEY -r $RPC status
```

Expect: signer is admin (sticky), `0x00` BLOCKED, `0x01`/`0x02`/`0x03` allowed requiring a
token, `0xffff` (top-ups) allowed token-free. Then grant slots:

```
docker run --rm -it pk910/gated-deposit-contract-cli -k $ADMIN_KEY -r $RPC \
  mint --to <DEPOSITOR_EOA> --amount <N>
```

`<DEPOSITOR_EOA>` is the address that will **send** the deposit transaction, not the
withdrawal address and not the validator pubkey. This is the most common mistake.

Test both directions after launch: a deposit from a token-less address must revert with
`Not enough tokens`, and the same deposit must succeed after minting. Only proving both
shows the gate is working rather than simply broken for everyone.
`docs/frames-testnet-permissioning.md` has the full policy and kill switch.

## 12. Later changes

`docs/frames-testnet-upgrading.md` covers upgrading ethrex and kurtosis, re-publishing the
bundle, rotating the gater admin, and what forces a re-genesis.

**Decide the upgrade path now, before launch.** Before anyone else has joined, the
cheapest correct upgrade is a re-genesis. Once joiners exist, a re-genesis is a relaunch
of a different network and the binary has to be replaced inside running containers
instead — and that only works if the container entrypoint was built for it. Retrofitting
it later means recreating containers, which is the thing it exists to avoid. Record which
path this host is built for.
