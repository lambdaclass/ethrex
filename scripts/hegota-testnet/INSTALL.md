# Hegotá testnet — installing on a fresh host

Written for someone who has never seen the Hegotá devnet. Follow it top to bottom on a
host that has nothing on it yet.

What you end up with: three ethrex + Lighthouse pairs under kurtosis, a gated deposit
contract, a block explorer and a faucet behind HTTPS, and a published artifact bundle
another client can join with.

Companion documents: `docs/hegota-testnet-joining.md` (what joiners consume, and the
firewall surface), `docs/hegota-testnet-permissioning.md` (validator gating),
`docs/hegota-testnet-upgrading.md` (changing it later),
`docs/hegota-devnet-genesis.md` (the twelve verification checks).

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

The CLI and the engine must be the same version; the CLI refuses to talk to a mismatched
engine. On a fresh host the engine starts on first use and will match.

## 1. Build the execution client image

The config names `ethrex:hegota-testnet` explicitly, not `ethrex:local`, so the tag
matters:

```
git clone https://github.com/lambdaclass/ethrex && cd ethrex
git checkout hegota-testnet
make build-image TAG=hegota-testnet
docker run --rm ethrex:hegota-testnet --version
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
./scripts/hegota-testnet/gen-deployment-keys.sh > /root/hegota-testnet-keys.env
chmod 600 /root/hegota-testnet-keys.env
```

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
cp fixtures/networks/hegota-testnet.yaml /root/hegota-testnet.yaml
```

Replace every `REPLACE_WITH_*` marker in `/root/hegota-testnet.yaml`:

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
grep -n 'REPLACE_WITH' /root/hegota-testnet.yaml     # expect no output
```

The gater admin address appears in both `prefunded_accounts` and
`DEPOSIT_CONTRACT_ADMINS`, and they must be the same address, or the account that can mint
deposit tokens is not the one holding gas to do it with.

`nat_exit_ip` must be the literal public IP, not `auto` and never `127.0.0.1`. It is the
single source of the address every node advertises: the package hands it to each launcher,
which is how it reaches the EL's `--nat.extip`, the CL's advertised address, and every
enode and ENR you publish. Do not add `--nat.extip` to `el_extra_params` — the launcher
already emits it, and a second copy is a value nothing checks against the first.

## 5. Launch

```
kurtosis run --enclave hegota-testnet ethereum-package --args-file /root/hegota-testnet.yaml
kurtosis enclave inspect hegota-testnet
```

Expect six client services (`el-1..3`, `cl-1..3`) plus `dora`, all `RUNNING`. The chain
reaches Amsterdam at genesis+192s and Hegotá at genesis+384s, so give it ~7 minutes before
judging it.

## 6. Verify

Run the twelve checks in `docs/hegota-devnet-genesis.md`. Do not skip them: most of the
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
PUBLIC_IP=<host public ip> ENCLAVE=hegota-testnet \
  OUT_DIR=/srv/hegota-testnet/artifacts \
  ./scripts/hegota-testnet/publish-artifacts.sh
```

Then check by hand, before anything is served:

```
ls /srv/hegota-testnet/artifacts        # must NOT contain mnemonics.yaml
```

The script publishes an explicit allowlist for this reason: the generator's
`/network-configs` also holds `mnemonics.yaml`, the phrase every genesis validator key
derives from, and `OUT_DIR` is public. Never replace the allowlist with a directory copy.

Fetch the bundle from another machine and start a node from it before announcing the
network. A consensus client consumes the directory as a whole, so a missing file fails at
startup rather than at first use.

## 8. Reverse proxy

Serve the RPC, the explorer, the faucet and the artifact bundle over HTTPS. Caddy gets
certificates automatically from the hostnames you declare:

```
rpc.hegota.example {
    reverse_proxy localhost:32003
}

explorer.hegota.example {
    reverse_proxy localhost:31500
}

faucet.hegota.example {
    reverse_proxy localhost:8080
}

artifacts.hegota.example {
    root * /srv/hegota-testnet/artifacts
    file_server browse
}
```

**Do not add `header Access-Control-*` directives.** ethrex's RPC server already sends a
complete permissive CORS set (`CorsLayer::permissive()`), Caddy's `header` *appends*
rather than replaces, and a duplicated `Access-Control-Allow-Origin` is hard-rejected by
browsers and by MetaMask's request layer. The Caddyfile at
`scripts/eip8141-devnet/Caddyfile` does add them; do not copy that part of it.

Only execution node 0's RPC (32003) is proxied. Nodes 1 and 2 keep their RPC closed so a
client cannot be pointed at a node the operator is not watching.

The explorer sits at a fixed 31500 because `port_publisher.additional_services` pins it.
The Hegotá devnet needed a socat unit to give the explorer a stable local port; this chain
does not, and you should not add one.

## 9. Faucet

Run the faucet with **its own key**, funded from the `FAUCET_ADDR` account, and keep that
key in a host env file — never in the repository, never in the kurtosis config. Point it
at `http://localhost:32003` and give it the public RPC and explorer URLs for its own
links. `scripts/hegota-devnet/faucet/` is a working reference implementation.

Fund it from the faucet account rather than a rich account, so a drained faucet cannot
touch the accounts you hold for testing.

## 10. Firewall

The full table is in `docs/hegota-testnet-joining.md`. The shape of it:

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
`docs/hegota-testnet-permissioning.md` has the full policy and kill switch.

## 12. Later changes

`docs/hegota-testnet-upgrading.md` covers upgrading ethrex and kurtosis, re-publishing the
bundle, rotating the gater admin, and what forces a re-genesis.

**Decide the upgrade path now, before launch.** Before anyone else has joined, the
cheapest correct upgrade is a re-genesis. Once joiners exist, a re-genesis is a relaunch
of a different network and the binary has to be replaced inside running containers
instead — and that only works if the container entrypoint was built for it. Retrofitting
it later means recreating containers, which is the thing it exists to avoid. Record which
path this host is built for.
