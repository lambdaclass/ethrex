#!/usr/bin/env bash
# Run an ethrex node on the Daisugi testnet: full sync from genesis over devp2p,
# driven by follow_head.py instead of a consensus client.
#
# usage: scripts/daisugi/run_node.sh DATA_DIR [extra ethrex flags...]
#
# Ports default away from the usual ones so the node can share a machine with
# other clients: p2p/discovery 30313, JSON-RPC 18545, engine API 18551.
set -euo pipefail

cd "$(dirname "$0")/../.."
data_dir="${1:?usage: run_node.sh DATA_DIR [extra flags]}"
shift
mkdir -p "$data_dir"
[ -f "$data_dir/jwt.hex" ] || openssl rand -hex 32 > "$data_dir/jwt.hex"

BOOTNODE="enode://f2edd1bcf88ade896c0bafc296582d5d357f2237db1a6e91ec7a8b452ca370e932b11967ca880aafddf18cc5fec78a271b4b65095b4a53537cef58c71f8ebea1@161.35.146.175:30303"

exec ./target/release/ethrex \
  --network fixtures/genesis/daisugi.json \
  --datadir "$data_dir/db" \
  --bootnodes "$BOOTNODE" \
  --syncmode full \
  --p2p.port "${P2P_PORT:-30313}" \
  --discovery.port "${P2P_PORT:-30313}" \
  --http.port "${HTTP_PORT:-18545}" \
  --authrpc.port "${AUTHRPC_PORT:-18551}" \
  --authrpc.jwtsecret "$data_dir/jwt.hex" \
  --mempool.max-verify-gas 500000 \
  "$@"
