#!/usr/bin/env bash
# Extract the joiner bundle from a running Hegotá testnet enclave.
#
# Writes every file a second client needs to follow this chain into a directory
# the reverse proxy serves, plus a MANIFEST.txt of sha256 sums. Read-only with
# respect to the enclave: nothing here restarts, reconfigures or writes to a
# running service.
#
#   PUBLIC_IP=203.0.113.10 ./publish-artifacts.sh
#   PUBLIC_IP=203.0.113.10 ENCLAVE=hegota-testnet OUT_DIR=/srv/hegota ./publish-artifacts.sh
#
# Requires: kurtosis, jq, curl, sha256sum.
set -euo pipefail

ENCLAVE="${ENCLAVE:-hegota-testnet}"
OUT_DIR="${OUT_DIR:-/srv/hegota-testnet/artifacts}"
GENESIS_ARTIFACT="el_cl_genesis_data"

# The address external peers must dial. Every enode and ENR is rewritten to it,
# because the values the clients report carry whatever address they resolved
# inside the enclave, which no outside peer can reach.
: "${PUBLIC_IP:?set PUBLIC_IP to the address external peers dial}"

# The single Hegotá activation timestamp, in the Nethermind chainspec's
# per-EIP form (see "patch the chainspec" below).
BOGOTA_KEY="bogotaTime"

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

STAGE="$WORK_DIR/stage"
mkdir -p "$STAGE"
cp -a "$SRC"/. "$STAGE"/

# ---------------------------------------------------------------------------
# 2. Bootnodes
#
# Enodes come from each execution node's admin_nodeInfo, ENRs from each beacon
# node's /eth/v1/node/identity. Both report an address resolved inside the
# enclave, so the host part is rewritten to PUBLIC_IP. The ports are NOT
# rewritten: they are the published ones and are already correct.
# ---------------------------------------------------------------------------

# Service names are assigned by the package as el-<n>-<el>-<cl> / cl-<n>-<cl>-<el>,
# so match on the prefix rather than hardcoding the client names.
mapfile -t EL_SERVICES < <(
  kurtosis enclave inspect "$ENCLAVE" -o json \
    | jq -r '.services | keys[] | select(startswith("el-"))' | sort
)
mapfile -t CL_SERVICES < <(
  kurtosis enclave inspect "$ENCLAVE" -o json \
    | jq -r '.services | keys[] | select(startswith("cl-"))' | sort
)

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

echo "==> collecting ${#EL_SERVICES[@]} enode(s)"
: > "$STAGE/bootnodes.txt"
for svc in "${EL_SERVICES[@]}"; do
  url="$(endpoint "$svc" rpc)"
  enode="$(
    curl -sf -X POST -H 'Content-Type: application/json' \
      --data '{"jsonrpc":"2.0","id":1,"method":"admin_nodeInfo","params":[]}' \
      "$url" | jq -r '.result.enode // empty'
  )"
  [[ -n "$enode" ]] || { echo "no enode from $svc (is the admin namespace enabled?)" >&2; exit 1; }
  echo "$enode" | rewrite_host >> "$STAGE/bootnodes.txt"
done

echo "==> collecting ${#CL_SERVICES[@]} ENR(s)"
: > "$STAGE/bootnodes-cl.txt"
for svc in "${CL_SERVICES[@]}"; do
  url="$(endpoint "$svc" http)"
  enr="$(curl -sf "$url/eth/v1/node/identity" | jq -r '.data.enr // empty')"
  [[ -n "$enr" ]] || { echo "no ENR from $svc" >&2; exit 1; }
  # An ENR is a signed, base64url-encoded record: its embedded IP cannot be
  # rewritten without invalidating the signature. It is published as reported,
  # and it is correct as long as the node was started with the right advertised
  # address — which is what `nat_exit_ip` and `--nat.extip` are for.
  echo "$enr" >> "$STAGE/bootnodes-cl.txt"
done

# ---------------------------------------------------------------------------
# 3. Patch the chainspec for the two EIPs the generator does not know about
#
# The Nethermind chainspec format activates per EIP. At generator v6.1.6
# (apps/el-gen/generate_genesis.sh) this fork emits only
# eip7805TransitionTimestamp and eip8141TransitionTimestamp, so EIP-8250 and
# EIP-8272 have no key at all. Both activate at the same timestamp as the rest.
#
# If the generator has caught up and already writes either key, this patch is
# stale and could contradict it, so refuse rather than overwrite.
# ---------------------------------------------------------------------------
CHAINSPEC="$STAGE/chainspec.json"
if [[ -f "$CHAINSPEC" ]]; then
  for key in eip8250TransitionTimestamp eip8272TransitionTimestamp; do
    if jq -e --arg k "$key" '.params | has($k)' "$CHAINSPEC" >/dev/null; then
      echo "chainspec.json already carries $key: the generator has caught up and this patch is stale" >&2
      echo "remove the chainspec patch from $0 and re-run" >&2
      exit 1
    fi
  done

  BOGOTA_TIME="$(jq -r --arg k "$BOGOTA_KEY" '.config[$k] // empty' "$STAGE/genesis.json")"
  [[ -n "$BOGOTA_TIME" ]] || { echo "genesis.json has no $BOGOTA_KEY" >&2; exit 1; }

  # Nethermind reads these as hex strings, matching the keys the generator
  # already emits for the same fork.
  HEX_TIME="$(printf '0x%x' "$BOGOTA_TIME")"
  jq --arg t "$HEX_TIME" \
     '.params.eip8250TransitionTimestamp = $t | .params.eip8272TransitionTimestamp = $t' \
     "$CHAINSPEC" > "$CHAINSPEC.patched"
  mv "$CHAINSPEC.patched" "$CHAINSPEC"
  echo "==> chainspec: added eip8250/eip8272 transition timestamps at $HEX_TIME"
else
  echo "==> chainspec.json absent from the artifact; skipping the per-EIP patch" >&2
fi

# ---------------------------------------------------------------------------
# 4. Manifest and publish
# ---------------------------------------------------------------------------
( cd "$STAGE" && find . -type f ! -name MANIFEST.txt -printf '%P\n' | sort \
    | xargs sha256sum > MANIFEST.txt )

mkdir -p "$OUT_DIR"
cp -a "$STAGE"/. "$OUT_DIR"/

echo "==> published $(wc -l < "$OUT_DIR/MANIFEST.txt") file(s) to $OUT_DIR"
echo
cat "$OUT_DIR/MANIFEST.txt"
echo
echo "Joining instructions: docs/hegota-testnet-joining.md"
echo "The five-EIP rule set and the consensus inputs that are NOT in genesis.json"
echo "(AA_VOPS_SLOT_COUNT, the VERIFY budgets, the per-IL code-byte budget and the"
echo "two-endpoint omission rule) are stated there and nowhere else."
