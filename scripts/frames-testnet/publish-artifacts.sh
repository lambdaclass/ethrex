#!/usr/bin/env bash
# Extract the joiner bundle from a running Hegotá testnet enclave.
#
# Writes every file a second client needs to follow this chain into a directory
# the reverse proxy serves, plus a MANIFEST.txt of sha256 sums. Read-only with
# respect to the enclave: nothing here restarts, reconfigures or writes to a
# running service.
#
#   PUBLIC_IP=203.0.113.10 ./publish-artifacts.sh
#   PUBLIC_IP=203.0.113.10 ENCLAVE=frames-testnet OUT_DIR=/srv/hegota ./publish-artifacts.sh
#
# Requires: kurtosis, jq, curl, sha256sum.
set -euo pipefail

ENCLAVE="${ENCLAVE:-frames-testnet}"
OUT_DIR="${OUT_DIR:-/srv/frames-testnet/artifacts}"
GENESIS_ARTIFACT="el_cl_genesis_data"

# The address external peers must dial. Every enode and ENR is rewritten to it,
# because the values the clients report carry whatever address they resolved
# inside the enclave, which no outside peer can reach.
: "${PUBLIC_IP:?set PUBLIC_IP to the address external peers dial}"

for bin in kurtosis jq curl sha256sum; do
  command -v "$bin" >/dev/null || { echo "missing required binary: $bin" >&2; exit 1; }
done


kurtosis enclave inspect "$ENCLAVE" >/dev/null 2>&1 \
  || { echo "enclave '$ENCLAVE' is not running" >&2; exit 1; }

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "==> enclave $ENCLAVE, publishing to $OUT_DIR as $PUBLIC_IP"

# ---------------------------------------------------------------------------
# 1. The generator's output
#
# The package stores /network-configs/ as one files artifact and mounts it into
# every client. Downloading the artifact gets the whole set in one step and
# without reaching into a container: genesis.json, chainspec.json, besu.json,
# config.yaml, genesis.ssz, deposit_contract_block_hash.txt,
# genesis_validators_root.txt and whatever else the generator wrote.
# ---------------------------------------------------------------------------
echo "==> downloading $GENESIS_ARTIFACT"
kurtosis files download "$ENCLAVE" "$GENESIS_ARTIFACT" "$WORK_DIR/network-configs" >/dev/null

# The download nests the artifact under its own directory name on some kurtosis
# versions and not on others. Normalise to a flat directory.
SRC="$WORK_DIR/network-configs"
if [[ ! -f "$SRC/genesis.json" && -f "$SRC/network-configs/genesis.json" ]]; then
  SRC="$SRC/network-configs"
fi
[[ -f "$SRC/genesis.json" ]] || { echo "genesis.json not found in the artifact" >&2; exit 1; }

# Publish an explicit allowlist, never the whole directory.
#
# `/network-configs` is the generator's working output, not a joiner bundle: it
# also contains `mnemonics.yaml`, which is the BIP-39 phrase every genesis
# validator key is derived from. OUT_DIR is served publicly, so copying the
# directory wholesale hands out the validator set's signing keys. Anything a
# joiner needs is named here; anything not named here does not leave the host.
#
# This list is the artifact table in scripts/frames-testnet/USER-GUIDE.md. Keep them
# in step: a file added here without a row there is undocumented, and a row
# there without an entry here is a broken download.
PUBLISHED_FILES=(
  genesis.json
  chainspec.json
  besu.json
  config.yaml
  genesis.ssz
  deposit_contract.txt
  deposit_contract_block.txt
  deposit_contract_block_hash.txt
  genesis_validators_root.txt
)

STAGE="$WORK_DIR/stage"
mkdir -p "$STAGE"
for f in "${PUBLISHED_FILES[@]}"; do
  [[ -f "$SRC/$f" ]] || { echo "generator output is missing $f" >&2; exit 1; }
  cp -a "$SRC/$f" "$STAGE/$f"
done

# Nethermind consumes chainspec.json, and the genesis generator's heze step writes
# both eip8141TransitionTimestamp AND eip7805TransitionTimestamp at the fork time
# (`genesis_add_heze`, "Enabled EIPs: 7805, 8141" -- unconditional through v6.1.7).
# This chain runs EIP-8141 alone; FOCIL is deliberately absent. A Nethermind fed the
# raw file schedules FOCIL at block 45, expects inclusion-list engine methods and
# FOCIL-shaped payloads from then on, and stalls at exactly that block. Publish a
# derived copy with the FOCIL line removed; genesis and accounts are untouched, so
# the genesis hash is identical. The original stays published byte-for-byte.
python3 - "$STAGE/chainspec.json" "$STAGE/chainspec-nethermind.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for k in [k for k in d["params"] if k.startswith("eip7805")]:
    del d["params"][k]
json.dump(d, open(sys.argv[2], "w"), indent=2)
PY

# Refuse to publish a secret even if the allowlist is edited carelessly later.
for forbidden in mnemonics.yaml validator_names.yaml; do
  if [[ -e "$STAGE/$forbidden" ]]; then
    echo "refusing to publish $forbidden: it carries validator key material" >&2
    exit 1
  fi
done

# ---------------------------------------------------------------------------
# 2. Bootnodes
#
# Enodes and execution-layer ENRs both come from each execution node's
# admin_nodeInfo; consensus ENRs from each beacon node's
# /eth/v1/node/identity. An enode reports the address the node resolved inside
# the enclave, which no outside peer can reach, so its host part is rewritten to
# PUBLIC_IP; the ports are NOT rewritten, being the published ones already. An
# ENR is signed over the address the node advertises and cannot be rewritten at
# all, so it is published as reported and is correct exactly when the node was
# started with the right advertised address — which is what `nat_exit_ip` and
# `--nat.extip` are for.
# ---------------------------------------------------------------------------

# Service names are assigned by the package as el-<n>-<el>-<cl> / cl-<n>-<cl>-<el>,
# so match on the prefix rather than hardcoding the client names.
#
# Read from the human-readable table: `kurtosis enclave inspect` has no
# machine-readable output mode in any release through 1.20.0, where the only
# flags are `--full-uuids` and `--help`. The name is the one field on its row
# that starts with the prefix, which survives the column widths shifting.
list_services() {
  kurtosis enclave inspect "$ENCLAVE" \
    | awk -v p="^$1[0-9]+-" '{for (i = 1; i <= NF; i++) if ($i ~ p) print $i}' \
    | sort -u
}

mapfile -t EL_SERVICES < <(list_services "el-")
mapfile -t CL_SERVICES < <(list_services "cl-")

[[ ${#EL_SERVICES[@]} -gt 0 ]] || { echo "no el-* services in $ENCLAVE" >&2; exit 1; }
[[ ${#CL_SERVICES[@]} -gt 0 ]] || { echo "no cl-* services in $ENCLAVE" >&2; exit 1; }

# `kurtosis port print` resolves a service's port id to a host-reachable URL,
# which is what lets this script run without knowing the published port layout.
endpoint() {
  kurtosis port print "$ENCLAVE" "$1" "$2"
}

# Replace the host in an enode/ENR-derived URL with PUBLIC_IP, leaving the port
# and the node id alone.
rewrite_host() {
  sed -E "s#@[^:]+:#@$PUBLIC_IP:#"
}

echo "==> collecting ${#EL_SERVICES[@]} enode(s) and execution ENR(s)"
: > "$STAGE/bootnodes.txt"
: > "$STAGE/bootnodes-enr.txt"
for svc in "${EL_SERVICES[@]}"; do
  url="$(endpoint "$svc" rpc)"
  info="$(
    curl -sf -X POST -H 'Content-Type: application/json' \
      --data '{"jsonrpc":"2.0","id":1,"method":"admin_nodeInfo","params":[]}' \
      "$url"
  )"
  enode="$(jq -r '.result.enode // empty' <<<"$info")"
  [[ -n "$enode" ]] || { echo "no enode from $svc (is the admin namespace enabled?)" >&2; exit 1; }
  echo "$enode" | rewrite_host >> "$STAGE/bootnodes.txt"

  # The ENR carries the discovery and RLPx ports and the fork id, so a client
  # speaking discv5 gets a complete record from it where an enode gives only an
  # address. ethrex reports an empty string when it could not build a record
  # (`crates/networking/rpc/admin/mod.rs`), which is a node worth naming but not
  # a reason to withhold a genesis bundle — the enode above still reaches it.
  enr="$(jq -r '.result.enr // empty' <<<"$info")"
  if [[ -n "$enr" ]]; then
    echo "$enr" >> "$STAGE/bootnodes-enr.txt"
  else
    echo "warning: $svc reports no ENR; publishing its enode only" >&2
  fi
done

[[ -s "$STAGE/bootnodes-enr.txt" ]] \
  || { echo "no execution ENR from any node: check the advertised address" >&2; exit 1; }

echo "==> collecting ${#CL_SERVICES[@]} consensus ENR(s)"
: > "$STAGE/bootnodes-cl.txt"
for svc in "${CL_SERVICES[@]}"; do
  url="$(endpoint "$svc" http)"
  enr="$(curl -sf "$url/eth/v1/node/identity" | jq -r '.data.enr // empty')"
  [[ -n "$enr" ]] || { echo "no ENR from $svc" >&2; exit 1; }
  echo "$enr" >> "$STAGE/bootnodes-cl.txt"
done

# ---------------------------------------------------------------------------
# 3. Manifest and publish
#
# Everything the generator wrote is published as it was written. Only
# genesis.json is a supported artifact: it is the geth-style execution genesis
# ethrex reads, and with the fork timestamp and the EIP-8141 rule set in
# scripts/frames-testnet/USER-GUIDE.md it is the complete definition of this chain.
# The generator's other execution-genesis formats ship because it emits them,
# not because this chain vouches for them.
# ---------------------------------------------------------------------------
( cd "$STAGE" && find . -type f ! -name MANIFEST.txt -printf '%P\n' | sort \
    | xargs sha256sum > MANIFEST.txt )

mkdir -p "$OUT_DIR"
cp -a "$STAGE"/. "$OUT_DIR"/

# The bundle is read by two other users than the one that publishes it: the
# reverse proxy serves it, and the faucet mounts it to serve the bootnode lists.
# `cp -a` preserves whatever modes the generator's artifact happened to carry, so
# the readable bit is set here rather than left to that. Every file is public by
# construction — the allowlist above is what keeps a secret out.
chmod 0755 "$OUT_DIR"
find "$OUT_DIR" -maxdepth 1 -type f -exec chmod 0644 {} +

echo "==> published $(wc -l < "$OUT_DIR/MANIFEST.txt") file(s) to $OUT_DIR"
echo
cat "$OUT_DIR/MANIFEST.txt"
echo
echo "Joining instructions: scripts/frames-testnet/USER-GUIDE.md"
echo "This chain activates EIP-8141 alone. The consensus inputs that are NOT in"
echo "genesis.json -- the frame-transaction VERIFY budgets -- are stated there"
echo "and nowhere else."
